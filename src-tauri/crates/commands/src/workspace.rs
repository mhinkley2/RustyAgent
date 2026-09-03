// Workspace management commands — open a workspace folder, list recent workspaces.

use db::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tauri::{Emitter};
use db::timestamps::NOW_ISO8601;

// ---------------------------------------------------------------------------
// Active workspace state (managed by Tauri runtime)
// ---------------------------------------------------------------------------

/// Shared in-process state tracking the currently-active workspace ID.
/// Set when the user opens a workspace; used by story/agent queries to scope data.
pub struct ActiveWorkspace(pub std::sync::Mutex<Option<String>>);

impl ActiveWorkspace {
    pub fn new() -> Self {
        Self(std::sync::Mutex::new(None))
    }

    pub fn set(&self, id: Option<String>) {
        *self.0.lock().unwrap() = id;
    }

    pub fn get(&self) -> Option<String> {
        self.0.lock().unwrap().clone()
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub path: String,
    pub name: String,
    pub last_opened_at: String,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn row_to_workspace(row: &sqlx::sqlite::SqliteRow) -> Workspace {
    Workspace {
        id:             row.try_get("id").unwrap_or_default(),
        path:           row.try_get("path").unwrap_or_default(),
        name:           row.try_get("name").unwrap_or_default(),
        last_opened_at: row.try_get("last_opened_at").unwrap_or_default(),
        created_at:     row.try_get("created_at").unwrap_or_default(),
    }
}

/// Returns the absolute path of the most recently opened workspace (if any).
pub async fn get_active_workspace_path(db: &DbPool) -> Option<std::path::PathBuf> {
    db::get_active_workspace_path(db).await
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Record a workspace as opened (upsert), update the ActiveWorkspace state,
/// and return the workspace. Called by the frontend after the user picks a folder.
pub async fn open_workspace(
    path: String,
    db: &DbPool,
    active_ws: &ActiveWorkspace,
    app: tauri::AppHandle,
) -> Result<Workspace, String> {
    // Validate the path exists and is a directory.
    let canonical = std::fs::canonicalize(&path)
        .map_err(|e| format!("Cannot open workspace: {e}"))?;
    if !canonical.is_dir() {
        return Err("Path is not a directory".into());
    }
    // Spelled by the shared normalizer, so the row this writes is the row every
    // lookup elsewhere resolves to.
    let canonical_str = db::normalize_workspace_path(&canonical);

    let name = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&canonical_str)
        .to_string();

    let id = uuid::Uuid::new_v4().to_string();

    sqlx::query(
        &format!("INSERT INTO workspaces (id, path, name) VALUES (?, ?, ?)
         ON CONFLICT(path) DO UPDATE SET
           name           = excluded.name,
           last_opened_at = {NOW_ISO8601}")
    )
    .bind(&id)
    .bind(&canonical_str)
    .bind(&name)
    .execute(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    let row = sqlx::query(
        "SELECT id, path, name, last_opened_at, created_at FROM workspaces WHERE path = ?"
    )
    .bind(&canonical_str)
    .fetch_one(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    let ws = row_to_workspace(&row);

    // Update in-process active workspace state.
    active_ws.set(Some(ws.id.clone()));

    // Notify the frontend so it can refresh board, agents, etc.
    if let Err(e) = app.emit("workspace-changed", &ws) {
        tracing::warn!("Failed to emit workspace-changed: {e}");
    }

    // Initialise .rusty/ directory structure and sync TOML profiles.
    let ws_path = std::path::Path::new(&ws.path);
    if let Err(e) = workspace::ensure_rusty_dir(ws_path) {
        tracing::warn!("ensure_rusty_dir failed: {e}");
    }
    if let Err(e) = workspace::sync_profiles_for_workspace(db, &ws.id, Some(ws_path)).await {
        tracing::warn!("sync_profiles failed: {e}");
    }

    Ok(ws)
}

/// List the most recently opened workspaces.
pub async fn get_recent_workspaces(
    db: &DbPool,
) -> Result<Vec<Workspace>, String> {
    let rows = sqlx::query(
        "SELECT id, path, name, last_opened_at, created_at
         FROM workspaces ORDER BY last_opened_at DESC LIMIT 20"
    )
    .fetch_all(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    Ok(rows.iter().map(row_to_workspace).collect())
}

/// Remove a workspace from the recent list.
pub async fn remove_workspace(
    id: String,
    db: &DbPool,
) -> Result<(), String> {
    sqlx::query("DELETE FROM workspaces WHERE id = ?")
        .bind(&id)
        .execute(db)
        .await
        .map_err(|e| format!("DB error: {e}"))?;
    Ok(())
}
