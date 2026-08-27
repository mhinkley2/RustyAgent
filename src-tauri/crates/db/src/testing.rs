//! Shared test fixtures.
//!
//! Gated behind the `testing` feature so it never compiles into a release
//! binary. Every crate that needs a database in its tests depends on
//! `db = { path = "../db", features = ["testing"] }` under `[dev-dependencies]`
//! rather than hand-rolling its own pool.

use sqlx::sqlite::SqlitePoolOptions;

use crate::DbPool;

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
pub async fn seed_workspace(db: &DbPool, id: &str, path: &str) {
    sqlx::query(
        "INSERT INTO workspaces (id, path, name, last_opened_at)
         VALUES (?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
    )
    .bind(id)
    .bind(path)
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
