// Cron-based and continuous-mode scheduler for agent execution.
// RUSTYAGE-8 implementation.

use std::{
    collections::HashMap,
    str::FromStr,
    sync::{Arc, Mutex},
};

use chrono::Utc;
use cron::Schedule;
use dashmap::DashMap;
use db::DbPool;
use serde::{Deserialize, Serialize};
use tauri::Manager;
use tokio::{task::JoinHandle, time::sleep};
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// Inline settings loader (avoids a dep on commands crate)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct LocalSettings {
    anthropic_api_key: Option<String>,
    openrouter_api_key: Option<String>,
    deepseek_api_key: Option<String>,
    ollama_base_url: Option<String>,
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
// AgentRuntimeStatus — returned to the frontend
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuntimeStatus {
    pub profile_id: String,
    /// Scheduler mode config: "manual" | "continuous" | "scheduled"
    pub scheduler_mode: String,
    /// Live runtime state:
    /// "idle" | "checking_for_work" | "running_story" | "waiting_for_approval"
    /// | "waiting_for_human_input" | "failed" | "completed_recently"
    pub state: String,
    /// Plain-language state label for UI surfaces.
    pub state_label: String,
    /// ISO-8601 timestamp of the next scheduled fire (scheduled mode only)
    pub next_run_at: Option<String>,
    /// run_id of the currently-executing run (running state only)
    pub active_run_id: Option<String>,
    /// story metadata for active runs when available
    pub active_story_id: Option<String>,
    pub active_story_title: Option<String>,
    /// short failure reason when state = failed
    pub failure_summary: Option<String>,
}

fn state_label(state: &str) -> String {
    match state {
        "idle" => "Idle".to_string(),
        "checking_for_work" => "Checking for work".to_string(),
        "running_story" => "Running story".to_string(),
        "waiting_for_approval" => "Waiting for approval".to_string(),
        "waiting_for_human_input" => "Waiting for human input".to_string(),
        "failed" => "Failed".to_string(),
        "completed_recently" => "Completed just now".to_string(),
        other => other.replace('_', " "),
    }
}

// ---------------------------------------------------------------------------
// SchedulerState — Tauri app-managed state
// ---------------------------------------------------------------------------

/// Tracks the background tasks (continuous poll loops, cron loops) running
/// for each agent profile. Values are (cancel_tx, join_handle).
pub struct SchedulerState {
    /// profile_id → (stop_sender, task_handle)
    tasks: DashMap<String, (tokio::sync::watch::Sender<bool>, JoinHandle<()>)>,
    /// profile_id → currently active run_id
    active_runs: DashMap<String, String>,
    /// profile_id → active story_id/title for the run in progress
    active_story: DashMap<String, (String, String)>,
    /// profile_id → next scheduled fire timestamp (ISO-8601)
    next_fire: DashMap<String, String>,
    /// profile_id → most recent rich runtime status snapshot
    live_status: DashMap<String, AgentRuntimeStatus>,
    /// shared run registry: run_id → CancelFlag (same map as RunRegistry in commands)
    run_registry: Arc<Mutex<HashMap<String, runtime::CancelFlag>>>,
}

impl SchedulerState {
    pub fn new(run_registry: Arc<Mutex<HashMap<String, runtime::CancelFlag>>>) -> Self {
        Self {
            tasks: DashMap::new(),
            active_runs: DashMap::new(),
            active_story: DashMap::new(),
            next_fire: DashMap::new(),
            live_status: DashMap::new(),
            run_registry,
        }
    }

    // -----------------------------------------------------------------------
    // Query helpers
    // -----------------------------------------------------------------------

    pub fn status_for(&self, profile_id: &str) -> AgentRuntimeStatus {
        if let Some(existing) = self.live_status.get(profile_id) {
            return existing.clone();
        }

        let scheduled = self.tasks.contains_key(profile_id);
        let next_run_at = self.next_fire.get(profile_id).map(|s| s.clone());
        let scheduler_mode = if scheduled && next_run_at.is_some() {
            "scheduled"
        } else if scheduled {
            "continuous"
        } else {
            "manual"
        }
        .to_string();

        AgentRuntimeStatus {
            profile_id: profile_id.to_string(),
            scheduler_mode,
            state: "idle".to_string(),
            state_label: state_label("idle"),
            next_run_at,
            active_run_id: None,
            active_story_id: None,
            active_story_title: None,
            failure_summary: None,
        }
    }

