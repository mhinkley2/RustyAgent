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

/// The data directory and database this process should use.
struct Paths {
    /// `None` when nothing resolved a data directory and `RUSTYAGENT_DB_PATH`
    /// alone said where the database is.
    data_dir: Option<db::paths::DataDir>,
    db_path: PathBuf,
}

impl Paths {
    /// The directory the MCP context reports as the app data directory.
    ///
    /// Falls back to the database's own parent, which is what this binary used
    /// before it understood a directory-level override.
    fn app_data_dir(&self) -> Option<PathBuf> {
        self.data_dir
            .as_ref()
            .map(|dir| dir.path.clone())
            .or_else(|| self.db_path.parent().map(PathBuf::from))
    }
}

/// Locate the data directory and database, matching where the desktop app puts
/// them.
///
/// The resolution itself lives in `db::paths` -- shared with the app so the two
/// binaries cannot drift apart again -- and the precedence between
/// `RUSTYAGENT_DB_PATH` and `RUSTYAGENT_DATA_DIR` is documented there.
fn resolve_paths() -> Result<Paths, String> {
    // Read both overrides up front, database first, before any other work.
    // Either can answer the question on its own, and the bundle identifier
    // below is only needed to build the platform default.
    let db_override = db::paths::db_path_override();
    let dir_override = db::paths::data_dir_override();

    let platform_default = match dir_override {
        Some(_) => None,
        None => db::paths::platform_data_dir(
            env::var("APPDATA").ok().as_deref(),
            env::var("HOME").ok().as_deref(),
            &bundle_identifier()?,
        ),
    };

    let data_dir = db::paths::resolve_data_dir(dir_override.as_deref(), platform_default).ok();

    let db_path = match &data_dir {
        Some(dir) => db::paths::resolve_db_path(db_override.as_deref(), &dir.path),
        // No data directory anywhere, but an explicit database path still
        // fully determines where to open the database.
        None => match &db_override {
            Some(path) => PathBuf::from(path),
            None => {
                return Err(format!(
                    "Unable to determine the RustyAgent database path. Set {} or {}.",
                    db::paths::DATA_DIR_ENV,
                    db::paths::DB_PATH_ENV
                ))
            }
        },
    };

    Ok(Paths { data_dir, db_path })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

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

    /// Every environment case in one test, deliberately.
    ///
    /// `env::set_var` is process-global, so two tests owning the same variable
    /// in one binary race — this binary has already been bitten by exactly
    /// that. The resolution *logic* is covered without touching the
    /// environment at all in `db::paths`; what is left to check here is only
    /// that this binary reads the variables it documents, and that is one
    /// sequential story, not several concurrent ones.
    #[test]
    fn the_binary_reads_the_documented_environment_overrides() {
        env::remove_var("RUSTYAGENT_DB_PATH");
        env::remove_var("RUSTYAGENT_DATA_DIR");

        // Unset: the database lands under the bundle identifier. Guards the
        // drift that once pointed this binary at a stale database.
        let identifier = bundle_identifier().expect("identifier");
        let default = resolve_paths().expect("paths");
        assert!(
            default.db_path.to_string_lossy().contains(&identifier),
            "{} should sit under {identifier}",
            default.db_path.display()
        );
        assert!(default.db_path.ends_with("rustyagent.db"));

        // RUSTYAGENT_DATA_DIR moves the whole directory, database included.
        env::set_var("RUSTYAGENT_DATA_DIR", "/branch/data");
        let moved = resolve_paths().expect("paths");
        assert_eq!(moved.app_data_dir(), Some(PathBuf::from("/branch/data")));
        assert_eq!(
            moved.db_path,
            Path::new("/branch/data").join("rustyagent.db")
        );

        // RUSTYAGENT_DB_PATH is the more specific of the two, so it takes the
        // database and leaves the rest of the directory where it was.
        env::set_var("RUSTYAGENT_DB_PATH", "/tmp/custom.db");
        let split = resolve_paths().expect("paths");
        assert_eq!(split.db_path, PathBuf::from("/tmp/custom.db"));
        assert_eq!(split.app_data_dir(), Some(PathBuf::from("/branch/data")));

        // ...and on its own it still points the database wherever it says.
        env::remove_var("RUSTYAGENT_DATA_DIR");
        let db_only = resolve_paths().expect("paths");
        assert_eq!(db_only.db_path, PathBuf::from("/tmp/custom.db"));

        env::remove_var("RUSTYAGENT_DB_PATH");
    }
}

async fn run() -> Result<(), String> {
    let paths = resolve_paths()?;
    let app_data_dir = paths.app_data_dir();
    let db_path = &paths.db_path;

    // Refuse to start rather than fall back to the shared default: an override
    // that silently does nothing is the bug reading an override is meant to
    // avoid.
    if let Some(dir) = &paths.data_dir {
        db::paths::prepare_data_dir(dir)?;
    }
    db::paths::prepare_db_parent(db_path)?;

    // Diagnostics on stderr: stdout carries the framed protocol.
    match app_data_dir.as_deref() {
        Some(dir) => eprintln!("Data directory: {}", dir.display()),
        None => eprintln!("Data directory: (none)"),
    }
    eprintln!("Database: {}", db_path.display());

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
