// Multi-agent pipeline engine: sequential handoff, parallel fan-out, supervisor delegation.
// RUSTYAGE-10 implementation.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

#[cfg(test)]
mod pipeline_tests;

use anyhow::{anyhow, Context, Result};
use dashmap::DashMap;
use db::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tauri::Manager;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

#[cfg(test)]
mod story_tests;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Pipeline configuration — stored as JSON in stories.pipeline_config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineConfig {
    /// "sequential" | "parallel"
    pub mode: String,
    pub steps: Vec<PipelineStep>,
    /// Maximum recursion depth when agents spawn further subtasks.
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineStep {
    /// Human-readable label shown in the UI.
    pub label: String,
    /// Story to run for this step.
    pub story_id: String,
    /// Agent profile to execute the step.
    pub agent_id: String,
}

fn default_max_depth() -> u32 { 5 }

// ---------------------------------------------------------------------------
// Runtime progress types returned to the frontend
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineProgress {
    pub pipeline_run_id: String,
    pub story_id: String,
    pub mode: String,
    /// "running" | "done" | "failed" | "cancelled"
    pub status: String,
    pub steps: Vec<StepProgress>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepProgress {
    pub index: usize,
    pub label: String,
    pub story_id: String,
    pub agent_id: String,
    /// run_id of the underlying story_run (set once the step starts)
    pub run_id: Option<String>,
    /// "pending" | "running" | "done" | "failed"
    pub status: String,
}

// ---------------------------------------------------------------------------
// PipelineState — Tauri app-managed state
// ---------------------------------------------------------------------------

pub struct PipelineState {
    /// pipeline_run_id → live progress snapshot
    runs: DashMap<String, PipelineProgress>,
    /// pipeline_run_id → task JoinHandle (so we can cancel)
    tasks: DashMap<String, JoinHandle<()>>,
    /// Shared run registry from the main binary (run_id → CancelFlag)
    pub run_registry: Arc<Mutex<HashMap<String, runtime::CancelFlag>>>,
}

impl PipelineState {
    pub fn new(run_registry: Arc<Mutex<HashMap<String, runtime::CancelFlag>>>) -> Self {
        Self {
            runs: DashMap::new(),
            tasks: DashMap::new(),
            run_registry,
        }
    }
}

// ---------------------------------------------------------------------------
// Inline settings loader (mirrors scheduler's pattern)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct LocalSettings {
    anthropic_api_key: Option<String>,
    openrouter_api_key: Option<String>,
    deepseek_api_key: Option<String>,
    ollama_base_url: Option<String>,
    event_retention_runs: Option<u32>,
    /// How many parallel pipeline steps may execute at once.
    max_parallel_steps: Option<u32>,
}

fn load_settings(app: &tauri::AppHandle) -> LocalSettings {
    let path = db::paths::with_override(app.path().app_data_dir().ok())
        .expect("app data dir")
        .join("settings.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Concurrency ceiling
// ---------------------------------------------------------------------------

/// Steps allowed to run at once when nothing is configured.
///
/// Four rather than "all of them": each in-flight step is a full checkout on
/// disk and an independent stream of provider calls, so an unbounded fan-out
/// is unbounded disk and unbounded spend.
pub const DEFAULT_MAX_PARALLEL_STEPS: usize = 4;


/// Resolve the parallel-step ceiling from the two places it can be set.
///
/// The active workspace's override wins over the global `settings.json` value,
/// which wins over [`DEFAULT_MAX_PARALLEL_STEPS`]. A configured zero would mean
/// "run nothing", which is never what anyone means, so the floor is one.
pub fn resolve_parallel_limit(workspace: Option<u32>, global: Option<u32>) -> usize {
    workspace
        .or(global)
        .map(|n| n.max(1) as usize)
        .unwrap_or(DEFAULT_MAX_PARALLEL_STEPS)
}

/// Tell the open board that a card moved underneath it.
fn emit_stories_changed(app: &tauri::AppHandle) {
    use tauri::Emitter;
    if let Err(e) = app.emit(db::story_status::STORIES_CHANGED_EVENT, ()) {
        warn!("Failed to announce a board change: {e}");
    }
}

/// Claim a pipeline's own story card as the pipeline starts.
///
/// This used to be a bare `UPDATE stories SET status = 'in_progress'`, which
/// answered to nothing: not the workspace toggle, and not the rule that an
/// automatic transition may not overwrite a status somebody chose. A pipeline
/// started against a card a human had moved to `blocked` dragged it back into
/// the in-progress column.
///
/// Split out for the same reason as [`settle_pipeline_story`]: the function
/// around it needs a concrete `tauri::AppHandle` and this does not.
/// Returns whether the card actually moved, so the caller can announce it.
/// The announcement is the caller's because it needs an `AppHandle`, and
/// taking one here would put this function back out of reach of a test.
pub(crate) async fn claim_pipeline_story(db: &DbPool, story_id: &str) -> bool {
    if !db::story_status::auto_advance_enabled(db).await {
        return false;
    }
    match db::story_status::claim_story(db, story_id).await {
        Ok(true) => {
            debug!(story_id = %story_id, "Pipeline story claimed");
            true
        }
        Ok(false) => {
            debug!(
                story_id = %story_id,
                "Pipeline story was not ready; leaving it where it is"
            );
            false
        }
        Err(e) => {
            error!(story_id = %story_id, "Failed to claim the pipeline story: {e}");
            false
        }
    }
}

/// Move a finished pipeline's own story card.
///
/// Each step owns a separate story and settles its own card through
/// `finish_run`; this is the *parent's*, and it moves once — here, on the
/// whole pipeline's outcome — rather than on whichever step happened to finish
/// last.
///
/// Split out of the spawned completion task so it can be tested: the task
/// around it needs a concrete `tauri::AppHandle`, and this does not.
/// Returns whether the card actually moved — see [`claim_pipeline_story`] for
/// why the announcement belongs to the caller.
pub(crate) async fn settle_pipeline_story(
    db: &DbPool,
    pipeline_run_id: &str,
    story_id: &str,
    final_status: &str,
) -> bool {
    if !db::story_status::auto_advance_enabled(db).await {
        return false;
    }

    let outcome = db::story_status::RunOutcome::from_run_status(final_status);
    match db::story_status::settle_story(db, story_id, outcome).await {
        Ok(true) => {
            info!(
                story_id = %story_id,
                "Pipeline story moved to {}",
                outcome.story_status()
            );
            // On the pipeline run's own timeline, so this move is attributable
            // to the run that caused it exactly as a single run's is.
            if let Err(e) = db::story_status::record_transition(
                db,
                pipeline_run_id,
                story_id,
                outcome.story_status(),
                &format!("the pipeline finished with status '{final_status}'"),
            )
            .await
            {
                warn!(story_id = %story_id, "Could not record the story transition: {e}");
            }
            true
        }
        // Not an error: somebody moved the card deliberately, and that
        // outranks this.
        Ok(false) => {
            debug!(
                story_id = %story_id,
                "Pipeline story was not in_progress; leaving it where it is"
            );
            false
        }
        Err(e) => {
            error!(
                story_id = %story_id,
                "Failed to move the pipeline story off in_progress: {e}"
            );
            false
        }
    }
}

/// Read `max_parallel_steps` from the active workspace's settings override.
pub async fn workspace_parallel_limit(db: &DbPool) -> Option<u32> {
    let row = sqlx::query(
        "SELECT ws.settings_json FROM workspace_settings ws          JOIN workspaces w ON w.id = ws.workspace_id          ORDER BY w.last_opened_at DESC LIMIT 1",
    )
    .fetch_optional(db)
    .await
    .ok()??;
    let json: String = row.try_get("settings_json").ok()?;
    serde_json::from_str::<serde_json::Value>(&json)
        .ok()?
        .get("max_parallel_steps")?
        .as_u64()
        .map(|n| n.min(u32::MAX as u64) as u32)
}

/// Run `count` tasks with at most `limit` of them in flight at any moment.
///
/// Results come back in index order regardless of completion order. Every task
/// is spawned immediately, but a task does nothing until it holds a permit —
/// so the ceiling bounds the work, the disk, and the API spend, not just the
/// number of futures.
pub(crate) async fn run_bounded<T, F, Fut>(
    limit: usize,
    count: usize,
    make: F,
) -> Vec<std::result::Result<T, tokio::task::JoinError>>
where
    F: Fn(usize) -> Fut,
    Fut: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let semaphore = Arc::new(tokio::sync::Semaphore::new(limit.max(1)));
    let mut handles = Vec::with_capacity(count);

    for index in 0..count {
        let task = make(index);
        let semaphore = semaphore.clone();
        handles.push(tokio::spawn(async move {
            // Holding the permit until the task returns is what enforces the
            // ceiling. `acquire_owned` only fails on a closed semaphore, and
            // this one is created in `run_bounded` and never closed — but bind
            // the permit rather than the `Result`, so that if someone later
            // does close it this fails loudly instead of quietly running the
            // whole fan-out unbounded.
            let _permit = semaphore
                .acquire_owned()
                .await
                .expect("run_bounded's semaphore is never closed");
            task.await
        }));
    }

    let mut results = Vec::with_capacity(count);
    for handle in handles {
        results.push(handle.await);
    }
    results
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Start a pipeline run for a pipeline-type story.
///
/// Returns the `pipeline_run_id` which is also the root `story_runs.id`.
pub async fn start_pipeline(
    story_id: String,
    agent_profile_id: String,
    pipeline: Arc<PipelineState>,
    db: DbPool,
    app: tauri::AppHandle,
) -> Result<String, String> {
    // Load the pipeline config from the story
    let config = load_pipeline_config(&story_id, &db).await
        .map_err(|e| format!("Failed to load pipeline config: {e}"))?;

    // Cycle / duplicate story_id detection
    validate_pipeline_config(&story_id, &config)
        .map_err(|e| format!("Invalid pipeline config: {e}"))?;

    // Create the root story_run row that represents the whole pipeline.
    let pipeline_run_id = Uuid::new_v4().to_string();
    sqlx::query(
        // `owner_instance_id` marks the row as belonging to this launch of the
        // app, so the startup sweep in `db::recovery` leaves a live pipeline
        // alone and only reconciles one a previous process died holding.
        "INSERT INTO story_runs \
             (id, story_id, agent_profile_id, status, started_at, owner_instance_id) \
         VALUES (?, ?, ?, 'running', CURRENT_TIMESTAMP, ?)",
    )
    .bind(&pipeline_run_id)
    .bind(&story_id)
    .bind(&agent_profile_id)
    .bind(db::recovery::instance_id())
    .execute(&db)
    .await
    .map_err(|e| format!("DB error inserting pipeline run: {e}"))?;

    // Seed pipeline_step_runs rows (all 'pending')
    for (i, step) in config.steps.iter().enumerate() {
        let step_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO pipeline_step_runs \
             (id, pipeline_run_id, step_index, story_id, agent_profile_id, status) \
             VALUES (?, ?, ?, ?, ?, 'pending')",
        )
        .bind(&step_id)
        .bind(&pipeline_run_id)
        .bind(i as i64)
        .bind(&step.story_id)
        .bind(&step.agent_id)
        .execute(&db)
        .await
        .map_err(|e| format!("DB error inserting pipeline step: {e}"))?;
    }

    // Build initial progress snapshot
    let progress = PipelineProgress {
        pipeline_run_id: pipeline_run_id.clone(),
        story_id: story_id.clone(),
        mode: config.mode.clone(),
        status: "running".to_string(),
        steps: config
            .steps
            .iter()
            .enumerate()
            .map(|(i, s)| StepProgress {
                index: i,
                label: s.label.clone(),
                story_id: s.story_id.clone(),
                agent_id: s.agent_id.clone(),
                run_id: None,
                status: "pending".to_string(),
            })
            .collect(),
    };
    pipeline.runs.insert(pipeline_run_id.clone(), progress);

    // Claim the card before the executor starts, not after it is spawned. A
    // pipeline short enough to finish first would otherwise settle a card
    // still sitting in `ready` — a no-op — and then be handed `in_progress` by
    // a claim arriving behind it, leaving it stuck exactly as this feature
    // exists to prevent.
    if claim_pipeline_story(&db, &story_id).await {
        emit_stories_changed(&app);
    }

    // Spawn the async pipeline executor
    let pid = pipeline_run_id.clone();
    let story_id_for_settle = story_id.clone();
    let pipeline_clone = pipeline.clone();
    let db_clone = db.clone();
    let app_clone = app.clone();
    // Kept past the executor, which consumes `app_clone`, so the completion
    // block can still announce the card's move.
    let app_for_board = app.clone();

    let handle = tokio::spawn(async move {
        let result = match config.mode.as_str() {
            "parallel" => run_parallel(&pid, &config, &pipeline_clone, db_clone.clone(), app_clone).await,
            _ => run_sequential(&pid, &config, &pipeline_clone, db_clone.clone(), app_clone).await,
        };

        let final_status = match result {
            Ok(()) => "done",
            Err(ref e) => {
                error!(pipeline_run_id = %pid, "Pipeline failed: {e}");
                "failed"
            }
        };

        if let Some(mut p) = pipeline_clone.runs.get_mut(&pid) {
            p.status = final_status.to_string();
        }

        if let Err(e) = sqlx::query(
            "UPDATE story_runs SET status = ?, finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(final_status)
        .bind(&pid)
        .execute(&db_clone)
        .await
        {
            error!(pipeline_run_id = %pid, "Failed to update story_run status: {e}");
        }

        if settle_pipeline_story(&db_clone, &pid, &story_id_for_settle, final_status).await {
            emit_stories_changed(&app_for_board);
        }

        pipeline_clone.tasks.remove(&pid);
        info!(pipeline_run_id = %pid, status = %final_status, "Pipeline complete");
    });

    pipeline.tasks.insert(pipeline_run_id.clone(), handle);

    Ok(pipeline_run_id)
}

/// Return a progress snapshot for a running pipeline.
pub fn get_pipeline_progress(
    pipeline_run_id: &str,
    pipeline: Arc<PipelineState>,
) -> Option<PipelineProgress> {
    pipeline.runs.get(pipeline_run_id).map(|p| p.clone())
}

/// List all active pipeline run IDs and their progress.
pub fn list_active_pipelines(pipeline: Arc<PipelineState>) -> Vec<PipelineProgress> {
    pipeline.runs.iter().map(|e| e.value().clone()).collect()
}

// ---------------------------------------------------------------------------
// Sequential executor
// ---------------------------------------------------------------------------

async fn run_sequential(
    pipeline_run_id: &str,
    config: &PipelineConfig,
    pipeline: &Arc<PipelineState>,
    db: DbPool,
    app: tauri::AppHandle,
) -> Result<()> {
    let mut context_from_previous: Option<String> = None;
    // Commit the previous step's worktree ended on. Each step branches from it
    // so that a sequential pipeline still hands work forward: isolation must
    // not turn "review what the last step wrote" into "review an empty tree".
    let mut tip: Option<String> = None;

    for (index, step) in config.steps.iter().enumerate() {
        update_step_status(pipeline_run_id, index, "running", None, None, pipeline, &db).await;

        let extra_context = context_from_previous.as_deref().map(|prev| {
            format!("\n\n## Output from previous pipeline step:\n{prev}")
        });

        let result = fire_step_run(
            &step.story_id,
            &step.agent_id,
            pipeline_run_id,
            0,
            extra_context,
            tip.clone(),
            pipeline,
            db.clone(),
            app.clone(),
        )
        .await;

        match result {
            Ok((run_id, output)) => {
                update_step_status(
                    pipeline_run_id, index, "done",
                    Some(&run_id), output.as_deref(), pipeline, &db,
                ).await;
                context_from_previous = output;
                // A step that changed nothing has no commit of its own; the
                // next one then branches from the same place this one did,
                // which is exactly right.
                tip = step_tip(&run_id, &db).await.or(tip);
            }
            Err(e) => {
                update_step_status(
                    pipeline_run_id, index, "failed",
                    None, Some(&e.to_string()), pipeline, &db,
                ).await;
                return Err(e);
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Parallel executor
// ---------------------------------------------------------------------------

async fn run_parallel(
    pipeline_run_id: &str,
    config: &PipelineConfig,
    pipeline: &Arc<PipelineState>,
    db: DbPool,
    app: tauri::AppHandle,
) -> Result<()> {
    let limit = resolve_parallel_limit(
        workspace_parallel_limit(&db).await,
        load_settings(&app).max_parallel_steps,
    );
    info!(
        pipeline_run_id,
        steps = config.steps.len(),
        limit,
        "Running pipeline steps in parallel"
    );

    let steps = config.steps.clone();
    let results = run_bounded(limit, steps.len(), |index| {
        let step = steps[index].clone();
        let pid = pipeline_run_id.to_string();
        let pipeline_c = pipeline.clone();
        let db_c = db.clone();
        let app_c = app.clone();
        async move {
            // Marked running once the step actually holds a permit, so a queued
            // step is not reported as executing while it waits its turn.
            update_step_status(&pid, index, "running", None, None, &pipeline_c, &db_c).await;
            fire_step_run(
                &step.story_id, &step.agent_id, &pid, 0, None, None, &pipeline_c, db_c, app_c,
            )
            .await
        }
    })
    .await;

    let mut any_failed = false;
    for (index, result) in results.into_iter().enumerate() {
        match result {
            Ok(Ok((run_id, output))) => {
                update_step_status(
                    pipeline_run_id, index, "done",
                    Some(&run_id), output.as_deref(), pipeline, &db,
                ).await;
            }
            Ok(Err(e)) => {
                error!(index, "Parallel step failed: {e}");
                update_step_status(
                    pipeline_run_id, index, "failed",
                    None, Some(&e.to_string()), pipeline, &db,
                ).await;
                any_failed = true;
            }
            Err(join_err) => {
                error!(index, "Parallel step task panicked: {join_err}");
                update_step_status(
                    pipeline_run_id, index, "failed",
                    None, Some(&format!("Step task panicked: {join_err}")), pipeline, &db,
                ).await;
                any_failed = true;
            }
        }
    }

    if any_failed { Err(anyhow!("One or more parallel steps failed")) } else { Ok(()) }
}

/// The commit a finished step left on its branch, or the commit its worktree
/// started from when it changed nothing.
///
/// `None` when the step was not isolated, in which case there is no chain to
/// continue and the next step branches from `HEAD` like any other run.
async fn step_tip(run_id: &str, db: &DbPool) -> Option<String> {
    let row = sqlx::query(
        "SELECT after_sha, before_sha, isolation_status FROM story_runs WHERE id = ?",
    )
    .bind(run_id)
    .fetch_optional(db)
    .await
    .ok()??;

    let status: Option<String> = row.try_get("isolation_status").ok().flatten();
    if status.as_deref() != Some(runtime::worktree::STATUS_ISOLATED) {
        return None;
    }
    row.try_get::<Option<String>, _>("after_sha")
        .ok()
        .flatten()
        .or_else(|| row.try_get::<Option<String>, _>("before_sha").ok().flatten())
}

// ---------------------------------------------------------------------------
// Fire a single step run
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn fire_step_run(
    story_id: &str,
    agent_id: &str,
    pipeline_run_id: &str,
    depth: u32,
    extra_context: Option<String>,
    // Commit the previous sequential step left behind, so this one's worktree
    // starts from that step's output rather than from the workspace's HEAD.
    // `None` for a parallel step, which branches from HEAD by design.
    base_commit: Option<String>,
    pipeline: &Arc<PipelineState>,
    db: DbPool,
    app: tauri::AppHandle,
) -> Result<(String, Option<String>)> {

    // Load agent profile
    let profile_row = sqlx::query(
        "SELECT provider, model, system_prompt, max_iterations, max_retries, max_output_tokens, persistent_memory, \
                context_strategy, max_input_tokens \
         FROM agent_profiles WHERE id = ?",
    )
    .bind(agent_id)
    .fetch_optional(&db)
    .await
    .context("DB error loading agent profile")?
    .ok_or_else(|| anyhow!("Agent profile not found: {agent_id}"))?;

    let llm_provider: String = profile_row.try_get("provider").unwrap_or_default();
    let model_id: String = profile_row.try_get("model").unwrap_or_default();
    let mut system_prompt: Option<String> = profile_row.try_get("system_prompt").ok().flatten();
    let max_iterations: i64 = profile_row.try_get("max_iterations").unwrap_or(20);
    let max_retries: i64 = profile_row.try_get("max_retries").unwrap_or(2);
    let max_tokens: i64 = profile_row.try_get("max_output_tokens").unwrap_or(4096);
    let persistent_memory: bool = profile_row.try_get::<i64, _>("persistent_memory").unwrap_or(0) != 0;
    let context_strategy: String = profile_row.try_get("context_strategy").unwrap_or_default();
    let max_input_tokens: Option<i64> = profile_row.try_get("max_input_tokens").ok().flatten();

    // Load story
    let story_row = sqlx::query("SELECT title, description, track_history FROM stories WHERE id = ?")
        .bind(story_id)
        .fetch_optional(&db)
        .await
        .context("DB error loading story")?
        .ok_or_else(|| anyhow!("Story not found: {story_id}"))?;

    let story_title: String = story_row.try_get("title").unwrap_or_default();
    let story_desc: Option<String> = story_row.try_get("description").ok().flatten();
    let track_history: bool = story_row.try_get::<i64, _>("track_history").unwrap_or(1) != 0;

    // Append sequential context
    if let Some(extra) = extra_context {
        let existing = system_prompt.take().unwrap_or_default();
        system_prompt = Some(format!("{existing}{extra}"));
    }

    let user_content = match &story_desc {
        Some(d) => format!("Story ID: {story_id}\nTitle: {story_title}\n\n{d}"),
        None => format!("Story ID: {story_id}\nTitle: {story_title}"),
    };

    let initial_messages = vec![api::ChatMessage::user(user_content)];

    let settings = load_settings(&app);
    let event_retention_runs = settings.event_retention_runs.unwrap_or(10);

    let provider: Box<dyn api::LlmProvider> = match llm_provider.as_str() {
        "anthropic" => {
            let key = settings.anthropic_api_key
                .ok_or_else(|| anyhow!("Anthropic API key not configured"))?;
            Box::new(api::AnthropicClient::new(key))
        }
        "openrouter" => {
            let key = settings.openrouter_api_key
                .ok_or_else(|| anyhow!("OpenRouter API key not configured"))?;
            Box::new(api::OpenRouterClient::new(key))
        }
        "deepseek" => {
            let key = settings.deepseek_api_key
                .ok_or_else(|| anyhow!("DeepSeek API key not configured"))?;
            Box::new(api::DeepSeekClient::new(key))
        }
        "ollama" => {
            let base_url = settings.ollama_base_url
                .as_deref()
                .unwrap_or("http://localhost:11434");
            Box::new(api::OllamaClient::with_base_url(base_url))
        }
        other => return Err(anyhow!("Unknown provider: '{other}'")),
    };

    // Load permissions
    let perm_row = sqlx::query(
        "SELECT allowed_tools, allow_file_read_paths, allow_file_write_paths, \
                allow_shell_commands, require_approval_on_write \
         FROM agent_permissions WHERE profile_id = ?",
    )
    .bind(agent_id)
    .fetch_optional(&db)
    .await
    .context("DB error loading permissions")?;

    let permission_policy = match perm_row {
        Some(pr) => {
            let t: String  = pr.try_get("allowed_tools").unwrap_or_else(|_| "[]".into());
            let r: String  = pr.try_get("allow_file_read_paths").unwrap_or_else(|_| "[]".into());
            let w: String  = pr.try_get("allow_file_write_paths").unwrap_or_else(|_| "[]".into());
            let c: String  = pr.try_get("allow_shell_commands").unwrap_or_else(|_| "[]".into());
            let req: i64   = pr.try_get("require_approval_on_write").unwrap_or(0);
            runtime::PermissionPolicy::from_db_permissions(&t, &r, &w, &c, req != 0)
        }
        None => runtime::PermissionPolicy::allow_all(),
    };

    let approval_gate = Arc::new(runtime::ApprovalGate::new());

    // Build tool registry
    let mut registry = tools::ToolRegistry::new();
    tools::builtin::register_builtins(&mut registry, db.clone());
    // Load custom shell command tools bound to this agent profile.
    match tools::shell::load_for_agent(agent_id, &db).await {
        Ok(shell_tools) => {
            for ct in shell_tools { registry.register(Box::new(ct)); }
        }
        Err(e) => tracing::error!("Failed to load custom tools for agent '{agent_id}': {e}"),
    }

    // Build spawn_subtask callback.
    // IMPORTANT: Do NOT call fire_step_run recursively here — that would create a
    // self-referential future that the compiler cannot prove is Send.
    // Instead, delegate to run_subtask_impl which is a separate non-recursive fn.
    let db_for_spawn = db.clone();
    let app_for_spawn = app.clone();
    let run_registry_for_spawn = pipeline.run_registry.clone();
    let pipeline_run_id_for_spawn = pipeline_run_id.to_string();

    let spawn_fn: tools::SpawnSubtaskFn = Arc::new(move |sid, aid, _prid, d, workspace_root| {
        let db_cc = db_for_spawn.clone();
        let app_cc = app_for_spawn.clone();
        let reg_cc = run_registry_for_spawn.clone();
        let pid = pipeline_run_id_for_spawn.clone();
        Box::pin(run_subtask_impl(sid, aid, pid, d, workspace_root, db_cc, app_cc, reg_cc))
    });

    // Memory store
    let memory_store: Option<memory::MemoryStore> = if persistent_memory {
        Some(memory::MemoryStore::new(db.clone(), agent_id).await)
    } else {
        None
    };

    let mut completion_config = api::types::CompletionConfig::new(&model_id, max_tokens as u32);
    completion_config.system_prompt = system_prompt;

    let cancel = runtime::CancelFlag::new();

    let pipeline_run_id_str = pipeline_run_id.to_string();
    let registry_arc = Arc::new(tokio::sync::Mutex::new(registry));
    let app_for_isolation = app.clone();

    let mut rt = runtime::ConversationRuntime::new_with_pipeline(
        story_id,
        agent_id,
        provider,
        registry_arc,
        permission_policy,
        approval_gate,
        initial_messages,
        completion_config,
        max_iterations as u32,
        db.clone(),
        std::sync::Arc::new(app),
        cancel,
        memory_store,
        Some(pipeline_run_id_str),
        depth,
        Some(spawn_fn),
        db::get_active_workspace_path(&db).await,
        track_history,
        event_retention_runs,
    )
    .await
    .context("Failed to create ConversationRuntime for pipeline step")?;

    // Give the loop the profile's context settings. Set here rather than
    // passed to the constructor, which already carries far too many
    // arguments; the default is the safe one, so a caller that forgets
    // still gets `recent` compaction at a per-model budget.
    rt.context_policy =
        runtime::ContextPolicy::from_profile(&context_strategy, max_input_tokens);

    // Let the run reach the user: notifications for a parked approval or a
    // finished run, and the approval wait the user configured (indefinite by
    // default, so an unattended run parks rather than fail-closing).
    // A transient provider failure is retried inside the step, so the
    // conversation and its completed tool work survive the wait.
    rt.max_retries = max_retries.max(0) as u32;

    rt.notifier = Some(runtime::AppNotifier::arc(app_for_isolation.clone()));
    rt.approval_timeout = runtime::notifier::unattended_settings(&app_for_isolation).approval_timeout();

    // Give the step its own checkout before the loop starts. Parallel steps
    // otherwise edit the same files in the same tree and the last writer wins.
    if let Some(dir) = runtime::worktree::dir_for(&app_for_isolation) {
        rt.isolate_from(&dir, base_commit.as_deref()).await;
    }

    let run_id = rt.run_id.clone();
    let cancel_tok = rt.cancel.clone();

    // Register in shared run_registry so stop_run works
    {
        let mut reg = pipeline.run_registry.lock().unwrap();
        reg.insert(run_id.clone(), cancel_tok);
    }

    let join = tokio::spawn(async move { rt.run().await });
    let result = join.await;

    {
        let mut reg = pipeline.run_registry.lock().unwrap();
        reg.remove(&run_id);
    }

    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let _ = sqlx::query(
                "UPDATE story_runs SET status='failed', finished_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id=?",
            )
            .bind(&run_id).execute(&db).await;
            return Err(e);
        }
        Err(e) => {
            let _ = sqlx::query(
                "UPDATE story_runs SET status='failed', finished_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id=?",
            )
            .bind(&run_id).execute(&db).await;
            return Err(anyhow!("Step task panicked: {e}"));
        }
    }

    // Extract last assistant message for sequential handoff
    let last_assistant = sqlx::query(
        "SELECT content FROM run_events \
         WHERE run_id = ? AND role = 'assistant' AND content IS NOT NULL \
         ORDER BY sequence_num DESC LIMIT 1",
    )
    .bind(&run_id)
    .fetch_optional(&db)
    .await
    .ok()
    .flatten()
    .and_then(|row| row.try_get::<String, _>("content").ok());

    let output = last_assistant.map(|s| {
        if s.len() > 8192 { format!("{}…", &s[..8192]) } else { s }
    });

    Ok((run_id, output))
}

// ---------------------------------------------------------------------------
// run_subtask_impl — spawned by the spawn_subtask built-in tool
// ---------------------------------------------------------------------------
//
// This is deliberately NOT recursive: it creates a ConversationRuntime without
// a spawn_subtask callback (spawn_subtask: None), so agents executing as
// subtasks can call get_story to poll for results but cannot further spawn.
// This breaks the circular closure that would otherwise make the future !Send.

#[allow(clippy::too_many_arguments)]
async fn run_subtask_impl(
    story_id: String,
    agent_id: String,
    pipeline_run_id: String,
    depth: u32,
    // The spawning run's workspace root — its worktree, when it has one.
    parent_workspace_root: Option<std::path::PathBuf>,
    db: DbPool,
    app: tauri::AppHandle,
    run_registry: Arc<Mutex<HashMap<String, runtime::CancelFlag>>>,
) -> anyhow::Result<String> {
    use sqlx::Row;

    // Load profile
    let profile_row = sqlx::query(
        "SELECT provider, model, system_prompt, max_iterations, max_retries, max_output_tokens, persistent_memory, \
                context_strategy, max_input_tokens \
         FROM agent_profiles WHERE id = ?",
    )
    .bind(&agent_id)
    .fetch_optional(&db)
    .await
    .context("DB error loading agent profile")?
    .ok_or_else(|| anyhow!("Agent profile not found: {agent_id}"))?;

    let llm_provider: String = profile_row.try_get("provider").unwrap_or_default();
    let model_id: String = profile_row.try_get("model").unwrap_or_default();
    let system_prompt: Option<String> = profile_row.try_get("system_prompt").ok().flatten();
    let max_iterations: i64 = profile_row.try_get("max_iterations").unwrap_or(20);
    let max_retries: i64 = profile_row.try_get("max_retries").unwrap_or(2);
    let max_tokens: i64 = profile_row.try_get("max_output_tokens").unwrap_or(4096);
    let persistent_memory: bool = profile_row.try_get::<i64, _>("persistent_memory").unwrap_or(0) != 0;
    let context_strategy: String = profile_row.try_get("context_strategy").unwrap_or_default();
    let max_input_tokens: Option<i64> = profile_row.try_get("max_input_tokens").ok().flatten();

    // Load story
    let story_row = sqlx::query("SELECT title, description, track_history FROM stories WHERE id = ?")
        .bind(&story_id)
        .fetch_optional(&db)
        .await
        .context("DB error loading story")?
        .ok_or_else(|| anyhow!("Story not found: {story_id}"))?;

    let story_title: String = story_row.try_get("title").unwrap_or_default();
    let story_desc: Option<String> = story_row.try_get("description").ok().flatten();
    let track_history: bool = story_row.try_get::<i64, _>("track_history").unwrap_or(1) != 0;

    let user_content = match &story_desc {
        Some(d) => format!("Story ID: {story_id}\nTitle: {story_title}\n\n{d}"),
        None => format!("Story ID: {story_id}\nTitle: {story_title}"),
    };

    let settings = load_settings(&app);
    let event_retention_runs = settings.event_retention_runs.unwrap_or(10);

    let provider: Box<dyn api::LlmProvider> = match llm_provider.as_str() {
        "anthropic" => {
            let key = settings.anthropic_api_key
                .ok_or_else(|| anyhow!("Anthropic API key not configured"))?;
            Box::new(api::AnthropicClient::new(key))
        }
        "openrouter" => {
            let key = settings.openrouter_api_key
                .ok_or_else(|| anyhow!("OpenRouter API key not configured"))?;
            Box::new(api::OpenRouterClient::new(key))
        }
        "deepseek" => {
            let key = settings.deepseek_api_key
                .ok_or_else(|| anyhow!("DeepSeek API key not configured"))?;
            Box::new(api::DeepSeekClient::new(key))
        }
        "ollama" => {
            let base_url = settings.ollama_base_url
                .as_deref()
                .unwrap_or("http://localhost:11434");
            Box::new(api::OllamaClient::with_base_url(base_url))
        }
        other => return Err(anyhow!("Unknown provider: '{other}'")),
    };

    let perm_row = sqlx::query(
        "SELECT allowed_tools, allow_file_read_paths, allow_file_write_paths, \
                allow_shell_commands, require_approval_on_write \
         FROM agent_permissions WHERE profile_id = ?",
    )
    .bind(&agent_id)
    .fetch_optional(&db)
    .await
    .context("DB error loading permissions")?;

    let permission_policy = match perm_row {
        Some(pr) => {
            let t: String  = pr.try_get("allowed_tools").unwrap_or_else(|_| "[]".into());
            let r: String  = pr.try_get("allow_file_read_paths").unwrap_or_else(|_| "[]".into());
            let w: String  = pr.try_get("allow_file_write_paths").unwrap_or_else(|_| "[]".into());
            let c: String  = pr.try_get("allow_shell_commands").unwrap_or_else(|_| "[]".into());
            let req: i64   = pr.try_get("require_approval_on_write").unwrap_or(0);
            runtime::PermissionPolicy::from_db_permissions(&t, &r, &w, &c, req != 0)
        }
        None => runtime::PermissionPolicy::allow_all(),
    };

    let approval_gate = Arc::new(runtime::ApprovalGate::new());
    let mut registry = tools::ToolRegistry::new();
    tools::builtin::register_builtins(&mut registry, db.clone());
    // Load custom shell command tools bound to this agent profile.
    match tools::shell::load_for_agent(&agent_id, &db).await {
        Ok(shell_tools) => {
            for ct in shell_tools { registry.register(Box::new(ct)); }
        }
        Err(e) => tracing::error!("Failed to load custom tools for agent '{agent_id}': {e}"),
    }
    let registry_arc = Arc::new(tokio::sync::Mutex::new(registry));

    let memory_store: Option<memory::MemoryStore> = if persistent_memory {
        Some(memory::MemoryStore::new(db.clone(), &agent_id).await)
    } else {
        None
    };

    let mut config = api::types::CompletionConfig::new(&model_id, max_tokens as u32);
    config.system_prompt = system_prompt;

    let cancel = runtime::CancelFlag::new();

    // Kept because the constructor takes the handle by value.
    let app_for_notifications = app.clone();
    let mut rt = runtime::ConversationRuntime::new_with_pipeline(
        story_id,
        agent_id,
        provider,
        registry_arc,
        permission_policy,
        approval_gate,
        vec![api::ChatMessage::user(user_content)],
        config,
        max_iterations as u32,
        db.clone(),
        std::sync::Arc::new(app),
        cancel,
        memory_store,
        Some(pipeline_run_id),
        depth,
        None, // No further spawn_subtask recursion
        // Inherit the spawning run's root so the subtask writes inside the
        // parent's worktree, where the parent can see it and where it stays out
        // of the user's checkout. Only a subtask spawned by an un-isolated run
        // falls back to the workspace itself.
        match parent_workspace_root {
            Some(root) => Some(root),
            None => db::get_active_workspace_path(&db).await,
        },
        track_history,
        event_retention_runs,
    )
    .await
    .context("Failed to create ConversationRuntime for subtask")?;

    // Give the loop the profile's context settings. Set here rather than
    // passed to the constructor, which already carries far too many
    // arguments; the default is the safe one, so a caller that forgets
    // still gets `recent` compaction at a per-model budget.
    rt.context_policy =
        runtime::ContextPolicy::from_profile(&context_strategy, max_input_tokens);

    // Let the run reach the user: notifications for a parked approval or a
    // finished run, and the approval wait the user configured (indefinite by
    // default, so an unattended run parks rather than fail-closing).
    // A transient provider failure is retried inside the step, so the
    // conversation and its completed tool work survive the wait.
    rt.max_retries = max_retries.max(0) as u32;

    rt.notifier = Some(runtime::AppNotifier::arc(app_for_notifications.clone()));
    rt.approval_timeout =
        runtime::notifier::unattended_settings(&app_for_notifications).approval_timeout();

    let run_id = rt.run_id.clone();
    let cancel_tok = rt.cancel.clone();

    {
        let mut reg = run_registry.lock().unwrap();
        reg.insert(run_id.clone(), cancel_tok);
    }

    let reg_clone = run_registry.clone();
    let run_id_clone = run_id.clone();
    tokio::spawn(async move {
        if let Err(e) = rt.run().await {
            tracing::error!(run_id = %run_id_clone, "Subtask run failed: {e}");
        }
        reg_clone.lock().unwrap().remove(&run_id_clone);
    });

    Ok(run_id)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load and parse a pipeline story's config. Public so the MCP server can
/// offer a dry-run validation without starting the pipeline.
pub async fn load_pipeline_config(story_id: &str, db: &DbPool) -> Result<PipelineConfig> {
    let row = sqlx::query(
        "SELECT pipeline_config FROM stories WHERE id = ? AND story_type = 'pipeline'",
    )
    .bind(story_id)
    .fetch_optional(db)
    .await
    .context("DB error")?
    .ok_or_else(|| anyhow!("Story {story_id} not found or not a pipeline type"))?;

    let config_json: Option<String> = row.try_get("pipeline_config").ok().flatten();
    let json = config_json
        .ok_or_else(|| anyhow!("Story {story_id} has no pipeline_config JSON"))?;

    serde_json::from_str::<PipelineConfig>(&json)
        .context("Failed to parse pipeline_config JSON")
}

/// Check a pipeline config for an empty step list, a self-reference, or a
/// duplicate step story. Pure — safe to call as a dry run.
pub fn validate_pipeline_config(pipeline_story_id: &str, config: &PipelineConfig) -> Result<()> {
    if config.steps.is_empty() {
        return Err(anyhow!("Pipeline must have at least one step"));
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for step in &config.steps {
        if step.story_id == pipeline_story_id {
            return Err(anyhow!(
                "Cycle: step references the pipeline story itself ({})",
                step.story_id
            ));
        }
        if !seen.insert(step.story_id.as_str()) {
            return Err(anyhow!(
                "Cycle: story_id '{}' appears more than once in pipeline steps",
                step.story_id
            ));
        }
    }
    Ok(())
}

async fn update_step_status(
    pipeline_run_id: &str,
    index: usize,
    status: &str,
    run_id: Option<&str>,
    output: Option<&str>,
    pipeline: &Arc<PipelineState>,
    db: &DbPool,
) {
    if let Some(mut p) = pipeline.runs.get_mut(pipeline_run_id) {
        if let Some(step) = p.steps.get_mut(index) {
            step.status = status.to_string();
            if let Some(rid) = run_id {
                step.run_id = Some(rid.to_string());
            }
        }
    }

    let result = if let Some(rid) = run_id {
        sqlx::query(
            "UPDATE pipeline_step_runs \
             SET status = ?, run_id = ?, output = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
             WHERE pipeline_run_id = ? AND step_index = ?",
        )
        .bind(status).bind(rid).bind(output).bind(pipeline_run_id).bind(index as i64)
        .execute(db).await
    } else {
        sqlx::query(
            "UPDATE pipeline_step_runs \
             SET status = ?, output = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
             WHERE pipeline_run_id = ? AND step_index = ?",
        )
        .bind(status).bind(output).bind(pipeline_run_id).bind(index as i64)
        .execute(db).await
    };

    if let Err(e) = result {
        error!(pipeline_run_id, index, "Failed to update step in DB: {e}");
    }
}

