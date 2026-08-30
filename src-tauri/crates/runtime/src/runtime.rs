use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use tokio::sync::Mutex;

use anyhow::{Context, Result};
use db::DbPool;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use api::{ChatMessage, CompletionConfig, LlmProvider, StreamEvent, ToolCall, ToolDefinition, Usage};
use tools::{ToolContext, ToolRegistry};
use memory::MemoryStore;

use crate::approval_gate::ApprovalGate;
use crate::context::{self, ContextPolicy, ContextStrategy};
use crate::{PermissionPolicy, PolicyDecision};

/// Output cap for the summarisation call `context_strategy = "summary"` makes.
///
/// Small on purpose: the summary is reinserted into the very context that was
/// over budget, so a long one defeats the compaction that produced it.
const SUMMARY_MAX_OUTPUT_TOKENS: u32 = 700;

/// Space held back from the budget when `context_strategy = "summary"`, so the
/// reinserted summary fits without pushing the request over.
///
/// The summarisation call is capped at [`SUMMARY_MAX_OUTPUT_TOKENS`], so the
/// message can carry at most that much text; the remainder covers the
/// `[compacted context]` marker and the per-message envelope. Erring high
/// costs one extra evicted turn — erring low costs the invariant.
const SUMMARY_RESERVE_TOKENS: u64 = SUMMARY_MAX_OUTPUT_TOKENS as u64 + 64;

/// Bounds on how far a reconciled estimate may be scaled by the provider's
/// real figures.
///
/// The ratio is one observation, and a provider that reports something odd —
/// or a call whose usage covers a different request than the one measured —
/// must not be able to move the budget arbitrarily far in either direction.
const MIN_ESTIMATE_SCALE: f64 = 0.5;
const MAX_ESTIMATE_SCALE: f64 = 4.0;

// ---------------------------------------------------------------------------
// Tauri event payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    /// Partial text token from the LLM.
    Token { run_id: String, content: String },
    /// The assistant is invoking a tool.
    ToolCall { run_id: String, tool_name: String, input: serde_json::Value },
    /// A tool has returned a result.
    ToolResult { run_id: String, tool_name: String, output: String, is_error: bool },
    /// The run completed successfully.
    Complete { run_id: String, stop_reason: String },
    /// The run was cancelled by the user.
    Cancelled { run_id: String },
    /// The run failed with an error.
    Failed { run_id: String, message: String },
    /// A transient provider failure is about to be retried.
    ///
    /// Emitted before the wait, not after, so a run that appears to hang for
    /// thirty seconds says why while it is happening rather than afterwards.
    Retrying {
        run_id: String,
        /// Which retry this is, counting from one.
        attempt: u32,
        /// The budget it is counting against.
        max_retries: u32,
        /// The classified failure being retried.
        reason: String,
        delay_secs: f64,
        /// Whether the delay is the provider's own instruction or our backoff.
        provider_requested_delay: bool,
    },
    /// A gated tool call is parked, waiting for the user to decide.
    ///
    /// The run is not making progress and will not resume on its own. Emitted
    /// so a live view can say so; without it a parked run is indistinguishable
    /// from a slow one.
    AwaitingApproval {
        run_id: String,
        approval_request_id: String,
        tool_name: String,
    },
    /// The parked tool call was answered and the run is moving again.
    ApprovalResolved {
        run_id: String,
        approval_request_id: String,
        tool_name: String,
        approved: bool,
        /// How the decision was reached — see `ApprovalOutcome`.
        outcome: String,
    },
    /// The conversation was compacted to fit the input budget.
    ///
    /// Emitted once per compaction, before the provider call it made room
    /// for, so an operator watching a long run can see history being dropped
    /// rather than inferring it from an agent that forgot something.
    ContextCompacted {
        run_id: String,
        /// The `context_strategy` that did it: `"recent"` or `"summary"`.
        strategy: String,
        /// Estimated input tokens before compaction.
        before_tokens: u64,
        /// Estimated input tokens after it.
        after_tokens: u64,
        /// The budget both are measured against.
        budget_tokens: u64,
        /// How many messages were dropped.
        evicted_messages: usize,
        /// Whether the dropped prefix was replaced by a generated summary.
        /// `false` under `recent`, and also under `summary` when the
        /// summarisation call failed and it degraded to plain eviction.
        summarized: bool,
    },
}

// ---------------------------------------------------------------------------
// Why a run ended
// ---------------------------------------------------------------------------

/// A run that ended on a provider failure, with the classification intact.
///
/// `run()` returns this inside its `anyhow::Error`, so a caller that only logs
/// `{e}` is unaffected while one deciding whether to try again can
/// `downcast_ref` and ask. The alternative — formatting the `ApiError` into a
/// string at the failure site — is what this story exists to undo: the
/// evidence needed to tell a rate limit from a bad API key was being destroyed
/// one line before it would have been useful.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct RunFailure {
    /// What the timeline and the logs say.
    pub message: String,
    /// Whether another attempt is worth making, and how long to wait first.
    pub kind: api::FailureKind,
}

impl RunFailure {
    /// A failure with no provider error behind it — a ceiling the run hit
    /// rather than a call that went wrong. Never retryable: a fresh attempt
    /// meets the same ceiling with the same prompt.
    pub fn deterministic(message: impl Into<String>) -> Self {
        Self { message: message.into(), kind: api::FailureKind::Permanent }
    }
}

/// Longest a backoff will ever wait, however many attempts have failed.
pub(crate) const BACKOFF_CAP: std::time::Duration = std::time::Duration::from_secs(30);

/// How long to wait before retry number `attempt` (counting from one) when the
/// provider did not state a delay of its own.
///
/// Exponential and capped, scaled by a factor in `[0.75, 1.25]` derived from
/// the run id. The spread exists so a provider outage does not have every run
/// in flight return at the same instant and reproduce it; deriving it from the
/// run id rather than a random source spreads runs against each other while
/// keeping any single run's schedule reproducible in a test.
///
/// The scaling is one positive multiplication rather than a signed offset
/// added to the base, because `Duration::mul_f64` panics outright on a
/// negative factor — which is not a hypothetical: an earlier form of this
/// computed `±25%` as an offset and panicked for every run whose id happened
/// to hash into the lower half of the range.
pub(crate) fn backoff_delay(run_id: &str, attempt: u32) -> std::time::Duration {
    const BASE: std::time::Duration = std::time::Duration::from_secs(1);

    // The `min` exists only to keep the shift in range; the growth is bounded
    // by BACKOFF_CAP below, not by the exponent. Ceilinged too low, the cap
    // becomes unreachable and the constant is a lie.
    let exponential = BASE.saturating_mul(1u32 << attempt.min(8).saturating_sub(1));

    // A cheap stable hash of the run id, in [0, 1000).
    let spread = run_id
        .bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(u32::from(b)))
        % 1000;

    let factor = 0.75 + 0.5 * (f64::from(spread) / 1000.0);
    exponential.mul_f64(factor).min(BACKOFF_CAP)
}

// ---------------------------------------------------------------------------
// Approval outcomes
// ---------------------------------------------------------------------------

/// How a gated tool call stopped waiting.
///
/// Three of these four are *not* approved, and the difference between them
/// matters to whoever reads the record later: only `Denied` is a decision a
/// user made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    /// The user approved the call.
    Approved,
    /// The user rejected the call.
    Denied,
    /// A configured `approval_timeout` elapsed with no answer.
    Expired,
    /// The gate entry was dropped without a decision — the app is shutting
    /// down, or the run was torn down around it.
    Abandoned,
}

