// SQLite database pool, migrations, and query helpers.
// See RUSTYAGE-1 for implementation details.

use anyhow::{Context, Result};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use tracing::info;

pub type DbPool = SqlitePool;

/// Where the data directory and the database live, and how the environment
/// overrides them. Shared by the desktop app and the standalone MCP binary.
pub mod paths;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

#[cfg(test)]
mod paths_tests;

pub struct WorkspaceRecord {
    pub id: String,
    pub name: String,
    pub path: String,
    pub last_opened_at: String,
    pub created_at: String,
}

fn normalize_workspace_path(path: &std::path::Path) -> String {
    let raw = path.to_string_lossy();
    raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_string()
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

    sqlx::query_as::<_, (String, String, String, String, String)>(
        "SELECT id, name, path, last_opened_at, created_at
         FROM workspaces
         WHERE path = ?",
    )
    .bind(&normalized_path)
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
    let workspace_name = path
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
}
