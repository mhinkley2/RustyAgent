// File-system watcher for live-reload of .rusty/agents/ TOML files.
//
// Spawns a background Tokio task that:
//   1. Watches the global ~/.rusty/agents/ directory.
//   2. Watches {workspace_root}/.rusty/agents/ (if a workspace is open).
//
// On any CREATE / MODIFY event for a *.toml file the affected profile is
// re-parsed and upserted into SQLite, then a `"profiles-changed"` Tauri
// event is emitted so the frontend can refresh.

use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use db::DbPool;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::loader::sync_profiles;

/// Handle returned by [`start_watcher`]. Dropping it stops the watcher.
pub struct WatcherHandle {
    _watcher: RecommendedWatcher,
}

/// Start the file-system watcher.
///
/// - `db`              — shared SQLite pool
/// - `workspace_root`  — currently open workspace (may change over lifetime)
/// - `emit`            — callback invoked when profiles change (e.g. emit Tauri event)
pub fn start_watcher(
    db: DbPool,
    workspace_root: Option<PathBuf>,
    emit: impl Fn() + Send + Sync + 'static,
) -> Result<WatcherHandle> {
    let db   = Arc::new(db);
    let emit = Arc::new(emit);
    let ws   = Arc::new(Mutex::new(workspace_root.clone()));

    let db_c    = Arc::clone(&db);
    let emit_c  = Arc::clone(&emit);
    let ws_c    = Arc::clone(&ws);

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        let event = match res {
            Ok(e)  => e,
            Err(e) => { warn!("Watcher error: {e}"); return; }
        };

        // Only react to create/modify events on .toml files.
        let is_toml_change = matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(_)
        ) && event.paths.iter().any(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("toml")
        });

        if !is_toml_change {
            return;
        }

        info!("TOML change detected: {:?}", event.paths);

        let db_cc   = Arc::clone(&db_c);
        let emit_cc = Arc::clone(&emit_c);
        let ws_cc   = Arc::clone(&ws_c);

        tokio::spawn(async move {
            let ws_root = ws_cc.lock().await.clone();
            if let Err(e) = sync_profiles(&db_cc, ws_root.as_deref()).await {
                warn!("sync_profiles after watch event: {e}");
            }
            emit_cc();
        });
    })?;

    // Watch the global agents dir (best-effort — may not exist yet).
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .ok();

    if let Some(h) = &home {
        let global_dir = h.join(".rusty").join("agents");
        if global_dir.is_dir() {
            if let Err(e) = watcher.watch(&global_dir, RecursiveMode::NonRecursive) {
                warn!("Cannot watch global agents dir: {e}");
            }
        }
    }

    if let Some(ws) = workspace_root {
        let ws_dir = ws.join(".rusty").join("agents");
        // The directory may not exist yet; watch closest parent.
        let target = if ws_dir.is_dir() { ws_dir } else { ws.join(".rusty") };
        if target.is_dir() {
            if let Err(e) = watcher.watch(&target, RecursiveMode::Recursive) {
                warn!("Cannot watch workspace agents dir: {e}");
            }
        }
    }

    Ok(WatcherHandle { _watcher: watcher })
}