    pub fn all_statuses(&self) -> Vec<AgentRuntimeStatus> {
        let mut keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for entry in self.tasks.iter() {
            keys.insert(entry.key().clone());
        }
        for entry in self.live_status.iter() {
            keys.insert(entry.key().clone());
        }
        keys.into_iter().map(|k| self.status_for(&k)).collect()
    }

    pub fn is_running(&self) -> bool {
        !self.tasks.is_empty()
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn stop_task(&self, profile_id: &str) {
        if let Some((_, (tx, handle))) = self.tasks.remove(profile_id) {
            let _ = tx.send(true);
            handle.abort();
        }
        self.active_runs.remove(profile_id);
        self.active_story.remove(profile_id);
        self.next_fire.remove(profile_id);
        self.live_status.insert(
            profile_id.to_string(),
            AgentRuntimeStatus {
                profile_id: profile_id.to_string(),
                scheduler_mode: "manual".to_string(),
                state: "idle".to_string(),
                state_label: state_label("idle"),
                next_run_at: None,
                active_run_id: None,
                active_story_id: None,
                active_story_title: None,
                failure_summary: None,
            },
        );
    }

    fn set_live_state(
        &self,
        profile_id: &str,
        state: &str,
        scheduler_mode: Option<&str>,
        next_run_at: Option<String>,
        active_run_id: Option<String>,
        active_story_id: Option<String>,
        active_story_title: Option<String>,
        failure_summary: Option<String>,
    ) {
        let fallback_mode = if self.tasks.contains_key(profile_id) {
            if self.next_fire.get(profile_id).is_some() {
                "scheduled"
            } else {
                "continuous"
            }
        } else {
            "manual"
        };

        self.live_status.insert(
            profile_id.to_string(),
            AgentRuntimeStatus {
                profile_id: profile_id.to_string(),
                scheduler_mode: scheduler_mode.unwrap_or(fallback_mode).to_string(),
                state: state.to_string(),
                state_label: state_label(state),
                next_run_at,
                active_run_id,
                active_story_id,
                active_story_title,
                failure_summary,
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Start a continuous-poll loop for the given profile.
///
/// The loop waits `poll_interval_secs`, then finds the oldest Ready story
/// assigned to this profile and fires `start_run` for it. Only one story
/// runs at a time.
pub async fn start_continuous(
    profile_id: &str,
    poll_interval_secs: u64,
    scheduler: Arc<SchedulerState>,
    db: DbPool,
    app: tauri::AppHandle,
) -> Result<(), String> {
    if scheduler.tasks.contains_key(profile_id) {
        return Err(format!("Continuous mode already active for profile '{profile_id}'"));
    }

    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
    let pid = profile_id.to_string();
    let sched_clone = scheduler.clone();
    let db_clone = db.clone();
    let app_clone = app.clone();

    let handle = tokio::spawn(async move {
        info!(profile_id = %pid, "Continuous poll loop started");
        sched_clone.set_live_state(
            &pid,
            "checking_for_work",
            Some("continuous"),
            None,
            None,
            None,
            None,
            None,
        );
        loop {
            // Check for stop signal
            if *stop_rx.borrow() {
                break;
            }

            // Skip if a run is already active
            if sched_clone.active_runs.contains_key(&pid) {
                let interval = std::time::Duration::from_secs(poll_interval_secs);
                tokio::select! {
                    _ = sleep(interval) => {}
                    _ = stop_rx.changed() => { break; }
                }
                continue;
            }

            // Find the oldest Ready story assigned to this profile
            let story_row = sqlx::query(
                "SELECT id, title FROM stories \
                  WHERE status = 'ready' AND assigned_agent_id = ? \
                 ORDER BY created_at ASC LIMIT 1",
            )
            .bind(&pid)
            .fetch_optional(&db_clone)
            .await;

            match story_row {
                Ok(Some(row)) => {
                    use sqlx::Row;
                    let story_id: String = row.try_get("id").unwrap_or_default();
                    let story_title: String = row.try_get("title").unwrap_or_else(|_| "Story".to_string());
                    info!(profile_id = %pid, story_id = %story_id, "Continuous: picked up story");

                    match fire_run(&pid, &story_id, &sched_clone, db_clone.clone(), app_clone.clone()).await {
                        Ok(run_id) => {
                            sched_clone.active_runs.insert(pid.clone(), run_id.clone());
                            sched_clone.active_story.insert(pid.clone(), (story_id.clone(), story_title.clone()));
                            sched_clone.set_live_state(
                                &pid,
                                "running_story",
                                Some("continuous"),
                                None,
                                Some(run_id.clone()),
                                Some(story_id.clone()),
                                Some(story_title.clone()),
                                None,
                            );
                            // Wait for the run to finish
                            wait_for_run(
                                &pid,
                                &run_id,
                                &story_id,
                                &story_title,
                                &sched_clone,
                                db_clone.clone(),
                                &mut stop_rx,
                            )
                            .await;
                            sched_clone.active_runs.remove(&pid);
                            sched_clone.active_story.remove(&pid);
                            sched_clone.set_live_state(
                                &pid,
                                "completed_recently",
                                Some("continuous"),
                                None,
                                None,
                                None,
                                None,
                                None,
                            );
                        }
                        Err(e) => {
                            error!(profile_id = %pid, "Failed to start run: {e}");
                            sched_clone.set_live_state(
                                &pid,
                                "failed",
                                Some("continuous"),
                                None,
                                None,
                                Some(story_id),
                                Some(story_title),
                                Some(e),
                            );
                        }
                    }
                }
                Ok(None) => {
                    // No ready story — sleep then poll again
                    sched_clone.set_live_state(
                        &pid,
                        "checking_for_work",
                        Some("continuous"),
                        None,
                        None,
                        None,
                        None,
                        None,
                    );
                }
                Err(e) => {
                    error!(profile_id = %pid, "DB error during continuous poll: {e}");
                    sched_clone.set_live_state(
                        &pid,
                        "failed",
                        Some("continuous"),
                        None,
                        None,
                        None,
                        None,
                        Some(format!("DB error while checking work: {e}")),
                    );
                }
            }

            let interval = std::time::Duration::from_secs(poll_interval_secs);
            tokio::select! {
                _ = sleep(interval) => {}
                _ = stop_rx.changed() => { break; }
            }
        }
        info!(profile_id = %pid, "Continuous poll loop stopped");
    });

    scheduler.tasks.insert(profile_id.to_string(), (stop_tx, handle));
    Ok(())
}

/// Start a cron-based schedule loop for the given profile.
///
/// `cron_expr` is a 5-field cron expression (minute-level, no seconds field).
/// On each fire, `start_run` is called with the provided `story_id` override,
/// or the oldest Ready story assigned to the profile if `story_id` is None.
pub async fn start_scheduled(
    profile_id: &str,
    cron_expr: &str,
    scheduler: Arc<SchedulerState>,
    db: DbPool,
    app: tauri::AppHandle,
) -> Result<(), String> {
    if scheduler.tasks.contains_key(profile_id) {
        return Err(format!("Scheduler already active for profile '{profile_id}'"));
    }

    // cron crate expects 7-field (secs + year), so we prepend "0 " and append " *"
    let expr_7 = format!("0 {cron_expr} *");
    let schedule = Schedule::from_str(&expr_7)
        .map_err(|e| format!("Invalid cron expression '{cron_expr}': {e}"))?;

    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
    let pid = profile_id.to_string();
    let sched_clone = scheduler.clone();
    let db_clone = db.clone();
    let app_clone = app.clone();

    let handle = tokio::spawn(async move {
        info!(profile_id = %pid, "Cron loop started");
        let mut upcoming = schedule.upcoming(Utc);

        loop {
            // Compute the next fire time
            let next = match upcoming.next() {
                Some(t) => t,
                None => {
                    warn!(profile_id = %pid, "Cron schedule exhausted");
                    break;
                }
            };

            // Store next fire for UI display
            let next_str = next.to_rfc3339();
            sched_clone.next_fire.insert(pid.clone(), next_str.clone());
            sched_clone.set_live_state(
                &pid,
                "checking_for_work",
                Some("scheduled"),
                Some(next_str),
                None,
                None,
                None,
                None,
            );

            // Sleep until next fire (or stop signal)
            let now = Utc::now();
            let delay_secs = (next - now).num_seconds().max(0) as u64;
            let interval = std::time::Duration::from_secs(delay_secs);

            tokio::select! {
                _ = sleep(interval) => {}
                _ = stop_rx.changed() => { break; }
            }

            if *stop_rx.borrow() {
                break;
            }

            // Find the oldest Ready story assigned to this profile
            let story_row = sqlx::query(
                "SELECT id, title FROM stories \
                  WHERE status = 'ready' AND assigned_agent_id = ? \
                 ORDER BY created_at ASC LIMIT 1",
            )
            .bind(&pid)
            .fetch_optional(&db_clone)
            .await;

            match story_row {
                Ok(Some(row)) => {
                    use sqlx::Row;
                    let story_id: String = row.try_get("id").unwrap_or_default();
                    let story_title: String = row.try_get("title").unwrap_or_else(|_| "Story".to_string());
                    info!(profile_id = %pid, story_id = %story_id, "Cron: firing run");
                    match fire_run(&pid, &story_id, &sched_clone, db_clone.clone(), app_clone.clone()).await {
                        Ok(run_id) => {
                            sched_clone.active_runs.insert(pid.clone(), run_id.clone());
                            sched_clone.active_story.insert(pid.clone(), (story_id.clone(), story_title.clone()));
                            sched_clone.set_live_state(
                                &pid,
                                "running_story",
                                Some("scheduled"),
                                sched_clone.next_fire.get(&pid).map(|v| v.clone()),
                                Some(run_id.clone()),
                                Some(story_id.clone()),
                                Some(story_title.clone()),
                                None,
                            );
                            wait_for_run_no_stop(
                                &pid,
                                &run_id,
                                &story_id,
                                &story_title,
                                &sched_clone,
                                db_clone.clone(),
                            )
                            .await;
                            sched_clone.active_runs.remove(&pid);
                            sched_clone.active_story.remove(&pid);
                            sched_clone.set_live_state(
                                &pid,
                                "completed_recently",
                                Some("scheduled"),
                                sched_clone.next_fire.get(&pid).map(|v| v.clone()),
                                None,
                                None,
                                None,
                                None,
                            );
                        }
                        Err(e) => {
                            error!(profile_id = %pid, "Cron: failed to start run: {e}");
                            sched_clone.set_live_state(
                                &pid,
                                "failed",
                                Some("scheduled"),
                                sched_clone.next_fire.get(&pid).map(|v| v.clone()),
                                None,
                                Some(story_id),
                                Some(story_title),
                                Some(e),
                            );
                        }
                    }
                }
                Ok(None) => {
                    info!(profile_id = %pid, "Cron fired but no ready stories");
                    sched_clone.set_live_state(
                        &pid,
                        "checking_for_work",
                        Some("scheduled"),
                        sched_clone.next_fire.get(&pid).map(|v| v.clone()),
                        None,
                        None,
                        None,
                        None,
                    );
                }
                Err(e) => {
                    error!(profile_id = %pid, "Cron: DB error: {e}");
                    sched_clone.set_live_state(
                        &pid,
                        "failed",
                        Some("scheduled"),
                        sched_clone.next_fire.get(&pid).map(|v| v.clone()),
                        None,
                        None,
                        None,
                        Some(format!("DB error while checking work: {e}")),
                    );
                }
            }
        }

        sched_clone.next_fire.remove(&pid);
        info!(profile_id = %pid, "Cron loop stopped");
    });

    scheduler.tasks.insert(profile_id.to_string(), (stop_tx, handle));
    Ok(())
}

/// Stop the continuous or scheduled loop for a profile.
pub fn stop_agent_scheduler(profile_id: &str, scheduler: Arc<SchedulerState>) -> Result<(), String> {
    if scheduler.tasks.contains_key(profile_id) {
        scheduler.stop_task(profile_id);
        info!(profile_id = %profile_id, "Scheduler stopped");
        Ok(())
    } else {
        Err(format!("No active scheduler for profile '{profile_id}'"))
    }
}

/// On app startup, re-register all profiles with `run_mode != 'manual'`.
pub async fn restore_schedulers(
    scheduler: Arc<SchedulerState>,
    db: DbPool,
    app: tauri::AppHandle,
) {
    let rows = sqlx::query(
        "SELECT id, run_mode, cron_expression, continuous_poll_interval_secs \
         FROM agent_profiles WHERE run_mode != 'manual'",
    )
    .fetch_all(&db)
    .await;

    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to load profiles for scheduler restore: {e}");
            return;
        }
    };

    use sqlx::Row;
    for row in rows {
        let id: String = row.try_get("id").unwrap_or_default();
        let mode: String = row.try_get("run_mode").unwrap_or_default();
        let cron: Option<String> = row.try_get("cron_expression").ok().flatten();
        let interval: i64 = row.try_get("continuous_poll_interval_secs").unwrap_or(30);

        match mode.as_str() {
            "continuous" => {
                if let Err(e) = start_continuous(
                    &id, interval as u64, scheduler.clone(), db.clone(), app.clone(),
                ).await {
                    error!(profile_id = %id, "Failed to restore continuous mode: {e}");
                } else {
                    info!(profile_id = %id, "Restored continuous mode on startup");
                }
            }
            "scheduled" => {
                if let Some(expr) = cron {
                    if let Err(e) = start_scheduled(
                        &id, &expr, scheduler.clone(), db.clone(), app.clone(),
                    ).await {
                        error!(profile_id = %id, "Failed to restore scheduled mode: {e}");
                    } else {
                        info!(profile_id = %id, interval_expr = %expr, "Restored scheduled mode on startup");
                    }
                }
            }
            _ => {}
        }
    }
}

/// Return the runtime status for all known scheduled profiles + any currently active ones.
pub fn get_all_statuses(scheduler: Arc<SchedulerState>) -> Vec<AgentRuntimeStatus> {
    scheduler.all_statuses()
}

/// Return runtime status for a single profile (always returns Some — "idle" if not tracked).
pub fn get_status(profile_id: &str, scheduler: Arc<SchedulerState>) -> AgentRuntimeStatus {
    scheduler.status_for(profile_id)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Invoke `commands::start_run` and return the run_id.
async fn fire_run(
    profile_id: &str,
    story_id: &str,
    scheduler: &Arc<SchedulerState>,
    db: DbPool,
    app: tauri::AppHandle,
) -> Result<String, String> {
    // We need to replicate what commands::start_run does but without Tauri State.
    // Build the run directly from the parts we have.
    use sqlx::Row;

    // Load profile
    let row = sqlx::query(
        "SELECT provider, model, system_prompt, max_iterations, max_output_tokens, persistent_memory \
         FROM agent_profiles WHERE id = ?",
    )
    .bind(profile_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| format!("DB error loading profile: {e}"))?
    .ok_or_else(|| format!("Profile not found: {profile_id}"))?;

    let llm_provider: String = row.try_get("provider").unwrap_or_default();
    let model_id: String = row.try_get("model").unwrap_or_default();
    let system_prompt: Option<String> = row.try_get("system_prompt").ok().flatten();
    let max_iterations: i64 = row.try_get("max_iterations").unwrap_or(20);
    let max_tokens: i64 = row.try_get("max_output_tokens").unwrap_or(4096);
    let persistent_memory: bool = row.try_get::<i64, _>("persistent_memory").unwrap_or(0) != 0;

    // Load story
    let story_row = sqlx::query("SELECT title, description FROM stories WHERE id = ?")
        .bind(story_id)
        .fetch_optional(&db)
        .await
        .map_err(|e| format!("DB error loading story: {e}"))?
        .ok_or_else(|| format!("Story not found: {story_id}"))?;

    let story_title: String = story_row.try_get("title").unwrap_or_default();
    let story_description: Option<String> = story_row.try_get("description").ok().flatten();

    // Load settings for API key
    let app_settings = load_settings(&app);

    let provider: Box<dyn api::LlmProvider> = match llm_provider.as_str() {
        "anthropic" => {
            let key = app_settings.anthropic_api_key
                .ok_or_else(|| "Anthropic API key not configured".to_string())?;
            Box::new(api::AnthropicClient::new(key))
        }
        "openrouter" => {
            let key = app_settings.openrouter_api_key
                .ok_or_else(|| "OpenRouter API key not configured".to_string())?;
            Box::new(api::OpenRouterClient::new(key))
        }
        "deepseek" => {
            let key = app_settings.deepseek_api_key
                .ok_or_else(|| "DeepSeek API key not configured".to_string())?;
            Box::new(api::DeepSeekClient::new(key))
        }
        "ollama" => {
            let base_url = app_settings.ollama_base_url
                .as_deref()
                .unwrap_or("http://localhost:11434");
            Box::new(api::OllamaClient::with_base_url(base_url))
        }
        other => return Err(format!("Unknown provider: '{other}'")),
    };
    let perm_row = sqlx::query(
        "SELECT allowed_tools, allow_file_read_paths, allow_file_write_paths, \
                allow_shell_commands, allow_network_hosts, require_approval_on_write \
         FROM agent_permissions WHERE profile_id = ?",
    )
    .bind(profile_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| format!("DB error loading permissions: {e}"))?;

    let permission_policy = match perm_row {
        Some(pr) => {
            let tools: String  = pr.try_get("allowed_tools").unwrap_or_else(|_| "[]".into());
            let reads: String  = pr.try_get("allow_file_read_paths").unwrap_or_else(|_| "[]".into());
            let writes: String = pr.try_get("allow_file_write_paths").unwrap_or_else(|_| "[]".into());
            let cmds: String   = pr.try_get("allow_shell_commands").unwrap_or_else(|_| "[]".into());
            let hosts: String  = pr.try_get("allow_network_hosts").unwrap_or_else(|_| "[]".into());
            let req_approval: i64 = pr.try_get("require_approval_on_write").unwrap_or(0);
            runtime::PermissionPolicy::from_db_permissions(
                &tools, &reads, &writes, &cmds, &hosts, req_approval != 0,
            )
        }
        None => runtime::PermissionPolicy::allow_all(),
    };

    let mut config = api::types::CompletionConfig::new(&model_id, max_tokens as u32);
    config.system_prompt = system_prompt;

    let user_content = format!(
        "Process the following story:\n\nStory ID: {story_id}\nTitle: {story_title}\n\n{}",
        story_description.as_deref().unwrap_or("(no description provided)")
    );

    let initial_messages = vec![api::ChatMessage::user(user_content)];

    let mut registry = tools::ToolRegistry::new();
    tools::builtin::register_builtins(&mut registry, db.clone());
    // Load custom shell command tools bound to this agent profile.
    match tools::shell::load_for_agent(profile_id, &db).await {
        Ok(shell_tools) => {
            for ct in shell_tools { registry.register(Box::new(ct)); }
        }
        Err(e) => tracing::error!("Failed to load custom tools for agent '{profile_id}': {e}"),
    }
    let registry = Arc::new(tokio::sync::Mutex::new(registry));

    // No approval gate for scheduled/continuous runs (auto-approve or skip)
    let gate = Arc::new(runtime::ApprovalGate::new());

    let cancel = runtime::CancelFlag::new();
    let cancel_token = cancel.clone();

    let memory = if persistent_memory {
        Some(memory::MemoryStore::new(db.clone(), profile_id).await)
    } else {
        None
    };

    let rt = runtime::ConversationRuntime::new(
        story_id,
        profile_id,
        provider,
        registry,
        permission_policy,
        gate,
        initial_messages,
        config,
        max_iterations as u32,
        db.clone(),
        std::sync::Arc::new(app.clone()),
        cancel,
        memory,
        db::get_active_workspace_path(&db).await,
        true,  // scheduled runs always track history
        10,    // default retention cap
    )
    .await
    .map_err(|e| format!("Failed to create runtime: {e}"))?;

    let run_id = rt.run_id.clone();

    {
        let mut map = scheduler.run_registry.lock().unwrap();
        map.insert(run_id.clone(), cancel_token);
    }

    let run_id_clone = run_id.clone();
    let registry_ref = scheduler.run_registry.clone();

    tokio::spawn(async move {
        if let Err(e) = rt.run().await {
            error!(run_id = %run_id_clone, "Scheduled run failed: {e}");
        }
        registry_ref.lock().unwrap().remove(&run_id_clone);
    });

    Ok(run_id)
}

/// Poll until the run_id disappears from the run_registry (run finished), or stop signal.
async fn wait_for_run(
    profile_id: &str,
    run_id: &str,
    story_id: &str,
    story_title: &str,
    scheduler: &Arc<SchedulerState>,
    db: DbPool,
    stop_rx: &mut tokio::sync::watch::Receiver<bool>,
) {
    loop {
        if *stop_rx.borrow() {
            break;
        }

        refresh_live_state_for_active_run(profile_id, run_id, story_id, story_title, scheduler, &db).await;

        let still_running = scheduler.run_registry.lock().unwrap().contains_key(run_id);
        if !still_running {
            refresh_live_state_for_finished_run(profile_id, run_id, scheduler, &db).await;
            break;
        }
        tokio::select! {
            _ = sleep(std::time::Duration::from_secs(2)) => {}
            _ = stop_rx.changed() => { break; }
        }
    }
}

/// Same but without listening to stop signal (cron fires one-shot runs).
async fn wait_for_run_no_stop(
    profile_id: &str,
    run_id: &str,
    story_id: &str,
    story_title: &str,
    scheduler: &Arc<SchedulerState>,
    db: DbPool,
) {
    loop {
        refresh_live_state_for_active_run(profile_id, run_id, story_id, story_title, scheduler, &db).await;

        let still_running = scheduler.run_registry.lock().unwrap().contains_key(run_id);
        if !still_running {
            refresh_live_state_for_finished_run(profile_id, run_id, scheduler, &db).await;
            break;
        }
        sleep(std::time::Duration::from_secs(2)).await;
    }
}

async fn refresh_live_state_for_active_run(
    profile_id: &str,
    run_id: &str,
    story_id: &str,
    story_title: &str,
    scheduler: &Arc<SchedulerState>,
    db: &DbPool,
) {
    let pending_approval = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM approval_requests WHERE run_id = ? AND status = 'pending'",
    )
    .bind(run_id)
    .fetch_one(db)
    .await
    .unwrap_or(0)
        > 0;

    if pending_approval {
        scheduler.set_live_state(
            profile_id,
            "waiting_for_approval",
            None,
            scheduler.next_fire.get(profile_id).map(|v| v.clone()),
            Some(run_id.to_string()),
            Some(story_id.to_string()),
            Some(story_title.to_string()),
            None,
        );
        return;
    }

    let waiting_human = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM stories \
         WHERE story_type = 'human' AND parent_run_id = ? \
           AND status NOT IN ('done', 'failed')",
    )
    .bind(run_id)
    .fetch_one(db)
    .await
    .unwrap_or(0)
        > 0;

    if waiting_human {
        scheduler.set_live_state(
            profile_id,
            "waiting_for_human_input",
            None,
            scheduler.next_fire.get(profile_id).map(|v| v.clone()),
            Some(run_id.to_string()),
            Some(story_id.to_string()),
            Some(story_title.to_string()),
            None,
        );
        return;
    }

    scheduler.set_live_state(
        profile_id,
        "running_story",
        None,
        scheduler.next_fire.get(profile_id).map(|v| v.clone()),
        Some(run_id.to_string()),
        Some(story_id.to_string()),
        Some(story_title.to_string()),
        None,
    );
}

async fn refresh_live_state_for_finished_run(
    profile_id: &str,
    run_id: &str,
    scheduler: &Arc<SchedulerState>,
    db: &DbPool,
) {
    let row = sqlx::query(
        "SELECT status FROM story_runs WHERE id = ?"
    )
    .bind(run_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();

    let run_status: Option<String> = row.and_then(|r| {
        use sqlx::Row;
        r.try_get("status").ok()
    });

    if run_status.as_deref() == Some("failed") {
        let failure_summary = sqlx::query_scalar::<_, String>(
            "SELECT content FROM run_events \
             WHERE run_id = ? AND event_type IN ('failed', 'error') \
             ORDER BY sequence_num DESC LIMIT 1",
        )
        .bind(run_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .map(|v| {
            let text = v.replace('\n', " ").trim().to_string();
            if text.len() > 140 {
                format!("{}...", &text[..140])
            } else {
                text
            }
        })
        .or_else(|| Some("Run failed. Open run details for context.".to_string()));

        let story_meta = scheduler.active_story.get(profile_id).map(|m| m.clone());
        let (story_id, story_title) = match story_meta {
            Some((sid, stitle)) => (Some(sid), Some(stitle)),
            None => (None, None),
        };

        scheduler.set_live_state(
            profile_id,
            "failed",
            None,
            scheduler.next_fire.get(profile_id).map(|v| v.clone()),
            Some(run_id.to_string()),
            story_id,
            story_title,
            failure_summary,
        );
    }
}