impl ApprovalOutcome {
    /// The value written to `approval_requests.status`, and carried on the
    /// `ApprovalResolved` event.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Denied => "rejected",
            Self::Expired => "expired",
            Self::Abandoned => "abandoned",
        }
    }

    /// A sentence for the model and for `rejection_reason`.
    pub fn explanation(self) -> &'static str {
        match self {
            Self::Approved => "the user approved it",
            Self::Denied => "the user declined it",
            Self::Expired => concat!(
                "the approval request expired before anyone answered it; ",
                "this is not a decision the user made",
            ),
            Self::Abandoned => concat!(
                "the approval request was discarded before anyone answered it; ",
                "this is not a decision the user made",
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Event sink
// ---------------------------------------------------------------------------

/// Where a run's events go.
///
/// In the app this is the Tauri `AppHandle`. Tests substitute a recorder,
/// because `tauri::test::mock_app()` yields an `AppHandle<MockRuntime>` which
/// cannot stand in for the concrete `AppHandle<Wry>` the app uses.
pub trait EventSink: Send + Sync + 'static {
    fn emit_event(&self, name: &str, payload: serde_json::Value);
}

impl<R: tauri::Runtime> EventSink for tauri::AppHandle<R> {
    fn emit_event(&self, name: &str, payload: serde_json::Value) {
        if let Err(e) = self.emit(name, payload) {
            warn!("Failed to emit '{name}': {e}");
        }
    }
}

// ---------------------------------------------------------------------------
// Cancel flag — a simple shared bool so stop_run can signal the loop.
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct CancelFlag(Arc<std::sync::atomic::AtomicBool>);

impl CancelFlag {
    pub fn new() -> Self {
        Self(Arc::new(std::sync::atomic::AtomicBool::new(false)))
    }

    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Default for CancelFlag {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ConversationRuntime
// ---------------------------------------------------------------------------

pub struct ConversationRuntime {
    /// Unique identifier for this run (also stored in `story_runs`).
    pub run_id: String,
    pub story_id: String,
    pub agent_profile_id: String,

    /// LLM provider (Anthropic, OpenRouter, Ollama, …)
    pub provider: Box<dyn LlmProvider>,
    /// All built-in and profile-installed tools.
    pub tool_registry: Arc<Mutex<ToolRegistry>>,
    /// Controls which tools this profile may call.
    pub permission_policy: PermissionPolicy,
    /// Wakes waiting runtime tasks when the user approves/rejects a tool call.
    pub approval_gate: Arc<ApprovalGate>,

    /// Conversation history (grows as the run progresses).
    pub messages: Vec<ChatMessage>,
    /// LLM configuration (model, max_tokens, temperature, system_prompt).
    pub config: CompletionConfig,
    /// Hard cap on LLM iterations.
    pub max_iterations: u32,

    pub db: DbPool,
    pub app: Arc<dyn EventSink>,
    pub cancel: CancelFlag,
    /// Semantic + episodic memory for this agent profile. None when the profile
    /// has `persistent_memory = false`.
    pub memory: Option<MemoryStore>,
    /// Set when this runtime is executing as part of a pipeline. Scopes the
    /// shared_scratchpad memory and enables spawn_subtask depth guards.
    pub pipeline_run_id: Option<String>,
    /// Recursion depth inside a pipeline chain. 0 for top-level runs.
    pub pipeline_depth: u32,
    /// Injected by the pipeline engine so `spawn_subtask` can fire child runs.
    pub spawn_subtask: Option<tools::SpawnSubtaskFn>,
    /// Directory agents are restricted to when using file tools. None = no restriction.
    pub workspace_root: Option<std::path::PathBuf>,
    /// Whether non-critical run events (messages, tool calls/results) are persisted for
    /// this story. Critical events (error, approval_request, etc.) are always written.
    pub track_history: bool,
    /// How many most-recent runs' events to retain per story (0 = unlimited).
    pub event_retention_runs: u32,
    /// Monotonically increasing counter for `sequence_num` in `run_events`.
    pub sequence_counter: Arc<AtomicU32>,
    /// Git HEAD SHA captured at run start (None if workspace is not a git repo).
    pub before_sha: Option<String>,
    /// The isolated worktree this run executes in.
    ///
    /// `None` means the run is writing into the user's own directory, which
    /// only happens when [`Self::isolate`] was never called or could not
    /// isolate — and in the latter case `story_runs.isolation_status` and a
    /// persisted `isolation` event both say so.
    pub worktree: Option<crate::worktree::RunWorktree>,

    /// Token usage summed over every provider call this run has made.
    ///
    /// A run is many calls, so this is a running total, not the last call's
    /// figures. It is persisted to `story_runs` whenever the run reaches a
    /// terminal state — including failure and cancellation, where it accounts
    /// for the work already paid for.
    pub usage_total: Usage,
    /// Usage reported by the most recent provider call, or `None` before the
    /// first call completes or when the provider does not measure tokens.
    ///
    /// Unlike `usage_total` this does not accumulate: its
    /// `total_input_tokens()` is the size of the context that was last sent,
    /// which is the figure a context budget has to be measured against.
    pub last_usage: Option<Usage>,

    /// Where automatic notifications go, and how `send_notification` reaches
    /// the desktop. `None` in tests and wherever no desktop exists.
    ///
    /// Defaulted rather than taken as a constructor argument, for the reason
    /// given on `context_policy`: the constructors already carry nineteen
    /// parameters. A runtime built without one still runs — it just cannot
    /// call the user back.
    pub notifier: Option<Arc<dyn tools::Notifier>>,

    /// How many times a failed provider call may be retried before the run
    /// gives up.
    ///
    /// Retries happen *within* the run: the conversation, its completed tool
    /// work and its worktree all survive, so a rate limit costs the wait and
    /// nothing else. Zero — the default — is the behaviour every caller had
    /// before retries existed.
    pub max_retries: u32,

    /// How long a gated tool call waits for the user before giving up.
    ///
    /// `None` — the default — waits indefinitely, which is the whole point of
    /// the setting: an unattended run must survive the user walking away, and
    /// a timeout that denies is a decision the user never made. A `Some` value
    /// is honoured for anyone who would rather a run fail than sit parked, and
    /// expiry is then recorded as `expired`, never as a rejection.
    pub approval_timeout: Option<std::time::Duration>,

    /// How this profile wants an over-budget conversation handled.
    ///
    /// Defaulted rather than taken as a constructor argument: the two
    /// constructors already carry sixteen and nineteen parameters, and every
    /// caller that has a profile row to read sets this straight after
    /// building. The default — `recent`, with a per-model budget — is the
    /// safe one, so a caller that forgets still gets compaction.
    pub context_policy: ContextPolicy,

    /// Our own estimate of the input size of the last request sent, before
    /// any calibration scaling.
    ///
    /// Kept so the next `Usage` can be divided by it to learn how far off the
    /// estimator is for this model.
    last_raw_estimate: Option<u64>,
    /// Multiplier applied to raw estimates, learned from the provider's real
    /// figures. 1.0 until the first call reports usage.
    estimate_scale: f64,
}

impl ConversationRuntime {
    /// Create a new runtime and insert the `story_runs` row.
    pub async fn new(
        story_id: impl Into<String>,
        agent_profile_id: impl Into<String>,
        provider: Box<dyn LlmProvider>,
        tool_registry: Arc<Mutex<ToolRegistry>>,
        permission_policy: PermissionPolicy,
        approval_gate: Arc<ApprovalGate>,
        initial_messages: Vec<ChatMessage>,
        config: CompletionConfig,
        max_iterations: u32,
        db: DbPool,
        app: Arc<dyn EventSink>,
        cancel: CancelFlag,
        memory: Option<MemoryStore>,
        workspace_root: Option<std::path::PathBuf>,
        track_history: bool,
        event_retention_runs: u32,
    ) -> Result<Self> {
        Self::new_with_pipeline(
            story_id, agent_profile_id, provider, tool_registry, permission_policy,
            approval_gate, initial_messages, config, max_iterations, db, app, cancel, memory,
            None, 0, None, workspace_root, track_history, event_retention_runs,
        ).await
    }

    /// Full constructor including pipeline context fields.
    pub async fn new_with_pipeline(
        story_id: impl Into<String>,
        agent_profile_id: impl Into<String>,
        provider: Box<dyn LlmProvider>,
        tool_registry: Arc<Mutex<ToolRegistry>>,
        permission_policy: PermissionPolicy,
        approval_gate: Arc<ApprovalGate>,
        initial_messages: Vec<ChatMessage>,
        config: CompletionConfig,
        max_iterations: u32,
        db: DbPool,
        app: Arc<dyn EventSink>,
        cancel: CancelFlag,
        memory: Option<MemoryStore>,
        pipeline_run_id: Option<String>,
        pipeline_depth: u32,
        spawn_subtask: Option<tools::SpawnSubtaskFn>,
        workspace_root: Option<std::path::PathBuf>,
        track_history: bool,
        event_retention_runs: u32,
    ) -> Result<Self> {
        let run_id = Uuid::new_v4().to_string();
        let story_id = story_id.into();
        let agent_profile_id = agent_profile_id.into();

        // Capture git HEAD SHA before the run modifies anything.
        let before_sha = workspace_root
            .as_deref()
            .and_then(|root| crate::git::get_head_sha(root));

        // Persist the run record.
        //
        // `owner_instance_id` stamps the row with this launch of the app, so
        // the startup sweep in `db::recovery` can tell a run this process is
        // executing from one a previous process died holding.
        sqlx::query(
            "INSERT INTO story_runs
                 (id, story_id, agent_profile_id, status, before_sha, started_at, owner_instance_id)
             VALUES (?, ?, ?, 'running', ?, CURRENT_TIMESTAMP, ?)"
        )
        .bind(&run_id)
        .bind(&story_id)
        .bind(&agent_profile_id)
        .bind(&before_sha)
        .bind(db::recovery::instance_id())
        .execute(&db)
        .await
        .context("Failed to insert story_run")?;

        // Claim the card, so the board shows the work as in flight while it
        // actually is. Only pipelines did this before, which left a manually
        // started run looking untouched for its whole duration — and left the
        // completion side with nothing in `in_progress` to move.
        if db::story_status::auto_advance_enabled(&db).await {
            match db::story_status::claim_story(&db, &story_id).await {
                Ok(true) => {
                    app.emit_event(db::story_status::STORIES_CHANGED_EVENT, serde_json::Value::Null)
                }
                Ok(false) => {}
                Err(e) => {
                    warn!(run_id = %run_id, story_id = %story_id, "Failed to claim the story: {e}")
                }
            }
        }

        Ok(Self {
            run_id,
            story_id,
            agent_profile_id,
            provider,
            tool_registry,
            permission_policy,
            approval_gate,
            messages: initial_messages,
            config,
            max_iterations,
            db,
            app,
            cancel,
            memory,
            pipeline_run_id,
            pipeline_depth,
            spawn_subtask,
            workspace_root,
            track_history,
            event_retention_runs,
            sequence_counter: Arc::new(AtomicU32::new(0)),
            before_sha,
            worktree: None,
            usage_total: Usage::default(),
            last_usage: None,
            notifier: None,
            max_retries: 0,
            approval_timeout: None,
            context_policy: ContextPolicy::default(),
            last_raw_estimate: None,
            estimate_scale: 1.0,
        })
    }

    // -----------------------------------------------------------------------
    // Isolation
    // -----------------------------------------------------------------------

    /// Give this run a private git worktree and point it at that instead of the
    /// user's checkout.
    ///
    /// Called after construction rather than from the constructor, which
    /// already carries nineteen arguments, and for the same reason as
    /// `context_policy`: every caller that has an app data directory to offer
    /// sets it straight after building, and a caller that forgets gets the old
    /// un-isolated behaviour rather than a broken run.
    ///
    /// **Must be called before [`Self::run`].** Once the loop starts, the
    /// workspace root has already been handed to tool contexts.
    ///
    /// This never fails the run. A workspace that cannot be isolated — no git,
    /// no commits, no disk — records why in `story_runs.isolation_status` and
    /// `isolation_note`, persists an `isolation` event so the reason appears in
    /// the run timeline, and proceeds un-isolated. Silence is the one outcome
    /// that is not allowed.
    pub async fn isolate(&mut self, worktrees_dir: &std::path::Path) -> crate::worktree::Isolation {
        self.isolate_from(worktrees_dir, None).await
    }

    /// As [`Self::isolate`], but branching the worktree from `base` rather than
    /// the workspace's `HEAD`.
    ///
    /// The steps of a sequential pipeline pass the previous step's commit, so
    /// each one starts from what the last one produced instead of from an
    /// empty slate. Without it, isolation would quietly break the handoff that
    /// makes a sequential pipeline worth running.
    pub async fn isolate_from(
        &mut self,
        worktrees_dir: &std::path::Path,
        base: Option<&str>,
    ) -> crate::worktree::Isolation {
        let Some(root) = self.workspace_root.clone() else {
            let note = "This run has no workspace directory, so nothing was isolated. \
                        File tools are unrestricted."
                .to_string();
            self.record_isolation(crate::worktree::STATUS_NO_WORKSPACE, Some(&note), None)
                .await;
            self.persist_event("isolation", &serde_json::json!({ "message": note }))
                .await;
            return crate::worktree::Isolation::Unavailable(note);
        };

        let outcome = crate::worktree::create_from(&root, worktrees_dir, &self.run_id, base);

        if let Some(wt) = outcome.worktree() {
            info!(
                run_id = %self.run_id,
                branch = %wt.branch,
                path = %wt.path.display(),
                "Run isolated in a git worktree"
            );
            // Everything downstream keys off `workspace_root`: the run's
            // `ToolContext`, `resolve_path` in the file tools, and
            // `resolve_working_dir` in the shell tool.
            self.workspace_root = Some(wt.path.clone());
            self.before_sha = Some(wt.base_sha.clone());
            self.worktree = Some(wt.clone());
        } else {
            warn!(run_id = %self.run_id, "Run is NOT isolated: {}", outcome.note().unwrap_or_default());
        }

        let note = outcome.note();
        self.record_isolation(outcome.status(), note.as_deref(), outcome.worktree())
            .await;
        if let Some(note) = note {
            self.persist_event("isolation", &serde_json::json!({ "message": note }))
                .await;
        }

        outcome
    }

    /// Write the isolation columns on the `story_runs` row.
    async fn record_isolation(
        &self,
        status: &str,
        note: Option<&str>,
        worktree: Option<&crate::worktree::RunWorktree>,
    ) {
        let _ = sqlx::query(
            "UPDATE story_runs \
             SET worktree_path = ?, branch_name = ?, before_sha = ?, \
                 isolation_status = ?, isolation_note = ? \
             WHERE id = ?",
        )
        .bind(worktree.map(|w| w.path.to_string_lossy().to_string()))
        .bind(worktree.map(|w| w.branch.clone()))
        .bind(self.before_sha.clone())
        .bind(status)
        .bind(note)
        .bind(&self.run_id)
        .execute(&self.db)
        .await;
    }

    // -----------------------------------------------------------------------
    // Execution loop
    // -----------------------------------------------------------------------

    /// Run the agent loop until the LLM returns end_turn, max_iterations is
    /// reached, or the cancel flag is set.
    pub async fn run(mut self) -> Result<()> {
        info!(run_id = %self.run_id, story_id = %self.story_id, "Starting agent run");

        // -----------------------------------------------------------------------
        // Inject relevant past semantic memories into the system prompt.
        // -----------------------------------------------------------------------
        if let Some(mem) = &self.memory {
            // Use the story_id as the context query — past runs on similar stories
            // will surface higher similarity scores.
            let query = format!("story {}", self.story_id);
            match mem.search_semantic(&query, 3).await {
                Ok(results) if !results.is_empty() => {
                    let context = results
                        .iter()
                        .map(|r| format!("- {}", r.content))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let memory_section = format!(
                        "\n\n## Relevant context from past runs:\n{context}"
                    );
                    if let Some(ref mut prompt) = self.config.system_prompt {
                        prompt.push_str(&memory_section);
                    } else {
                        self.config.system_prompt = Some(memory_section);
                    }
                    info!(run_id = %self.run_id, count = results.len(), "Injected past memories into system prompt");
                }
                Ok(_) => {}
                Err(e) => warn!(run_id = %self.run_id, "Semantic memory search failed: {e}"),
            }
        }

        let mut iterations = 0u32;
        let mut final_stop_reason = String::from("end_turn");

        loop {
            if self.cancel.is_cancelled() {
                info!(run_id = %self.run_id, "Run cancelled before iteration {iterations}");
                self.emit(RunEvent::Cancelled { run_id: self.run_id.clone() });
                self.finish_run("cancelled", iterations).await;
                return Ok(());
            }

            if iterations >= self.max_iterations {
                warn!(run_id = %self.run_id, "Max iterations ({}) reached", self.max_iterations);
                self.emit(RunEvent::Failed {
                    run_id: self.run_id.clone(),
                    message: format!("Max iterations ({}) reached", self.max_iterations),
                });
                self.finish_run("failed", iterations).await;
                return Ok(());
            }
            iterations += 1;
            // Written as the loop turns, not only at the end. A run the app
            // never got to finish is reconciled by
            // `db::recovery::reconcile_orphaned_runs`, which cannot see this
            // counter and can only leave the column saying whatever the run
            // itself last said — so if it were written once at the end, every
            // interrupted run would report zero iterations.
            self.persist_iteration_count(iterations).await;

            // ---------------------------------------------------------------
            // Collect tool definitions visible to this profile.
            // ---------------------------------------------------------------
            let tool_defs = {
                let registry = self.tool_registry.lock().await;
                registry
                    .all_definitions()
                    .into_iter()
                    .filter(|def| self.permission_policy.check(&def.name))
                    .collect::<Vec<_>>()
            };

            // ---------------------------------------------------------------
            // Keep the conversation inside the model's input budget.
            //
            // Immediately before the call, because the tool list is part of
            // what is measured and the message list has just grown by a whole
            // tool round trip.
            // ---------------------------------------------------------------
            if let Err(msg) = self.enforce_context_budget(&tool_defs).await {
                error!(run_id = %self.run_id, "{msg}");
                self.persist_event("error", &serde_json::json!({ "message": msg })).await;
                self.emit(RunEvent::Failed { run_id: self.run_id.clone(), message: msg.clone() });
                self.finish_run("failed", iterations).await;
                return Err(anyhow::anyhow!(msg));
            }

            // ---------------------------------------------------------------
            // Call the LLM and stream events.
            // ---------------------------------------------------------------
            let mut stream = match self.stream_with_retries(tool_defs).await {
                Ok(s) => s,
                Err(failure) => {
                    error!(
                        run_id = %self.run_id,
                        retryable = failure.kind.is_transient(),
                        "{}", failure.message
                    );
                    self.emit(RunEvent::Failed {
                        run_id: self.run_id.clone(),
                        message: failure.message.clone(),
                    });
                    self.finish_run("failed", iterations).await;
                    return Err(anyhow::Error::new(failure));
                }
            };

            // Accumulate text and tool calls in this iteration.
            let mut text_buf = String::new();
            let mut pending_tool_calls: Vec<ToolCall> = Vec::new();

            while let Some(event) = stream.next().await {
                if self.cancel.is_cancelled() {
                    info!(run_id = %self.run_id, "Run cancelled mid-stream");
                    self.emit(RunEvent::Cancelled { run_id: self.run_id.clone() });
                    self.finish_run("cancelled", iterations).await;
                    return Ok(());
                }

                match event {
                    Ok(StreamEvent::TextDelta(token)) => {
                        text_buf.push_str(&token);
                        // Persist incremental token to run_events.
                        self.persist_event("token", &serde_json::json!({ "content": token })).await;
                        self.emit(RunEvent::Token {
                            run_id: self.run_id.clone(),
                            content: token,
                        });
                    }
                    Ok(StreamEvent::ToolCallDelta(call)) => {
                        pending_tool_calls.push(call);
                    }
                    Ok(StreamEvent::Done { stop_reason, usage }) => {
                        if let Some(u) = usage {
                            self.record_usage(u);
                        }
                        final_stop_reason = stop_reason;
                        break;
                    }
                    Ok(StreamEvent::Error(msg)) => {
                        warn!(run_id = %self.run_id, "Stream error (continuing): {msg}");
                    }
                    Err(e) => {
                        warn!(run_id = %self.run_id, "Stream item error (continuing): {e}");
                    }
                }
            }

            // ---------------------------------------------------------------
            // Add accumulated assistant turn to the conversation history.
            // ---------------------------------------------------------------
            if !text_buf.is_empty() || !pending_tool_calls.is_empty() {
                if pending_tool_calls.is_empty() {
                    self.messages.push(ChatMessage::assistant(text_buf.clone()));
                } else {
                    self.messages.push(ChatMessage::assistant_with_tool_calls(
                        text_buf.clone(),
                        pending_tool_calls.clone(),
                    ));
                }
            }

            // ---------------------------------------------------------------
            // Dispatch tool calls, if any.
            // ---------------------------------------------------------------
            if !pending_tool_calls.is_empty() {
                let tool_results = self.execute_tools(pending_tool_calls).await;
                for result_msg in tool_results {
                    self.messages.push(result_msg);
                }
                // Continue the loop — pass results back to the LLM.
                continue;
            }

            // ---------------------------------------------------------------
            // No tool calls → the LLM finished its turn.
            // ---------------------------------------------------------------
            break;
        }

        info!(run_id = %self.run_id, stop_reason = %final_stop_reason, "Run completed");
        self.persist_event("complete", &serde_json::json!({ "stop_reason": final_stop_reason })).await;
        self.emit(RunEvent::Complete {
            run_id: self.run_id.clone(),
            stop_reason: final_stop_reason,
        });

        // -----------------------------------------------------------------------
        // Write a run summary to semantic memory for future context retrieval.
        // -----------------------------------------------------------------------
        if let Some(ref mem) = self.memory {
            let summary = self.generate_run_summary();
            if let Err(e) = mem.write_semantic(&summary).await {
                warn!(run_id = %self.run_id, "Failed to write semantic memory: {e}");
            } else {
                info!(run_id = %self.run_id, "Run summary written to semantic memory");
            }
        }

        // "done" — not "completed" — is the shared vocabulary: the pipeline
        // engine writes it and the frontend's RunStatus union only knows it.
        self.finish_run("done", iterations).await;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Tool execution
    // -----------------------------------------------------------------------

    async fn execute_tools(&self, calls: Vec<ToolCall>) -> Vec<ChatMessage> {
        let mut results = Vec::new();

        for call in calls {
            // Full permission check (defend in depth — tools are pre-filtered but
            // the policy may impose path restrictions or require approval).
            //
            // The tool's own declaration comes from the registry: the policy
            // cannot know from a name and a JSON blob that `file_list` reads the
            // disk, that a custom tool shells out to `git`, or which input key
            // holds a path. A name the registry does not know yields `None`,
            // which the policy treats as unclassifiable and fails closed on.
            let permission_info = {
                let registry = self.tool_registry.lock().await;
                registry.permission_info(&call.name)
            };
            let request = crate::ToolRequest::new(&call.name, &call.input)
                .with_info(permission_info)
                .with_workspace_root(self.workspace_root.as_deref());

            match self.permission_policy.check_tool(&request) {
                PolicyDecision::Allow => {
                    // fall through to execution below
                }
                PolicyDecision::Deny(reason) => {
                    warn!(run_id = %self.run_id, tool=%call.name, "Tool denied: {reason}");
                    self.emit(RunEvent::ToolResult {
                        run_id: self.run_id.clone(),
                        tool_name: call.name.clone(),
                        output: reason.clone(),
                        is_error: true,
                    });
                    results.push(ChatMessage::tool_result(&call.id, &reason));
                    continue;
                }
                PolicyDecision::RequiresApproval => {
                    let approval_id = Uuid::new_v4().to_string();
                    let input_str = call.input.to_string();

                    // Persist the approval request so the frontend can query it.
                    let insert_result = sqlx::query(
                        "INSERT INTO approval_requests (id, run_id, tool_name, tool_input) \
                         VALUES (?, ?, ?, ?)",
                    )
                    .bind(&approval_id)
                    .bind(&self.run_id)
                    .bind(&call.name)
                    .bind(&input_str)
                    .execute(&self.db)
                    .await;

                    if let Err(e) = insert_result {
                        let msg = format!("Approval gate DB error: {e}");
                        error!(run_id = %self.run_id, "{msg}");
                        self.emit(RunEvent::ToolResult {
                            run_id: self.run_id.clone(),
                            tool_name: call.name.clone(),
                            output: msg.clone(),
                            is_error: true,
                        });
                        results.push(ChatMessage::tool_result(&call.id, &msg));
                        continue;
                    }

                    // Register before emitting so the frontend can resolve immediately.
                    let rx = self.approval_gate.register(&approval_id);

                    let payload = serde_json::json!({
                        "approvalRequestId": approval_id,
                        "runId": self.run_id,
                        "toolName": call.name,
                    });
                    self.app.emit_event("approval-request-created", payload);

                    // The run stops here until somebody answers. Say so on the
                    // timeline and on the event bus, so a parked run reads as
                    // parked rather than as one that has simply gone quiet.
                    self.persist_event(
                        "approval_request",
                        &serde_json::json!({
                            "approvalRequestId": approval_id,
                            "toolName": call.name,
                        }),
                    )
                    .await;
                    self.emit(RunEvent::AwaitingApproval {
                        run_id: self.run_id.clone(),
                        approval_request_id: approval_id.clone(),
                        tool_name: call.name.clone(),
                    });

                    // Reach the user directly rather than hoping the agent
                    // chooses to call `send_notification`. This is the one
                    // point in a run where nothing further happens until a
                    // human acts, so it is the one that must travel.
                    self.notify(
                        tools::NotificationCategory::Approval,
                        "Approval needed",
                        &format!(
                            "{} is waiting to run '{}'.",
                            self.story_title().await,
                            call.name
                        ),
                    )
                    .await;

                    let outcome = self.await_approval(&approval_id, rx).await;
                    let approved = outcome == ApprovalOutcome::Approved;

                    self.emit(RunEvent::ApprovalResolved {
                        run_id: self.run_id.clone(),
                        approval_request_id: approval_id.clone(),
                        tool_name: call.name.clone(),
                        approved,
                        outcome: outcome.as_str().to_string(),
                    });
                    self.persist_event(
                        "approval_response",
                        &serde_json::json!({
                            "approvalRequestId": approval_id,
                            "toolName": call.name,
                            "approved": approved,
                            "outcome": outcome.as_str(),
                        }),
                    )
                    .await;

                    if !approved {
                        let msg = format!(
                            "Tool '{}' was not approved: {}",
                            call.name,
                            outcome.explanation()
                        );
                        warn!(run_id = %self.run_id, "{msg}");
                        self.emit(RunEvent::ToolResult {
                            run_id: self.run_id.clone(),
                            tool_name: call.name.clone(),
                            output: msg.clone(),
                            is_error: true,
                        });
                        results.push(ChatMessage::tool_result(&call.id, &msg));
                        continue;
                    }
                    // Approved — fall through to execution.
                }
            }

            self.emit(RunEvent::ToolCall {
                run_id: self.run_id.clone(),
                tool_name: call.name.clone(),
                input: call.input.clone(),
            });
            self.persist_event(
                "tool_call",
                &serde_json::json!({ "tool_name": call.name, "input": call.input }),
            ).await;

            let ctx = ToolContext {
                db: self.db.clone(),
                notifier: self.notifier.clone(),
                agent_profile_id: self.agent_profile_id.clone(),
                run_id: self.run_id.clone(),
                pipeline_run_id: self.pipeline_run_id.clone(),
                pipeline_depth: self.pipeline_depth,
                spawn_subtask: self.spawn_subtask.clone(),
                workspace_root: self.workspace_root.clone(),
            };

            // Clone the Arc out of the registry before the await so we don't
            // hold the lock across the async boundary.
            let tool_arc = {
                let registry = self.tool_registry.lock().await;
                registry.get_arc(&call.name)
            };

            let output = match tool_arc {
                Some(tool) => tool.execute(call.input.clone(), &ctx).await,
                None => tools::ToolOutput::err(format!("Tool '{}' not found", call.name)),
            };

            debug!(
                run_id = %self.run_id,
                tool = %call.name,
                is_error = output.is_error,
                "Tool returned"
            );

            self.emit(RunEvent::ToolResult {
                run_id: self.run_id.clone(),
                tool_name: call.name.clone(),
                output: output.content.clone(),
                is_error: output.is_error,
            });
            self.persist_event(
                "tool_result",
                &serde_json::json!({
                    "tool_name": call.name,
                    "output": output.content,
                    "is_error": output.is_error,
                }),
            ).await;

            results.push(ChatMessage::tool_result(&call.id, &output.content));
        }

        results
    }

    // -----------------------------------------------------------------------
    // Context budget
    // -----------------------------------------------------------------------

    /// Bring `self.messages` inside the profile's input budget, or refuse to
    /// make the call.
    ///
    /// Returns `Err` with an operator-readable message when the request cannot
    /// be made to fit — under `full`, which never compacts, and under the
    /// compacting strategies when even the protected prefix is too large. That
    /// is deliberately a run failure: the alternative is handing the provider a
    /// request we already know it will reject, and failing with its error
    /// instead of ours.
    async fn enforce_context_budget(&mut self, tool_defs: &[ToolDefinition]) -> Result<(), String> {
        let outcome = self.compact_to_budget(tool_defs).await;

        // Whatever happened above, record the uncalibrated size of what is
        // about to be sent. The provider's reply divides its real input count
        // by this to learn how far off the estimator is.
        self.last_raw_estimate = Some(api::tokens::estimate_request(
            self.config.system_prompt.as_deref(),
            &self.messages,
            tool_defs,
        ));

        outcome
    }

    /// The decision itself. See [`Self::enforce_context_budget`], which wraps
    /// this so the estimate is recorded on every path out.
    async fn compact_to_budget(&mut self, tool_defs: &[ToolDefinition]) -> Result<(), String> {
        let overhead =
            api::tokens::estimate_overhead(self.config.system_prompt.as_deref(), tool_defs);
        let budget = self.context_policy.budget_tokens(&self.config);

        let strategy = self.context_policy.strategy;

        // When we intend to summarise, plan against a reduced budget so the
        // reinserted summary has somewhere to go. Planning against the full
        // budget and inserting afterwards would put the request back over the
        // limit this function exists to enforce — nothing re-checks the size
        // once the summary is in.
        let reserve = if strategy == ContextStrategy::Summary {
            context::calibrate(SUMMARY_RESERVE_TOKENS, self.estimate_scale)
        } else {
            0
        };
        let plan_budget = budget.saturating_sub(reserve);

        let plan =
            context::plan_compaction(&self.messages, overhead, plan_budget, self.estimate_scale);

        if plan.before_tokens <= budget {
            return Ok(());
        }
        if strategy == ContextStrategy::Full {
            return Err(format!(
                "Context budget exceeded: the request is an estimated {} input tokens \
                 against a budget of {budget}. context_strategy is \"full\", which never \
                 compacts. Raise max_input_tokens for this profile, or switch it to \
                 \"recent\" or \"summary\".",
                plan.before_tokens
            ));
        }

        if !plan.fits {
            return Err(format!(
                "Context budget exceeded: the request is an estimated {} input tokens \
                 against a budget of {budget}, and compaction cannot get below {} — the \
                 system prompt, tool definitions and originating task alone do not fit. \
                 Raise max_input_tokens, shorten the system prompt, or grant fewer tools.",
                plan.before_tokens, plan.after_tokens
            ));
        }

        info!(
            run_id = %self.run_id,
            strategy = strategy.as_str(),
            before = plan.before_tokens,
            budget,
            evicting = plan.evicted_count(),
            "Compacting conversation to fit the input budget"
        );

        // Summarise before draining — the summariser needs the messages, and a
        // failed call must leave the run exactly where plain `recent` would.
        let mut summary = None;
        if strategy == ContextStrategy::Summary {
            let evicted: Vec<ChatMessage> =
                self.messages[plan.protected..plan.evict_end].to_vec();
            match self.summarize_evicted(&evicted).await {
                Some(text) => summary = Some(ChatMessage::user(
                    format!("{}{}", context::SUMMARY_PREFIX, text),
                )),
                None => warn!(
                    run_id = %self.run_id,
                    "Summarisation failed; degrading to 'recent' for this compaction"
                ),
            }
        }

        self.messages.drain(plan.protected..plan.evict_end);

        let mut after_tokens = plan.after_tokens;
        let summarized = summary.is_some();
        if let Some(message) = summary {
            // Calibrated, like every other figure here — `plan.after_tokens`
            // is scaled, so adding a raw estimate would mix units and report a
            // number in neither.
            after_tokens += context::calibrate(
                api::tokens::estimate_message(&message),
                self.estimate_scale,
            );
            self.messages.insert(plan.protected, message);
        }

        debug_assert!(
            after_tokens <= budget || !plan.fits,
            "compaction left {after_tokens} tokens against a budget of {budget}"
        );

        let payload = serde_json::json!({
            "strategy": strategy.as_str(),
            "before_tokens": plan.before_tokens,
            "after_tokens": after_tokens,
            "budget_tokens": budget,
            "evicted_messages": plan.evicted_count(),
            "summarized": summarized,
        });
        self.persist_event("context_compacted", &payload).await;
        self.emit(RunEvent::ContextCompacted {
            run_id: self.run_id.clone(),
            strategy: strategy.as_str().to_string(),
            before_tokens: plan.before_tokens,
            after_tokens,
            budget_tokens: budget,
            evicted_messages: plan.evicted_count(),
            summarized,
        });

        Ok(())
    }

    /// Summarise the messages about to be dropped, with a cheap capped call.
    ///
    /// `None` on any failure — a summary is a nicety, and losing one is not
    /// worth losing the run. The caller degrades to plain eviction.
    async fn summarize_evicted(&mut self, evicted: &[ChatMessage]) -> Option<String> {
        if evicted.is_empty() {
            return None;
        }

        let mut config = self.config.clone();
        // Never spend more on a summary than the run itself is allowed to
        // write, and never more than the cap either way.
        config.max_tokens = SUMMARY_MAX_OUTPUT_TOKENS.min(self.config.max_tokens);
        config.temperature = None;
        config.system_prompt = Some(context::SUMMARY_SYSTEM_PROMPT.to_string());

        let prompt = ChatMessage::user(context::summary_prompt(evicted));

        // No tools: the summariser reads a transcript, it does not act.
        let mut stream = match self
            .provider
            .stream_completion(vec![prompt], Vec::new(), config)
            .await
        {
            Ok(stream) => stream,
            Err(e) => {
                warn!(run_id = %self.run_id, "Summarisation call failed: {e}");
                return None;
            }
        };

        let mut text = String::new();
        let mut usage = None;
        while let Some(event) = stream.next().await {
            match event {
                Ok(StreamEvent::TextDelta(token)) => text.push_str(&token),
                Ok(StreamEvent::Done { usage: u, .. }) => {
                    usage = u;
                    break;
                }
                Ok(StreamEvent::Error(msg)) => {
                    warn!(run_id = %self.run_id, "Summarisation stream error: {msg}");
                }
                Ok(StreamEvent::ToolCallDelta(_)) => {}
                Err(e) => {
                    warn!(run_id = %self.run_id, "Summarisation stream failed: {e}");
                    return None;
                }
            }
        }
        drop(stream);

        // The summarisation call is real spend and belongs in the run's total.
        // It deliberately does *not* touch `last_usage`: that figure is the
        // size of the agent conversation last sent, and this call sent
        // something else entirely. Folding it in would corrupt the calibration
        // the budget depends on.
        if let Some(u) = usage {
            self.usage_total += u;
        }

        let text = text.trim().to_string();
        if text.is_empty() {
            warn!(run_id = %self.run_id, "Summarisation returned nothing");
            return None;
        }
        Some(text)
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Build a concise text summary of the run's assistant output for semantic storage.
    fn generate_run_summary(&self) -> String {
        use api::MessageRole;
        let last_assistant = self.messages.iter().rev().find_map(|m| {
            if m.role == MessageRole::Assistant && !m.content.is_empty() {
                Some(m.content.as_str())
            } else {
                None
            }
        });

        let output = last_assistant.unwrap_or("(no output)");
        let summary = format!(
            "Run for story '{}' (profile '{}'): {}",
            self.story_id, self.agent_profile_id, output
        );

        // Cap at 1500 bytes so embeddings are over meaningful but bounded
        // content. The cut must land on a char boundary — slicing a String at a
        // fixed byte offset panics mid-codepoint, and assistant output is
        // routinely non-ASCII.
        if summary.len() > 1500 {
            let mut cut = 1497;
            while cut > 0 && !summary.is_char_boundary(cut) {
                cut -= 1;
            }
            format!("{}…", &summary[..cut])
        } else {
            summary
        }
    }

    /// Fold one provider call's usage into the run.
    ///
    /// Summed, never overwritten: a run makes one call per iteration and the
    /// cost of the run is all of them, not the last one.
    fn record_usage(&mut self, usage: Usage) {
        self.usage_total += usage;
        self.last_usage = Some(usage);
        self.calibrate_estimate(usage);
        debug!(
            run_id = %self.run_id,
            input = usage.input_tokens,
            output = usage.output_tokens,
            cache_read = usage.cache_read_input_tokens,
            "Provider reported usage"
        );
    }

    /// Correct the token estimator against what the provider actually counted.
    ///
    /// `Usage::total_input_tokens()` is the real size of the context that was
    /// just sent — the same content `last_raw_estimate` guessed at — so their
    /// ratio is how wrong the estimator is for this model and this kind of
    /// content. Applying it keeps a character-based guess from either
    /// compacting a conversation that would have fit or, worse, letting one
    /// through that will not.
    ///
    /// One observation, clamped, rather than a running average: an agent's
    /// content changes shape through a run, and the most recent call is the
    /// best evidence about the next one.
    fn calibrate_estimate(&mut self, usage: Usage) {
        let real = usage.total_input_tokens();
        let Some(estimated) = self.last_raw_estimate.filter(|e| *e > 0) else {
            return;
        };
        if real == 0 {
            // A provider that reports no input tokens is not evidence that the
            // request was empty.
            return;
        }

        let scale = (real as f64 / estimated as f64).clamp(MIN_ESTIMATE_SCALE, MAX_ESTIMATE_SCALE);
        if (scale - self.estimate_scale).abs() > f64::EPSILON {
            debug!(
                run_id = %self.run_id,
                estimated,
                real,
                scale,
                "Recalibrated the token estimator against reported usage"
            );
        }
        self.estimate_scale = scale;
    }

    fn emit(&self, event: RunEvent) {
        match serde_json::to_value(&event) {
            Ok(payload) => self.app.emit_event("run-event", payload),
            Err(e) => warn!(run_id = %self.run_id, "Failed to serialize run-event: {e}"),
        }
    }

    // -----------------------------------------------------------------------
    // Approvals
    // -----------------------------------------------------------------------

    /// Wait for the user's decision on a gated tool call.
    ///
    /// With no `approval_timeout` this waits for as long as it takes. That is
    /// the behaviour the feature exists for: the old fixed five-minute limit
    /// denied the call and failed the tool the moment the user stepped away,
    /// which turned "nobody was watching" into "the user said no" — a decision
    /// they never made, recorded against their name.
    ///
    /// Whatever ends the wait, the `approval_requests` row is left settled: a
    /// row still reading `pending` with nothing waiting on it would sit in the
    /// Approvals UI offering a decision that can no longer reach anything.
    async fn await_approval(
        &self,
        approval_id: &str,
        rx: tokio::sync::oneshot::Receiver<bool>,
    ) -> ApprovalOutcome {
        let decision = match self.approval_timeout {
            Some(limit) => tokio::time::timeout(limit, rx).await,
            None => Ok(rx.await),
        };

        match decision {
            Ok(Ok(true)) => ApprovalOutcome::Approved,
            Ok(Ok(false)) => ApprovalOutcome::Denied,
            // The sender was dropped without a decision — the gate entry was
            // cleared out from under us.
            Ok(Err(_)) => {
                self.settle_unanswered(approval_id, ApprovalOutcome::Abandoned).await;
                ApprovalOutcome::Abandoned
            }
            Err(_) => {
                self.approval_gate.cancel(approval_id);
                self.settle_unanswered(approval_id, ApprovalOutcome::Expired).await;
                ApprovalOutcome::Expired
            }
        }
    }

    /// Close out an approval nobody answered.
    ///
    /// The status is `expired`, not `rejected`. `rejected` means the user
    /// looked at the request and said no; conflating the two makes an audit of
    /// what a user actually refused unreadable, and it is the same conflation
    /// that made the old timeout so misleading.
    async fn settle_unanswered(&self, approval_id: &str, outcome: ApprovalOutcome) {
        let _ = sqlx::query(
            "UPDATE approval_requests \
             SET status = ?, rejection_reason = ?, \
                 decided_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
             WHERE id = ? AND status = 'pending'",
        )
        .bind(outcome.as_str())
        .bind(outcome.explanation())
        .bind(approval_id)
        .execute(&self.db)
        .await;
    }

    // -----------------------------------------------------------------------
    // Retrying a failed provider call
    // -----------------------------------------------------------------------

    /// Ask the provider for a completion, trying again if the failure looks
    /// transient.
    ///
    /// The retry happens here, around the one call that can fail transiently,
    /// rather than around the whole run. The conversation so far, the tool
    /// work already done and the run's worktree all survive a retry, so a rate
    /// limit fifteen iterations in costs the wait and nothing else. Retrying
    /// the whole run would discard that worktree and pay for those fifteen
    /// iterations a second time to reach the same place.
    ///
    /// The other two ways a run can fail — exhausting `max_iterations`, and
    /// exceeding the context budget under `context_strategy = "full"` — are
    /// not reached from here at all. Both are deterministic: a fresh attempt
    /// meets the same ceiling with the same prompt.
    async fn stream_with_retries(
        &self,
        tool_defs: Vec<api::ToolDefinition>,
    ) -> std::result::Result<api::provider::EventStream, RunFailure> {
        let mut retries_used = 0u32;

        loop {
            let attempt = self
                .provider
                .stream_completion(self.messages.clone(), tool_defs.clone(), self.config.clone())
                .await;

            let error = match attempt {
                Ok(stream) => return Ok(stream),
                Err(e) => e,
            };

            // Classify before formatting: `error` is a typed `ApiError` here
            // and a string everywhere after. Discarding it at this line is the
            // defect this story exists to undo.
            let kind = api::retry::classify(&error);
            let message = format!("LLM call failed: {error}");

            if !kind.is_transient() {
                return Err(RunFailure { message, kind });
            }
            if retries_used >= self.max_retries {
                return Err(RunFailure {
                    message: format!(
                        "{message} (gave up after {} attempt{})",
                        retries_used + 1,
                        if retries_used == 0 { "" } else { "s" }
                    ),
                    kind,
                });
            }

            retries_used += 1;
            let delay = kind.wait().unwrap_or_else(|| self.backoff_delay(retries_used));

            warn!(
                run_id = %self.run_id,
                attempt = retries_used,
                delay_secs = delay.as_secs_f64(),
                "Retrying after a transient provider failure: {message}"
            );
            self.emit(RunEvent::Retrying {
                run_id: self.run_id.clone(),
                attempt: retries_used,
                max_retries: self.max_retries,
                reason: message.clone(),
                delay_secs: delay.as_secs_f64(),
                provider_requested_delay: kind.wait().is_some(),
            });
            self.persist_event(
                "retry",
                &serde_json::json!({
                    "attempt": retries_used,
                    "maxRetries": self.max_retries,
                    "reason": message,
                    "delaySecs": delay.as_secs_f64(),
                    "providerRequestedDelay": kind.wait().is_some(),
                }),
            )
            .await;

            if self.sleep_unless_cancelled(delay).await {
                // Stopping is the user's decision and outranks the retry.
                return Err(RunFailure {
                    message: format!("{message} (cancelled while waiting to retry)"),
                    kind: api::FailureKind::Permanent,
                });
            }
        }
    }

    /// How long to wait before retry number `attempt` when the provider did
    /// not say.
    fn backoff_delay(&self, attempt: u32) -> std::time::Duration {
        backoff_delay(&self.run_id, attempt)
    }

    /// Wait `total`, giving up early if the run is cancelled.
    ///
    /// Returns whether the run was cancelled. `CancelFlag` is a polled bool
    /// rather than something awaitable, so the wait is taken in slices — a run
    /// sleeping out a sixty-second `retry-after` has to notice a stop button,
    /// not finish its nap first.
    async fn sleep_unless_cancelled(&self, total: std::time::Duration) -> bool {
        const SLICE: std::time::Duration = std::time::Duration::from_millis(100);

        let mut remaining = total;
        while !remaining.is_zero() {
            if self.cancel.is_cancelled() {
                return true;
            }
            let step = remaining.min(SLICE);
            tokio::time::sleep(step).await;
            remaining -= step;
        }
        self.cancel.is_cancelled()
    }

    // -----------------------------------------------------------------------
    // Board bookkeeping
    // -----------------------------------------------------------------------

    /// Move this run's story to the status its outcome implies.
    ///
    /// Every run settles its own card, pipeline steps included. A step owns a
    /// story of its own: `validate_pipeline_config` rejects a step that
    /// references the pipeline's story, and rejects two steps sharing one, so
    /// a step can never reach the parent's card. The parent's is moved once,
    /// by `pipeline::settle_pipeline_story`, when the whole pipeline finishes.
    ///
    /// Unlike the rest of `finish_run` this one checks its result. Every other
    /// statement here is best-effort because losing it costs a number in a
    /// report; losing this one leaves a card claiming to be in flight forever,
    /// which is the whole defect.
    async fn settle_story(&self, status: &str) {
        // Retries happen inside the run, so by the time this is reached the
        // chain is over either way. The card was claimed once at the start and
        // is settled once here — it never returns to `ready` between attempts,
        // and a continuous-mode profile never sees it mid-chain.
        if !db::story_status::auto_advance_enabled(&self.db).await {
            return;
        }

        let outcome = db::story_status::RunOutcome::from_run_status(status);

        match db::story_status::settle_story(&self.db, &self.story_id, outcome).await {
            Ok(true) => {
                let to = outcome.story_status();
                info!(run_id = %self.run_id, story_id = %self.story_id, "Story moved to {to}");
                // The card moved in SQL. Without this the open board goes on
                // showing the run as in progress until the app restarts — the
                // routine end of every successful run, invisible.
                self.app
                    .emit_event(db::story_status::STORIES_CHANGED_EVENT, serde_json::Value::Null);
                // On the run's own timeline, so a user can see why the card
                // moved instead of inferring it from a timestamp.
                self.persist_event(
                    db::story_status::TRANSITION_EVENT_TYPE,
                    &db::story_status::transition_payload(
                        &self.story_id,
                        to,
                        &format!("the run finished with status '{status}'"),
                    ),
                )
                .await;
            }
            // Not an error: the card was somewhere else, which means somebody
            // decided that on purpose and this had nothing to correct.
            Ok(false) => debug!(
                run_id = %self.run_id,
                story_id = %self.story_id,
                "Story was not in_progress; leaving it where it is"
            ),
            Err(e) => error!(
                run_id = %self.run_id,
                story_id = %self.story_id,
                "Failed to move the story off in_progress: {e}"
            ),
        }
    }

    // -----------------------------------------------------------------------
    // Notifications
    // -----------------------------------------------------------------------

    /// Best-effort call to the user, through whatever sink was injected.
    ///
    /// Failure is logged and dropped: a run must not fail because the OS
    /// refused a toast, and a suppressed category reports itself as an error
    /// here too. The one caller that must know whether delivery happened is
    /// `send_notification`, which asks the sink directly.
    async fn notify(&self, category: tools::NotificationCategory, title: &str, body: &str) {
        let Some(notifier) = self.notifier.as_ref() else { return };
        if let Err(e) = notifier.notify(category, title, body).await {
            debug!(run_id = %self.run_id, "Notification not delivered: {e}");
        }
    }

    /// The story's title, for a notification body that says which work it is
    /// about. Falls back to a generic phrase rather than failing the caller.
    async fn story_title(&self) -> String {
        sqlx::query_scalar::<_, String>("SELECT title FROM stories WHERE id = ?")
            .bind(&self.story_id)
            .fetch_optional(&self.db)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "A RustyAgent run".to_string())
    }

    /// Persist a run event to the `run_events` table.
    ///
    /// Non-critical event types (`message`, `tool_call`, `tool_result`, `thought`) are
    /// skipped when `track_history` is `false`. Critical events (`error`,
    /// `approval_request`, `approval_response`, `complete`, `cancelled`, `failed`,
    /// `context_compacted`) are always written regardless of the flag.
    async fn persist_event(&self, event_type: &str, payload: &serde_json::Value) {
        // `context_compacted` is critical: it records that history was
        // discarded. An operator debugging an agent that "forgot" something
        // needs to see it even on a story with history tracking off, which is
        // exactly the configuration where the messages themselves are gone.
        let critical = matches!(
            event_type,
            "error" | "approval_request" | "approval_response"
                | "complete" | "cancelled" | "failed" | "context_compacted"
                // Why a card moved on the board is not a detail of the
                // conversation, and it is the only record of an automatic
                // transition.
                | "story_status"
                // Whether a run was isolated decides whether its changes can be
                // reverted at all. That must survive `track_history = false`.
                | "isolation"
                // Why a run sat still for thirty seconds. Without it the pause
                // is indistinguishable from a hang.
                | "retry"
        );
        if !critical && !self.track_history {
            return;
        }

        let event_id = Uuid::new_v4().to_string();
        let seq = self.sequence_counter.fetch_add(1, Ordering::SeqCst);

        // Map the JSON payload to the structured columns expected by `run_events`.
        let (role, content, tool_name, tool_input, tool_output, is_error): (
            Option<String>, Option<String>, Option<String>,
            Option<String>, Option<String>, i64,
        ) = match event_type {
            "token" | "message" => {
                let c = payload["content"].as_str().map(|s| s.to_string());
                (Some("assistant".to_string()), c, None, None, None, 0)
            }
            "tool_call" => {
                let t = payload["tool_name"].as_str().map(|s| s.to_string());
                let i = payload.get("input").map(|v| v.to_string());
                (None, None, t, i, None, 0)
            }
            // The timeline renders `content` verbatim, so lift the message out
            // of the payload rather than showing the operator raw JSON.
            "error" => {
                let m = payload["message"].as_str().map(|s| s.to_string());
                (None, m.or_else(|| Some(payload.to_string())), None, None, None, 1)
            }
            // Same shape as `error` but not a failure: the run continues, the
            // operator just has to know it was not isolated.
            "isolation" => {
                let m = payload["message"].as_str().map(|s| s.to_string());
                (None, m.or_else(|| Some(payload.to_string())), None, None, None, 0)
            }
            "tool_result" => {
                let t  = payload["tool_name"].as_str().map(|s| s.to_string());
                let o  = payload["output"].as_str().map(|s| s.to_string());
                let err = if payload["is_error"].as_bool().unwrap_or(false) { 1 } else { 0 };
                (None, None, t, None, o, err)
            }
            _ => {
                // complete, cancelled, failed, error, approval_* → store JSON as content
                (None, Some(payload.to_string()), None, None, None, 0)
            }
        };

        let _ = sqlx::query(
            "INSERT INTO run_events \
             (id, run_id, event_type, role, content, tool_name, tool_input, tool_output, is_error, sequence_num) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&event_id)
        .bind(&self.run_id)
        .bind(event_type)
        .bind(role)
        .bind(content)
        .bind(tool_name)
        .bind(tool_input)
        .bind(tool_output)
        .bind(is_error)
        .bind(seq as i64)
        .execute(&self.db)
        .await;
    }

    /// Narrow a `u64` token counter for SQLite, which has no unsigned integer
    /// type, saturating instead of wrapping.
    ///
    /// `as i64` reinterprets the bits, so any value above `i64::MAX` lands in
    /// the database negative. `Usage` sums with `saturating_add` specifically so
    /// that "a bad estimate beats a negative one" — which makes the wrap
    /// reachable rather than hypothetical, since a saturated total sits at
    /// `u64::MAX` and `u64::MAX as i64` is `-1`. Clamping carries that existing
    /// decision through to the one place it was not applied.
    fn clamp_to_i64(value: u64) -> i64 {
        i64::try_from(value).unwrap_or(i64::MAX)
    }

    /// Record how far the loop has got, best-effort.
    ///
    /// One tiny UPDATE per LLM iteration, which is nothing beside the provider
    /// call it accompanies, and it is the only reason an interrupted run can
    /// report the iterations it actually performed.
    ///
    /// `iterations` counts iterations *entered*: the figure is bumped as an
    /// iteration begins, so a run cut off mid-iteration is credited with the
    /// work that iteration had already done rather than discarding it.
    async fn persist_iteration_count(&self, iterations: u32) {
        let _ = sqlx::query("UPDATE story_runs SET iteration_count = ? WHERE id = ?")
            .bind(i64::from(iterations))
            .bind(&self.run_id)
            .execute(&self.db)
            .await;
    }

    /// Mark the run terminal and write down what it spent.
    ///
    /// Called on every exit path — completion, failure, cancellation — because
    /// a run that died halfway still consumed the tokens it consumed, and a
    /// cost that only lands on the happy path is an undercount. `iterations`
    /// rides along in the same statement rather than as a second query, so the
    /// figure can never disagree with the status it was recorded against.
    async fn finish_run(&self, status: &str, iterations: u32) {
        let usage = self.usage_total;
        // `None` for a model the price table does not know. COALESCE then
        // leaves the column at its default rather than asserting $0.00 — the
        // token counts are still recorded, only the price is withheld.
        let cost = api::pricing::estimate_cost_usd(&self.config.model, &usage);
        if cost.is_none() && !usage.is_zero() {
            debug!(
                run_id = %self.run_id,
                model = %self.config.model,
                "No price table entry; recording tokens without a cost estimate"
            );
        }

        let _ = sqlx::query(
            "UPDATE story_runs \
             SET status = ?, finished_at = CURRENT_TIMESTAMP, \
                 iteration_count = ?, \
                 input_tokens = ?, output_tokens = ?, \
                 cache_read_input_tokens = ?, cache_creation_input_tokens = ?, \
                 estimated_cost_usd = COALESCE(?, estimated_cost_usd) \
             WHERE id = ?"
        )
        .bind(status)
        .bind(i64::from(iterations))
        .bind(Self::clamp_to_i64(usage.input_tokens))
        .bind(Self::clamp_to_i64(usage.output_tokens))
        .bind(Self::clamp_to_i64(usage.cache_read_input_tokens))
        .bind(Self::clamp_to_i64(usage.cache_creation_input_tokens))
        .bind(cost)
        .bind(&self.run_id)
        .execute(&self.db)
        .await;

        // Commit whatever the run wrote onto its own branch. This is what makes
        // accept and revert cheap later: the changes become one object the user
        // can squash-merge or throw away, instead of a pile of loose edits.
        //
        // The commit lands in the run's private worktree, so it cannot touch
        // the user's branch, index, or working tree.
        if let Some(wt) = &self.worktree {
            let message = format!(
                "RustyAgent run {} on story {} ({status})",
                &self.run_id, &self.story_id
            );
            match crate::worktree::commit_all(&wt.path, &message) {
                Ok(Some(sha)) => {
                    info!(run_id = %self.run_id, branch = %wt.branch, "Committed run output");
                    let _ = sqlx::query("UPDATE story_runs SET after_sha = ? WHERE id = ?")
                        .bind(&sha)
                        .bind(&self.run_id)
                        .execute(&self.db)
                        .await;
                }
                Ok(None) => {
                    debug!(run_id = %self.run_id, "Run changed no files; nothing to commit");
                }
                Err(e) => {
                    warn!(run_id = %self.run_id, "Could not commit the run's worktree: {e}");
                }
            }
        }

        // Capture git diff against the before-SHA (best-effort). Runs against
        // the run's own workspace root, which is the worktree when isolated.
        if let (Some(sha), Some(root)) = (self.before_sha.as_deref(), self.workspace_root.as_deref()) {
            if let Some(diff) = crate::git::get_diff_since(root, sha) {
                let _ = sqlx::query(
                    "UPDATE story_runs SET diff_output = ? WHERE id = ?"
                )
                .bind(&diff)
                .bind(&self.run_id)
                .execute(&self.db)
                .await;
            }
        }

        if self.event_retention_runs > 0 {
            self.prune_old_run_events().await;
        }

        // Move the story's card off `in_progress` to say how this ended.
        //
        // Placed after the run row is written so the two agree: a card in
        // `review` is always backed by a `story_runs` row that says `done`.
        self.settle_story(status).await;

        // Tell the user the run is over. Not on `cancelled`: the user stopped
        // it themselves, so they are at the desk and already know.
        //
        // One notification per run, at the end — never per token or per tool
        // call. A stream of toasts gets the whole feature muted, and a muted
        // feature is exactly today's behaviour.
        let title = self.story_title().await;
        // `done`, not `completed` — see migration 20260410000016, which had to
        // repair exactly that drift once already.
        match status {
            "done" => {
                self.notify(
                    tools::NotificationCategory::RunCompleted,
                    "Run finished",
                    &format!("{title} completed after {iterations} iterations."),
                )
                .await;
            }
            "failed" => {
                self.notify(
                    tools::NotificationCategory::RunFailed,
                    "Run failed",
                    &format!("{title} failed after {iterations} iterations."),
                )
                .await;
            }
            _ => {}
        }
    }

    /// Delete `run_events` rows belonging to runs older than the retention cap.
    /// Also clears `diff_output` for those same runs to reclaim storage.
    /// The `story_runs` rows themselves are kept for audit purposes.
    async fn prune_old_run_events(&self) {
        let cap = self.event_retention_runs as i64;
        let _ = sqlx::query(
            "DELETE FROM run_events \
             WHERE run_id IN ( \
                 SELECT id FROM story_runs \
                 WHERE story_id = ? \
                 ORDER BY started_at DESC \
                 LIMIT -1 OFFSET ? \
             )"
        )
        .bind(&self.story_id)
        .bind(cap)
        .execute(&self.db)
        .await;

        let _ = sqlx::query(
            "UPDATE story_runs SET diff_output = NULL \
             WHERE story_id = ? AND id NOT IN ( \
                 SELECT id FROM story_runs \
                 WHERE story_id = ? \
                 ORDER BY started_at DESC \
                 LIMIT ? \
             )"
        )
        .bind(&self.story_id)
        .bind(&self.story_id)
        .bind(cap)
        .execute(&self.db)
        .await;
    }
}
