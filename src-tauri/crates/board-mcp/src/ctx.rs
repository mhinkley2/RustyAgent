//! Request context and the host bridge.

use std::{path::PathBuf, sync::Arc};

use serde_json::Value;

/// Everything an MCP tool is allowed to touch.
///
/// Deliberately free of Tauri types, so the entire tool surface can be tested
/// against an in-memory pool with no app, no window, and no event loop.
#[derive(Clone)]
pub struct McpCtx {
    pub db: db::DbPool,
    /// Absolute path of the workspace this request is scoped to.
    pub workspace_root: Option<PathBuf>,
    /// `workspaces.id` matching `workspace_root`, resolved at refresh time.
    ///
    /// Cached here so no tool needs `tools::builtin::story::resolve_workspace_id`,
    /// which *inserts* a workspace row when it can't find one — a path an MCP
    /// client must never be able to reach.
    pub workspace_id: Option<String>,
    /// The app-data directory, for tools that read files the app owns (logs).
    /// Both transports can supply this: the stdio binary already derives it to
    /// locate the database.
    pub app_data_dir: Option<PathBuf>,
    /// Present only when the server is running inside the desktop app.
    pub host: Option<Arc<dyn HostBridge>>,
}

impl McpCtx {
    pub fn new(db: db::DbPool) -> Self {
        Self {
            db,
            workspace_root: None,
            workspace_id: None,
            app_data_dir: None,
            host: None,
        }
    }

    pub fn with_app_data_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.app_data_dir = dir;
        self
    }

    pub fn with_host(mut self, host: Option<Arc<dyn HostBridge>>) -> Self {
        self.host = host;
        self
    }

    pub fn host_available(&self) -> bool {
        self.host.is_some()
    }

    /// Re-read the active workspace from the database.
    ///
    /// Called once per JSON-RPC message so a `use_workspace` — from this client,
    /// another client, or the app's own UI — is picked up identically on both
    /// transports. The database is the single source of truth: `touch_workspace`
    /// promotes `last_opened_at` and `get_active_workspace_path` reads the most
    /// recently promoted row.
    pub async fn refresh_workspace(&mut self) {
        self.workspace_root = db::get_active_workspace_path(&self.db).await;
        self.workspace_id = match &self.workspace_root {
            Some(path) => db::find_workspace_by_path(&self.db, path)
                .await
                .map(|w| w.id),
            None => None,
        };
    }

    /// Adapter for the six existing `tools::Tool` story implementations.
    ///
    /// `agent_profile_id` and `run_id` are placeholders — no such rows exist.
    /// The story tools never read either field, so this is safe for them, but
    /// any other `tools::Tool` (memory, notify, subtask) would hit a foreign-key
    /// failure and must not be adapted this way.
    pub fn tool_ctx(&self) -> tools::ToolContext {
        tools::ToolContext {
            db: self.db.clone(),
            agent_profile_id: "rustyagent-board-mcp".to_string(),
            run_id: "rustyagent-board-mcp".to_string(),
            pipeline_run_id: None,
            pipeline_depth: 0,
            spawn_subtask: None,
            workspace_root: self.workspace_root.clone(),
            // No desktop to notify through from this process. `send_notification`
            // is not among the adapted tools, and if it ever were it would now
            // report the failure rather than claim delivery.
            notifier: None,
        }
    }
}

/// Capabilities that exist only inside the running desktop app.
///
/// The standalone stdio binary supplies `None`. Every tool whose
/// `requires_host()` is true is then hidden from `tools/list` *and* rejected by
/// `tools/call`, rather than being answered with a stale `"idle"` / `null` /
/// `[]` — a silently wrong answer is worse for a client than an error.
pub trait HostBridge: Send + Sync {
    /// Fired after `use_workspace` commits, so the desktop UI follows the client.
    fn workspace_changed(&self, workspace: &db::WorkspaceRecord);

    fn agent_runtime_status(&self, profile_id: &str) -> Value;
    fn agent_runtime_statuses(&self) -> Value;
    fn pipeline_progress(&self, pipeline_run_id: &str) -> Option<Value>;
    fn active_pipelines(&self) -> Value;
}
