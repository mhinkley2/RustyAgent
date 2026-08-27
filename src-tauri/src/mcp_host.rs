//! Hosts the MCP HTTP server inside the desktop app.
//!
//! Everything protocol-related lives in the `board-mcp` crate; this module is
//! only the glue that gives it the capabilities that exist solely in the running
//! app — emitting `workspace-changed` to the UI, and reading live scheduler and
//! pipeline state out of memory.

use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use board_mcp::{auth::AuthConfig, transport::http, HostBridge, McpCtx};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};
use tracing::{info, warn};

type SchedulerState = scheduler::SchedulerState;
type PipelineState = pipeline::PipelineState;

/// Supplies the MCP server with the parts of RustyAgent that only exist while
/// the desktop app is running.
struct TauriHostBridge {
    app: AppHandle,
}

impl HostBridge for TauriHostBridge {
    fn workspace_changed(&self, workspace: &db::WorkspaceRecord) {
        // Keep the in-process notion of "active workspace" in step with the DB,
        // then tell the UI so it re-queries.
        let active = self.app.state::<commands::ActiveWorkspace>();
        active.set(Some(workspace.id.clone()));

        let payload = commands::Workspace {
            id: workspace.id.clone(),
            name: workspace.name.clone(),
            path: workspace.path.clone(),
            last_opened_at: workspace.last_opened_at.clone(),
            created_at: workspace.created_at.clone(),
        };
        if let Err(error) = self.app.emit("workspace-changed", &payload) {
            warn!("Failed to emit workspace-changed from the MCP server: {error}");
        }
    }

    fn agent_runtime_status(&self, profile_id: &str) -> Value {
        let sched = self.app.state::<Arc<SchedulerState>>();
        to_json(scheduler::get_status(profile_id, sched.inner().clone()))
    }

    fn agent_runtime_statuses(&self) -> Value {
        let sched = self.app.state::<Arc<SchedulerState>>();
        to_json(scheduler::get_all_statuses(sched.inner().clone()))
    }

    fn pipeline_progress(&self, pipeline_run_id: &str) -> Option<Value> {
        let state = self.app.state::<Arc<PipelineState>>();
        pipeline::get_pipeline_progress(pipeline_run_id, state.inner().clone()).map(to_json)
    }

    fn active_pipelines(&self) -> Value {
        let state = self.app.state::<Arc<PipelineState>>();
        to_json(pipeline::list_active_pipelines(state.inner().clone()))
    }
}

fn to_json<T: serde::Serialize>(value: T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// Start the MCP HTTP server.
///
/// Returns `Err` when the port is unavailable. The caller treats that as a
/// warning, not a fatal error — RustyAgent is perfectly usable without external
/// MCP access, and a port conflict should never stop the app from launching.
pub fn spawn(app: AppHandle, db: db::DbPool, app_data_dir: Option<PathBuf>) -> Result<(), String> {
    let port = board_mcp::mcp_port();
    let auth = AuthConfig::resolve(app_data_dir.as_deref(), port);

    let ctx = McpCtx::new(db)
        .with_app_data_dir(app_data_dir.clone())
        .with_host(Some(Arc::new(TauriHostBridge { app })));

    // Bind synchronously so a port conflict is returned to the caller, then let
    // Tauri's runtime own the accept loop — the setup hook this runs from has
    // no Tokio reactor entered, so `tokio::spawn` would panic here.
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let server = http::bind(http::state(ctx, auth.clone()), addr)?;
    tauri::async_runtime::spawn(server.serve());

    log_client_config(port, auth.token.as_deref(), app_data_dir.as_deref());
    Ok(())
}

/// Print a paste-ready client configuration, so the token is discoverable
/// without hunting through the app-data directory.
fn log_client_config(port: u16, token: Option<&str>, app_data_dir: Option<&std::path::Path>) {
    let url = format!("http://127.0.0.1:{port}{}", http::MCP_ENDPOINT_PATH);

    match token {
        None => info!("MCP server at {url} (authentication disabled)"),
        Some(token) => {
            let location = app_data_dir
                .map(|dir| AuthConfig::token_path(dir).display().to_string())
                .unwrap_or_else(|| "(not persisted)".to_string());

            info!(
                "MCP server at {url}\n\
                 Token file: {location}\n\
                 Add to your MCP client with:\n\
                 \x20 claude mcp add --transport http rustyagent-board {url} \\\n\
                 \x20   --header \"Authorization: Bearer {token}\"",
            );
        }
    }
}
