//! Request context and the host bridge.

use std::{path::PathBuf, sync::Arc};

use serde_json::Value;

/// How a client asked to be confined to one workspace.
///
/// Carried so a refusal can name the thing that would have to change. Both
/// transports confine for the same reason, but a client can only act on advice
/// about the mechanism it actually used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinScope {
    /// Resolved once at startup by the stdio binary — one process per editor
    /// window — from `RUSTYAGENT_WORKSPACE` or its working directory.
    Process,
    /// Named per request by a header, on the one HTTP server the app hosts for
    /// every client. Per request because that server is stateless: there is no
    /// session for a scope to live in.
    Request,
}

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
    /// This client is confined to `workspace_root`, and how it asked to be.
    ///
    /// [`refresh_workspace`](Self::refresh_workspace) then leaves the scope
    /// alone instead of re-reading the database's single most-recently-opened
    /// workspace, and `use_workspace` refuses — a pin a client can undo on its
    /// own is a default, not a confinement.
    ///
    /// `None` follows the workspace the app has open. That is what the app's
    /// own webview wants, and what a client that named no project gets.
    pub pin: Option<PinScope>,
}

impl McpCtx {
    pub fn new(db: db::DbPool) -> Self {
        Self {
            db,
            workspace_root: None,
            workspace_id: None,
            app_data_dir: None,
            host: None,
            pin: None,
        }
    }

    /// Confine this context to one workspace.
    ///
    /// Both halves of the workspace are supplied by the caller because only it
    /// can do the lookup that refuses an unknown path — this must never be
    /// able to register a workspace, or a client could point itself at any
    /// directory and then read it through `read_file`.
    pub fn pinned_to(mut self, root: PathBuf, id: Option<String>, scope: PinScope) -> Self {
        self.workspace_root = Some(root);
        self.workspace_id = id;
        self.pin = Some(scope);
        self
    }

    /// Whether this client is confined to a single workspace.
    pub fn pinned(&self) -> bool {
        self.pin.is_some()
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
    ///
    /// Unless this context is [`pinned`](Self::pinned), in which case following
    /// that pointer is exactly the bug: two editor windows on two projects share
    /// one database, and whichever activated its workspace last would silently
    /// become the scope for both.
    pub async fn refresh_workspace(&mut self) {
        if self.pinned() {
            return;
        }

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
            run_control: None,
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
