// SQLite database pool, migrations, and query helpers.
// See RUSTYAGE-1 for implementation details.

use anyhow::{Context, Result};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use tracing::info;

pub type DbPool = SqlitePool;

/// Where the data directory and the database live, and how the environment
/// overrides them. Shared by the desktop app and the standalone MCP binary.
pub mod paths;

/// Startup reconciliation of runs and approvals a previous session left
/// mid-flight, plus the per-process id that makes it safe.
pub mod recovery;
pub mod story_status;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

#[cfg(test)]
mod paths_tests;

#[cfg(test)]
mod migration_lock_tests;

#[cfg(test)]
mod recovery_tests;

pub struct WorkspaceRecord {
    pub id: String,
    pub name: String,
    pub path: String,
    pub last_opened_at: String,
    pub created_at: String,
}

/// The one spelling of a workspace path, for both storing and looking up.
///
/// One folder must resolve to one row, and `workspaces.path` is a `UNIQUE`
/// column compared with SQLite's default `BINARY` collation — so the spelling
/// *is* the identity. On Windows the filesystem disagrees: `c:\users\...` and
/// `C:\Users\...` are the same directory, and storing one while looking up the
/// other silently mints a second board for a project that already has one.
///
/// `canonicalize` settles it. It returns the true on-disk casing, so every
/// spelling of a real folder converges on what the volume actually holds, and
/// it resolves symlinks, so a project opened through a link and one named
/// directly land on the same row. It also prepends the `\\?\` extended-length
/// prefix, which has to come back off: every row already in the database is
/// stored without it.
///
/// A path that is not on disk cannot be canonicalized — an unmounted drive, a
/// client naming a folder that does not exist, or one of the `/tmp/w` literals
/// the tests seed. Those keep the raw spelling with the prefix stripped, which
/// is what this function did before, and still the right answer for a lookup
/// that is about to miss anyway.
pub fn normalize_workspace_path(path: &std::path::Path) -> String {
    let canonical = std::fs::canonicalize(path);
    let resolved = canonical.as_deref().unwrap_or(path);
    let raw = resolved.to_string_lossy();
    let stripped = raw.strip_prefix(r"\\?\").unwrap_or(&raw);
    trim_trailing_separator(stripped).to_string()
}

/// Drop a trailing `/` or `\`, so `…\project\` and `…\project` are one row.
///
/// Never down to nothing: `/` and `C:\` are directories in their own right,
/// and trimming them would leave an empty string or a bare drive letter that
/// no longer names anything.
fn trim_trailing_separator(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() || trimmed.ends_with(':') {
        path
    } else {
        trimmed
    }
}

/// Initialize the SQLite database pool.
///
/// - Creates the database file at `db_path` if it does not exist.
/// - Enables WAL mode for better concurrent read performance.
/// - Runs all pending migrations automatically.
pub async fn init_db(db_path: &str) -> Result<DbPool> {
    let connection_string = format!("sqlite://{}?mode=rwc", db_path);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&connection_string)
        .await
        .with_context(|| format!("Failed to open SQLite database at {db_path}"))?;

    // Enable WAL mode for better performance under concurrent reads.
    sqlx::query("PRAGMA journal_mode=WAL")
        .execute(&pool)
        .await
        .context("Failed to enable WAL mode")?;

    // Enable foreign key enforcement.
    sqlx::query("PRAGMA foreign_keys=ON")
        .execute(&pool)
        .await
        .context("Failed to enable foreign keys")?;

    // Run all pending migrations from the `migrations/` folder.
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("Failed to run database migrations")?;

    info!("Database initialized at {db_path}");

    Ok(pool)
}

/// Returns the absolute path of the most recently opened workspace, if any.
/// The active workspace is the one with the most recent `last_opened_at` timestamp.
///
/// The `created_at` tiebreak matches [`list_workspaces`] exactly. `last_opened_at`
/// has millisecond resolution, so two workspaces touched in the same millisecond
/// can tie; ordering by `created_at` keeps this function consistent with
/// `list_workspaces` when that happens.
pub async fn get_active_workspace_path(db: &DbPool) -> Option<std::path::PathBuf> {
    let row = sqlx::query(
        "SELECT path FROM workspaces ORDER BY last_opened_at DESC, created_at DESC LIMIT 1"
    )
    .fetch_optional(db)
    .await
    .ok()??;
    let path: String = sqlx::Row::try_get(&row, "path").ok()?;
    Some(std::path::PathBuf::from(path))
}

