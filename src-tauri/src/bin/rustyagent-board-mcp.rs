//! Standalone MCP server over stdio.
//!
//! Serves everything backed by the database, so it works with the RustyAgent
//! desktop app closed. Tools that need live in-app state (scheduler status,
//! pipeline progress) are hidden here and refused if called — use the HTTP
//! transport the app hosts for those.
//!
//! Diagnostics go to stderr only: stdout carries the framed protocol and
//! nothing else.

use std::{
    env,
    io::{self, BufReader},
    path::PathBuf,
};

use board_mcp::{transport::stdio, McpCtx};

/// The app's Tauri config, embedded at compile time.
///
/// The app-data directory is named after the bundle identifier, so this binary
/// has to agree with the app about it or the two open different databases.
/// Reading it from the same file the app is built from makes that impossible to
/// get wrong — a previously hardcoded identifier had already drifted, leaving
/// this binary pointed at a stale database.
const TAURI_CONF: &str = include_str!("../../tauri.conf.json");

fn bundle_identifier() -> Result<String, String> {
    serde_json::from_str::<serde_json::Value>(TAURI_CONF)
        .map_err(|error| format!("Failed to parse tauri.conf.json: {error}"))?
        .get("identifier")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| "tauri.conf.json has no \"identifier\" field".to_string())
}

/// Locate the database, matching where the desktop app puts it.
fn default_db_path() -> Result<PathBuf, String> {
    if let Ok(path) = env::var("RUSTYAGENT_DB_PATH") {
        if !path.trim().is_empty() {
            return Ok(PathBuf::from(path));
        }
    }

    let identifier = bundle_identifier()?;

    if let Ok(appdata) = env::var("APPDATA") {
        return Ok(PathBuf::from(appdata)
            .join(&identifier)
            .join("rustyagent.db"));
    }
    if let Ok(home) = env::var("HOME") {
        return Ok(PathBuf::from(home)
            .join(".local")
            .join("share")
            .join(&identifier)
            .join("rustyagent.db"));
    }
    Err("Unable to determine the RustyAgent database path. Set RUSTYAGENT_DB_PATH.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_identifier_is_read_from_the_apps_own_config() {
        let identifier = bundle_identifier().expect("tauri.conf.json must declare an identifier");

        assert!(!identifier.is_empty());
        assert!(
            identifier.contains("rustyagent"),
            "unexpected identifier: {identifier}"
        );
    }

    #[test]
    fn the_default_db_path_lands_under_the_bundle_identifier() {
        // Guards the drift that pointed this binary at a stale database.
        env::remove_var("RUSTYAGENT_DB_PATH");
        let identifier = bundle_identifier().expect("identifier");

        let path = default_db_path().expect("db path");

        assert!(
            path.to_string_lossy().contains(&identifier),
            "{} should sit under {identifier}",
            path.display()
        );
        assert!(path.ends_with("rustyagent.db"));
    }

    #[test]
    fn an_explicit_db_path_overrides_the_default() {
        env::set_var("RUSTYAGENT_DB_PATH", "/tmp/custom.db");
        let path = default_db_path().expect("db path");
        env::remove_var("RUSTYAGENT_DB_PATH");

        assert_eq!(path, PathBuf::from("/tmp/custom.db"));
    }
}

async fn run() -> Result<(), String> {
    let db_path = default_db_path()?;
    let app_data_dir = db_path.parent().map(PathBuf::from);

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create '{}': {error}", parent.display()))?;
    }

    let db = db::init_db(&db_path.to_string_lossy())
        .await
        .map_err(|error| format!("Failed to open the database: {error}"))?;

    // No host bridge: this process is not the desktop app.
    let ctx = McpCtx::new(db).with_app_data_dir(app_data_dir);
    let registry = board_mcp::build_registry();

    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = io::stdout().lock();

    stdio::serve(&mut reader, &mut writer, ctx, &registry)
        .await
        .map_err(|error| format!("MCP stdio transport failed: {error}"))
}

fn main() {
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Failed to start the async runtime: {error}");
            std::process::exit(1);
        }
    };

    if let Err(error) = runtime.block_on(run()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
