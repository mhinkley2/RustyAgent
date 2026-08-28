// Tauri entry point. Command handlers are implemented in the `commands` crate.
mod mcp_host;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use commands::{ApprovalGate, RunRegistry};
use tauri::{Manager, State};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

// Scheduler state alias
type SchedulerState = scheduler::SchedulerState;
// Pipeline state alias
type PipelineState = pipeline::PipelineState;

struct LogFileState {
    path: PathBuf,
    _guard: tracing_appender::non_blocking::WorkerGuard,
}

#[derive(serde::Serialize)]
struct AppLogPayload {
    path: String,
    content: String,
}

fn init_logging(log_dir: &Path) -> Result<LogFileState, String> {
    fs::create_dir_all(log_dir).map_err(|e| format!("Failed to create log directory: {e}"))?;

    let log_file_name = "rustyagent.log";
    let file_appender = tracing_appender::rolling::never(log_dir, log_file_name);
    let (writer, guard) = tracing_appender::non_blocking(file_appender);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer),
        )
        .try_init();

    Ok(LogFileState {
        path: log_dir.join(log_file_name),
        _guard: guard,
    })
}

async fn resolve_active_workspace_id(
    db: &db::DbPool,
    active_ws: &commands::ActiveWorkspace,
) -> Option<String> {
    if let Some(id) = active_ws.get() {
        return Some(id);
    }

    // Fallback for fresh app sessions: recover the most recently opened workspace.
    let recent = db::get_most_recent_workspace(db).await.map(|w| w.id);
    if let Some(ref id) = recent {
        active_ws.set(Some(id.clone()));
    }
    recent
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn get_app_logs(logs: State<'_, LogFileState>) -> Result<AppLogPayload, String> {
    let content = fs::read_to_string(&logs.path).unwrap_or_default();
    Ok(AppLogPayload {
        path: logs.path.display().to_string(),
        content,
    })
}

#[tauri::command]
fn clear_app_logs(logs: State<'_, LogFileState>) -> Result<(), String> {
    fs::write(&logs.path, "").map_err(|e| format!("Failed to clear app logs: {e}"))
}

// ---------------------------------------------------------------------------
// Tauri command wrappers
// #[tauri::command] must live in the binary crate (not library crates).
// ---------------------------------------------------------------------------

#[tauri::command]
async fn start_run(
    story_id: String,
    profile_id: String,
    app: tauri::AppHandle,
    db: tauri::State<'_, db::DbPool>,
    run_registry: tauri::State<'_, RunRegistry>,
    gate: tauri::State<'_, Arc<ApprovalGate>>,
) -> Result<String, String> {
    commands::start_run(story_id, profile_id, app, db.inner(), run_registry, gate).await
}

#[tauri::command]
async fn stop_run(
    run_id: String,
    run_registry: tauri::State<'_, RunRegistry>,
) -> Result<(), String> {
    commands::stop_run(run_id, run_registry).await
}

#[tauri::command]
async fn start_chat_run(
    profile_id: String,
    messages: Vec<api::ChatMessage>,
    session_id: Option<String>,
    session_title: Option<String>,
    app: tauri::AppHandle,
    db: tauri::State<'_, db::DbPool>,
    active_ws: tauri::State<'_, commands::ActiveWorkspace>,
    run_registry: tauri::State<'_, RunRegistry>,
    gate: tauri::State<'_, Arc<ApprovalGate>>,
) -> Result<commands::ChatRunResponse, String> {
    let workspace_id = resolve_active_workspace_id(db.inner(), active_ws.inner()).await;
    commands::start_chat_run(
        profile_id,
        messages,
        session_id,
        session_title,
        workspace_id,
        app,
        db.inner(),
        run_registry,
        gate,
    )
    .await
}

#[tauri::command]
async fn list_chat_sessions(
    limit: Option<i64>,
    db: tauri::State<'_, db::DbPool>,
    active_ws: tauri::State<'_, commands::ActiveWorkspace>,
) -> Result<Vec<commands::ChatSessionSummary>, String> {
    let workspace_id = resolve_active_workspace_id(db.inner(), active_ws.inner()).await;
    commands::chat_sessions::list_chat_sessions(workspace_id, limit, db.inner()).await
}

#[tauri::command]
async fn create_chat_session(
    title: Option<String>,
    db: tauri::State<'_, db::DbPool>,
    active_ws: tauri::State<'_, commands::ActiveWorkspace>,
) -> Result<commands::ChatSessionSummary, String> {
    let workspace_id = resolve_active_workspace_id(db.inner(), active_ws.inner()).await;
    commands::chat_sessions::create_chat_session(workspace_id, title, db.inner()).await
}

#[tauri::command]
async fn get_chat_session_messages(
    session_id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<Vec<commands::ChatSessionMessage>, String> {
    commands::chat_sessions::get_chat_session_messages(session_id, db.inner()).await
}

#[tauri::command]
async fn append_chat_session_message(
    session_id: String,
    role: String,
    content: String,
    agent_profile_id: Option<String>,
    db: tauri::State<'_, db::DbPool>,
) -> Result<(), String> {
    commands::chat_sessions::append_chat_session_message(
        session_id,
        role,
        content,
        agent_profile_id,
        db.inner(),
    )
    .await
}

// ── Agent profile CRUD ──────────────────────────────────────────────────────

#[tauri::command]
async fn get_profiles(
    db: tauri::State<'_, db::DbPool>,
    active_ws: tauri::State<'_, commands::ActiveWorkspace>,
) -> Result<Vec<commands::AgentProfile>, String> {
    let workspace_id = resolve_active_workspace_id(db.inner(), active_ws.inner()).await;
    commands::agent_profiles::get_profiles(db.inner(), workspace_id).await
}

#[tauri::command]
async fn get_profile(
    id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<commands::AgentProfile, String> {
    commands::agent_profiles::get_profile(id, db.inner()).await
}

#[tauri::command]
async fn create_profile(
    input: commands::CreateProfileInput,
    db: tauri::State<'_, db::DbPool>,
    active_ws: tauri::State<'_, commands::ActiveWorkspace>,
) -> Result<commands::AgentProfile, String> {
    let workspace_id = resolve_active_workspace_id(db.inner(), active_ws.inner()).await;
    commands::agent_profiles::create_profile(input, db.inner(), workspace_id).await
}

#[tauri::command]
async fn update_profile(
    id: String,
    input: commands::UpdateProfileInput,
    db: tauri::State<'_, db::DbPool>,
) -> Result<commands::AgentProfile, String> {
    commands::agent_profiles::update_profile(id, input, db.inner()).await
}

#[tauri::command]
async fn delete_profile(
    id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<(), String> {
    commands::agent_profiles::delete_profile(id, db.inner()).await
}

// ── Story CRUD ──────────────────────────────────────────────────────────────

#[tauri::command]
async fn get_stories(
    db: tauri::State<'_, db::DbPool>,
    active_ws: tauri::State<'_, commands::ActiveWorkspace>,
) -> Result<Vec<commands::Story>, String> {
    let workspace_id = resolve_active_workspace_id(db.inner(), active_ws.inner()).await;
    commands::stories::get_stories(db.inner(), workspace_id).await
}

#[tauri::command]
async fn get_story(
    id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<commands::Story, String> {
    commands::stories::get_story(id, db.inner()).await
}

#[tauri::command]
async fn create_story(
    input: commands::CreateStoryInput,
    db: tauri::State<'_, db::DbPool>,
    active_ws: tauri::State<'_, commands::ActiveWorkspace>,
) -> Result<commands::Story, String> {
    let workspace_id = resolve_active_workspace_id(db.inner(), active_ws.inner()).await;
    commands::stories::create_story(input, db.inner(), workspace_id).await
}

#[tauri::command]
async fn update_story(
    id: String,
    input: commands::UpdateStoryInput,
    db: tauri::State<'_, db::DbPool>,
    active_ws: tauri::State<'_, commands::ActiveWorkspace>,
) -> Result<commands::Story, String> {
    let workspace_id = resolve_active_workspace_id(db.inner(), active_ws.inner()).await;
    commands::stories::update_story(id, input, db.inner(), workspace_id).await
}

#[tauri::command]
async fn delete_story(
    id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<(), String> {
    commands::stories::delete_story(id, db.inner()).await
}

#[tauri::command]
async fn batch_update_story_order(
    updates: Vec<commands::StoryOrderUpdate>,
    db: tauri::State<'_, db::DbPool>,
) -> Result<(), String> {
    commands::stories::batch_update_story_order(updates, db.inner()).await
}

// ── Run history ─────────────────────────────────────────────────────────────

#[tauri::command]
async fn get_runs(
    filters: Option<commands::RunFilters>,
    db: tauri::State<'_, db::DbPool>,
    active_ws: tauri::State<'_, commands::ActiveWorkspace>,
) -> Result<Vec<commands::StoryRun>, String> {
    let workspace_id = resolve_active_workspace_id(db.inner(), active_ws.inner()).await;
    commands::runs::get_runs(filters, workspace_id, db.inner()).await
}

#[tauri::command]
async fn get_run(
    id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<commands::StoryRun, String> {
    commands::runs::get_run(id, db.inner()).await
}

#[tauri::command]
async fn get_run_events(
    run_id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<Vec<commands::RunEvent>, String> {
    commands::runs::get_run_events(run_id, db.inner()).await
}

#[tauri::command]
async fn delete_run(
    id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<(), String> {
    commands::runs::delete_run(id, db.inner()).await
}

#[tauri::command]
async fn export_run_events(
    run_id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<String, String> {
    commands::runs::export_run_events(run_id, db.inner()).await
}

#[tauri::command]
async fn get_run_diff(
    run_id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<commands::runs::RunDiff, String> {
    commands::runs::get_run_diff(run_id, db.inner()).await
}

/// Bring a finished run's changes into the user's working tree.
///
/// Staged, not committed, and git refuses the merge rather than overwriting
/// uncommitted local work.
#[tauri::command]
async fn accept_run(
    run_id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<String, String> {
    commands::runs::accept_run(run_id, db.inner()).await
}

/// Throw a finished run's changes away by deleting its worktree and branch.
///
/// The user's working tree is not touched: the run never wrote there.
#[tauri::command]
async fn revert_run(
    run_id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<String, String> {
    commands::runs::revert_run(run_id, db.inner()).await
}

// ── Human-in-the-loop ───────────────────────────────────────────────────────

#[tauri::command]
async fn get_pending_human_requests(
    db: tauri::State<'_, db::DbPool>,
) -> Result<Vec<commands::HumanRequest>, String> {
    commands::human::get_pending_human_requests(db.inner()).await
}

#[tauri::command]
async fn respond_to_human_request(
    story_id: String,
    response: String,
    app: tauri::AppHandle,
    db: tauri::State<'_, db::DbPool>,
) -> Result<(), String> {
    commands::human::respond_to_human_request(story_id, response, app, db.inner()).await
}

#[tauri::command]
async fn create_human_request(
    run_id: String,
    question: String,
    context: Option<String>,
    app: tauri::AppHandle,
    db: tauri::State<'_, db::DbPool>,
) -> Result<String, String> {
    commands::human::create_human_request(run_id, question, context, app, db.inner()).await
}

#[tauri::command]
async fn get_pending_approvals(
    db: tauri::State<'_, db::DbPool>,
) -> Result<Vec<commands::ApprovalRequest>, String> {
    commands::human::get_pending_approvals(db.inner()).await
}

#[tauri::command]
async fn create_approval_request(
    run_id: String,
    tool_name: String,
    tool_input: String,
    app: tauri::AppHandle,
    db: tauri::State<'_, db::DbPool>,
) -> Result<String, String> {
    commands::human::create_approval_request(run_id, tool_name, tool_input, app, db.inner()).await
}

#[tauri::command]
async fn decide_approval(
    id: String,
    approved: bool,
    rejection_reason: Option<String>,
    app: tauri::AppHandle,
    db: tauri::State<'_, db::DbPool>,
    gate: tauri::State<'_, Arc<ApprovalGate>>,
) -> Result<(), String> {
    commands::human::decide_approval(id, approved, rejection_reason, app, db.inner(), gate).await
}

// ── MCP Servers CRUD ────────────────────────────────────────────────────────

#[tauri::command]
async fn get_mcp_servers(
    db: tauri::State<'_, db::DbPool>,
) -> Result<Vec<commands::McpServer>, String> {
    commands::mcp_servers::get_mcp_servers(db.inner()).await
}

#[tauri::command]
async fn get_mcp_server(
    id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<commands::McpServer, String> {
    commands::mcp_servers::get_mcp_server(id, db.inner()).await
}

#[tauri::command]
async fn create_mcp_server(
    input: commands::CreateMcpServerInput,
    db: tauri::State<'_, db::DbPool>,
) -> Result<commands::McpServer, String> {
    commands::mcp_servers::create_mcp_server(input, db.inner()).await
}

#[tauri::command]
async fn update_mcp_server(
    id: String,
    input: commands::UpdateMcpServerInput,
    db: tauri::State<'_, db::DbPool>,
) -> Result<commands::McpServer, String> {
    commands::mcp_servers::update_mcp_server(id, input, db.inner()).await
}

#[tauri::command]
async fn delete_mcp_server(
    id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<(), String> {
    commands::mcp_servers::delete_mcp_server(id, db.inner()).await
}

// ── Tool Bindings ────────────────────────────────────────────────────────────

#[tauri::command]
async fn get_tool_bindings(
    agent_profile_id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<Vec<commands::ToolBinding>, String> {
    commands::mcp_servers::get_tool_bindings(agent_profile_id, db.inner()).await
}

#[tauri::command]
async fn create_tool_binding(
    input: commands::CreateToolBindingInput,
    db: tauri::State<'_, db::DbPool>,
) -> Result<commands::ToolBinding, String> {
    commands::mcp_servers::create_tool_binding(input, db.inner()).await
}

#[tauri::command]
async fn delete_tool_binding(
    id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<(), String> {
    commands::mcp_servers::delete_tool_binding(id, db.inner()).await
}

#[tauri::command]
async fn update_tool_binding_allowed_tools(
    id: String,
    allowed_tools: Option<Vec<String>>,
    db: tauri::State<'_, db::DbPool>,
) -> Result<commands::ToolBinding, String> {
    commands::mcp_servers::update_tool_binding_allowed_tools(id, allowed_tools, db.inner()).await
}

// ── Agent Permissions ────────────────────────────────────────────────────────

#[tauri::command]
async fn get_agent_permissions(
    profile_id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<commands::AgentPermissions, String> {
    commands::permissions::get_agent_permissions(profile_id, db.inner()).await
}

#[tauri::command]
async fn upsert_agent_permissions(
    perms: commands::AgentPermissions,
    db: tauri::State<'_, db::DbPool>,
) -> Result<(), String> {
    commands::permissions::upsert_agent_permissions(perms, db.inner()).await
}

// ── Custom Tools ──────────────────────────────────────────────────────────────

#[tauri::command]
async fn get_custom_tools(
    workspace_id: Option<String>,
    db: tauri::State<'_, db::DbPool>,
) -> Result<Vec<commands::CustomTool>, String> {
    commands::custom_tools::get_custom_tools(workspace_id, db.inner()).await
}

#[tauri::command]
async fn create_custom_tool(
    input: commands::CreateCustomToolInput,
    db: tauri::State<'_, db::DbPool>,
) -> Result<commands::CustomTool, String> {
    commands::custom_tools::create_custom_tool(input, db.inner()).await
}

#[tauri::command]
async fn update_custom_tool(
    id: String,
    input: commands::UpdateCustomToolInput,
    db: tauri::State<'_, db::DbPool>,
) -> Result<commands::CustomTool, String> {
    commands::custom_tools::update_custom_tool(id, input, db.inner()).await
}

#[tauri::command]
async fn delete_custom_tool(
    id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<(), String> {
    commands::custom_tools::delete_custom_tool(id, db.inner()).await
}

#[tauri::command]
async fn get_custom_tool_bindings(
    agent_profile_id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<Vec<commands::CustomToolBinding>, String> {
    commands::custom_tools::get_custom_tool_bindings(agent_profile_id, db.inner()).await
}

#[tauri::command]
async fn create_custom_tool_binding(
    agent_profile_id: String,
    custom_tool_id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<commands::CustomToolBinding, String> {
    commands::custom_tools::create_custom_tool_binding(agent_profile_id, custom_tool_id, db.inner()).await
}

#[tauri::command]
async fn delete_custom_tool_binding(
    agent_profile_id: String,
    custom_tool_id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<(), String> {
    commands::custom_tools::delete_custom_tool_binding(agent_profile_id, custom_tool_id, db.inner()).await
}

// ── Settings ─────────────────────────────────────────────────────────────────

#[tauri::command]
async fn get_settings(
    app: tauri::AppHandle,
) -> Result<commands::AppSettings, String> {
    commands::settings::get_settings(app).await
}

#[tauri::command]
async fn save_settings(
    settings: commands::AppSettings,
    app: tauri::AppHandle,
) -> Result<(), String> {
    commands::settings::save_settings(settings, app).await
}

#[tauri::command]
async fn get_workspace_settings(
    workspace_id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<serde_json::Value, String> {
    commands::settings::get_workspace_settings(workspace_id, db.inner()).await
}

#[tauri::command]
async fn save_workspace_settings(
    workspace_id: String,
    overrides: serde_json::Value,
    db: tauri::State<'_, db::DbPool>,
) -> Result<(), String> {
    commands::settings::save_workspace_settings(workspace_id, overrides, db.inner()).await
}

// ── Workspace ────────────────────────────────────────────────────────────────

#[tauri::command]
async fn open_workspace(
    path: String,
    db: tauri::State<'_, db::DbPool>,
    active_ws: tauri::State<'_, commands::ActiveWorkspace>,
    app: tauri::AppHandle,
) -> Result<commands::Workspace, String> {
    commands::workspace::open_workspace(path, db.inner(), active_ws.inner(), app).await
}

#[tauri::command]
async fn get_recent_workspaces(
    db: tauri::State<'_, db::DbPool>,
) -> Result<Vec<commands::Workspace>, String> {
    commands::workspace::get_recent_workspaces(db.inner()).await
}

#[tauri::command]
async fn remove_workspace(
    id: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<(), String> {
    commands::workspace::remove_workspace(id, db.inner()).await
}

#[tauri::command]
async fn save_profile_toml(
    id: String,
    scope: String,
    workspace_root: Option<String>,
    db: tauri::State<'_, db::DbPool>,
) -> Result<String, String> {
    commands::agent_profiles::save_profile_toml(id, scope, workspace_root, db.inner()).await
}

#[tauri::command]
async fn sync_toml_profiles(
    workspace_root: Option<String>,
    db: tauri::State<'_, db::DbPool>,
) -> Result<(), String> {
    commands::agent_profiles::sync_toml_profiles(workspace_root, db.inner()).await
}

// ── Filesystem ───────────────────────────────────────────────────────────────

#[tauri::command]
async fn list_directory(
    path: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<Vec<commands::FileEntry>, String> {
    commands::filesystem::list_directory(path, db.inner()).await
}

#[tauri::command]
async fn read_file_text(
    path: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<String, String> {
    commands::filesystem::read_file_text(path, db.inner()).await
}

#[tauri::command]
async fn write_file_text(
    path: String,
    content: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<(), String> {
    commands::filesystem::write_file_text(path, content, db.inner()).await
}

#[tauri::command]
async fn rename_path(
    old_path: String,
    new_name: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<String, String> {
    commands::filesystem::rename_path(old_path, new_name, db.inner()).await
}

#[tauri::command]
async fn duplicate_file(
    path: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<String, String> {
    commands::filesystem::duplicate_file(path, db.inner()).await
}

#[tauri::command]
async fn delete_path(
    path: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<(), String> {
    commands::filesystem::delete_path(path, db.inner()).await
}

#[tauri::command]
async fn create_empty_file(
    path: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<String, String> {
    commands::filesystem::create_empty_file(path, db.inner()).await
}

#[tauri::command]
async fn create_dir_fs(
    path: String,
    db: tauri::State<'_, db::DbPool>,
) -> Result<String, String> {
    commands::filesystem::create_dir_fs(path, db.inner()).await
}

#[tauri::command]
async fn reveal_in_explorer(
    path: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let p = std::path::PathBuf::from(&path);
    let dir = if p.is_dir() {
        p
    } else {
        p.parent().map(|d| d.to_path_buf()).unwrap_or_default()
    };
    app.opener()
        .open_path(dir.to_string_lossy().to_string(), None::<String>)
        .map_err(|e| format!("Failed to open file manager: {e}"))
}

// ── Scheduler ────────────────────────────────────────────────────────────────

#[tauri::command]
async fn start_continuous_mode(
    profile_id: String,
    poll_interval_secs: Option<u64>,
    app: tauri::AppHandle,
    db: tauri::State<'_, db::DbPool>,
    sched: tauri::State<'_, Arc<SchedulerState>>,
) -> Result<(), String> {
    let interval = poll_interval_secs.unwrap_or(30);
    scheduler::start_continuous(
        &profile_id,
        interval,
        sched.inner().clone(),
        db.inner().clone(),
        app,
    )
    .await
}

#[tauri::command]
async fn start_scheduled_mode(
    profile_id: String,
    cron_expression: String,
    app: tauri::AppHandle,
    db: tauri::State<'_, db::DbPool>,
    sched: tauri::State<'_, Arc<SchedulerState>>,
) -> Result<(), String> {
    scheduler::start_scheduled(
        &profile_id,
        &cron_expression,
        sched.inner().clone(),
        db.inner().clone(),
        app,
    )
    .await
}

#[tauri::command]
fn stop_agent_scheduler(
    profile_id: String,
    sched: tauri::State<'_, Arc<SchedulerState>>,
) -> Result<(), String> {
    scheduler::stop_agent_scheduler(&profile_id, sched.inner().clone())
}

#[tauri::command]
fn get_agent_runtime_status(
    profile_id: String,
    sched: tauri::State<'_, Arc<SchedulerState>>,
) -> scheduler::AgentRuntimeStatus {
    scheduler::get_status(&profile_id, sched.inner().clone())
}

#[tauri::command]
fn get_all_agent_runtime_statuses(
    sched: tauri::State<'_, Arc<SchedulerState>>,
) -> Vec<scheduler::AgentRuntimeStatus> {
    scheduler::get_all_statuses(sched.inner().clone())
}

// ── Pipeline ─────────────────────────────────────────────────────────────────

#[tauri::command]
async fn start_pipeline_run(
    story_id: String,
    profile_id: String,
    app: tauri::AppHandle,
    db: tauri::State<'_, db::DbPool>,
    pipeline: tauri::State<'_, Arc<PipelineState>>,
) -> Result<String, String> {
    pipeline::start_pipeline(
        story_id,
        profile_id,
        pipeline.inner().clone(),
        db.inner().clone(),
        app,
    )
    .await
}

#[tauri::command]
fn get_pipeline_progress(
    pipeline_run_id: String,
    pipeline: tauri::State<'_, Arc<PipelineState>>,
) -> Option<pipeline::PipelineProgress> {
    pipeline::get_pipeline_progress(&pipeline_run_id, pipeline.inner().clone())
}

#[tauri::command]
fn list_active_pipelines(
    pipeline: tauri::State<'_, Arc<PipelineState>>,
) -> Vec<pipeline::PipelineProgress> {
    pipeline::list_active_pipelines(pipeline.inner().clone())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(RunRegistry::new())
        .manage(Arc::new(ApprovalGate::new()))
        .manage(commands::ActiveWorkspace::new())
        .setup(|app| {
            // Resolve where everything lives before anything is opened, and
            // abort loudly if the answer is unusable. Falling back to the
            // default when an override was asked for is the failure this
            // guards against: two builds would silently share one database,
            // and the first migration on either would brick the other.
            //
            // These panics reach stderr only -- the log file lives in the
            // directory being resolved, so there is nowhere else to put them.
            let data_dir = db::paths::data_dir(app.path().app_data_dir().ok())
                .unwrap_or_else(|error| panic!("{error}"));
            db::paths::prepare_data_dir(&data_dir).unwrap_or_else(|error| panic!("{error}"));
            let app_data_dir = data_dir.path.clone();

            let log_state = init_logging(&app_data_dir.join("logs"))
                .expect("Failed to initialize application logs");
            app.manage(log_state);

            let db_path = db::paths::db_path(&app_data_dir);
            db::paths::prepare_db_parent(&db_path).unwrap_or_else(|error| panic!("{error}"));

            // The first diagnostic question when two builds disagree about
            // their data is which files each one actually opened.
            if data_dir.is_overridden() {
                tracing::info!(
                    "Data directory: {} (overridden by {})",
                    app_data_dir.display(),
                    db::paths::DATA_DIR_ENV
                );
            } else {
                tracing::info!("Data directory: {}", app_data_dir.display());
            }
            if !db_path.exists() {
                // An empty override is indistinguishable from data loss unless
                // the first run against it says so out loud.
                // Says "this location", not "this data directory": the database
                // can be moved on its own by RUSTYAGENT_DB_PATH while the data
                // directory stays exactly where it was.
                tracing::info!(
                    "No database at {} yet - creating one (first run against this location)",
                    db_path.display()
                );
            }
            tracing::info!("Database: {}", db_path.display());

            let db_path_str = db_path.to_string_lossy().into_owned();
            let pool = tauri::async_runtime::block_on(db::init_db(&db_path_str))
                .expect("Failed to initialize database");

            // Share the run registry Arc with the scheduler
            let run_reg = app.state::<RunRegistry>().inner().0.clone();
            let sched_state = Arc::new(scheduler::SchedulerState::new(run_reg.clone()));
            let sched_clone = sched_state.clone();
            let pipeline_state = Arc::new(pipeline::PipelineState::new(run_reg));
            let pool_clone = pool.clone();
            let app_handle = app.handle().clone();
            app.manage(pool);
            app.manage(sched_state);
            app.manage(pipeline_state);

            // A port conflict must not stop RustyAgent from launching: external
            // MCP access is optional, the app is fully usable without it.
            if let Err(error) = mcp_host::spawn(
                app.handle().clone(),
                pool_clone.clone(),
                Some(app_data_dir.clone()),
            ) {
                tracing::warn!(
                    "MCP server not started: {error}. RustyAgent will run normally                      without external MCP access."
                );
            }

            // Restore active workspace from the most-recently-opened entry.
            let pool_ws = pool_clone.clone();
            if let Some(ws) = tauri::async_runtime::block_on(db::get_most_recent_workspace(&pool_ws)) {
                app.state::<commands::ActiveWorkspace>().set(Some(ws.id));
            }

            // Drop worktrees no run claims any more. A run that finished but
            // has not been accepted or reverted still claims its own, and is
            // left alone — the user has not decided about it yet. Nothing
            // outside `<app data>/worktrees` is ever considered.
            let pool_sweep = pool_clone.clone();
            let worktrees_dir = app_data_dir.join("worktrees");
            tauri::async_runtime::spawn(async move {
                match commands::runs::sweep_orphaned_worktrees(&worktrees_dir, &pool_sweep).await {
                    Ok(0) => {}
                    Ok(n) => tracing::info!("Swept {n} orphaned run worktree(s) at startup"),
                    Err(e) => tracing::warn!("Worktree sweep failed: {e}"),
                }
            });

            // Restore continuous/scheduled profiles in the background
            tauri::async_runtime::spawn(async move {
                scheduler::restore_schedulers(sched_clone, pool_clone, app_handle).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            get_app_logs,
            clear_app_logs,
            start_run,
            stop_run,
            get_profiles,
            get_profile,
            create_profile,
            update_profile,
            delete_profile,
            get_stories,
            get_story,
            create_story,
            update_story,
            delete_story,
            batch_update_story_order,
            get_runs,
            get_run,
            get_run_events,
            delete_run,
            export_run_events,
            get_run_diff,
            accept_run,
            revert_run,
            get_pending_human_requests,
            respond_to_human_request,
            create_human_request,
            get_pending_approvals,
            create_approval_request,
            decide_approval,
            get_agent_permissions,
            upsert_agent_permissions,
            get_mcp_servers,
            get_mcp_server,
            create_mcp_server,
            update_mcp_server,
            delete_mcp_server,
            get_tool_bindings,
            create_tool_binding,
            delete_tool_binding,
            update_tool_binding_allowed_tools,
            get_custom_tools,
            create_custom_tool,
            update_custom_tool,
            delete_custom_tool,
            get_custom_tool_bindings,
            create_custom_tool_binding,
            delete_custom_tool_binding,
            get_settings,
            save_settings,
            get_workspace_settings,
            save_workspace_settings,
            start_chat_run,
            list_chat_sessions,
            create_chat_session,
            get_chat_session_messages,
            append_chat_session_message,
            open_workspace,
            get_recent_workspaces,
            remove_workspace,
            save_profile_toml,
            sync_toml_profiles,
            list_directory,
            read_file_text,
            write_file_text,
            rename_path,
            duplicate_file,
            delete_path,
            create_empty_file,
            create_dir_fs,
            reveal_in_explorer,
            start_continuous_mode,
            start_scheduled_mode,
            stop_agent_scheduler,
            get_agent_runtime_status,
            get_all_agent_runtime_statuses,
            start_pipeline_run,
            get_pipeline_progress,
            list_active_pipelines,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

