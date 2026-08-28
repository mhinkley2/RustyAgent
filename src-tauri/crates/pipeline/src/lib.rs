// Multi-agent pipeline engine: sequential handoff, parallel fan-out, supervisor delegation.
// RUSTYAGE-10 implementation.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use anyhow::{anyhow, Context, Result};
use dashmap::DashMap;
use db::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tauri::Manager;
use tokio::task::JoinHandle;
use tracing::{error, info};
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
}

fn load_settings(app: &tauri::AppHandle) -> LocalSettings {
    let path = app
        .path()
        .app_data_dir()
        .expect("app data dir")
        .join("settings.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
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
        "INSERT INTO story_runs (id, story_id, agent_profile_id, status, started_at) \
         VALUES (?, ?, ?, 'running', CURRENT_TIMESTAMP)",
    )
    .bind(&pipeline_run_id)
    .bind(&story_id)
    .bind(&agent_profile_id)
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

    // Spawn the async pipeline executor
    let pid = pipeline_run_id.clone();
    let pipeline_clone = pipeline.clone();
    let db_clone = db.clone();
    let app_clone = app.clone();

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

        pipeline_clone.tasks.remove(&pid);
        info!(pipeline_run_id = %pid, status = %final_status, "Pipeline complete");
    });

    pipeline.tasks.insert(pipeline_run_id.clone(), handle);

    // Mark the pipeline story as in_progress
    let _ = sqlx::query("UPDATE stories SET status = 'in_progress' WHERE id = ?")
        .bind(&story_id)
        .execute(&db)
        .await;

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
    let mut handles: Vec<JoinHandle<(usize, Result<(String, Option<String>)>)>> =
        Vec::with_capacity(config.steps.len());

    for (index, step) in config.steps.iter().enumerate() {
        update_step_status(pipeline_run_id, index, "running", None, None, pipeline, &db).await;

        let pid = pipeline_run_id.to_string();
        let s = step.clone();
        let pipeline_c = pipeline.clone();
        let db_c = db.clone();
        let app_c = app.clone();

        let h = tokio::spawn(async move {
            let res = fire_step_run(&s.story_id, &s.agent_id, &pid, 0, None, &pipeline_c, db_c, app_c).await;
            (index, res)
        });
        handles.push(h);
    }

    let mut any_failed = false;
    for h in handles {
        match h.await {
            Ok((index, Ok((run_id, output)))) => {
                update_step_status(
                    pipeline_run_id, index, "done",
                    Some(&run_id), output.as_deref(), pipeline, &db,
                ).await;
            }
            Ok((index, Err(e))) => {
                error!(index, "Parallel step failed: {e}");
                update_step_status(
                    pipeline_run_id, index, "failed",
                    None, Some(&e.to_string()), pipeline, &db,
                ).await;
                any_failed = true;
            }
            Err(join_err) => {
                error!("Parallel step task panicked: {join_err}");
                any_failed = true;
            }
        }
    }

    if any_failed { Err(anyhow!("One or more parallel steps failed")) } else { Ok(()) }
}

// ---------------------------------------------------------------------------
// Fire a single step run
// ---------------------------------------------------------------------------

async fn fire_step_run(
    story_id: &str,
    agent_id: &str,
    pipeline_run_id: &str,
    depth: u32,
    extra_context: Option<String>,
    pipeline: &Arc<PipelineState>,
    db: DbPool,
    app: tauri::AppHandle,
) -> Result<(String, Option<String>)> {

    // Load agent profile
    let profile_row = sqlx::query(
        "SELECT provider, model, system_prompt, max_iterations, max_output_tokens, persistent_memory, \
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
                allow_shell_commands, allow_network_hosts, require_approval_on_write \
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
            let h: String  = pr.try_get("allow_network_hosts").unwrap_or_else(|_| "[]".into());
            let req: i64   = pr.try_get("require_approval_on_write").unwrap_or(0);
            runtime::PermissionPolicy::from_db_permissions(&t, &r, &w, &c, &h, req != 0)
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

    let spawn_fn: tools::SpawnSubtaskFn = Arc::new(move |sid, aid, _prid, d| {
        let db_cc = db_for_spawn.clone();
        let app_cc = app_for_spawn.clone();
        let reg_cc = run_registry_for_spawn.clone();
        let pid = pipeline_run_id_for_spawn.clone();
        Box::pin(run_subtask_impl(sid, aid, pid, d, db_cc, app_cc, reg_cc))
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

async fn run_subtask_impl(
    story_id: String,
    agent_id: String,
    pipeline_run_id: String,
    depth: u32,
    db: DbPool,
    app: tauri::AppHandle,
    run_registry: Arc<Mutex<HashMap<String, runtime::CancelFlag>>>,
) -> anyhow::Result<String> {
    use sqlx::Row;

    // Load profile
    let profile_row = sqlx::query(
        "SELECT provider, model, system_prompt, max_iterations, max_output_tokens, persistent_memory, \
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
                allow_shell_commands, allow_network_hosts, require_approval_on_write \
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
            let h: String  = pr.try_get("allow_network_hosts").unwrap_or_else(|_| "[]".into());
            let req: i64   = pr.try_get("require_approval_on_write").unwrap_or(0);
            runtime::PermissionPolicy::from_db_permissions(&t, &r, &w, &c, &h, req != 0)
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
        db::get_active_workspace_path(&db).await,
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

