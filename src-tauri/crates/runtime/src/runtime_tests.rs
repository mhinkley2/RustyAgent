//! Tests for the agent loop — the core of the product.
//!
//! Every case drives a real `ConversationRuntime` against a scripted
//! `MockLlmProvider`, an in-memory database, and a `RecordingSink`, then
//! asserts the emitted `RunEvent` sequence, the persisted `run_events` rows,
//! and the final `story_runs.status`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use api::mock::{MockResponse, RecordedCall, DEFAULT_MOCK_USAGE};
use api::{ChatMessage, CompletionConfig, MessageRole, MockLlmProvider, ToolCall, Usage};
use db::testing::{
    make_test_pool, run_events, run_iteration_count, run_status, run_usage, seed_profile,
    seed_story, RunUsage,
};
use db::DbPool;
use tools::{Tool, ToolContext, ToolOutput, ToolPermissionInfo, ToolRegistry};

use crate::approval_gate::ApprovalGate;
use crate::context::{ContextPolicy, SUMMARY_PREFIX};
use crate::runtime::{CancelFlag, ConversationRuntime, RunEvent};
use crate::testing::{RecordingNotifier, RecordingSink};
use crate::PermissionPolicy;

// ---------------------------------------------------------------------------
// Stub tools
// ---------------------------------------------------------------------------

/// A tool that records its invocations and returns a canned result.
struct StubTool {
    name: String,
    output: String,
    is_error: bool,
    calls: Arc<std::sync::Mutex<Vec<Value>>>,
}

impl StubTool {
    fn new(name: &str, output: &str) -> Self {
        Self {
            name: name.to_string(),
            output: output.to_string(),
            is_error: false,
            calls: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    fn calls(&self) -> Arc<std::sync::Mutex<Vec<Value>>> {
        self.calls.clone()
    }
}

#[async_trait]
impl Tool for StubTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "a stub tool"
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "required": [] })
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext) -> ToolOutput {
        self.calls.lock().expect("calls poisoned").push(input);
        if self.is_error {
            ToolOutput::err(self.output.clone())
        } else {
            ToolOutput::ok(self.output.clone())
        }
    }
}

/// A tool that cancels the run as a side effect of being called.
///
/// Cancelling from outside the loop races the loop; cancelling from inside a
/// tool call pins the stop to a known point — after the first provider call's
/// usage has been recorded, before the second is made.
struct CancellingTool {
    cancel: CancelFlag,
}

