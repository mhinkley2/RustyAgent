// Tauri command handlers exposed to the frontend.
// Implements start_run / stop_run for the agent execution loop.

pub mod agent_profiles;
pub use agent_profiles::{AgentProfile, CreateProfileInput, UpdateProfileInput};

pub mod stories;
pub use stories::{Story, CreateStoryInput, UpdateStoryInput, StoryOrderUpdate};

pub mod runs;
pub use runs::{StoryRun, RunEvent, RunFilters};

#[cfg(test)]
mod runs_tests;

pub mod models;
pub use models::{list_provider_models, ModelOption};

pub mod human;
pub use human::{HumanRequest, ApprovalRequest};

pub mod mcp_servers;
pub use mcp_servers::{McpServer, ToolBinding, CreateMcpServerInput, UpdateMcpServerInput, CreateToolBindingInput};

pub mod settings;
pub use settings::AppSettings;

pub mod workspace;
pub use workspace::{Workspace, ActiveWorkspace};

pub mod filesystem;
pub use filesystem::FileEntry;

pub mod permissions;

#[cfg(test)]
mod permissions_tests;

#[cfg(test)]
mod agent_profiles_tests;

#[cfg(test)]
mod stories_tests;
pub use permissions::AgentPermissions;
pub use runtime::ApprovalGate;

pub mod custom_tools;
pub use custom_tools::{CustomTool, CustomToolBinding, CreateCustomToolInput, UpdateCustomToolInput};

pub mod chat_sessions;
pub use chat_sessions::{ChatSessionSummary, ChatSessionMessage};

/// Response type for `start_chat_run`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatRunResponse {
    pub run_id: String,
    pub session_id: String,
}

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use db::DbPool;
use memory::MemoryStore;
use runtime::{CancelFlag, ConversationRuntime, PermissionPolicy};
use api::{
    types::{ChatMessage, CompletionConfig},
    AnthropicClient, DeepSeekClient, OllamaClient, OpenRouterClient,
};
use tools::{builtin::register_builtins, ToolRegistry};
use tauri::{AppHandle, State};
use tokio::sync::Mutex as TokioMutex;
use tracing::{error, info};

// ---------------------------------------------------------------------------
// RunRegistry — shared state tracking active runs
// ---------------------------------------------------------------------------

/// Maps run_id → CancelFlag for all currently-executing runs.
pub struct RunRegistry(pub Arc<Mutex<HashMap<String, CancelFlag>>>);

impl RunRegistry {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(HashMap::new())))
    }
}

impl Default for RunRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// start_run
// ---------------------------------------------------------------------------

