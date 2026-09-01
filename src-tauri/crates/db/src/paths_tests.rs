//! Tests for `crate::paths`.
//!
//! Every resolution test passes values as arguments. Nothing here reads or
//! writes the process environment, so these cannot race with each other or with
//! the environment-reading tests in `src/bin/rustyagent-board-mcp.rs`.

use std::path::{Path, PathBuf};

use crate::paths::{
    platform_data_dir, prepare_data_dir, prepare_db_parent, resolve_data_dir, resolve_db_path,
    DataDir, DataDirSource, DATA_DIR_ENV, DB_FILE_NAME, DB_PATH_ENV,
};

const ID: &str = "com.rustyagent.app";

/// A unique empty directory under the system temp dir, removed by the caller.
fn scratch(label: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("rustyagent-paths-{label}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

// ---------------------------------------------------------------------------
// Data directory
// ---------------------------------------------------------------------------

#[test]
fn without_an_override_the_platform_default_is_returned_unchanged() {
    let default = PathBuf::from("/somewhere/com.rustyagent.app");

    let resolved = resolve_data_dir(None, Some(default.clone())).expect("resolves");

    assert_eq!(resolved.path, default);
    assert_eq!(resolved.source, DataDirSource::Platform);
    assert!(!resolved.is_overridden());
}

#[test]
fn an_override_replaces_the_platform_default() {
    let resolved = resolve_data_dir(
        Some("/branch/data"),
        Some(PathBuf::from("/somewhere/com.rustyagent.app")),
    )
    .expect("resolves");

    assert_eq!(resolved.path, PathBuf::from("/branch/data"));
    assert_eq!(resolved.source, DataDirSource::Override);
}

#[test]
fn a_blank_override_is_treated_as_unset() {
    // An exported-but-empty variable is a shell accident. Using it would put
    // the data directory at the process's working directory.
    for blank in ["", "   ", "\t", "\r\n"] {
        let resolved = resolve_data_dir(Some(blank), Some(PathBuf::from("/default")))
            .unwrap_or_else(|error| panic!("{blank:?} should fall back, got: {error}"));

        assert_eq!(resolved.path, PathBuf::from("/default"));
        assert_eq!(resolved.source, DataDirSource::Platform);
    }
}

#[test]
fn a_surrounding_whitespace_is_trimmed_from_an_override() {
    // Shell exports and .env files pick up trailing whitespace and CR on
    // Windows; a path with a stray \r fails to create with a confusing error.
    let resolved = resolve_data_dir(Some("  /branch/data\r\n"), None).expect("resolves");

    assert_eq!(resolved.path, PathBuf::from("/branch/data"));
}

#[test]
fn no_override_and_no_platform_default_fails_naming_the_variable_to_set() {
    let error = resolve_data_dir(None, None).expect_err("must not invent a directory");

    assert!(
        error.contains(DATA_DIR_ENV),
        "unactionable message: {error}"
    );
}

#[test]
fn an_override_is_honoured_even_when_there_is_no_platform_default() {
    let resolved = resolve_data_dir(Some("/branch/data"), None).expect("resolves");

    assert_eq!(resolved.path, PathBuf::from("/branch/data"));
}

// ---------------------------------------------------------------------------
// Database path and precedence
// ---------------------------------------------------------------------------

#[test]
fn the_database_sits_inside_the_data_directory_by_default() {
    let db = resolve_db_path(None, Path::new("/data"));

    assert_eq!(db, Path::new("/data").join(DB_FILE_NAME));
    assert!(db.ends_with(DB_FILE_NAME));
}

#[test]
fn the_database_follows_an_overridden_data_directory() {
    let dir =
        resolve_data_dir(Some("/branch/data"), Some(PathBuf::from("/default"))).expect("resolves");

    let db = resolve_db_path(None, &dir.path);

    assert_eq!(db, Path::new("/branch/data").join(DB_FILE_NAME));
    assert!(
        !db.starts_with("/default"),
        "the override must move the database off the shared default"
    );
}

#[test]
fn the_db_path_override_beats_the_data_directory_override() {
    // Documented precedence: most specific wins. The database moves alone;
    // logs, worktrees and the token stay with the data directory.
    let dir = resolve_data_dir(Some("/branch/data"), None).expect("resolves");

    let db = resolve_db_path(Some("/elsewhere/custom.db"), &dir.path);

    assert_eq!(db, PathBuf::from("/elsewhere/custom.db"));
    assert_eq!(dir.path, PathBuf::from("/branch/data"));
}

#[test]
fn a_blank_db_path_override_is_treated_as_unset() {
    let db = resolve_db_path(Some("  "), Path::new("/data"));

    assert_eq!(db, Path::new("/data").join(DB_FILE_NAME));
}

#[test]
fn default_db_path_on_a_data_dir_ignores_the_db_override() {
    let dir = DataDir {
        path: PathBuf::from("/data"),
        source: DataDirSource::Platform,
    };

    assert_eq!(dir.default_db_path(), Path::new("/data").join(DB_FILE_NAME));
}

// ---------------------------------------------------------------------------
// Platform default
// ---------------------------------------------------------------------------

#[test]
fn the_windows_default_is_appdata_joined_with_the_bundle_identifier() {
    let dir = platform_data_dir(Some(r"C:\Users\dev\AppData\Roaming"), None, ID).expect("resolves");

    assert_eq!(dir, Path::new(r"C:\Users\dev\AppData\Roaming").join(ID));
    assert!(dir.ends_with(ID));
}

#[test]
fn the_posix_default_is_home_local_share_joined_with_the_bundle_identifier() {
    let dir = platform_data_dir(None, Some("/home/dev"), ID).expect("resolves");

    // Built with `join`, so this holds on Windows too, where the separator
    // differs — comparing against a hardcoded string would not.
    assert_eq!(
        dir,
        Path::new("/home/dev").join(".local").join("share").join(ID)
    );
}

#[test]
fn appdata_wins_over_home_when_both_are_present() {
    // Git Bash and WSL-ish shells on Windows set HOME as well as APPDATA;
    // picking HOME there would silently open a different database.
    let dir = platform_data_dir(Some("/appdata"), Some("/home/dev"), ID).expect("resolves");

    assert_eq!(dir, Path::new("/appdata").join(ID));
}

#[test]
fn a_blank_appdata_falls_through_to_home() {
    let dir = platform_data_dir(Some(""), Some("/home/dev"), ID).expect("resolves");

    assert_eq!(
        dir,
        Path::new("/home/dev").join(".local").join("share").join(ID)
    );
}

#[test]
fn no_appdata_and_no_home_has_no_platform_default() {
    assert!(platform_data_dir(None, None, ID).is_none());
    assert!(platform_data_dir(Some(" "), Some(""), ID).is_none());
}

// ---------------------------------------------------------------------------
// Preparing the directory
// ---------------------------------------------------------------------------

#[test]
fn preparing_a_missing_directory_creates_it_and_leaves_no_probe_behind() {
    let root = scratch("create");
    let dir = DataDir {
        path: root.join("nested").join("data"),
        source: DataDirSource::Override,
    };

    prepare_data_dir(&dir).expect("creates the directory");

    assert!(dir.path.is_dir());
    let leftovers: Vec<_> = std::fs::read_dir(&dir.path)
        .expect("read back")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .collect();
    assert!(
        leftovers.is_empty(),
        "probe file left behind: {leftovers:?}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn preparing_an_existing_directory_is_idempotent() {
    let root = scratch("idempotent");
    let dir = DataDir {
        path: root.clone(),
        source: DataDirSource::Platform,
    };
    std::fs::write(root.join("rustyagent.db"), b"pretend database").expect("seed");

    prepare_data_dir(&dir).expect("first");
    prepare_data_dir(&dir).expect("second");

    assert!(
        root.join("rustyagent.db").exists(),
        "existing data survives"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn an_uncreatable_override_fails_with_a_message_naming_the_variable() {
    // A path *through* a regular file cannot become a directory on any
    // platform, which makes this deterministic on Windows and Linux alike.
    let root = scratch("uncreatable");
    let blocker = root.join("not-a-directory");
    std::fs::write(&blocker, b"x").expect("seed blocker");

    let dir = DataDir {
        path: blocker.join("data"),
        source: DataDirSource::Override,
    };

    let error = prepare_data_dir(&dir).expect_err("must not succeed");

    assert!(
        error.contains(DATA_DIR_ENV),
        "message must name the variable to fix: {error}"
    );
    assert!(
        error.contains(&dir.path.display().to_string()),
        "message must name the path: {error}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_failure_on_the_platform_default_does_not_blame_the_override() {
    let root = scratch("platform-failure");
    let blocker = root.join("not-a-directory");
    std::fs::write(&blocker, b"x").expect("seed blocker");

    let dir = DataDir {
        path: blocker.join("data"),
        source: DataDirSource::Platform,
    };

    let error = prepare_data_dir(&dir).expect_err("must not succeed");

    assert!(
        !error.contains(&format!("(from {DATA_DIR_ENV})")),
        "the default was not chosen by the variable: {error}"
    );
    assert!(
        error.contains(DATA_DIR_ENV),
        "still suggests a fix: {error}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn preparing_the_db_parent_creates_a_directory_outside_the_data_dir() {
    let root = scratch("db-parent");
    let db = root.join("elsewhere").join("custom.db");

    prepare_db_parent(&db).expect("creates the parent");

    assert!(db.parent().expect("parent").is_dir());
    assert!(!db.exists(), "the database file itself is sqlx's job");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn an_uncreatable_db_parent_fails_naming_the_db_path_variable() {
    let root = scratch("db-parent-failure");
    let blocker = root.join("not-a-directory");
    std::fs::write(&blocker, b"x").expect("seed blocker");

    let error = prepare_db_parent(&blocker.join("custom.db")).expect_err("must not succeed");

    assert!(error.contains(DB_PATH_ENV), "unactionable message: {error}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_bare_relative_db_file_name_has_no_parent_to_create() {
    prepare_db_parent(Path::new("rustyagent.db")).expect("nothing to do");
}

// ---------------------------------------------------------------------------
// Which project a client is asking to be confined to
// ---------------------------------------------------------------------------
//
// Pure, taking both candidates as arguments. The environment is process-global
// and the test harness is not, and this crate has already been bitten by tests
// racing over `RUSTYAGENT_DB_PATH`; the reader that consults the environment is
// a one-line wrapper over this.

use crate::paths::{pin_request, PinSource};

#[test]
fn an_explicit_override_wins_over_the_working_directory() {
    let request = pin_request(Some("C:/work/asked-for"), Some(PathBuf::from("C:/work/cwd")))
        .expect("a request");

    assert_eq!(request.path, PathBuf::from("C:/work/asked-for"));
    assert_eq!(request.source, PinSource::Explicit);
}

#[test]
fn the_working_directory_is_the_fallback() {
    let request = pin_request(None, Some(PathBuf::from("C:/work/cwd"))).expect("a request");

    assert_eq!(request.path, PathBuf::from("C:/work/cwd"));
    assert_eq!(request.source, PinSource::WorkingDirectory);
}

#[test]
fn a_blank_override_is_not_an_override() {
    // An exported-but-empty variable is how a shell script sets one it did not
    // have a value for. Treating it as a path would refuse to start over a
    // variable the user thinks is unset.
    let request = pin_request(Some("   "), Some(PathBuf::from("C:/work/cwd"))).expect("a request");

    assert_eq!(request.source, PinSource::WorkingDirectory);
}

#[test]
fn an_override_is_trimmed_like_every_other_path_here() {
    let request = pin_request(Some("  C:/work/asked-for  "), None).expect("a request");

    assert_eq!(request.path, PathBuf::from("C:/work/asked-for"));
}

#[test]
fn nothing_to_go_on_is_no_request_rather_than_an_error() {
    // A process with no override and no readable working directory shares the
    // app's workspace, which is what every client did before this existed.
    assert_eq!(pin_request(None, None), None);
}

/// The source is carried so the two failures can be told apart.
///
/// An explicit variable naming an unregistered folder is a user mistake and
/// must be reported. A working directory that matches nothing is the ordinary
/// case for a client launched outside any project, and must fall through.
#[test]
fn the_source_distinguishes_a_request_from_a_guess() {
    let asked = pin_request(Some("C:/x"), None).expect("a request");
    let guessed = pin_request(None, Some(PathBuf::from("C:/x"))).expect("a request");

    assert_eq!(asked.path, guessed.path);
    assert_ne!(asked.source, guessed.source);
}