/// Minimal workspace record returned by [`get_most_recent_workspace`].
pub struct MostRecentWorkspace {
    pub id: String,
    pub path: String,
}

/// Returns the id + path of the most recently opened workspace, if any.
pub async fn get_most_recent_workspace(db: &DbPool) -> Option<MostRecentWorkspace> {
    let row = sqlx::query(
        // Same tiebreak as `list_workspaces` and `get_active_workspace_path`,
        // so all three agree on "most recent" when timestamps collide.
        "SELECT id, path FROM workspaces ORDER BY last_opened_at DESC, created_at DESC LIMIT 1"
    )
    .fetch_optional(db)
    .await
    .ok()??;
    Some(MostRecentWorkspace {
        id:   sqlx::Row::try_get(&row, "id").ok()?,
        path: sqlx::Row::try_get(&row, "path").ok()?,
    })
}

pub async fn list_workspaces(db: &DbPool) -> Result<Vec<WorkspaceRecord>> {
    let rows = sqlx::query(
        "SELECT id, name, path, last_opened_at, created_at
         FROM workspaces
         ORDER BY last_opened_at DESC, created_at DESC"
    )
    .fetch_all(db)
    .await
    .context("Failed to list workspaces")?;

    rows.into_iter()
        .map(|row| {
            Ok(WorkspaceRecord {
                id: sqlx::Row::try_get(&row, "id")?,
                name: sqlx::Row::try_get(&row, "name")?,
                path: sqlx::Row::try_get(&row, "path")?,
                last_opened_at: sqlx::Row::try_get(&row, "last_opened_at")?,
                created_at: sqlx::Row::try_get(&row, "created_at")?,
            })
        })
        .collect::<std::result::Result<Vec<_>, sqlx::Error>>()
        .context("Failed to decode workspace rows")
}

/// Look up a workspace by path, without creating one.
///
/// Unlike [`touch_workspace`], this never inserts. It is what lets an MCP
/// client select only from workspaces the user has already opened in the app,
/// rather than registering arbitrary directories on the machine.
pub async fn find_workspace_by_path(
    db: &DbPool,
    path: &std::path::Path,
) -> Option<WorkspaceRecord> {
    let normalized_path = normalize_workspace_path(path);

    if let Some(found) = select_workspace(db, "WHERE path = ?", &normalized_path).await {
        return Some(found);
    }

    // Rows written before paths were canonicalized can differ from the
    // canonical spelling by case alone, and where the filesystem is
    // case-insensitive they name the same folder. Refusing those would tell a
    // user their own project is not a workspace this app has opened, which is
    // both wrong and unfixable from their side — the row is already there and
    // nothing re-spells it.
    //
    // Only where the filesystem agrees. On a case-sensitive volume
    // `/srv/Board` and `/srv/board` are two directories, and folding them
    // together would hand a client the wrong project's board.
    //
    // `NOCASE` folds ASCII only, which covers drive letters and ordinary
    // repository paths. A non-ASCII folder whose stored casing differs still
    // misses, falling back to the same refusal as before rather than to
    // something worse.
    if !CASE_INSENSITIVE_PATHS {
        return None;
    }

    select_workspace(
        db,
        "WHERE path = ? COLLATE NOCASE ORDER BY last_opened_at DESC",
        &normalized_path,
    )
    .await
}

/// Whether this platform's filesystem treats two casings as one path.
///
/// Windows and macOS ship case-insensitive by default; Linux does not. A
/// case-sensitive volume mounted on Windows (or a case-sensitive APFS one)
/// would make this too generous, but only for a path that already failed an
/// exact match against a workspace the user registered themselves — the
/// fallback can hand back a different registered board, never an unregistered
/// directory.
const CASE_INSENSITIVE_PATHS: bool = cfg!(any(windows, target_os = "macos"));

/// One row of `workspaces`, selected by a fixed clause.
///
/// The clause is always a literal from this module — never a caller's string —
/// so the interpolation carries no input into SQL.
async fn select_workspace(db: &DbPool, clause: &str, path: &str) -> Option<WorkspaceRecord> {
    sqlx::query_as::<_, (String, String, String, String, String)>(&format!(
        "SELECT id, name, path, last_opened_at, created_at
         FROM workspaces
         {clause}
         LIMIT 1"
    ))
    .bind(path)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .map(|(id, name, path, last_opened_at, created_at)| WorkspaceRecord {
        id,
        name,
        path,
        last_opened_at,
        created_at,
    })
}

