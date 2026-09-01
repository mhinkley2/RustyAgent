//! Where RustyAgent keeps its on-disk state, and how the environment moves it.
//!
//! This is the single place both binaries agree on. The desktop app
//! (`src-tauri/src/lib.rs`) and the standalone stdio MCP server
//! (`src-tauri/src/bin/rustyagent-board-mcp.rs`) used to answer "where is the
//! database?" independently, and drifted: only the stdio binary honoured an
//! override, so every build of the app on a machine shared one database. A
//! branch that adds a migration then bricks every other build, because sqlx
//! hard-fails on a migration recorded in `_sqlx_migrations` but absent on disk.
//!
//! # Precedence
//!
//! | Variable | Scope | Beats |
//! |---|---|---|
//! | [`DB_PATH_ENV`] | the database file only | [`DATA_DIR_ENV`] and the platform default |
//! | [`DATA_DIR_ENV`] | the whole data directory — database, logs, worktrees, `settings.json`, MCP token | the platform default |
//! | *(neither)* | platform default | — |
//!
//! Most specific wins. [`DB_PATH_ENV`] moves the database alone and leaves
//! everything else where the data directory says; it exists because the stdio
//! binary already documented it and that must not regress.
//!
//! # Testing
//!
//! The resolution functions here are pure: values are passed in as arguments,
//! never read from the process environment. `std::env` is touched only by the
//! thin wrappers at the bottom of this file, which have no logic of their own.
//! Tests take the pure path — `env::set_var` is process-global, and this
//! workspace has already been bitten by two tests racing over one variable.

use std::{
    io,
    path::{Path, PathBuf},
};

/// Relocates the whole data directory: database, logs, run worktrees,
/// `settings.json` and the MCP auth token move together.
pub const DATA_DIR_ENV: &str = "RUSTYAGENT_DATA_DIR";

/// Relocates the database file alone. More specific than [`DATA_DIR_ENV`], so
/// it wins for the database and nothing else.
pub const DB_PATH_ENV: &str = "RUSTYAGENT_DB_PATH";

/// The database file name inside the data directory.
pub const DB_FILE_NAME: &str = "rustyagent.db";

/// Confines an MCP client to one workspace for its whole lifetime.
///
/// Unlike the two above, this relocates nothing. It answers a different
/// question — *which project is this client working on* — which the board has
/// only ever been able to answer once per database, from the most recently
/// opened workspace. That is right for the app, whose window shows one project
/// at a time, and wrong for a stdio client, which is one process per editor
/// window with its own checkout.
pub const WORKSPACE_ENV: &str = "RUSTYAGENT_WORKSPACE";

/// Which of the two answers the data directory came from.
///
/// Carried alongside the path so a failure can name the variable to fix rather
/// than emitting an unactionable "permission denied".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataDirSource {
    /// Read from [`DATA_DIR_ENV`].
    Override,
    /// The platform default supplied by the caller.
    Platform,
}

/// A resolved data directory and where the answer came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDir {
    pub path: PathBuf,
    pub source: DataDirSource,
}

impl DataDir {
    pub fn is_overridden(&self) -> bool {
        matches!(self.source, DataDirSource::Override)
    }

    /// The database file for this directory, ignoring [`DB_PATH_ENV`].
    pub fn default_db_path(&self) -> PathBuf {
        self.path.join(DB_FILE_NAME)
    }
}

/// Blank and whitespace-only values count as unset.
///
/// An exported-but-empty variable is a shell accident, not a request to use the
/// current directory as the data directory.
fn present(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

// ---------------------------------------------------------------------------
// Pure resolution
// ---------------------------------------------------------------------------

/// Where a workspace pin came from, and therefore how hard to insist on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinSource {
    /// [`WORKSPACE_ENV`] was set. The user asked for this by name.
    Explicit,
    /// The process's working directory. A guess, and usually a good one — an
    /// editor launches a stdio server from the folder it has open.
    WorkingDirectory,
}

/// A workspace a client wants to be confined to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinRequest {
    pub path: PathBuf,
    pub source: PinSource,
}

/// What workspace, if any, a client is asking to be confined to.
///
/// The environment first, then the working directory. Both are candidates at
/// this stage: neither has been checked against the workspaces that exist, and
/// [`PinSource`] is what lets the caller treat the two failures differently.
///
/// The asymmetry is the point. An explicit variable naming a folder the app has
/// never opened is a user mistake and must be reported — an override that
/// silently does nothing is the bug reading an override is meant to avoid. A
/// working directory that matches nothing is the ordinary case for a client
/// launched anywhere else, and must fall through to the shared behaviour
/// rather than refusing to start.
pub fn pin_request(override_value: Option<&str>, working_dir: Option<PathBuf>) -> Option<PinRequest> {
    if let Some(path) = present(override_value) {
        return Some(PinRequest {
            path: PathBuf::from(path),
            source: PinSource::Explicit,
        });
    }
    working_dir.map(|path| PinRequest {
        path,
        source: PinSource::WorkingDirectory,
    })
}

/// Resolve the data directory from an override and a platform default.
///
/// Fails rather than inventing a location: a silent fallback to the shared
/// default is exactly the bug this module exists to remove.
pub fn resolve_data_dir(
    override_value: Option<&str>,
    platform_default: Option<PathBuf>,
) -> Result<DataDir, String> {
    if let Some(path) = present(override_value) {
        return Ok(DataDir {
            path: PathBuf::from(path),
            source: DataDirSource::Override,
        });
    }

    platform_default
        .map(|path| DataDir {
            path,
            source: DataDirSource::Platform,
        })
        .ok_or_else(|| {
            format!(
                "Unable to determine the RustyAgent data directory. \
                 Set {DATA_DIR_ENV} to a writable directory."
            )
        })
}