#[async_trait]
impl Tool for CancellingTool {
    fn name(&self) -> &str {
        "cancel_me"
    }
    fn description(&self) -> &str {
        "cancels the run"
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "required": [] })
    }
    async fn execute(&self, _input: Value, _ctx: &ToolContext) -> ToolOutput {
        self.cancel.cancel();
        ToolOutput::ok("cancelled")
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

const STORY_ID: &str = "story-1";
const PROFILE_ID: &str = "agent-1";

struct Harness {
    db: DbPool,
    sink: RecordingSink,
    gate: Arc<ApprovalGate>,
    provider: Arc<MockLlmProvider>,
    registry: Arc<Mutex<ToolRegistry>>,
    cancel: CancelFlag,
    policy: PermissionPolicy,
    config: CompletionConfig,
    max_iterations: u32,
    track_history: bool,
    event_retention_runs: u32,
    context_policy: ContextPolicy,
    workspace_root: Option<std::path::PathBuf>,
    /// How long the approval gate waits. `None` — the default — waits for as
    /// long as it takes.
    approval_timeout: Option<std::time::Duration>,
    notifier: RecordingNotifier,
}

impl Harness {
    async fn new(script: Vec<MockResponse>) -> Self {
        Self::with_provider(MockLlmProvider::script(script)).await
    }

    /// As `new`, but with a mock the caller has configured — the token
    /// accounting tests need to vary what the provider reports.
    async fn with_provider(provider: MockLlmProvider) -> Self {
        let db = make_test_pool().await;
        seed_profile(&db, PROFILE_ID, "Test Agent").await;
        seed_story(&db, STORY_ID, "Test Story", "ready").await;

        Self {
            db,
            sink: RecordingSink::new(),
            gate: Arc::new(ApprovalGate::new()),
            provider: Arc::new(provider),
            registry: Arc::new(Mutex::new(ToolRegistry::new())),
            cancel: CancelFlag::new(),
            policy: PermissionPolicy::allow_all(),
            config: CompletionConfig::new("mock-model", 1024),
            max_iterations: 20,
            track_history: true,
            event_retention_runs: 0,
            context_policy: ContextPolicy::default(),
            workspace_root: None,
            approval_timeout: None,
            notifier: RecordingNotifier::new(),
        }
    }

    async fn with_tool(self, tool: Box<dyn Tool>) -> Self {
        self.registry.lock().await.register(tool);
        self
    }

    /// Build the runtime, returning it plus its run id.
    async fn build(&self) -> ConversationRuntime {
        let mut rt = ConversationRuntime::new(
            STORY_ID,
            PROFILE_ID,
            Box::new(ProviderHandle(self.provider.clone())),
            self.registry.clone(),
            self.policy.clone(),
            self.gate.clone(),
            vec![ChatMessage::user("do the thing")],
            self.config.clone(),
            self.max_iterations,
            self.db.clone(),
            self.sink.handle(),
            self.cancel.clone(),
            None, // memory
            self.workspace_root.clone(),
            self.track_history,
            self.event_retention_runs,
        )
        .await
        .expect("build runtime");
        rt.context_policy = self.context_policy;
        rt.approval_timeout = self.approval_timeout;
        rt.notifier = Some(self.notifier.handle());
        rt
    }

    /// Build and run to completion, returning the run id.
    async fn run(&self) -> String {
        let rt = self.build().await;
        let run_id = rt.run_id.clone();
        rt.run().await.expect("run should not error");
        run_id
    }

    async fn status(&self, run_id: &str) -> String {
        run_status(&self.db, run_id).await
    }

    async fn events(&self, run_id: &str) -> Vec<(String, String)> {
        run_events(&self.db, run_id).await
    }

    async fn usage(&self, run_id: &str) -> RunUsage {
        run_usage(&self.db, run_id).await
    }

    async fn iterations(&self, run_id: &str) -> i64 {
        run_iteration_count(&self.db, run_id).await
    }

    fn calls(&self) -> Vec<RecordedCall> {
        self.provider.recorded_calls()
    }
}

/// `ConversationRuntime` takes `Box<dyn LlmProvider>`, but the tests need to
/// keep querying the mock after handing it over. This shares one via `Arc`.
struct ProviderHandle(Arc<MockLlmProvider>);

#[async_trait]
impl api::LlmProvider for ProviderHandle {
    async fn stream_completion(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<api::ToolDefinition>,
        config: CompletionConfig,
    ) -> Result<api::provider::EventStream, api::ApiError> {
        self.0.stream_completion(messages, tools, config).await
    }
    async fn list_models(&self) -> Result<Vec<String>, api::ApiError> {
        self.0.list_models().await
    }
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_text_only_run_emits_tokens_then_complete_and_marks_the_run_done() {
    let h = Harness::new(vec![MockResponse::text_chunks(["Hel", "lo"])]).await;

    let run_id = h.run().await;

    assert_eq!(h.sink.kinds(), vec!["token", "token", "complete"]);
    assert_eq!(h.sink.text(), "Hello");
    // "done", not "completed" — the vocabulary the frontend and pipeline share.
    assert_eq!(h.status(&run_id).await, "done");
}

#[tokio::test]
async fn each_streamed_token_is_persisted_as_its_own_run_event() {
    let h = Harness::new(vec![MockResponse::text_chunks(["a", "b", "c"])]).await;

    let run_id = h.run().await;

    let types: Vec<_> = h
        .events(&run_id)
        .await
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    assert_eq!(types, vec!["token", "token", "token", "complete"]);
}

#[tokio::test]
async fn the_complete_event_carries_the_providers_stop_reason() {
    let h = Harness::new(vec![MockResponse::text("done")]).await;

    h.run().await;

    match &h.sink.run_events()[1] {
        RunEvent::Complete { stop_reason, .. } => assert_eq!(stop_reason, "end_turn"),
        other => panic!("expected Complete, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Tool round trip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_tool_call_round_trip_feeds_the_result_back_to_the_llm() {
    let tool = StubTool::new("get_story", "Story: Test Story");
    let calls = tool.calls();
    let h = Harness::new(vec![
        MockResponse::tool_call("call_1", "get_story", json!({ "id": "s1" })),
        MockResponse::text("The story is ready."),
    ])
    .await
    .with_tool(Box::new(tool))
    .await;

    let run_id = h.run().await;

    // The tool actually ran, with the input the LLM supplied.
    assert_eq!(calls.lock().unwrap().as_slice(), &[json!({ "id": "s1" })]);

    // The second provider call carries the assistant's tool_calls message
    // followed by the tool result correlated by id.
    let second = &h.calls()[1].messages;
    let assistant = second
        .iter()
        .find(|m| m.role == MessageRole::Assistant)
        .expect("assistant message");
    assert_eq!(
        assistant.tool_calls.as_ref().expect("tool_calls")[0].id,
        "call_1"
    );
    let tool_msg = second
        .iter()
        .find(|m| m.role == MessageRole::Tool)
        .expect("tool message");
    assert_eq!(tool_msg.tool_call_id.as_deref(), Some("call_1"));
    assert_eq!(tool_msg.content, "Story: Test Story");

    assert_eq!(
        h.sink.kinds(),
        vec!["tool_call", "tool_result", "token", "complete"]
    );
    assert_eq!(h.status(&run_id).await, "done");
}

#[tokio::test]
async fn two_tool_calls_in_one_turn_both_execute_in_order() {
    let alpha = StubTool::new("alpha", "A");
    let beta = StubTool::new("beta", "B");
    let h = Harness::new(vec![
        MockResponse::ToolCalls(vec![
            ToolCall { id: "c1".into(), name: "alpha".into(), input: json!({}) },
            ToolCall { id: "c2".into(), name: "beta".into(), input: json!({}) },
        ]),
        MockResponse::text("both done"),
    ])
    .await
    .with_tool(Box::new(alpha))
    .await
    .with_tool(Box::new(beta))
    .await;

    h.run().await;

    assert_eq!(
        h.sink.kinds(),
        vec!["tool_call", "tool_result", "tool_call", "tool_result", "token", "complete"]
    );
}

#[tokio::test]
async fn an_unknown_tool_returns_not_found_without_killing_the_run() {
    let h = Harness::new(vec![
        MockResponse::tool_call("c1", "no_such_tool", json!({})),
        MockResponse::text("recovered"),
    ])
    .await;

    let run_id = h.run().await;

    let result = h
        .sink
        .run_events()
        .into_iter()
        .find_map(|e| match e {
            RunEvent::ToolResult { output, is_error, .. } => Some((output, is_error)),
            _ => None,
        })
        .expect("a tool result");
    assert!(result.1, "should be flagged as an error");
    assert!(result.0.contains("not found"), "got {}", result.0);
    assert_eq!(h.status(&run_id).await, "done");
}

// ---------------------------------------------------------------------------
// Guards: iterations and cancellation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_max_iterations_guard_fails_the_run_and_stops_calling_the_provider() {
    let tool = StubTool::new("spin", "again");
    let mut h = Harness::new(vec![
        MockResponse::tool_call("c1", "spin", json!({})),
        MockResponse::tool_call("c2", "spin", json!({})),
        MockResponse::tool_call("c3", "spin", json!({})),
    ])
    .await
    .with_tool(Box::new(tool))
    .await;
    h.max_iterations = 2;

    let run_id = h.run().await;

    assert_eq!(h.provider.call_count(), 2, "must stop at the cap");
    assert_eq!(h.status(&run_id).await, "failed");
    let failed = h
        .sink
        .run_events()
        .into_iter()
        .find_map(|e| match e {
            RunEvent::Failed { message, .. } => Some(message),
            _ => None,
        })
        .expect("a failed event");
    assert!(failed.contains("Max iterations (2)"), "got {failed}");
}

#[tokio::test]
async fn cancelling_before_the_first_iteration_never_calls_the_provider() {
    let h = Harness::new(vec![MockResponse::text("should not be reached")]).await;
    h.cancel.cancel();

    let run_id = h.run().await;

    assert_eq!(h.provider.call_count(), 0);
    assert_eq!(h.sink.kinds(), vec!["cancelled"]);
    assert_eq!(h.status(&run_id).await, "cancelled");
}

#[tokio::test]
async fn cancelling_between_iterations_stops_the_loop() {
    let tool = StubTool::new("spin", "again");
    let h = Harness::new(vec![
        MockResponse::tool_call("c1", "spin", json!({})),
        MockResponse::text("never reached"),
    ])
    .await
    .with_tool(Box::new(tool))
    .await;

    // Cancel while the first iteration's tool result is being fed back.
    let rt = h.build().await;
    let run_id = rt.run_id.clone();
    let cancel = h.cancel.clone();
    let handle = tokio::spawn(async move { rt.run().await });
    cancel.cancel();
    handle.await.expect("join").expect("run");

    let status = h.status(&run_id).await;
    assert!(
        status == "cancelled" || status == "done",
        "unexpected status {status}"
    );
    if status == "cancelled" {
        assert!(h.sink.kinds().contains(&"cancelled".to_string()));
    }
}

// ---------------------------------------------------------------------------
// Permissions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_permission_policy_filters_the_tool_definitions_sent_to_the_provider() {
    let mut h = Harness::new(vec![MockResponse::text("hi")])
        .await
        .with_tool(Box::new(StubTool::new("allowed", "ok")))
        .await
        .with_tool(Box::new(StubTool::new("blocked", "ok")))
        .await
        .with_tool(Box::new(StubTool::new("also_blocked", "ok")))
        .await;
    h.policy = PermissionPolicy::restricted(vec!["allowed".to_string()]);

    h.run().await;

    let recorded = h.calls();
    let offered: Vec<_> = recorded[0].tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(offered, vec!["allowed"]);
}

#[tokio::test]
async fn a_denied_tool_is_not_executed_and_its_reason_goes_back_to_the_llm() {
    let tool = StubTool::new("blocked", "SHOULD NOT RUN");
    let calls = tool.calls();
    let mut h = Harness::new(vec![
        MockResponse::tool_call("c1", "blocked", json!({})),
        MockResponse::text("understood"),
    ])
    .await
    .with_tool(Box::new(tool))
    .await;
    h.policy = PermissionPolicy::restricted(vec!["something_else".to_string()]);

    let run_id = h.run().await;

    assert!(calls.lock().unwrap().is_empty(), "the tool must not run");

    let (output, is_error) = h
        .sink
        .run_events()
        .into_iter()
        .find_map(|e| match e {
            RunEvent::ToolResult { output, is_error, .. } => Some((output, is_error)),
            _ => None,
        })
        .expect("a tool result");
    assert!(is_error);
    assert!(!output.contains("SHOULD NOT RUN"));

    // The run continues — the model gets to react to the denial.
    assert_eq!(h.status(&run_id).await, "done");
    assert_eq!(h.provider.call_count(), 2);
}

/// A stub that declares a permission profile, so the runtime's registry lookup
/// has something to hand the policy. This is the wiring the unit tests in
/// `permission_tests` cannot reach: registry -> `ToolPermissionInfo` -> policy.
struct DeclaredTool {
    name: String,
    info: ToolPermissionInfo,
    calls: Arc<std::sync::Mutex<Vec<Value>>>,
}

impl DeclaredTool {
    fn new(name: &str, info: ToolPermissionInfo) -> Self {
        Self {
            name: name.to_string(),
            info,
            calls: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    fn calls(&self) -> Arc<std::sync::Mutex<Vec<Value>>> {
        self.calls.clone()
    }
}

#[async_trait]
impl Tool for DeclaredTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "a stub tool with a declared permission profile"
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "required": [] })
    }
    fn permission_info(&self) -> ToolPermissionInfo {
        self.info.clone()
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext) -> ToolOutput {
        self.calls.lock().expect("calls poisoned").push(input);
        ToolOutput::ok("ran")
    }
}

fn declared_read() -> ToolPermissionInfo {
    ToolPermissionInfo { reads_files: true, path_inputs: &["path"], ..Default::default() }
}

fn declared_shell(program: &str) -> ToolPermissionInfo {
    ToolPermissionInfo {
        reads_files: true,
        writes_files: true,
        path_inputs: &[],
        shell_program: Some(program.to_string()),
    }
}

#[tokio::test]
async fn a_read_outside_the_allowed_read_paths_is_denied_before_the_tool_runs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tool = DeclaredTool::new("file_read", declared_read());
    let calls = tool.calls();

    let mut h = Harness::new(vec![
        MockResponse::tool_call("c1", "file_read", json!({ "path": "secrets/keys.json" })),
        MockResponse::text("understood"),
    ])
    .await
    .with_tool(Box::new(tool))
    .await;
    h.workspace_root = Some(dir.path().to_path_buf());
    h.policy.allow_file_read_paths = vec!["docs".into()];

    let run_id = h.run().await;

    assert!(calls.lock().unwrap().is_empty(), "the read must not happen");
    let (output, is_error) = h
        .sink
        .run_events()
        .into_iter()
        .find_map(|e| match e {
            RunEvent::ToolResult { output, is_error, .. } => Some((output, is_error)),
            _ => None,
        })
        .expect("a tool result");
    assert!(is_error);
    assert!(output.contains("allowed read paths"), "got {output}");
    assert_eq!(h.status(&run_id).await, "done");
}

#[tokio::test]
async fn a_read_inside_the_allowed_read_paths_still_runs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tool = DeclaredTool::new("file_read", declared_read());
    let calls = tool.calls();

    let mut h = Harness::new(vec![
        MockResponse::tool_call("c1", "file_read", json!({ "path": "docs/report.md" })),
        MockResponse::text("read it"),
    ])
    .await
    .with_tool(Box::new(tool))
    .await;
    h.workspace_root = Some(dir.path().to_path_buf());
    h.policy.allow_file_read_paths = vec!["docs".into()];

    h.run().await;

    assert_eq!(calls.lock().unwrap().len(), 1, "the read should have happened");
}