/// Start an agent run for the given story and profile.
///
/// Returns the run_id that can be used to stop the run or filter events
/// from the `run-event` Tauri event channel.
#[allow(clippy::too_many_arguments)]
pub async fn start_run(
    story_id: String,
    profile_id: String,
    app: AppHandle,
    db: &DbPool,
    run_registry: State<'_, RunRegistry>,
    gate: State<'_, Arc<ApprovalGate>>,
) -> Result<String, String> {
    let db = db.clone();

    // ------------------------------------------------------------------
    // Load the agent profile from the database.
    // ------------------------------------------------------------------
    let row = sqlx::query(
        "SELECT provider, model, system_prompt, max_iterations, max_retries, max_output_tokens, persistent_memory, \
                context_strategy, max_input_tokens \
         FROM agent_profiles WHERE id = ?",
    )
    .bind(&profile_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| format!("DB error loading profile: {e}"))?
    .ok_or_else(|| format!("Profile '{profile_id}' not found"))?;

    use sqlx::Row;
    let llm_provider: String = row.try_get("provider").unwrap_or_default();
    let model_id: String = row.try_get("model").unwrap_or_default();
    let system_prompt: Option<String> = row.try_get("system_prompt").ok().flatten();
    let max_iterations: i64 = row.try_get("max_iterations").unwrap_or(20);
    let max_retries: i64 = row.try_get("max_retries").unwrap_or(2);
    let max_tokens_per_run: i64 = row.try_get("max_output_tokens").unwrap_or(4096);
    let persistent_memory: bool = row.try_get::<i64, _>("persistent_memory").unwrap_or(0) != 0;
    let context_strategy: String = row.try_get("context_strategy").unwrap_or_default();
    let max_input_tokens: Option<i64> = row.try_get("max_input_tokens").ok().flatten();

    // ------------------------------------------------------------------
    // Load the agent_permissions row (if any).
    // ------------------------------------------------------------------
    let perm_row = sqlx::query(
        "SELECT allowed_tools, allow_file_read_paths, allow_file_write_paths, \
                allow_shell_commands, require_approval_on_write \
         FROM agent_permissions WHERE profile_id = ?",
    )
    .bind(&profile_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| format!("DB error loading permissions: {e}"))?;

    // ------------------------------------------------------------------
    // Load the story to build the initial user message.
    // ------------------------------------------------------------------
    let story_row = sqlx::query(
        "SELECT title, description, track_history FROM stories WHERE id = ?",
    )
    .bind(&story_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| format!("DB error loading story: {e}"))?
    .ok_or_else(|| format!("Story '{story_id}' not found"))?;

    let story_title: String = story_row.try_get("title").unwrap_or_default();
    let story_description: Option<String> = story_row.try_get("description").ok().flatten();
    let track_history: bool = story_row.try_get::<i64, _>("track_history").unwrap_or(1) != 0;

    // ------------------------------------------------------------------
    // Build the LLM provider.
    // ------------------------------------------------------------------
    let app_settings = settings::AppSettings::load_from(
        &settings::AppSettings::settings_path(&app),
    );
    let event_retention_runs = app_settings.event_retention_runs.unwrap_or(10);

    let provider: Box<dyn api::LlmProvider> = match llm_provider.as_str() {
        "anthropic" => {
            let key = app_settings.anthropic_api_key
                .ok_or_else(|| "Anthropic API key not set — configure it in Settings.".to_string())?;
            Box::new(AnthropicClient::new(key))
        }
        "openrouter" => {
            let key = app_settings.openrouter_api_key
                .ok_or_else(|| "OpenRouter API key not set — configure it in Settings.".to_string())?;
            Box::new(OpenRouterClient::new(key))
        }
        "deepseek" => {
            let key = app_settings.deepseek_api_key
                .ok_or_else(|| "DeepSeek API key not set — configure it in Settings.".to_string())?;
            Box::new(DeepSeekClient::new(key))
        }
        "ollama" => {
            let base_url = app_settings.ollama_base_url
                .as_deref()
                .unwrap_or("http://localhost:11434");
            Box::new(OllamaClient::with_base_url(base_url))
        }
        other => return Err(format!("Unknown LLM provider: '{other}'")),
    };

    // ------------------------------------------------------------------
    // Build the tool registry.
    // ------------------------------------------------------------------
    let mut registry = ToolRegistry::new();
    register_builtins(&mut registry, db.clone());
    // Load custom shell command tools bound to this agent profile.
    match tools::shell::load_for_agent(&profile_id, &db).await {
        Ok(shell_tools) => {
            for ct in shell_tools {
                registry.register(Box::new(ct));
            }
        }
        Err(e) => {
            error!("Failed to load custom tools for agent '{profile_id}': {e}");
        }
    }
    let registry = Arc::new(TokioMutex::new(registry));
    let permission_policy = match perm_row {
        Some(pr) => {
            let tools: String  = pr.try_get("allowed_tools").unwrap_or_else(|_| "[]".into());
            let reads: String  = pr.try_get("allow_file_read_paths").unwrap_or_else(|_| "[]".into());
            let writes: String = pr.try_get("allow_file_write_paths").unwrap_or_else(|_| "[]".into());
            let cmds: String   = pr.try_get("allow_shell_commands").unwrap_or_else(|_| "[]".into());
            let req_approval: i64 = pr.try_get("require_approval_on_write").unwrap_or(0);
            PermissionPolicy::from_db_permissions(
                &tools, &reads, &writes, &cmds, req_approval != 0,
            )
        }
        None => PermissionPolicy::allow_all(),
    };

    // ------------------------------------------------------------------
    // Build initial messages.
    // ------------------------------------------------------------------
    let mut config = CompletionConfig::new(&model_id, max_tokens_per_run as u32);
    config.system_prompt = system_prompt;

    let user_content = format!(
        "Process the following story:\n\nStory ID: {story_id}\nTitle: {story_title}\n\n{}",
        story_description.as_deref().unwrap_or("(no description provided)")
    );

    let initial_messages = vec![ChatMessage::user(user_content)];

    // ------------------------------------------------------------------
    // Create the runtime and spawn the run.
    // ------------------------------------------------------------------
    let cancel = CancelFlag::new();
    let cancel_clone = cancel.clone();

    let memory = if persistent_memory {
        Some(MemoryStore::new(db.clone(), &profile_id).await)
    } else {
        None
    };

    let mut runtime = ConversationRuntime::new(
        &story_id,
        &profile_id,
        provider,
        registry,
        permission_policy,
        gate.inner().clone(),
        initial_messages,
        config,
        max_iterations as u32,
        db.clone(),
        std::sync::Arc::new(app.clone()),
        cancel,
        memory,
        workspace::get_active_workspace_path(&db).await,
        track_history,
        event_retention_runs,
    )
    .await
    .map_err(|e| format!("Failed to create runtime: {e}"))?;

    // Give the loop the profile's context settings. Set here rather than
    // passed to the constructor, which already carries far too many
    // arguments; the default is the safe one, so a caller that forgets
    // still gets `recent` compaction at a per-model budget.
    runtime.context_policy =
        runtime::ContextPolicy::from_profile(&context_strategy, max_input_tokens);

    // Let the run reach the user: notifications for a parked approval or a
    // finished run, and the approval wait the user configured (indefinite by
    // default, so an unattended run parks rather than fail-closing).
    // A transient provider failure is retried inside the run, so the
    // conversation and its completed tool work survive the wait.
    runtime.max_retries = max_retries.max(0) as u32;

    runtime.notifier = Some(runtime::AppNotifier::arc(app.clone()));
    runtime.approval_timeout = runtime::notifier::unattended_settings(&app).approval_timeout();

    // Move the run into its own git worktree before the loop starts, so the
    // agent's file tools are pointed at an isolated checkout rather than the
    // user's. A workspace that cannot be isolated records why on the run row
    // and in the timeline; it never proceeds silently.
    if let Some(dir) = runtime::worktree::dir_for(&app) {
        runtime.isolate(&dir).await;
    }

    let run_id = runtime.run_id.clone();

    // Register the cancel flag before spawning so stop_run can find it.
    {
        let mut map = run_registry.0.lock().unwrap();
        map.insert(run_id.clone(), cancel_clone);
    }

    let run_id_clone = run_id.clone();
    let registry_ref = run_registry.0.clone();

    tauri::async_runtime::spawn(async move {
        info!(run_id = %run_id_clone, "Spawned agent run task");
        if let Err(e) = runtime.run().await {
            error!(run_id = %run_id_clone, "Run failed: {e}");
        }
        // Clean up the registry entry when the run finishes.
        registry_ref.lock().unwrap().remove(&run_id_clone);
        info!(run_id = %run_id_clone, "Agent run task finished");
    });

    Ok(run_id)
}