/// Resolve the database file: [`DB_PATH_ENV`] if set, else inside `data_dir`.
pub fn resolve_db_path(override_value: Option<&str>, data_dir: &Path) -> PathBuf {
    match present(override_value) {
        Some(path) => PathBuf::from(path),
        None => data_dir.join(DB_FILE_NAME),
    }
}

/// A best-effort default data directory for a bundle identifier.
///
/// For the standalone binary, which has no `AppHandle` to ask. **This is not
/// full parity with Tauri's `app_data_dir()`** and must not be treated as
/// such: it honours `%APPDATA%` on Windows and falls back to
/// `$HOME/.local/share`, but ignores `XDG_DATA_HOME` and does not implement
/// macOS's `~/Library/Application Support`. Anyone who sets `XDG_DATA_HOME`,
/// or runs on macOS, can therefore see this binary and the desktop app
/// disagree about the default location.
///
/// Bringing the two into line would move the default path for existing
/// installs, so it is deliberately left alone; set `RUSTYAGENT_DATA_DIR` to
/// make both agree explicitly.
///
/// `None` when neither `appdata` nor `home` is available.
pub fn platform_data_dir(
    appdata: Option<&str>,
    home: Option<&str>,
    identifier: &str,
) -> Option<PathBuf> {
    if let Some(appdata) = present(appdata) {
        return Some(Path::new(appdata).join(identifier));
    }
    if let Some(home) = present(home) {
        return Some(
            Path::new(home)
                .join(".local")
                .join("share")
                .join(identifier),
        );
    }
    None
}

// ---------------------------------------------------------------------------
// Making the directory usable
// ---------------------------------------------------------------------------

/// Create the data directory and prove it is writable.
///
/// Creating it is not enough: an existing directory the process cannot write to
/// fails later, deep inside sqlx or the log appender, with an error that names
/// neither the directory nor the variable that chose it. Probing here turns
/// that into one actionable message at startup.
pub fn prepare_data_dir(dir: &DataDir) -> Result<(), String> {
    std::fs::create_dir_all(&dir.path).map_err(|error| data_dir_failure(dir, "create", &error))?;

    // Named per-process so two RustyAgent processes starting at once cannot
    // delete each other's probe.
    let probe = dir
        .path
        .join(format!(".rustyagent-write-check-{}", std::process::id()));
    std::fs::write(&probe, b"").map_err(|error| data_dir_failure(dir, "write to", &error))?;
    let _ = std::fs::remove_file(&probe);

    Ok(())
}

fn data_dir_failure(dir: &DataDir, verb: &str, error: &io::Error) -> String {
    let (origin, remedy) = match dir.source {
        DataDirSource::Override => (
            format!(" (from {DATA_DIR_ENV})"),
            format!(
                "Point {DATA_DIR_ENV} at a writable directory, or unset it to use the default \
                 location."
            ),
        ),
        DataDirSource::Platform => (
            String::new(),
            format!("Set {DATA_DIR_ENV} to a writable directory."),
        ),
    };

    format!(
        "Failed to {verb} the RustyAgent data directory '{}'{origin}: {error}. {remedy}",
        dir.path.display()
    )
}

/// Create the directory holding the database file.
///
/// Only does anything when [`DB_PATH_ENV`] points outside the data directory;
/// otherwise the parent is the data directory, already prepared.
pub fn prepare_db_parent(db_path: &Path) -> Result<(), String> {
    let Some(parent) = db_path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Ok(());
    };

    std::fs::create_dir_all(parent).map_err(|error| {
        format!(
            "Failed to create '{}' for the RustyAgent database '{}': {error}. \
             Point {DB_PATH_ENV} at a writable location, or unset it.",
            parent.display(),
            db_path.display()
        )
    })
}

// ---------------------------------------------------------------------------
// Environment wrappers — no logic, so nothing here needs a test that mutates
// the process environment.
// ---------------------------------------------------------------------------

fn env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// [`DATA_DIR_ENV`] as set in this process, blank treated as unset.
pub fn data_dir_override() -> Option<String> {
    env_value(DATA_DIR_ENV)
}

/// [`DB_PATH_ENV`] as set in this process, blank treated as unset.
pub fn db_path_override() -> Option<String> {
    env_value(DB_PATH_ENV)
}

/// [`WORKSPACE_ENV`] as set in this process, blank treated as unset.
pub fn workspace_override() -> Option<String> {
    env_value(WORKSPACE_ENV)
}

/// [`resolve_data_dir`] reading [`DATA_DIR_ENV`] from the environment.
pub fn data_dir(platform_default: Option<PathBuf>) -> Result<DataDir, String> {
    resolve_data_dir(data_dir_override().as_deref(), platform_default)
}

/// [`resolve_db_path`] reading [`DB_PATH_ENV`] from the environment.
pub fn db_path(data_dir: &Path) -> PathBuf {
    resolve_db_path(db_path_override().as_deref(), data_dir)
}

/// Apply the [`DATA_DIR_ENV`] override to a platform default, discarding the
/// reason it could not be resolved.
///
/// For the call sites that already treat a missing app-data directory as
/// `None` or panic with their own message.
pub fn with_override(platform_default: Option<PathBuf>) -> Option<PathBuf> {
    data_dir(platform_default).ok().map(|dir| dir.path)
}