pub async fn touch_workspace(db: &DbPool, path: &std::path::Path) -> Result<WorkspaceRecord> {
    let normalized_path = normalize_workspace_path(path);
    // Named from the normalized path, not the caller's spelling: the display
    // name should read the way the folder is actually cased on disk.
    let workspace_name = std::path::Path::new(&normalized_path)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(&normalized_path)
        .to_string();
    let workspace_id = uuid::Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO workspaces (id, path, name, last_opened_at)
         VALUES (?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         ON CONFLICT(path) DO UPDATE SET
            name = excluded.name,
            last_opened_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')"
    )
    .bind(&workspace_id)
    .bind(&normalized_path)
    .bind(&workspace_name)
    .execute(db)
    .await
    .with_context(|| format!("Failed to upsert workspace '{}'", normalized_path))?;

    let row = sqlx::query(
        "SELECT id, name, path, last_opened_at, created_at
         FROM workspaces
         WHERE path = ?"
    )
    .bind(&normalized_path)
    .fetch_one(db)
    .await
    .with_context(|| format!("Failed to reload workspace '{}'", normalized_path))?;

    Ok(WorkspaceRecord {
        id: sqlx::Row::try_get(&row, "id").context("Missing workspace id")?,
        name: sqlx::Row::try_get(&row, "name").context("Missing workspace name")?,
        path: sqlx::Row::try_get(&row, "path").context("Missing workspace path")?,
        last_opened_at: sqlx::Row::try_get(&row, "last_opened_at").context("Missing workspace last_opened_at")?,
        created_at: sqlx::Row::try_get(&row, "created_at").context("Missing workspace created_at")?,
    })
}

#[cfg(test)]
mod tests {
    /// The retry budget ships enabled. A column added with a default of zero
    /// would be a setting that exists and does nothing, which is the failure
    /// mode this repo has hit before with `context_strategy` and
    /// `allow_network_hosts`.
    #[tokio::test]
    async fn an_agent_profile_gets_a_retry_budget_by_default() {
        let db = crate::testing::make_test_pool().await;
        crate::testing::seed_profile(&db, "p1", "An agent").await;

        let max_retries: i64 = sqlx::query_scalar("SELECT max_retries FROM agent_profiles WHERE id = ?")
            .bind("p1")
            .fetch_one(&db)
            .await
            .expect("read max_retries");

        assert_eq!(max_retries, 2, "three attempts in total");
    }

    use super::*;
    use sqlx::Row;
    use std::env;

    /// Returns a temp-file path unique to this test run.
    fn temp_db_path() -> std::path::PathBuf {
        env::temp_dir().join(format!("rustyagent_test_{}.db", uuid::Uuid::new_v4()))
    }

