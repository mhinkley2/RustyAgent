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
            .or_else(|| {
                // `Path::parent` yields `Some("")` for a bare filename, so a
                // RUSTYAGENT_DB_PATH of "rustyagent.db" would otherwise report
                // an empty data directory. Reporting nothing beats reporting a
                // path that does not exist.
                self.db_path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .map(PathBuf::from)
            })
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
        None => match db_override.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
            // Trimmed, like every other path this module reads. An env var
            // exported from a shell or a .env routinely carries trailing
            // whitespace, and `db::paths::present` strips it everywhere else.
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

    /// This binary's own source, for the sweep guard below.
    const OWN_SOURCE: &str = include_str!("rustyagent-board-mcp.rs");

    /// This process must never reconcile runs.
    ///
    /// It opens the same database as the desktop app and is routinely launched
    /// *while* the app is running — an MCP client starting it is not a signal
    /// that nothing is executing. `db::recovery::reconcile_orphaned_runs`
    /// marks every run it considers orphaned as failed, so calling it from
    /// here would kill live runs in the app on the client's behalf.
    ///
    /// The rule is "this code path does not exist", which is a property of the
    /// source rather than of any value a test can compute, so the source is
    /// what is checked. `run()` is the only thing here that touches the
    /// database, and it is a dozen lines long: if a future change reaches for
    /// the sweep, this fails and says why.
    #[test]
    fn the_stdio_binary_never_sweeps_runs_out_from_under_the_running_app() {
        // Split so the constant does not match itself.
        let sweep = concat!("reconcile_", "orphaned_runs");
        let call_sites: Vec<&str> = OWN_SOURCE
            .lines()
            // Every line-comment form is skipped, not just `///`. The rule being
            // enforced is "no executable call site"; a `//` note explaining why,
            // or a commented-out snippet, is not one, and failing on those would
            // make the guard punish documentation.
            .filter(|line| line.contains(sweep) && !line.trim_start().starts_with("//"))
            .collect();

        assert!(
            call_sites.is_empty(),
            "the stdio MCP binary must not run the startup sweep — it would mark runs \
             the desktop app is executing as failed. Offending lines: {call_sites:?}"
        );
    }

    /// A bare `RUSTYAGENT_DB_PATH` must not report an empty data directory.
    ///
    /// `Path::parent` yields `Some("")` for a filename with no directory
    /// component, so the fallback used to hand the MCP context an empty path
    /// and print an empty "Data directory:" line.
    #[test]
    fn a_bare_database_filename_reports_no_data_directory_rather_than_an_empty_one() {
        let paths = Paths {
            data_dir: None,
            db_path: PathBuf::from("rustyagent.db"),
        };

        assert_eq!(paths.app_data_dir(), None);
    }

    #[test]
    fn a_database_path_with_a_directory_still_reports_its_parent() {
        let paths = Paths {
            data_dir: None,
            db_path: Path::new("sub").join("rustyagent.db"),
        };

        assert_eq!(paths.app_data_dir(), Some(PathBuf::from("sub")));
    }

    #[test]
    fn the_bundle_identifier_is_read_from_the_apps_own_config() {
        let identifier = bundle_identifier().expect("tauri.conf.json must declare an identifier");

        assert!(!identifier.is_empty());
        assert!(
            identifier.contains("rustyagent"),
            "unexpected identifier: {identifier}"
        );
    }

    /// The variables this binary reads, and the lock that owns them.
    ///
    /// The environment is process-global while the test harness is not: it runs
    /// tests in one process on parallel threads. Two tests owning the same
    /// variable therefore race, and this binary has already been bitten by
    /// exactly that — a default-path test and an override test, failing about
    /// one run in six depending on which observed the other's mutation.
    ///
    /// One test needs no lock. This exists so that the *second* one is safe by
    /// construction rather than by whoever writes it noticing, which is the
    /// same reliance on memory that produced the original race.
    const OWNED_VARS: [&str; 2] = [db::paths::DB_PATH_ENV, db::paths::DATA_DIR_ENV];

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// What [`OWNED_VARS`] currently hold.
    fn snapshot() -> Vec<(&'static str, Option<String>)> {
        OWNED_VARS.iter().map(|key| (*key, env::var(key).ok())).collect()
    }

    /// Start from nothing, whatever the developer's shell had.
    fn clear() {
        for key in OWNED_VARS {
            env::remove_var(key);
        }
    }

    /// Put back exactly what [`snapshot`] found, absence included.
    fn restore(saved: &[(&'static str, Option<String>)]) {
        for (key, value) in saved {
            match value {
                Some(value) => env::set_var(key, value),
                None => env::remove_var(key),
            }
        }
    }

    /// Exclusive use of [`OWNED_VARS`], restored on drop.
    ///
    /// Restored, not cleared. The previous version removed them outright, so a
    /// developer who had `RUSTYAGENT_DB_PATH` exported in their shell had it
    /// silently unset for the rest of the test binary — harmless while one test
    /// reads it, and a fresh mystery the day another does.
    ///
    /// The save and restore are free functions rather than methods so the test
    /// below can exercise them while already holding the lock. Constructing a
    /// second guard to test the first would deadlock, and a deadlocked test is
    /// a worse thing to discover than a failing one.
    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn take() -> Self {
            // A test that panics while holding this poisons the mutex. The data
            // is a unit, so there is nothing to be inconsistent about, and
            // failing every later test with a poison error would hide the one
            // real failure behind a cascade.
            let lock = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let saved = snapshot();
            clear();
            Self { _lock: lock, saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            restore(&self.saved);
        }
    }

    /// Every environment case in one test, deliberately.
    ///
    /// The resolution *logic* is covered without touching the environment at
    /// all in `db::paths`; what is left to check here is only that this binary
    /// reads the variables it documents, and that is one sequential story
    /// rather than several concurrent ones.
    #[test]
    fn the_binary_reads_the_documented_environment_overrides() {
        let _env = EnvGuard::take();

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
        env::set_var(db::paths::DATA_DIR_ENV, "/branch/data");
        let moved = resolve_paths().expect("paths");
        assert_eq!(moved.app_data_dir(), Some(PathBuf::from("/branch/data")));
        assert_eq!(
            moved.db_path,
            Path::new("/branch/data").join("rustyagent.db")
        );

        // RUSTYAGENT_DB_PATH is the more specific of the two, so it takes the
        // database and leaves the rest of the directory where it was.
        env::set_var(db::paths::DB_PATH_ENV, "/tmp/custom.db");
        let split = resolve_paths().expect("paths");
        assert_eq!(split.db_path, PathBuf::from("/tmp/custom.db"));
        assert_eq!(split.app_data_dir(), Some(PathBuf::from("/branch/data")));

        // ...and on its own it still points the database wherever it says.
        env::remove_var(db::paths::DATA_DIR_ENV);
        let db_only = resolve_paths().expect("paths");
        assert_eq!(db_only.db_path, PathBuf::from("/tmp/custom.db"));
    }

    /// The guard puts back what it found, including nothing.
    ///
    /// Asserted rather than assumed because the failure is invisible: a
    /// developer's exported variable would vanish, and the next test to read one
    /// would see an environment nobody set up.
    ///
    /// Every mutation here happens under the guard this test holds for its whole
    /// body. Touching the variables outside it would be the very race the guard
    /// exists to prevent, written into the test that proves it works.
    #[test]
    fn the_environment_guard_restores_what_it_found() {
        let _env = EnvGuard::take();

        // A value the developer's shell had.
        env::set_var(db::paths::DB_PATH_ENV, "/tmp/from-the-shell.db");
        let saved = snapshot();

        clear();
        assert_eq!(
            env::var(db::paths::DB_PATH_ENV).ok(),
            None,
            "a test must start from a known-empty environment",
        );
        env::set_var(db::paths::DB_PATH_ENV, "/tmp/set-by-the-test.db");

        restore(&saved);
        assert_eq!(
            env::var(db::paths::DB_PATH_ENV).ok().as_deref(),
            Some("/tmp/from-the-shell.db"),
            "the developer's own value was not put back",
        );

        // And absence is a value too: a variable that was unset must not be
        // left set by whatever the test did with it.
        clear();
        let saved_empty = snapshot();
        env::set_var(db::paths::DB_PATH_ENV, "/tmp/set-by-the-test.db");
        restore(&saved_empty);
        assert_eq!(env::var(db::paths::DB_PATH_ENV).ok(), None);
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