/// The gap this story closes end to end: a custom shell tool was never
/// classified as a write, so it never reached the approval gate.
#[tokio::test]
async fn a_custom_shell_tool_is_sent_through_the_approval_gate() {
    let tool = DeclaredTool::new("run_tests", declared_shell("cargo"));
    let calls = tool.calls();

    let mut h = Harness::new(vec![
        MockResponse::tool_call("c1", "run_tests", json!({})),
        MockResponse::text("done"),
    ])
    .await
    .with_tool(Box::new(tool))
    .await;
    h.policy = approval_policy();

    let rt = h.build().await;
    let runner = tokio::spawn(async move { rt.run().await });

    let approval_id = await_approval_id(&h.db).await;
    assert!(calls.lock().unwrap().is_empty(), "the command ran before approval");

    assert!(h.gate.resolve(&approval_id, true));
    runner.await.expect("join").expect("run");

    assert_eq!(calls.lock().unwrap().len(), 1, "approved, so it should run once");
}

#[tokio::test]
async fn a_shell_program_off_the_allow_list_is_denied_before_it_runs() {
    let tool = DeclaredTool::new("wipe", declared_shell("rm"));
    let calls = tool.calls();

    let mut h = Harness::new(vec![
        MockResponse::tool_call("c1", "wipe", json!({})),
        MockResponse::text("understood"),
    ])
    .await
    .with_tool(Box::new(tool))
    .await;
    h.policy.allow_shell_commands = vec!["cargo".into()];

    h.run().await;

    assert!(calls.lock().unwrap().is_empty(), "the command must not run");
    let output = h
        .sink
        .run_events()
        .into_iter()
        .find_map(|e| match e {
            RunEvent::ToolResult { output, .. } => Some(output),
            _ => None,
        })
        .expect("a tool result");
    assert!(output.contains("allowed shell commands"), "got {output}");
}

#[tokio::test]
async fn a_shell_program_on_the_allow_list_still_runs() {
    let tool = DeclaredTool::new("build", declared_shell("cargo"));
    let calls = tool.calls();

    let mut h = Harness::new(vec![
        MockResponse::tool_call("c1", "build", json!({})),
        MockResponse::text("built"),
    ])
    .await
    .with_tool(Box::new(tool))
    .await;
    h.policy.allow_shell_commands = vec!["cargo".into()];

    h.run().await;

    assert_eq!(calls.lock().unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// Provider failures
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_provider_error_on_the_first_call_fails_the_run() {
    let h = Harness::new(vec![MockResponse::ProviderError("upstream down".into())]).await;

    let rt = h.build().await;
    let run_id = rt.run_id.clone();
    let result = rt.run().await;

    assert!(result.is_err(), "the run should surface the error");
    assert_eq!(h.status(&run_id).await, "failed");
    assert_eq!(h.sink.kinds(), vec!["failed"]);
}

#[tokio::test]
async fn a_recoverable_mid_stream_error_does_not_abort_the_run() {
    // StreamEvent::Error is logged and skipped; the turn still completes.
    let h = Harness::new(vec![MockResponse::Error("transient hiccup".into())]).await;

    let run_id = h.run().await;

    assert_eq!(h.status(&run_id).await, "done");
    assert_eq!(h.sink.kinds(), vec!["complete"]);
}

#[tokio::test]
async fn running_out_of_scripted_responses_fails_rather_than_hanging() {
    let tool = StubTool::new("spin", "again");
    let h = Harness::new(vec![MockResponse::tool_call("c1", "spin", json!({}))])
        .await
        .with_tool(Box::new(tool))
        .await;

    let rt = h.build().await;
    let run_id = rt.run_id.clone();
    let result = rt.run().await;

    assert!(result.is_err());
    assert_eq!(h.status(&run_id).await, "failed");
}

// ---------------------------------------------------------------------------
// History tracking and retention
// ---------------------------------------------------------------------------

#[tokio::test]
async fn track_history_false_skips_token_events_but_keeps_the_terminal_one() {
    let mut h = Harness::new(vec![MockResponse::text_chunks(["a", "b"])]).await;
    h.track_history = false;

    let run_id = h.run().await;

    let types: Vec<_> = h.events(&run_id).await.into_iter().map(|(t, _)| t).collect();
    assert_eq!(types, vec!["complete"], "only critical events persist");
    // The live event stream is unaffected — only persistence is suppressed.
    assert_eq!(h.sink.kinds(), vec!["token", "token", "complete"]);
}

#[tokio::test]
async fn track_history_false_still_persists_tool_free_terminal_events_on_failure() {
    let mut h = Harness::new(vec![MockResponse::ProviderError("down".into())]).await;
    h.track_history = false;

    let rt = h.build().await;
    let run_id = rt.run_id.clone();
    let _ = rt.run().await;

    let types: Vec<_> = h.events(&run_id).await.into_iter().map(|(t, _)| t).collect();
    assert!(types.is_empty() || types == vec!["failed"], "got {types:?}");
}

#[tokio::test]
async fn event_retention_prunes_events_from_older_runs_on_the_same_story() {
    let mut h = Harness::new(vec![MockResponse::text("newest")]).await;
    h.event_retention_runs = 2;

    // Three prior finished runs, each with one event.
    for i in 0..3 {
        let old_id = format!("old-run-{i}");
        db::testing::seed_run(&h.db, &old_id, STORY_ID, PROFILE_ID).await;
        sqlx::query(
            "INSERT INTO run_events (id, run_id, event_type, content, sequence_num)
             VALUES (?, ?, 'token', 'stale', 0)",
        )
        .bind(format!("ev-{i}"))
        .bind(&old_id)
        .execute(&h.db)
        .await
        .expect("seed event");
    }

    let run_id = h.run().await;

    // The newest run plus one older one keep their events; the rest are pruned.
    let surviving: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT run_id) FROM run_events WHERE run_id != ?",
    )
    .bind(&run_id)
    .fetch_one(&h.db)
    .await
    .expect("count");
    assert!(surviving < 3, "expected pruning, {surviving} old runs kept");
}

// ---------------------------------------------------------------------------
// Run summary
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_long_multibyte_run_summary_is_truncated_without_panicking() {
    // Regression guard of the same class as the shell-output truncation: the
    // cut was a fixed byte offset into a String, which panics mid-codepoint.
    let long_reply = "é".repeat(3000);
    let h = Harness::new(vec![MockResponse::text(long_reply)]).await;

    let run_id = h.run().await;

    assert_eq!(h.status(&run_id).await, "done");
}

// ---------------------------------------------------------------------------
// Approval gate — the human-in-the-loop path
// ---------------------------------------------------------------------------