    fn cleanup(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn init_db_creates_all_expected_tables() {
        let path = temp_db_path();
        let db = init_db(path.to_str().unwrap()).await.expect("init_db failed");

        let rows = sqlx::query("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .fetch_all(&db)
            .await
            .expect("query failed");

        let tables: Vec<String> = rows
            .into_iter()
            .map(|r| r.try_get::<String, _>("name").unwrap())
            .collect();

        let expected = [
            "agent_memory",
            "agent_profiles",
            "agent_tool_bindings",
            "mcp_servers",
            "run_events",
            "stories",
            "story_runs",
        ];
        for table in &expected {
            assert!(
                tables.iter().any(|t| t == table),
                "Expected table '{table}' not found; found: {tables:?}"
            );
        }

        drop(db);
        cleanup(&path);
    }

    #[tokio::test]
    async fn init_db_is_idempotent() {
        let path = temp_db_path();
        let path_str = path.to_str().unwrap().to_string();

        let db1 = init_db(&path_str).await.expect("first init_db failed");
        drop(db1);

        // Second call on the same file must not return an error.
        let db2 = init_db(&path_str).await.expect("second init_db failed");
        drop(db2);

        cleanup(&path);
    }

    #[tokio::test]
    async fn foreign_keys_are_enforced_after_init() {
        let path = temp_db_path();
        let db = init_db(path.to_str().unwrap()).await.expect("init_db failed");

        // Attempt to insert a story referencing a non-existent agent_profile_id.
        // With FK enforcement on, this must fail.
        let result = sqlx::query(
            "INSERT INTO stories (id, title, assigned_agent_id) VALUES ('s1', 'Test', 'no-such-agent')"
        )
        .execute(&db)
        .await;

        assert!(result.is_err(), "Expected FK violation but insert succeeded");

        drop(db);
        cleanup(&path);
    }

    /// A folder named in a different case is the same folder.
    ///
    /// The case this exists for: the app stores what `canonicalize` reports,
    /// and an MCP client hands back whatever its editor templated. On Windows
    /// those can differ by case and still be one directory. This asserts
    /// through a real temp directory rather than a string literal, because the
    /// answer comes from the filesystem.
    #[tokio::test]
    async fn a_workspace_is_found_under_a_differently_cased_spelling_of_its_path() {
        let path = temp_db_path();
        let db = init_db(path.to_str().unwrap()).await.expect("init_db failed");

        let root = env::temp_dir().join(format!("RustyAgentCase{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create workspace dir");

        let stored = touch_workspace(&db, &root).await.expect("touch_workspace");

        let shouted = std::path::PathBuf::from(root.to_string_lossy().to_uppercase());
        let found = find_workspace_by_path(&db, &shouted).await;

        // Only meaningful where the filesystem itself is case-insensitive; on
        // a case-sensitive volume the uppercase path is a different folder and
        // refusing it is the correct answer.
        if CASE_INSENSITIVE_PATHS {
            let found = found.expect("the same folder, shouted, is still that workspace");
            assert_eq!(found.id, stored.id, "must not be a second workspace row");
        }

        let _ = std::fs::remove_dir_all(&root);
        drop(db);
        cleanup(&path);
    }

    /// A trailing separator is not a different workspace.
    #[tokio::test]
    async fn a_trailing_separator_resolves_to_the_same_workspace() {
        let path = temp_db_path();
        let db = init_db(path.to_str().unwrap()).await.expect("init_db failed");

        let root = env::temp_dir().join(format!("rustyagent-slash-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create workspace dir");

        let stored = touch_workspace(&db, &root).await.expect("touch_workspace");

        let with_slash = std::path::PathBuf::from(format!("{}{}", root.display(), "/"));
        let found = find_workspace_by_path(&db, &with_slash)
            .await
            .expect("trailing separator still names the workspace");

        assert_eq!(found.id, stored.id);

        let _ = std::fs::remove_dir_all(&root);
        drop(db);
        cleanup(&path);
    }

    /// Rows that predate canonicalization must stay reachable.
    ///
    /// Seeded directly, in a casing `canonicalize` would never produce, so it
    /// stands in for what is already in a user's database. Without the
    /// case-insensitive fallback the user is told their own project is not a
    /// workspace this app has opened, and nothing they can do re-spells the
    /// row.
    #[tokio::test]
    async fn a_legacy_row_stored_in_another_case_is_still_found() {
        let path = temp_db_path();
        let db = init_db(path.to_str().unwrap()).await.expect("init_db failed");

        let root = env::temp_dir().join(format!("rustyagent-legacy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create workspace dir");

        let legacy = normalize_workspace_path(&root).to_uppercase();
        sqlx::query("INSERT INTO workspaces (id, path, name) VALUES ('ws-legacy', ?, 'legacy')")
            .bind(&legacy)
            .execute(&db)
            .await
            .expect("seed legacy workspace");

        let found = find_workspace_by_path(&db, &root).await;

        if CASE_INSENSITIVE_PATHS {
            let found = found.expect("the legacy row is still this folder's workspace");
            assert_eq!(found.id, "ws-legacy");
        }

        let _ = std::fs::remove_dir_all(&root);
        drop(db);
        cleanup(&path);
    }

    /// An exact match wins over one that differs only by case.
    ///
    /// Both spellings can already be in a database. The fallback exists to
    /// rescue a miss, and must never re-point a path that matched exactly.
    #[tokio::test]
    async fn an_exact_match_wins_over_a_case_insensitive_one() {
        let path = temp_db_path();
        let db = init_db(path.to_str().unwrap()).await.expect("init_db failed");

        // Not on disk, so normalization leaves the spelling alone and the two
        // rows stay distinguishable.
        for (id, ws_path, opened) in [
            ("ws-exact", "/tmp/Casing", "2026-01-01T00:00:00.000Z"),
            ("ws-other", "/tmp/CASING", "2026-06-01T00:00:00.000Z"),
        ] {
            sqlx::query(
                "INSERT INTO workspaces (id, path, name, last_opened_at, created_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(ws_path)
            .bind(id)
            .bind(opened)
            .bind(opened)
            .execute(&db)
            .await
            .expect("seed workspace");
        }

        let found = find_workspace_by_path(&db, std::path::Path::new("/tmp/Casing"))
            .await
            .expect("exact spelling resolves");

        assert_eq!(
            found.id, "ws-exact",
            "the more recently opened row must not win over an exact match"
        );

        drop(db);
        cleanup(&path);
    }

    /// Normalizing must not eat a root directory.
    #[test]
    fn a_root_path_survives_normalization() {
        assert_eq!(trim_trailing_separator("/"), "/");
        assert_eq!(trim_trailing_separator(r"C:\"), r"C:\");
        assert_eq!(trim_trailing_separator(r"C:\projects\board\"), r"C:\projects\board");
        assert_eq!(trim_trailing_separator("/tmp/w"), "/tmp/w");
    }

    /// A path that is not on disk keeps its spelling.
    ///
    /// `canonicalize` fails there, and the fallback must not be an empty
    /// string or a panic — a lookup for a folder that does not exist should
    /// simply miss.
    #[test]
    fn a_path_that_does_not_exist_normalizes_to_itself() {
        let missing = env::temp_dir().join(format!("rustyagent-absent-{}", uuid::Uuid::new_v4()));
        assert_eq!(
            normalize_workspace_path(&missing),
            missing.to_string_lossy(),
        );
    }

    /// The three "most recent workspace" queries must name the same row.
    ///
    /// Timestamps are written directly here rather than via `touch_workspace`,
    /// so the ordering under test is unambiguous instead of depending on how
    /// fast the machine happens to be.
    #[tokio::test]
    async fn the_most_recent_workspace_queries_agree_when_timestamps_differ() {
        let path = temp_db_path();
        let db = init_db(path.to_str().unwrap()).await.expect("init_db failed");

        for (id, ws_path, opened) in [
            ("ws-older", "/tmp/older", "2026-01-01T00:00:00.000Z"),
            ("ws-newer", "/tmp/newer", "2026-01-02T00:00:00.000Z"),
        ] {
            sqlx::query(
                "INSERT INTO workspaces (id, path, name, last_opened_at, created_at)                  VALUES (?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(ws_path)
            .bind(id)
            .bind(opened)
            .bind(opened)
            .execute(&db)
            .await
            .expect("seed workspace");
        }

        let listed = list_workspaces(&db).await.expect("list_workspaces failed");
        let active = get_active_workspace_path(&db).await.expect("active path");
        let recent = get_most_recent_workspace(&db).await.expect("recent workspace");

        assert_eq!(listed[0].path, "/tmp/newer");
        assert_eq!(active, std::path::PathBuf::from("/tmp/newer"));
        assert_eq!(recent.path, "/tmp/newer");
        assert_eq!(recent.id, "ws-newer");

        drop(db);
        cleanup(&path);
    }

    #[tokio::test]
    async fn touch_workspace_upserts_and_promotes_workspace() {
        let path = temp_db_path();
        let db = init_db(path.to_str().unwrap()).await.expect("init_db failed");

        let first_dir = env::temp_dir().join(format!("rustyagent-ws-a-{}", uuid::Uuid::new_v4()));
        let second_dir = env::temp_dir().join(format!("rustyagent-ws-b-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&first_dir).expect("failed to create first workspace dir");
        std::fs::create_dir_all(&second_dir).expect("failed to create second workspace dir");

        let first = touch_workspace(&db, &first_dir).await.expect("touch first workspace failed");
        let second = touch_workspace(&db, &second_dir).await.expect("touch second workspace failed");
        let promoted = touch_workspace(&db, &first_dir).await.expect("promote first workspace failed");

        assert_eq!(first.path, promoted.path);
        assert_eq!(first.id, promoted.id);
        assert_ne!(first.id, second.id);

        let workspaces = list_workspaces(&db).await.expect("list_workspaces failed");
        assert_eq!(workspaces.len(), 2, "upsert must not create a third row");

        // Deliberately not `workspaces[0].path == first.path`.
        //
        // `last_opened_at` is stamped by strftime at millisecond resolution, so
        // three touches in a row against an in-memory database routinely land
        // inside the same millisecond. The ordering between them is then a tie,
        // and asserting a winner asserts something the data does not express —
        // which is exactly how this test came to fail roughly one run in ten and
        // abort the whole workspace suite with it.
        //
        // What promotion actually guarantees is that the promoted row's
        // timestamp is not behind the row touched before it. Ordering itself is
        // covered deterministically by
        // `the_most_recent_workspace_queries_agree_when_timestamps_differ`.
        assert!(
            promoted.last_opened_at >= second.last_opened_at,
            "promoting must not move a workspace backwards: {} < {}",
            promoted.last_opened_at,
            second.last_opened_at
        );

        drop(db);
        let _ = std::fs::remove_dir_all(&first_dir);
        let _ = std::fs::remove_dir_all(&second_dir);
        cleanup(&path);
    }

    async fn column_names(db: &DbPool, table: &str) -> Vec<String> {
        sqlx::query(&format!("PRAGMA table_info({table})"))
            .fetch_all(db)
            .await
            .expect("table_info")
            .into_iter()
            .map(|r| r.try_get::<String, _>("name").expect("name"))
            .collect()
    }

    /// `allow_network_hosts` was stored and rendered but never read by any
    /// decision. It is gone from the schema; the controls that are actually
    /// enforced are not.
    #[tokio::test]
    async fn agent_permissions_carries_only_the_enforced_controls() {
        let path = temp_db_path();
        let db = init_db(path.to_str().unwrap()).await.expect("init_db failed");

        let columns = column_names(&db, "agent_permissions").await;

        assert!(
            !columns.iter().any(|c| c == "allow_network_hosts"),
            "allow_network_hosts should have been dropped; found {columns:?}"
        );
        for kept in [
            "profile_id",
            "allowed_tools",
            "allow_file_read_paths",
            "allow_file_write_paths",
            "allow_shell_commands",
            "require_approval_on_write",
        ] {
            assert!(columns.iter().any(|c| c == kept), "missing '{kept}' in {columns:?}");
        }

        drop(db);
        cleanup(&path);
    }

    /// Dropping a column from a table that already holds rows must not take the
    /// rows, or the other columns' values, with it. This rebuilds the
    /// pre-migration shape and applies the same statement the migration does.
    #[tokio::test]
    async fn dropping_allow_network_hosts_preserves_existing_permission_rows() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");

        sqlx::query(
            "CREATE TABLE agent_permissions (
                 profile_id TEXT PRIMARY KEY NOT NULL,
                 allowed_tools TEXT NOT NULL DEFAULT '[]',
                 allow_file_read_paths TEXT NOT NULL DEFAULT '[]',
                 allow_file_write_paths TEXT NOT NULL DEFAULT '[]',
                 allow_shell_commands TEXT NOT NULL DEFAULT '[]',
                 allow_network_hosts TEXT NOT NULL DEFAULT '[]',
                 require_approval_on_write INTEGER NOT NULL DEFAULT 0
             )",
        )
        .execute(&pool)
        .await
        .expect("create pre-migration table");

        sqlx::query(
            "INSERT INTO agent_permissions
                 (profile_id, allowed_tools, allow_file_read_paths, allow_file_write_paths,
                  allow_shell_commands, allow_network_hosts, require_approval_on_write)
             VALUES ('agent-1', '[\"file_read\"]', '[\"docs\"]', '[\"src\"]',
                     '[\"git\"]', '[\"api.github.com\"]', 1)",
        )
        .execute(&pool)
        .await
        .expect("seed a profile that configured a network allow-list");

        sqlx::query("ALTER TABLE agent_permissions DROP COLUMN allow_network_hosts")
            .execute(&pool)
            .await
            .expect("drop column");

        let row = sqlx::query(
            "SELECT allowed_tools, allow_file_read_paths, allow_file_write_paths,
                    allow_shell_commands, require_approval_on_write
             FROM agent_permissions WHERE profile_id = 'agent-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("the row must survive the drop");

        assert_eq!(row.try_get::<String, _>("allowed_tools").unwrap(), "[\"file_read\"]");
        assert_eq!(row.try_get::<String, _>("allow_file_read_paths").unwrap(), "[\"docs\"]");
        assert_eq!(row.try_get::<String, _>("allow_file_write_paths").unwrap(), "[\"src\"]");
        assert_eq!(row.try_get::<String, _>("allow_shell_commands").unwrap(), "[\"git\"]");
        assert_eq!(row.try_get::<i64, _>("require_approval_on_write").unwrap(), 1);
    }
}