// ---------------------------------------------------------------------------
// stop_run
// ---------------------------------------------------------------------------

/// Signal a running agent to stop after its current iteration.
pub async fn stop_run(
    run_id: String,
    run_registry: State<'_, RunRegistry>,
) -> Result<(), String> {
    let map = run_registry.0.lock().unwrap();
    match map.get(&run_id) {
        Some(flag) => {
            flag.cancel();
            info!(run_id = %run_id, "Cancel signal sent");
            Ok(())
        }
        None => Err(format!("No active run with id '{run_id}'")),
    }
}

// ---------------------------------------------------------------------------
// start_chat_run
// ---------------------------------------------------------------------------

/// Start a direct chat run against an agent profile.
///
/// Unlike `start_run`, this takes the full message history directly and does
/// not require a pre-existing story. It creates a chat-type story to anchor
/// the run if none is provided.
pub async fn start_chat_run(
    profile_id: String,
    messages: Vec<api::ChatMessage>,
    session_id: Option<String>,
    session_title: Option<String>,
    workspace_id: Option<String>,
    app: AppHandle,
    db: &DbPool,
    run_registry: State<'_, RunRegistry>,
    gate: State<'_, Arc<ApprovalGate>>,
) -> Result<ChatRunResponse, String> {
    use uuid::Uuid;

    let db = db.clone();

    // Load the agent profile.
    let row = sqlx::query(
        "SELECT provider, model, system_prompt, max_iterations, max_retries, max_output_tokens, persistent_memory, \
                context_strategy, max_input_tokens \
         FROM agent_profiles WHERE id = ?",
    )
    .bind(&profile_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| format!("DB error loading profile: {e}"))?
    .ok_or_else(|| format!("Profile '{profile_id}' not found"))?;

    use sqlx::Row;
    let llm_provider: String  = row.try_get("provider").unwrap_or_default();
    let model_id: String      = row.try_get("model").unwrap_or_default();
    let system_prompt: Option<String> = row.try_get("system_prompt").ok().flatten();
    let max_iterations: i64   = row.try_get("max_iterations").unwrap_or(20);
    let max_retries: i64      = row.try_get("max_retries").unwrap_or(2);
    let max_tokens: i64       = row.try_get("max_output_tokens").unwrap_or(4096);
    let persistent_memory: bool = row.try_get::<i64, _>("persistent_memory").unwrap_or(0) != 0;
    let context_strategy: String = row.try_get("context_strategy").unwrap_or_default();
    let max_input_tokens: Option<i64> = row.try_get("max_input_tokens").ok().flatten();

    // Load permissions for this profile.
    let perm_row = sqlx::query(
        "SELECT allowed_tools, allow_file_read_paths, allow_file_write_paths, \
                allow_shell_commands, require_approval_on_write \
         FROM agent_permissions WHERE profile_id = ?",
    )
    .bind(&profile_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| format!("DB error loading permissions: {e}"))?;

    // Build the LLM provider.
    let app_settings = settings::AppSettings::load_from(&settings::AppSettings::settings_path(&app));

    let provider: Box<dyn api::LlmProvider> = match llm_provider.as_str() {
        "anthropic" => {
            let key = app_settings.anthropic_api_key
                .ok_or_else(|| "Anthropic API key not set — configure it in Settings.".to_string())?;
            Box::new(AnthropicClient::new(key))
        }
        "openrouter" => {
            let key = app_settings.openrouter_api_key
                .ok_or_else(|| "OpenRouter API key not set — configure it in Settings.".to_string())?;
            Box::new(OpenRouterClient::new(key))
        }
        "deepseek" => {
            let key = app_settings.deepseek_api_key
                .ok_or_else(|| "DeepSeek API key not set — configure it in Settings.".to_string())?;
            Box::new(DeepSeekClient::new(key))
        }
        "ollama" => {
            let base_url = app_settings.ollama_base_url
                .as_deref()
                .unwrap_or("http://localhost:11434");
            Box::new(OllamaClient::with_base_url(base_url))
        }
        other => return Err(format!("Unknown LLM provider: '{other}'")),
    };

    // Ensure a chat session story exists in the DB (required by the FK constraint).
    let session_id = if let Some(id) = session_id {
        // Session already exists — ensure the row is present (defensive INSERT OR IGNORE).
        sqlx::query(
              "INSERT OR IGNORE INTO stories (id, title, story_type, status, priority, workspace_id) \
               VALUES (?, 'Chat Session', 'chat', 'in_progress', 'medium', ?)",
        )
        .bind(&id)
           .bind(&workspace_id)
        .execute(&db)
        .await
        .map_err(|e| format!("DB error upserting chat session: {e}"))?;

        if let Some(title) = session_title.as_ref().map(|value| value.trim()).filter(|value| !value.is_empty()) {
            sqlx::query(
                "UPDATE stories
                 SET title = ?
                 WHERE id = ?
                   AND story_type = 'chat'
                   AND TRIM(COALESCE(title, '')) IN ('', 'New Chat', 'Chat Session')",
            )
            .bind(title)
            .bind(&id)
            .execute(&db)
            .await
            .map_err(|e| format!("DB error updating chat session title: {e}"))?;
        }

        id
    } else {
        let id = Uuid::new_v4().to_string();
        let title = session_title
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Chat Session".to_string());
        sqlx::query(
              "INSERT INTO stories (id, title, story_type, status, priority, workspace_id) \
               VALUES (?, ?, 'chat', 'in_progress', 'medium', ?)",
        )
        .bind(&id)
        .bind(&title)
           .bind(&workspace_id)
        .execute(&db)
        .await
        .map_err(|e| format!("DB error creating chat session: {e}"))?;
        id
    };

    // Tool registry — same as start_run: builtins + profile-bound custom shell tools.
    let mut registry = ToolRegistry::new();
    register_builtins(&mut registry, db.clone());
    match tools::shell::load_for_agent(&profile_id, &db).await {
        Ok(shell_tools) => {
            for ct in shell_tools {
                registry.register(Box::new(ct));
            }
        }
        Err(e) => {
            error!("Failed to load custom tools for chat agent '{profile_id}': {e}");
        }
    }
    let registry = Arc::new(TokioMutex::new(registry));

    let permission_policy = match perm_row {
        Some(pr) => {
            let tools: String  = pr.try_get("allowed_tools").unwrap_or_else(|_| "[]".into());
            let reads: String  = pr.try_get("allow_file_read_paths").unwrap_or_else(|_| "[]".into());
            let writes: String = pr.try_get("allow_file_write_paths").unwrap_or_else(|_| "[]".into());
            let cmds: String   = pr.try_get("allow_shell_commands").unwrap_or_else(|_| "[]".into());
            let req_approval: i64 = pr.try_get("require_approval_on_write").unwrap_or(0);
            PermissionPolicy::from_db_permissions(
                &tools, &reads, &writes, &cmds, req_approval != 0,
            )
        }
        None => PermissionPolicy::allow_all(),
    };

    let memory = if persistent_memory {
        Some(MemoryStore::new(db.clone(), &profile_id).await)
    } else {
        None
    };

    let mut config = CompletionConfig::new(&model_id, max_tokens as u32);
    config.system_prompt = system_prompt;

    let cancel       = CancelFlag::new();
    let cancel_clone = cancel.clone();

    let mut runtime = ConversationRuntime::new(
        &session_id,
        &profile_id,
        provider,
        registry,
        permission_policy,
        gate.inner().clone(),
        messages,
        config,
        max_iterations as u32,
        db.clone(),
        std::sync::Arc::new(app.clone()),
        cancel,
        memory,
        workspace::get_active_workspace_path(&db).await,
        true, // chat sessions always track history
        app_settings.event_retention_runs.unwrap_or(10),
    )
    .await
    .map_err(|e| format!("Failed to create chat runtime: {e}"))?;

    // Give the loop the profile's context settings. Set here rather than
    // passed to the constructor, which already carries far too many
    // arguments; the default is the safe one, so a caller that forgets
    // still gets `recent` compaction at a per-model budget.
    runtime.context_policy =
        runtime::ContextPolicy::from_profile(&context_strategy, max_input_tokens);

    // Let the run reach the user: notifications for a parked approval or a
    // finished run, and the approval wait the user configured (indefinite by
    // default, so an unattended run parks rather than fail-closing).
    // A transient provider failure is retried inside the run, so the
    // conversation and its completed tool work survive the wait.
    runtime.max_retries = max_retries.max(0) as u32;

    runtime.notifier = Some(runtime::AppNotifier::arc(app.clone()));
    runtime.approval_timeout = runtime::notifier::unattended_settings(&app).approval_timeout();

    let run_id = runtime.run_id.clone();

    {
        let mut map = run_registry.0.lock().unwrap();
        map.insert(run_id.clone(), cancel_clone);
    }

    let run_id_clone   = run_id.clone();
    let registry_ref   = run_registry.0.clone();

    tauri::async_runtime::spawn(async move {
        info!(run_id = %run_id_clone, "Spawned chat run task");
        if let Err(e) = runtime.run().await {
            error!(run_id = %run_id_clone, "Chat run failed: {e}");
        }
        registry_ref.lock().unwrap().remove(&run_id_clone);
        info!(run_id = %run_id_clone, "Chat run task finished");
    });

    Ok(ChatRunResponse { run_id, session_id })
}


#[cfg(test)]
mod human_tests;
