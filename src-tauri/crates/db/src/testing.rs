//! Shared test fixtures.
//!
//! Gated behind the `testing` feature so it never compiles into a release
//! binary. Every crate that needs a database in its tests depends on
//! `db = { path = "../db", features = ["testing"] }` under `[dev-dependencies]`
//! rather than hand-rolling its own pool.

use sqlx::sqlite::SqlitePoolOptions;

use crate::DbPool;
use crate::timestamps::NOW_ISO8601;

/// Open a single-connection in-memory SQLite pool and run all migrations.
///
/// Foreign-key enforcement is deliberately left OFF so a test can seed only the
/// rows it actually cares about.
pub async fn make_test_pool() -> DbPool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("Failed to open in-memory SQLite");

    sqlx::query("PRAGMA foreign_keys=OFF")
        .execute(&pool)
        .await
        .expect("Failed to disable foreign keys");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

/// Insert a workspace row. Several tests need two of them to prove that a
/// query is scoped to the active workspace rather than returning everything.
///
/// Stored through [`normalize_workspace_path`](crate::normalize_workspace_path),
/// so a seeded row is spelled the way the app would have spelled it. A test
/// that seeds a real temp directory and then looks it up otherwise depends on
/// the path surviving canonicalization unchanged — true under `/tmp` on Linux,
/// false on macOS, where `/var` is a symlink into `/private/var` and every
/// lookup would miss a row seeded verbatim.
pub async fn seed_workspace(db: &DbPool, id: &str, path: &str) {
    let normalized = crate::normalize_workspace_path(std::path::Path::new(path));

    sqlx::query(
        &format!("INSERT INTO workspaces (id, path, name, last_opened_at)
         VALUES (?, ?, ?, {NOW_ISO8601})"),
    )
    .bind(id)
    .bind(&normalized)
    .bind(id)
    .execute(db)
    .await
    .expect("seed workspaces");
}

/// Insert a minimal agent profile.
pub async fn seed_profile(db: &DbPool, id: &str, name: &str) {
    sqlx::query(
        "INSERT INTO agent_profiles (id, name, provider, model, system_prompt)
         VALUES (?, ?, 'mock', 'mock-model', 'You are a test agent.')",
    )
    .bind(id)
    .bind(name)
    .execute(db)
    .await
    .expect("seed agent_profiles");
}

/// Insert a minimal story.
pub async fn seed_story(db: &DbPool, id: &str, title: &str, status: &str) {
    sqlx::query("INSERT INTO stories (id, title, status) VALUES (?, ?, ?)")
        .bind(id)
        .bind(title)
        .bind(status)
        .execute(db)
        .await
        .expect("seed stories");
}

/// Insert a run row in the `running` state, ready for a runtime to finish.
///
/// `owner_instance_id` is left NULL, which is what a run started by a previous
/// launch of the app looks like to the startup sweep. Use [`seed_run_owned`]
/// for a run that is meant to look live.
pub async fn seed_run(db: &DbPool, id: &str, story_id: &str, agent_profile_id: &str) {
    sqlx::query(
        "INSERT INTO story_runs (id, story_id, agent_profile_id, status)
         VALUES (?, ?, ?, 'running')",
    )
    .bind(id)
    .bind(story_id)
    .bind(agent_profile_id)
    .execute(db)
    .await
    .expect("seed story_runs");
}

/// As [`seed_run`], but claimed by a named application instance.
///
/// A run whose `owner_instance_id` matches the id the sweep is given is one
/// the sweeping process started itself, and must survive the sweep.
pub async fn seed_run_owned(
    db: &DbPool,
    id: &str,
    story_id: &str,
    agent_profile_id: &str,
    owner_instance_id: &str,
) {
    sqlx::query(
        "INSERT INTO story_runs (id, story_id, agent_profile_id, status, owner_instance_id)
         VALUES (?, ?, ?, 'running', ?)",
    )
    .bind(id)
    .bind(story_id)
    .bind(agent_profile_id)
    .bind(owner_instance_id)
    .execute(db)
    .await
    .expect("seed story_runs");
}

/// Read one run's `iteration_count` column.
pub async fn run_iteration_count(db: &DbPool, run_id: &str) -> i64 {
    use sqlx::Row;
    sqlx::query("SELECT iteration_count FROM story_runs WHERE id = ?")
        .bind(run_id)
        .fetch_one(db)
        .await
        .expect("fetch run iteration_count")
        .get::<i64, _>("iteration_count")
}

/// Read one run's `status` column.
pub async fn run_status(db: &DbPool, run_id: &str) -> String {
    use sqlx::Row;
    sqlx::query("SELECT status FROM story_runs WHERE id = ?")
        .bind(run_id)
        .fetch_one(db)
        .await
        .expect("fetch run status")
        .get::<String, _>("status")
}

/// One run's persisted token accounting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub estimated_cost_usd: f64,
}

/// Read the token and cost columns of one `story_runs` row.
pub async fn run_usage(db: &DbPool, run_id: &str) -> RunUsage {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT input_tokens, output_tokens, cache_read_input_tokens,
                cache_creation_input_tokens, estimated_cost_usd
         FROM story_runs WHERE id = ?",
    )
    .bind(run_id)
    .fetch_one(db)
    .await
    .expect("fetch run usage");

    RunUsage {
        input_tokens: row.get("input_tokens"),
        output_tokens: row.get("output_tokens"),
        cache_read_input_tokens: row.get("cache_read_input_tokens"),
        cache_creation_input_tokens: row.get("cache_creation_input_tokens"),
        estimated_cost_usd: row.get("estimated_cost_usd"),
    }
}

/// Read every `run_events` row for a run as `(event_type, content)`, in
/// insertion order.
pub async fn run_events(db: &DbPool, run_id: &str) -> Vec<(String, String)> {
    use sqlx::Row;
    sqlx::query(
        "SELECT event_type, content FROM run_events WHERE run_id = ? ORDER BY sequence_num ASC",
    )
    .bind(run_id)
    .fetch_all(db)
    .await
    .expect("fetch run events")
    .iter()
    .map(|r| {
        (
            r.get::<String, _>("event_type"),
            r.try_get::<String, _>("content").unwrap_or_default(),
        )
    })
    .collect()
}
