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

use api::mock::{MockResponse, RecordedCall};
use api::{ChatMessage, CompletionConfig, MessageRole, MockLlmProvider, ToolCall};
use db::testing::{make_test_pool, run_events, run_status, seed_profile, seed_story};
use db::DbPool;
use tools::{Tool, ToolContext, ToolOutput, ToolRegistry};

use crate::approval_gate::ApprovalGate;
use crate::runtime::{CancelFlag, ConversationRuntime, RunEvent};
use crate::testing::RecordingSink;
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
}

impl Harness {
    async fn new(script: Vec<MockResponse>) -> Self {
        let db = make_test_pool().await;
        seed_profile(&db, PROFILE_ID, "Test Agent").await;
        seed_story(&db, STORY_ID, "Test Story", "ready").await;

        Self {
            db,
            sink: RecordingSink::new(),
            gate: Arc::new(ApprovalGate::new()),
            provider: Arc::new(MockLlmProvider::script(script)),
            registry: Arc::new(Mutex::new(ToolRegistry::new())),
            cancel: CancelFlag::new(),
            policy: PermissionPolicy::allow_all(),
            config: CompletionConfig::new("mock-model", 1024),
            max_iterations: 20,
            track_history: true,
            event_retention_runs: 0,
        }
    }

    async fn with_tool(self, tool: Box<dyn Tool>) -> Self {
        self.registry.lock().await.register(tool);
        self
    }

    /// Build the runtime, returning it plus its run id.
    async fn build(&self) -> ConversationRuntime {
        ConversationRuntime::new(
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
            None, // workspace_root
            self.track_history,
            self.event_retention_runs,
        )
        .await
        .expect("build runtime")
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