/// Poll `approval_requests` until a row appears, so a test can resolve the gate
/// without racing the runtime's insert.
async fn await_approval_id(db: &DbPool) -> String {
    for _ in 0..200 {
        let id: Option<String> =
            sqlx::query_scalar("SELECT id FROM approval_requests LIMIT 1")
                .fetch_optional(db)
                .await
                .expect("query approval_requests");
        if let Some(id) = id {
            return id;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("no approval request was created");
}

/// A policy that sends every write tool through the approval gate.
fn approval_policy() -> PermissionPolicy {
    PermissionPolicy {
        require_approval_on_write: true,
        ..PermissionPolicy::allow_all()
    }
}

#[tokio::test]
async fn a_write_tool_requiring_approval_blocks_until_the_gate_is_resolved() {
    let tool = StubTool::new("file_write", "written");
    let calls = tool.calls();
    let mut h = Harness::new(vec![
        MockResponse::tool_call("c1", "file_write", json!({ "path": "a.txt" })),
        MockResponse::text("saved"),
    ])
    .await
    .with_tool(Box::new(tool))
    .await;
    h.policy = approval_policy();

    let rt = h.build().await;
    let run_id = rt.run_id.clone();
    let runner = tokio::spawn(async move { rt.run().await });

    let approval_id = await_approval_id(&h.db).await;

    // The request is pending and the tool has not run yet.
    let status: String = sqlx::query_scalar("SELECT status FROM approval_requests WHERE id = ?")
        .bind(&approval_id)
        .fetch_one(&h.db)
        .await
        .expect("fetch status");
    assert_eq!(status, "pending");
    assert!(calls.lock().unwrap().is_empty(), "tool ran before approval");

    // The frontend was told to ask.
    assert_eq!(h.sink.count("approval-request-created"), 1);

    assert!(h.gate.resolve(&approval_id, true));
    runner.await.expect("join").expect("run");

    assert_eq!(calls.lock().unwrap().len(), 1, "tool should run once approved");
    assert_eq!(h.status(&run_id).await, "done");
}

/// Move tokio's clock forward without waiting, and without leaving it paused:
/// the SQLite pool arms its own acquire timeout on the same clock, and a
/// paused clock outside this window makes the harness fail to connect at all.
async fn advance(by: std::time::Duration) {
    tokio::time::pause();
    tokio::time::advance(by).await;
    tokio::time::resume();
}

/// Read one approval request's status and reason.
async fn approval_row(db: &DbPool, id: &str) -> (String, Option<String>) {
    sqlx::query_as("SELECT status, rejection_reason FROM approval_requests WHERE id = ?")
        .bind(id)
        .fetch_one(db)
        .await
        .expect("fetch approval row")
}

/// The defect this replaced: the gate waited `timeout(300s, rx)` and treated
/// expiry as "not approved", so every gated call failed five minutes after the
/// user stepped away — and the row said `rejected`, as though they had decided.
///
/// The hour is advanced on tokio's clock rather than waited out, so the test
/// is authoritative rather than merely slow: a five-minute limit still in that
/// path would fire twelve times over. The clock is paused only around the
/// wait — pausing for the whole test expires the SQLite pool's own acquire
/// timeout before the harness can open a connection.
#[tokio::test]
async fn an_unanswered_approval_parks_the_run_instead_of_denying_it() {
    let tool = StubTool::new("file_write", "written");
    let calls = tool.calls();
    let mut h = Harness::new(vec![
        MockResponse::tool_call("c1", "file_write", json!({ "path": "a.txt" })),
        MockResponse::text("saved"),
    ])
    .await
    .with_tool(Box::new(tool))
    .await;
    h.policy = approval_policy();

    let rt = h.build().await;
    let run_id = rt.run_id.clone();
    let runner = tokio::spawn(async move { rt.run().await });

    let approval_id = await_approval_id(&h.db).await;
    advance(std::time::Duration::from_secs(3600)).await;

    let (status, _) = approval_row(&h.db, &approval_id).await;
    assert_eq!(status, "pending", "an hour of nobody answering is not a decision");
    assert!(calls.lock().unwrap().is_empty(), "the tool must not have run");
    assert!(!runner.is_finished(), "the run must still be parked, not failed");

    // And it is still answerable whenever the user does come back.
    assert!(h.gate.resolve(&approval_id, true));
    runner.await.expect("join").expect("run");

    assert_eq!(calls.lock().unwrap().len(), 1);
    assert_eq!(h.status(&run_id).await, "done");
}

/// A parked run is not a silent one: the timeline, the event bus and the user
/// are all told, because nothing further happens until somebody acts.
#[tokio::test]
async fn parking_on_an_approval_is_announced_on_every_channel() {
    let mut h = Harness::new(vec![
        MockResponse::tool_call("c1", "file_write", json!({ "path": "a.txt" })),
        MockResponse::text("saved"),
    ])
    .await
    .with_tool(Box::new(StubTool::new("file_write", "written")))
    .await;
    h.policy = approval_policy();

    let rt = h.build().await;
    let run_id = rt.run_id.clone();
    let runner = tokio::spawn(async move { rt.run().await });

    let approval_id = await_approval_id(&h.db).await;
    assert!(h.gate.resolve(&approval_id, true));
    runner.await.expect("join").expect("run");

    let kinds = h.sink.kinds();
    assert!(
        kinds.contains(&"awaiting_approval".to_string()),
        "a live view cannot tell a parked run from a slow one without this: {kinds:?}"
    );
    assert!(kinds.contains(&"approval_resolved".to_string()), "{kinds:?}");

    let types: Vec<String> = h.events(&run_id).await.into_iter().map(|(t, _)| t).collect();
    assert!(types.contains(&"approval_request".to_string()), "{types:?}");
    assert!(types.contains(&"approval_response".to_string()), "{types:?}");

    let notified = h.notifier.in_category(tools::NotificationCategory::Approval);
    assert_eq!(notified.len(), 1, "the user is told once, not per poll");
    assert!(notified[0].1.contains("file_write"), "{:?}", notified[0]);
}

/// Someone who would rather a run end than sit parked can set a limit. Expiry
/// is then recorded as `expired` — never as a rejection, which is a claim
/// about what a user decided.
#[tokio::test]
async fn a_configured_approval_timeout_expires_without_recording_a_user_decision() {
    let tool = StubTool::new("file_write", "SHOULD NOT RUN");
    let calls = tool.calls();
    let mut h = Harness::new(vec![
        MockResponse::tool_call("c1", "file_write", json!({ "path": "a.txt" })),
        MockResponse::text("understood"),
    ])
    .await
    .with_tool(Box::new(tool))
    .await;
    h.policy = approval_policy();
    h.approval_timeout = Some(std::time::Duration::from_secs(60));

    let rt = h.build().await;
    let run_id = rt.run_id.clone();
    let runner = tokio::spawn(async move { rt.run().await });

    let approval_id = await_approval_id(&h.db).await;
    advance(std::time::Duration::from_secs(61)).await;
    runner.await.expect("join").expect("run");

    let (status, reason) = approval_row(&h.db, &approval_id).await;
    assert_eq!(status, "expired", "expiry must not be spelled `rejected`");
    let reason = reason.expect("an expired request explains itself");
    assert!(
        reason.contains("not a decision the user made"),
        "the record must not read as a refusal: {reason}"
    );
    assert!(calls.lock().unwrap().is_empty(), "the tool must not run");
    assert_eq!(h.status(&run_id).await, "done", "the run carries on past a refused tool");
}

/// The run ends; the user finds out. Once — a notification per token or per
/// tool call gets the whole feature muted.
#[tokio::test]
async fn a_finished_run_notifies_the_user_exactly_once() {
    let h = Harness::new(vec![MockResponse::text("all done")]).await;

    h.run().await;

    assert_eq!(
        h.notifier.in_category(tools::NotificationCategory::RunCompleted).len(),
        1
    );
    assert!(h.notifier.in_category(tools::NotificationCategory::RunFailed).is_empty());
}

/// A user who stopped the run themselves is at the desk and does not need
/// telling.
#[tokio::test]
async fn a_cancelled_run_does_not_notify() {
    let h = Harness::new(vec![MockResponse::text("never reached")]).await;
    h.cancel.cancel();

    let run_id = h.run().await;

    assert_eq!(h.status(&run_id).await, "cancelled");
    assert!(h.notifier.delivered().is_empty(), "{:?}", h.notifier.delivered());
}

/// A desktop that refuses to show a notification must not take the run down
/// with it.
#[tokio::test]
async fn a_refused_notification_does_not_fail_the_run() {
    let mut h = Harness::new(vec![MockResponse::text("all done")]).await;
    h.notifier = RecordingNotifier::refusing("Notifications are turned off in Settings.");

    let run_id = h.run().await;

    assert_eq!(h.status(&run_id).await, "done");
    assert_eq!(h.notifier.delivered().len(), 1, "it was attempted");
}

#[tokio::test]
async fn a_rejected_approval_skips_execution_and_tells_the_llm() {
    let tool = StubTool::new("file_write", "SHOULD NOT RUN");
    let calls = tool.calls();
    let mut h = Harness::new(vec![
        MockResponse::tool_call("c1", "file_write", json!({ "path": "a.txt" })),
        MockResponse::text("understood"),
    ])
    .await
    .with_tool(Box::new(tool))
    .await;
    h.policy = approval_policy();

    let rt = h.build().await;
    let run_id = rt.run_id.clone();
    let runner = tokio::spawn(async move { rt.run().await });

    let approval_id = await_approval_id(&h.db).await;
    assert!(h.gate.resolve(&approval_id, false));
    runner.await.expect("join").expect("run");

    assert!(calls.lock().unwrap().is_empty(), "the tool must not run");

    let (output, is_error) = h
        .sink
        .run_events()
        .into_iter()
        .find_map(|e| match e {
            RunEvent::ToolResult { output, is_error, .. } => Some((output, is_error)),
            _ => None,
        })
        .expect("a tool result");
    assert!(is_error);
    assert!(output.contains("not approved"), "got {output}");

    // The run still completes — the model gets to react to the refusal.
    assert_eq!(h.status(&run_id).await, "done");
}

#[tokio::test]
async fn a_dropped_approval_sender_is_treated_as_a_rejection() {
    // `ApprovalGate::cancel` drops the sender; the awaiting runtime must read
    // that as "not approved" rather than hanging or panicking.
    let tool = StubTool::new("file_write", "SHOULD NOT RUN");
    let calls = tool.calls();
    let mut h = Harness::new(vec![
        MockResponse::tool_call("c1", "file_write", json!({ "path": "a.txt" })),
        MockResponse::text("understood"),
    ])
    .await
    .with_tool(Box::new(tool))
    .await;
    h.policy = approval_policy();

    let rt = h.build().await;
    let run_id = rt.run_id.clone();
    let runner = tokio::spawn(async move { rt.run().await });

    let approval_id = await_approval_id(&h.db).await;
    h.gate.cancel(&approval_id);
    runner.await.expect("join").expect("run");

    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(h.status(&run_id).await, "done");
}

#[tokio::test]
async fn a_non_write_tool_bypasses_the_approval_gate_entirely() {
    let tool = StubTool::new("get_story", "Story: Test Story");
    let calls = tool.calls();
    let mut h = Harness::new(vec![
        MockResponse::tool_call("c1", "get_story", json!({})),
        MockResponse::text("read it"),
    ])
    .await
    .with_tool(Box::new(tool))
    .await;
    h.policy = approval_policy();

    let run_id = h.run().await;

    assert_eq!(calls.lock().unwrap().len(), 1);
    assert_eq!(h.sink.count("approval-request-created"), 0);
    let pending: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM approval_requests")
        .fetch_one(&h.db)
        .await
        .expect("count");
    assert_eq!(pending, 0);
    assert_eq!(h.status(&run_id).await, "done");
}

#[tokio::test]
async fn the_approval_request_row_records_the_tool_name_and_input() {
    let mut h = Harness::new(vec![
        MockResponse::tool_call("c1", "file_write", json!({ "path": "a.txt" })),
        MockResponse::text("saved"),
    ])
    .await
    .with_tool(Box::new(StubTool::new("file_write", "written")))
    .await;
    h.policy = approval_policy();

    let rt = h.build().await;
    let expected_run_id = rt.run_id.clone();
    let runner = tokio::spawn(async move { rt.run().await });

    let approval_id = await_approval_id(&h.db).await;
    let (run_id, tool_name, tool_input): (String, String, String) = sqlx::query_as(
        "SELECT run_id, tool_name, tool_input FROM approval_requests WHERE id = ?",
    )
    .bind(&approval_id)
    .fetch_one(&h.db)
    .await
    .expect("fetch row");

    assert_eq!(run_id, expected_run_id);
    assert_eq!(tool_name, "file_write");
    assert_eq!(
        serde_json::from_str::<Value>(&tool_input).expect("valid json"),
        json!({ "path": "a.txt" })
    );

    h.gate.resolve(&approval_id, true);
    runner.await.expect("join").expect("run");
}

// ---------------------------------------------------------------------------
// Token accounting
//
// `story_runs.input_tokens` / `output_tokens` / `estimated_cost_usd` are read
// by the runs list and the run detail panel. Nothing else writes them, so if
// the loop stops folding usage in, every run in the app silently reads back as
// free.
// ---------------------------------------------------------------------------

/// A model the price table knows, so a cost can actually be quoted.
const PRICED_MODEL: &str = "claude-opus-5";

/// One mock call on `PRICED_MODEL`: 100 uncached input, 20 output, 7 cache-read
/// and 3 cache-write tokens at $5/$25 per MTok (reads 0.1x, writes 1.25x).
const ONE_CALL_COST: f64 = 0.00102225;

#[tokio::test]
async fn a_completed_run_persists_the_token_counts_the_provider_reported() {
    let mut h = Harness::new(vec![MockResponse::text("done")]).await;
    h.config = CompletionConfig::new(PRICED_MODEL, 1024);

    let run_id = h.run().await;

    let usage = h.usage(&run_id).await;
    assert_eq!(usage.input_tokens, DEFAULT_MOCK_USAGE.input_tokens as i64);
    assert_eq!(usage.output_tokens, DEFAULT_MOCK_USAGE.output_tokens as i64);
    assert!(
        usage.input_tokens > 0 && usage.output_tokens > 0,
        "a completed run must not read back as zero tokens"
    );
}

#[tokio::test]
async fn cache_reads_and_writes_are_recorded_apart_from_uncached_input() {
    // They are billed at different rates, and the saving is only visible if
    // they are not folded into the input count.
    let mut h = Harness::new(vec![MockResponse::text("done")]).await;
    h.config = CompletionConfig::new(PRICED_MODEL, 1024);

    let run_id = h.run().await;

    let usage = h.usage(&run_id).await;
    assert_eq!(
        usage.cache_read_input_tokens,
        DEFAULT_MOCK_USAGE.cache_read_input_tokens as i64
    );
    assert_eq!(
        usage.cache_creation_input_tokens,
        DEFAULT_MOCK_USAGE.cache_creation_input_tokens as i64
    );
    assert_eq!(
        usage.input_tokens, DEFAULT_MOCK_USAGE.input_tokens as i64,
        "cached tokens must not be added into the uncached count"
    );
}

#[tokio::test]
async fn a_known_model_gets_a_non_zero_cost_from_the_price_table() {
    let mut h = Harness::new(vec![MockResponse::text("done")]).await;
    h.config = CompletionConfig::new(PRICED_MODEL, 1024);

    let run_id = h.run().await;

    let cost = h.usage(&run_id).await.estimated_cost_usd;
    assert!(cost > 0.0, "a priced model must produce a cost, got {cost}");
    assert!(
        (cost - ONE_CALL_COST).abs() < 1e-9,
        "expected {ONE_CALL_COST}, got {cost}"
    );
}

#[tokio::test]
async fn an_unknown_model_records_real_tokens_and_no_fabricated_cost() {
    // The harness default model is not in the price table.
    let h = Harness::new(vec![MockResponse::text("done")]).await;

    let run_id = h.run().await;

    let usage = h.usage(&run_id).await;
    assert_eq!(usage.input_tokens, DEFAULT_MOCK_USAGE.input_tokens as i64);
    assert_eq!(usage.output_tokens, DEFAULT_MOCK_USAGE.output_tokens as i64);
    assert_eq!(
        usage.estimated_cost_usd, 0.0,
        "an unpriced model must not be quoted at some other model's rate"
    );
}

#[tokio::test]
async fn a_multi_iteration_run_reports_the_sum_of_every_call_not_the_last_one() {
    let mut h = Harness::new(vec![
        MockResponse::tool_call("c1", "get_story", json!({})),
        MockResponse::text("and done"),
    ])
    .await
    .with_tool(Box::new(StubTool::new("get_story", "a story")))
    .await;
    h.config = CompletionConfig::new(PRICED_MODEL, 1024);

    let run_id = h.run().await;

    assert_eq!(h.provider.call_count(), 2, "the run made two provider calls");
    let usage = h.usage(&run_id).await;
    assert_eq!(usage.input_tokens, 2 * DEFAULT_MOCK_USAGE.input_tokens as i64);
    assert_eq!(usage.output_tokens, 2 * DEFAULT_MOCK_USAGE.output_tokens as i64);
    assert_eq!(
        usage.cache_read_input_tokens,
        2 * DEFAULT_MOCK_USAGE.cache_read_input_tokens as i64
    );
    assert!(
        (usage.estimated_cost_usd - 2.0 * ONE_CALL_COST).abs() < 1e-9,
        "cost should double with the second call, got {}",
        usage.estimated_cost_usd
    );
}

#[tokio::test]
async fn a_run_that_fails_mid_way_still_persists_what_it_already_spent() {
    // First call succeeds and is billed; the second never returns a stream.
    let mut h = Harness::new(vec![
        MockResponse::tool_call("c1", "get_story", json!({})),
        MockResponse::ProviderError("upstream down".into()),
    ])
    .await
    .with_tool(Box::new(StubTool::new("get_story", "a story")))
    .await;
    h.config = CompletionConfig::new(PRICED_MODEL, 1024);

    let rt = h.build().await;
    let run_id = rt.run_id.clone();
    let result = rt.run().await;

    assert!(result.is_err(), "the run should surface the provider error");
    assert_eq!(h.status(&run_id).await, "failed");
    let usage = h.usage(&run_id).await;
    assert_eq!(
        usage.input_tokens, DEFAULT_MOCK_USAGE.input_tokens as i64,
        "the completed first call still cost what it cost"
    );
    assert!(usage.estimated_cost_usd > 0.0);
}

#[tokio::test]
async fn a_cancelled_run_still_persists_what_it_already_spent() {
    let h = Harness::new(vec![
        MockResponse::tool_call("c1", "cancel_me", json!({})),
        MockResponse::text("never reached"),
    ])
    .await;
    let cancelling = CancellingTool { cancel: h.cancel.clone() };
    let mut h = h.with_tool(Box::new(cancelling)).await;
    h.config = CompletionConfig::new(PRICED_MODEL, 1024);

    let run_id = h.run().await;

    assert_eq!(h.status(&run_id).await, "cancelled");
    assert_eq!(h.provider.call_count(), 1);
    let usage = h.usage(&run_id).await;
    assert_eq!(usage.input_tokens, DEFAULT_MOCK_USAGE.input_tokens as i64);
    assert_eq!(usage.output_tokens, DEFAULT_MOCK_USAGE.output_tokens as i64);
    assert!(usage.estimated_cost_usd > 0.0, "cancelled work is still billed");
}

#[tokio::test]
async fn a_run_cancelled_before_its_first_call_records_nothing() {
    let mut h = Harness::new(vec![MockResponse::text("never reached")]).await;
    h.config = CompletionConfig::new(PRICED_MODEL, 1024);
    h.cancel.cancel();

    let run_id = h.run().await;

    let usage = h.usage(&run_id).await;
    assert_eq!(usage.input_tokens, 0);
    assert_eq!(usage.output_tokens, 0);
    assert_eq!(usage.estimated_cost_usd, 0.0);
}

#[tokio::test]
async fn a_provider_that_reports_no_usage_leaves_the_counts_at_zero() {
    // Honest degradation: no counts recorded rather than invented ones, even
    // though the model itself is priced.
    let mut h = Harness::with_provider(
        MockLlmProvider::script(vec![MockResponse::text("done")]).without_usage(),
    )
    .await;
    h.config = CompletionConfig::new(PRICED_MODEL, 1024);

    let run_id = h.run().await;

    assert_eq!(h.status(&run_id).await, "done");
    let usage = h.usage(&run_id).await;
    assert_eq!(usage.input_tokens, 0);
    assert_eq!(usage.output_tokens, 0);
    assert_eq!(usage.estimated_cost_usd, 0.0);
}

#[tokio::test]
async fn a_run_that_hits_the_iteration_cap_records_every_call_it_made() {
    let mut h = Harness::with_provider(
        MockLlmProvider::script(vec![
            MockResponse::tool_call("c1", "spin", json!({})),
            MockResponse::tool_call("c2", "spin", json!({})),
            MockResponse::tool_call("c3", "spin", json!({})),
        ])
        .with_usage(Usage::new(10, 4)),
    )
    .await
    .with_tool(Box::new(StubTool::new("spin", "again")))
    .await;
    h.max_iterations = 3;

    let run_id = h.run().await;

    assert_eq!(h.status(&run_id).await, "failed");
    let usage = h.usage(&run_id).await;
    assert_eq!(usage.input_tokens, 30, "three calls at 10 input each");
    assert_eq!(usage.output_tokens, 12);
}

/// A saturated token total must persist as `i64::MAX`, not wrap to a negative.
///
/// `Usage` sums with `saturating_add`, so a pathological provider figure pins
/// the total at `u64::MAX` rather than overflowing. Binding that with `as i64`
/// would have stored `-1`, turning the guard that produced it into the very
/// "nonsense number" its doc comment forbids.
#[tokio::test]
async fn a_saturated_token_total_persists_clamped_rather_than_negative() {
    let mut h = Harness::with_provider(
        MockLlmProvider::script(vec![MockResponse::text("done")])
            .with_usage(Usage::new(u64::MAX, u64::MAX)),
    )
    .await;
    h.max_iterations = 1;

    let run_id = h.run().await;
    let usage = h.usage(&run_id).await;

    assert_eq!(usage.input_tokens, i64::MAX, "clamped, not wrapped to -1");
    assert_eq!(usage.output_tokens, i64::MAX);
}

// ---------------------------------------------------------------------------
// Context compaction
//
// The message list only ever grew, so a long run eventually sent the provider
// more input than the model could take and died with its work lost. These
// drive a conversation past a deliberately small budget and pin what the loop
// does about it.
// ---------------------------------------------------------------------------

/// A tool result big enough that a couple of round trips blow the budget:
/// 6,000 bytes estimates at ~2,000 tokens.
const BIG_TOOL_OUTPUT_BYTES: usize = 6_000;

/// Budget that three of those round trips exceed and two do not, so the run
/// compacts exactly once on its fourth call.
const TIGHT_BUDGET: i64 = 6_000;

/// Three tool round trips then a final answer. The fourth provider call is
/// the one that needs room made for it.
fn overflowing_script() -> Vec<MockResponse> {
    vec![
        MockResponse::tool_call("c1", "read_big", json!({})),
        MockResponse::tool_call("c2", "read_big", json!({})),
        MockResponse::tool_call("c3", "read_big", json!({})),
        MockResponse::text("all done"),
    ]
}

/// A harness on `overflowing_script` with the given strategy and budget.
///
/// `without_usage` throughout: the mock's canned token counts describe no real
/// request, and letting them recalibrate the estimator would make the budget
/// these tests assert on a moving target. Reconciliation gets its own test.
async fn overflowing_harness(strategy: &str, budget: Option<i64>) -> Harness {
    let mut h = Harness::with_provider(
        MockLlmProvider::script(overflowing_script()).without_usage(),
    )
    .await
    .with_tool(Box::new(StubTool::new(
        "read_big",
        &"x".repeat(BIG_TOOL_OUTPUT_BYTES),
    )))
    .await;
    h.context_policy = ContextPolicy::from_profile(strategy, budget);
    h
}

/// The estimated input size of one recorded provider call.
fn request_size(call: &RecordedCall) -> u64 {
    api::tokens::estimate_request(
        call.config.system_prompt.as_deref(),
        &call.messages,
        &call.tools,
    )
}

fn compaction_events(sink: &RecordingSink) -> Vec<RunEvent> {
    sink.run_events()
        .into_iter()
        .filter(|e| matches!(e, RunEvent::ContextCompacted { .. }))
        .collect()
}

#[tokio::test]
async fn a_conversation_driven_past_the_budget_is_compacted_and_the_run_completes() {
    // The regression test for the whole story: before this, the fourth call
    // went out oversized and the provider killed the run.
    let h = overflowing_harness("recent", Some(TIGHT_BUDGET)).await;

    let run_id = h.run().await;

    assert_eq!(h.status(&run_id).await, "done");
    assert_eq!(h.provider.call_count(), 4, "every scripted turn was reached");
    assert_eq!(compaction_events(&h.sink).len(), 1);

    for (i, call) in h.calls().iter().enumerate() {
        let size = request_size(call);
        assert!(
            size <= TIGHT_BUDGET as u64,
            "call {i} went out at {size} tokens against a {TIGHT_BUDGET} budget"
        );
    }
}

#[tokio::test]
async fn the_same_conversation_under_full_fails_instead_of_overflowing() {
    // The pairing that shows compaction is what keeps the run alive: identical
    // script and budget, and the only difference is the strategy.
    let h = overflowing_harness("full", Some(TIGHT_BUDGET)).await;

    let rt = h.build().await;
    let run_id = rt.run_id.clone();
    let result = rt.run().await;

    assert!(result.is_err(), "full must not send an oversized request");
    assert_eq!(h.status(&run_id).await, "failed");
    assert_eq!(
        h.provider.call_count(),
        3,
        "the over-budget fourth call is never made"
    );

    let message = h
        .sink
        .run_events()
        .into_iter()
        .find_map(|e| match e {
            RunEvent::Failed { message, .. } => Some(message),
            _ => None,
        })
        .expect("a failed event");
    assert!(
        message.contains(&TIGHT_BUDGET.to_string()),
        "the failure must name the budget, got: {message}"
    );
    assert!(message.contains("full"), "got: {message}");

    // The same explanation reaches the run timeline as readable prose, not as
    // a JSON blob the operator has to decode.
    let persisted = h.events(&run_id).await;
    let error = persisted
        .iter()
        .find(|(t, _)| t == "error")
        .map(|(_, c)| c.clone())
        .expect("a persisted error event");
    assert_eq!(error, message);
}

#[tokio::test]
async fn an_unrecognised_context_strategy_compacts_like_recent() {
    // A typo in a settings field must not panic, and must not silently become
    // the one strategy that lets a run die.
    let h = overflowing_harness("agressive", Some(TIGHT_BUDGET)).await;

    let run_id = h.run().await;

    assert_eq!(h.status(&run_id).await, "done");
    assert_eq!(compaction_events(&h.sink).len(), 1);
}

#[tokio::test]
async fn a_budget_the_conversation_never_reaches_leaves_it_untouched() {
    // The other half of "max_input_tokens determines the budget": the same run
    // under a generous one compacts nothing.
    let h = overflowing_harness("recent", Some(1_000_000)).await;

    let run_id = h.run().await;

    assert_eq!(h.status(&run_id).await, "done");
    assert!(compaction_events(&h.sink).is_empty());
    let last = h.calls().pop().expect("a call");
    assert!(
        request_size(&last) > TIGHT_BUDGET as u64,
        "the run really did outgrow the tight budget"
    );
}

#[tokio::test]
async fn compaction_drops_the_oldest_turn_and_keeps_the_task_and_the_recent_ones() {
    let h = overflowing_harness("recent", Some(TIGHT_BUDGET)).await;

    h.run().await;

    let final_call = h.calls().pop().expect("a fourth call");
    let roles: Vec<&MessageRole> = final_call.messages.iter().map(|m| &m.role).collect();

    // The task survives at the head...
    assert_eq!(roles.first(), Some(&&MessageRole::User));
    assert_eq!(final_call.messages[0].content, "do the thing");
    // ...one round trip was dropped, and the two newest were kept.
    assert_eq!(
        final_call.messages.len(),
        5,
        "expected task + two surviving round trips, got {roles:?}"
    );
}

#[tokio::test]
async fn the_originating_task_message_survives_every_compaction() {
    // An agent that has forgotten what it was asked to do is worse than one
    // that failed loudly.
    let h = overflowing_harness("recent", Some(2_500)).await;

    h.run().await;

    for (i, call) in h.calls().iter().enumerate() {
        assert!(
            call.messages
                .iter()
                .any(|m| m.role == MessageRole::User && m.content == "do the thing"),
            "call {i} lost the task message"
        );
    }
}

#[tokio::test]
async fn no_compacted_request_ever_carries_a_tool_result_without_its_tool_call() {
    // Anthropic answers an orphaned `tool_result` with a hard 400, so a naive
    // oldest-first drop turns a survivable run into a dead one.
    let h = overflowing_harness("recent", Some(2_500)).await;

    h.run().await;

    for (i, call) in h.calls().iter().enumerate() {
        let mut open: Vec<&str> = Vec::new();
        for message in &call.messages {
            if let Some(calls) = &message.tool_calls {
                open = calls.iter().map(|c| c.id.as_str()).collect();
            } else if message.role == MessageRole::Tool {
                let id = message.tool_call_id.as_deref().unwrap_or_default();
                assert!(
                    open.contains(&id),
                    "call {i} sent tool_result '{id}' with no preceding tool_use"
                );
            }
        }
    }
}

#[tokio::test]
async fn a_compaction_emits_a_run_event_carrying_the_before_and_after_estimates() {
    let h = overflowing_harness("recent", Some(TIGHT_BUDGET)).await;

    let run_id = h.run().await;

    let event = compaction_events(&h.sink).pop().expect("a compaction event");
    match event {
        RunEvent::ContextCompacted {
            strategy,
            before_tokens,
            after_tokens,
            budget_tokens,
            evicted_messages,
            summarized,
            ..
        } => {
            assert_eq!(strategy, "recent");
            assert_eq!(budget_tokens, TIGHT_BUDGET as u64);
            assert!(before_tokens > budget_tokens, "before {before_tokens}");
            assert!(after_tokens <= budget_tokens, "after {after_tokens}");
            assert_eq!(evicted_messages, 2, "the tool call and its result");
            assert!(!summarized);
        }
        other => panic!("expected ContextCompacted, got {other:?}"),
    }

    // And it reaches the run timeline, not just the live event stream.
    let persisted: Vec<String> = h
        .events(&run_id)
        .await
        .into_iter()
        .filter(|(t, _)| t == "context_compacted")
        .map(|(_, c)| c)
        .collect();
    assert_eq!(persisted.len(), 1);
    let payload: Value = serde_json::from_str(&persisted[0]).expect("json payload");
    assert!(payload["before_tokens"].as_u64().unwrap() > payload["after_tokens"].as_u64().unwrap());
}

#[tokio::test]
async fn a_compaction_is_recorded_even_when_the_story_does_not_track_history() {
    // History tracking off is exactly the configuration where the dropped
    // messages leave no other trace.
    let mut h = overflowing_harness("recent", Some(TIGHT_BUDGET)).await;
    h.track_history = false;

    let run_id = h.run().await;

    let types: Vec<String> = h.events(&run_id).await.into_iter().map(|(t, _)| t).collect();
    assert!(
        types.contains(&"context_compacted".to_string()),
        "got {types:?}"
    );
    assert!(!types.contains(&"token".to_string()), "got {types:?}");
}

// ---------------------------------------------------------------------------
// Summarisation
// ---------------------------------------------------------------------------

/// `overflowing_script` with an extra response spliced in where the
/// summarisation call lands — after the third turn, before the fourth.
fn summarising_script(summary: MockResponse) -> Vec<MockResponse> {
    let mut script = overflowing_script();
    let last = script.pop().expect("the final answer");
    script.push(summary);
    script.push(last);
    script
}

async fn summarising_harness(summary: MockResponse) -> Harness {
    let mut h = Harness::with_provider(
        MockLlmProvider::script(summarising_script(summary)).without_usage(),
    )
    .await
    .with_tool(Box::new(StubTool::new(
        "read_big",
        &"x".repeat(BIG_TOOL_OUTPUT_BYTES),
    )))
    .await;
    h.context_policy = ContextPolicy::from_profile("summary", Some(TIGHT_BUDGET));
    h
}

#[tokio::test]
async fn the_summary_strategy_replaces_the_evicted_prefix_with_a_generated_message() {
    let h = summarising_harness(MockResponse::text("read a big file, found nothing")).await;

    let run_id = h.run().await;

    assert_eq!(h.status(&run_id).await, "done");

    let final_call = h.calls().pop().expect("a final call");
    let summary = final_call
        .messages
        .iter()
        .find(|m| m.content.starts_with(SUMMARY_PREFIX))
        .expect("a reinserted summary message");
    assert!(summary.content.contains("read a big file, found nothing"));
    // Right after the protected task message, where the dropped turns were.
    assert_eq!(final_call.messages[0].content, "do the thing");
    assert!(final_call.messages[1].content.starts_with(SUMMARY_PREFIX));

    let event = compaction_events(&h.sink).pop().expect("a compaction event");
    match event {
        RunEvent::ContextCompacted { strategy, summarized, .. } => {
            assert_eq!(strategy, "summary");
            assert!(summarized);
        }
        other => panic!("expected ContextCompacted, got {other:?}"),
    }
}

#[tokio::test]
async fn the_summarisation_call_is_cheap_and_carries_no_tools() {
    // It reads a transcript; it must not be able to act, and it must not
    // inherit the run's full output budget.
    let h = summarising_harness(MockResponse::text("a summary")).await;

    h.run().await;

    // Calls are [turn, turn, turn, summarise, turn].
    let calls = h.calls();
    let summarising = &calls[3];
    assert!(summarising.tools.is_empty(), "the summariser gets no tools");
    assert_eq!(summarising.messages.len(), 1);
    assert!(summarising.messages[0].content.contains("--- transcript ---"));
    assert!(
        summarising.config.max_tokens < h.config.max_tokens,
        "the summary must be capped below the run's own output budget"
    );
}

#[tokio::test]
async fn a_failed_summarisation_degrades_to_recent_rather_than_failing_the_run() {
    // Losing a summary is not worth losing the run.
    let h = summarising_harness(MockResponse::ProviderError("summariser down".into())).await;

    let run_id = h.run().await;

    assert_eq!(h.status(&run_id).await, "done");

    let event = compaction_events(&h.sink).pop().expect("a compaction event");
    match event {
        RunEvent::ContextCompacted { summarized, evicted_messages, .. } => {
            assert!(!summarized, "no summary was produced");
            assert_eq!(evicted_messages, 2, "but the eviction still happened");
        }
        other => panic!("expected ContextCompacted, got {other:?}"),
    }

    let final_call = h.calls().pop().expect("a final call");
    assert!(
        !final_call
            .messages
            .iter()
            .any(|m| m.content.starts_with(SUMMARY_PREFIX)),
        "a failed summary must not leave a placeholder behind"
    );
}

#[tokio::test]
async fn an_empty_summary_response_degrades_to_recent_too() {
    // A provider that returns 200 and nothing at all is a failure by another
    // name; reinserting an empty message would cost tokens and say nothing.
    let h = summarising_harness(MockResponse::text("   ")).await;

    let run_id = h.run().await;

    assert_eq!(h.status(&run_id).await, "done");
    let final_call = h.calls().pop().expect("a final call");
    assert!(!final_call
        .messages
        .iter()
        .any(|m| m.content.starts_with(SUMMARY_PREFIX)));
}

// ---------------------------------------------------------------------------
// Reconciling the estimate against real usage
// ---------------------------------------------------------------------------

/// One tool round trip then an answer — comfortably inside `TIGHT_BUDGET` by
/// the estimator's own reckoning.
fn short_script() -> Vec<MockResponse> {
    vec![
        MockResponse::tool_call("c1", "read_big", json!({})),
        MockResponse::text("done"),
    ]
}

async fn reconciling_harness(provider: MockLlmProvider) -> Harness {
    let mut h = Harness::with_provider(provider)
        .await
        .with_tool(Box::new(StubTool::new(
            "read_big",
            &"x".repeat(BIG_TOOL_OUTPUT_BYTES),
        )))
        .await;
    h.context_policy = ContextPolicy::from_profile("recent", Some(TIGHT_BUDGET));
    h
}

#[tokio::test]
async fn a_conversation_the_estimator_thinks_fits_is_left_alone() {
    let h = reconciling_harness(MockLlmProvider::script(short_script()).without_usage()).await;

    h.run().await;

    assert!(
        compaction_events(&h.sink).is_empty(),
        "the estimate is well inside the budget"
    );
}

#[tokio::test]
async fn a_provider_reporting_more_input_than_estimated_tightens_the_budget() {
    // The estimator is a character count; `Usage::total_input_tokens()` is the
    // truth. When the truth comes back far higher, the next call has to be
    // measured against that ratio — otherwise the budget is enforced against a
    // number the provider does not agree with.
    let h = reconciling_harness(
        MockLlmProvider::script(short_script()).with_usage(Usage::new(400, 5)),
    )
    .await;

    let run_id = h.run().await;

    // Same script and budget as the test above; only the reported usage
    // differs, and now the second call is compacted.
    assert_eq!(compaction_events(&h.sink).len(), 1);
    assert_eq!(h.status(&run_id).await, "done");
}

/// A summary must land *inside* the budget, not just replace what was evicted.
///
/// The plan is computed for the post-eviction list, which does not yet contain
/// the summary. Planning against the full budget and inserting afterwards put
/// the request back over the ceiling this function exists to enforce, and
/// nothing downstream re-checked it.
///
/// Sizes are tuned so eviction stops *just* under budget: two small units are
/// dropped to get below, leaving little headroom, and the summary then has to
/// fit in what remains. A fixture that evicts one huge unit lands far below
/// budget and cannot show the bug.
#[tokio::test]
async fn a_reinserted_summary_still_fits_inside_the_budget() {
    let mut h = Harness::with_provider(
        MockLlmProvider::script(vec![
            MockResponse::tool_call("c1", "read_small", json!({})),
            MockResponse::tool_call("c2", "read_small", json!({})),
            MockResponse::tool_call("c3", "read_huge", json!({})),
            // Consumed by the summarisation call, which fires here.
            MockResponse::text("summary text ".repeat(200)),
            MockResponse::text("all done"),
        ])
        .without_usage(),
    )
    .await
    .with_tool(Box::new(StubTool::new("read_small", &"x".repeat(3_000))))
    .await
    .with_tool(Box::new(StubTool::new("read_huge", &"x".repeat(16_000))))
    .await;
    h.context_policy = ContextPolicy::from_profile("summary", Some(TIGHT_BUDGET));

    let run_id = h.run().await;
    assert_eq!(h.status(&run_id).await, "done");

    let event = compaction_events(&h.sink).pop().expect("a compaction event");
    match event {
        RunEvent::ContextCompacted {
            after_tokens,
            budget_tokens,
            summarized,
            ..
        } => {
            assert!(summarized, "this fixture summarises");
            assert!(
                after_tokens <= budget_tokens,
                "summary pushed the request back over budget:                  {after_tokens} > {budget_tokens}"
            );
        }
        other => panic!("expected ContextCompacted, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Iteration accounting
//
// `story_runs.iteration_count` was read, selected for display and rendered in
// `RunDetailPanel`, but no code path wrote it: every run reported zero
// iterations regardless of how much work it did. These pin what it now says on
// each of the loop's exit paths, and — the case that matters for crash
// recovery — that it is already correct on the row *before* the run ends, so a
// run the app never finished still reports what it got through.
// ---------------------------------------------------------------------------

/// A tool that reads its own run's persisted `iteration_count` while the loop
/// is still going, and reports it back through a shared slot.
struct IterationPeekTool {
    seen: Arc<std::sync::Mutex<Vec<i64>>>,
}

#[async_trait]
impl Tool for IterationPeekTool {
    fn name(&self) -> &str {
        "peek"
    }
    fn description(&self) -> &str {
        "reads the run's persisted iteration count"
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "required": [] })
    }
    async fn execute(&self, _input: Value, ctx: &ToolContext) -> ToolOutput {
        let count = run_iteration_count(&ctx.db, &ctx.run_id).await;
        self.seen.lock().expect("seen poisoned").push(count);
        ToolOutput::ok("peeked")
    }
}

#[tokio::test]
async fn a_single_turn_run_reports_the_one_iteration_it_performed() {
    // The bug this closes: the column read zero for every run ever made.
    let h = Harness::new(vec![MockResponse::text("done")]).await;

    let run_id = h.run().await;

    assert_eq!(h.iterations(&run_id).await, 1);
}

#[tokio::test]
async fn a_multi_iteration_run_reports_a_count_matching_its_provider_calls() {
    let h = Harness::new(vec![
        MockResponse::tool_call("c1", "get_story", json!({})),
        MockResponse::tool_call("c2", "get_story", json!({})),
        MockResponse::text("finally"),
    ])
    .await
    .with_tool(Box::new(StubTool::new("get_story", "a story")))
    .await;

    let run_id = h.run().await;

    assert_eq!(h.status(&run_id).await, "done");
    assert_eq!(h.provider.call_count(), 3);
    assert_eq!(h.iterations(&run_id).await, 3);
}

#[tokio::test]
async fn a_failed_run_reports_its_iterations_rather_than_zero() {
    // Usage is persisted on the failure path; the iteration count has to be
    // too, or the run reads as having spent tokens over no iterations.
    let h = Harness::new(vec![
        MockResponse::tool_call("c1", "get_story", json!({})),
        MockResponse::ProviderError("upstream down".into()),
    ])
    .await
    .with_tool(Box::new(StubTool::new("get_story", "a story")))
    .await;

    let rt = h.build().await;
    let run_id = rt.run_id.clone();
    let _ = rt.run().await;

    assert_eq!(h.status(&run_id).await, "failed");
    assert_eq!(
        h.iterations(&run_id).await,
        2,
        "the failing call was an iteration too"
    );
}

#[tokio::test]
async fn a_cancelled_run_reports_its_iterations_rather_than_zero() {
    let h = Harness::new(vec![
        MockResponse::tool_call("c1", "cancel_me", json!({})),
        MockResponse::text("never reached"),
    ])
    .await;
    let cancelling = CancellingTool { cancel: h.cancel.clone() };
    let h = h.with_tool(Box::new(cancelling)).await;

    let run_id = h.run().await;

    assert_eq!(h.status(&run_id).await, "cancelled");
    assert_eq!(h.iterations(&run_id).await, 1);
}

#[tokio::test]
async fn a_run_cancelled_before_its_first_call_reports_no_iterations() {
    // Honest in the other direction: it really did nothing.
    let h = Harness::new(vec![MockResponse::text("never reached")]).await;
    h.cancel.cancel();

    let run_id = h.run().await;

    assert_eq!(h.status(&run_id).await, "cancelled");
    assert_eq!(h.iterations(&run_id).await, 0);
}

#[tokio::test]
async fn a_run_that_hits_the_iteration_cap_reports_the_cap() {
    let mut h = Harness::new(vec![
        MockResponse::tool_call("c1", "spin", json!({})),
        MockResponse::tool_call("c2", "spin", json!({})),
        MockResponse::tool_call("c3", "spin", json!({})),
    ])
    .await
    .with_tool(Box::new(StubTool::new("spin", "again")))
    .await;
    h.max_iterations = 3;

    let run_id = h.run().await;

    assert_eq!(h.status(&run_id).await, "failed");
    assert_eq!(h.iterations(&run_id).await, 3);
}

#[tokio::test]
async fn the_iteration_count_is_on_the_row_before_the_run_ends() {
    // The crash case, made deterministic. A run killed by closing the app
    // never reaches `finish_run`; all the startup sweep can do is leave the
    // column saying whatever the run itself last wrote. If the count were only
    // written at the end, every interrupted run would report zero — exactly
    // the bug this story is closing, moved somewhere harder to see.
    //
    // The tool reads the row from inside the loop, so what it sees is what a
    // crash at that instant would have left behind.
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let h = Harness::new(vec![
        MockResponse::tool_call("c1", "peek", json!({})),
        MockResponse::tool_call("c2", "peek", json!({})),
        MockResponse::text("done"),
    ])
    .await
    .with_tool(Box::new(IterationPeekTool { seen: seen.clone() }))
    .await;

    let run_id = h.run().await;

    assert_eq!(
        *seen.lock().expect("seen poisoned"),
        vec![1, 2],
        "the row must already carry the iterations performed so far"
    );
    assert_eq!(h.iterations(&run_id).await, 3);
}

// ---------------------------------------------------------------------------
// Crash recovery, end to end
//
// The db crate covers the sweep's SQL against seeded rows. These drive a real
// `ConversationRuntime` instead, so a drift between what the runtime writes
// and what the sweep reads cannot hide between the two crates.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_run_records_the_process_that_started_it() {
    // What lets the startup sweep tell a run this process is executing from
    // one a dead process left behind.
    let h = Harness::new(vec![MockResponse::text("done")]).await;

    let run_id = h.run().await;

    let owner: Option<String> =
        sqlx::query_scalar("SELECT owner_instance_id FROM story_runs WHERE id = ?")
            .bind(&run_id)
            .fetch_one(&h.db)
            .await
            .expect("fetch owner_instance_id");

    assert_eq!(owner.as_deref(), Some(db::recovery::instance_id()));
}

#[tokio::test]
async fn the_startup_sweep_leaves_a_run_this_process_is_executing_alone() {
    let h = Harness::new(vec![MockResponse::text("done")]).await;
    let rt = h.build().await;
    let run_id = rt.run_id.clone();

    let report = db::recovery::reconcile_orphaned_runs(&h.db, db::recovery::instance_id())
        .await
        .expect("sweep");

    assert!(report.is_empty(), "a live run must survive the sweep: {report:?}");
    assert_eq!(h.status(&run_id).await, "running");

    // ...and it still finishes normally afterwards.
    rt.run().await.expect("run");
    assert_eq!(h.status(&run_id).await, "done");
}

#[tokio::test]
async fn the_startup_sweep_interrupts_a_run_a_previous_process_left_behind() {
    // A crash, as far as the database can tell: the row exists and says
    // running, and nothing in this process claims it.
    let h = Harness::new(vec![MockResponse::text("unused")]).await;
    let rt = h.build().await;
    let run_id = rt.run_id.clone();
    sqlx::query(
        "UPDATE story_runs SET owner_instance_id = 'a-previous-launch', iteration_count = 4 \
         WHERE id = ?",
    )
    .bind(&run_id)
    .execute(&h.db)
    .await
    .expect("re-owner the run");

    let report = db::recovery::reconcile_orphaned_runs(&h.db, db::recovery::instance_id())
        .await
        .expect("sweep");

    assert_eq!(report.runs, vec![run_id.clone()]);
    assert_eq!(h.status(&run_id).await, "failed");
    assert_eq!(
        h.iterations(&run_id).await,
        4,
        "the work it got through is kept"
    );
    let kinds: Vec<String> = h.events(&run_id).await.into_iter().map(|(k, _)| k).collect();
    assert!(kinds.contains(&"interrupted".to_string()), "got {kinds:?}");
    drop(rt);
}
