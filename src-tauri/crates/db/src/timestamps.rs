//! The one spelling of a timestamp, for every column the application writes.
//!
//! SQLite has no timestamp type. These are `TEXT` columns and the format is a
//! convention nothing but the application enforces — no constraint will ever
//! catch a writer that picks the other one, so a test has to.
//!
//! The schema settled the question already: every `DEFAULT` in the migrations
//! is `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')`, which is RFC 3339 and parses
//! where the app reads it. `CURRENT_TIMESTAMP` yields `YYYY-MM-DD HH:MM:SS` —
//! a space for the `T`, no fractional seconds, no offset — and does not. A row
//! that took its default parsed; a row written by application code did not.
//! `row_to_run` swallowed the resulting error with `.ok()?` and every finished
//! run rendered its duration as `—`.
//!
//! So the fix is on the writing side: application code matches the schema
//! rather than the parser being loosened to accept both, which would make
//! every future writer's choice invisible and leave two formats in the column
//! forever. [`NOW_ISO8601`] is that single place to be wrong.

/// A SQL expression yielding the current UTC time in the spelling the schema's
/// own defaults use.
///
/// Interpolate it into a statement rather than binding it — it is SQL, not a
/// value. Prefer letting a column's `DEFAULT` apply where the insert can omit
/// the column entirely; this is for the writes that must name it.
pub const NOW_ISO8601: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

/// The same instant, in the same spelling, produced in Rust.
///
/// For a value that has to exist before the statement runs. Kept beside
/// [`NOW_ISO8601`] so the two cannot drift apart unnoticed.
pub fn now_iso8601() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// Whether a stored timestamp is in the format every reader parses.
///
/// The guard the database cannot provide. A writer that reaches for
/// `CURRENT_TIMESTAMP` fails this, and fails it in a test rather than in a
/// blank column somebody eventually notices.
pub fn parses_as_rfc3339(value: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(value).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_side_timestamp_round_trips_through_the_parser_that_reads_it() {
        let now = now_iso8601();
        assert!(
            parses_as_rfc3339(&now),
            "now_iso8601 produced {now:?}, which the reader cannot parse"
        );
    }

    #[test]
    fn current_timestamp_is_what_this_module_exists_to_avoid() {
        // Pins the reason. If this ever starts parsing, the constant and the
        // migration that normalises history stop being needed.
        assert!(!parses_as_rfc3339("2026-08-28 18:06:15"));
    }

    #[tokio::test]
    async fn sql_side_timestamp_round_trips_through_the_parser_that_reads_it() {
        let pool = crate::testing::make_test_pool().await;
        let written: String = sqlx::query_scalar(&format!("SELECT {NOW_ISO8601}"))
            .fetch_one(&pool)
            .await
            .expect("evaluate the timestamp expression");
        assert!(
            parses_as_rfc3339(&written),
            "NOW_ISO8601 produced {written:?}, which the reader cannot parse"
        );
    }

    #[tokio::test]
    async fn the_two_spellings_agree_to_the_second() {
        // They are written by different mechanisms; a divergence here means
        // one of them changed and the other did not.
        let pool = crate::testing::make_test_pool().await;
        let sql: String = sqlx::query_scalar(&format!("SELECT {NOW_ISO8601}"))
            .fetch_one(&pool)
            .await
            .expect("evaluate the timestamp expression");
        let rust = now_iso8601();

        let sql = chrono::DateTime::parse_from_rfc3339(&sql).expect("SQL side parses");
        let rust = chrono::DateTime::parse_from_rfc3339(&rust).expect("Rust side parses");
        assert!(
            (sql - rust).num_seconds().abs() <= 5,
            "SQL gave {sql} and Rust gave {rust}; they are not the same clock"
        );
    }

    /// The migration rewrites what the old writers left behind. Running its
    /// statements twice must not append a second suffix — a user upgrading
    /// through several versions runs migrations once, but the same UPDATE
    /// shape is the one to reach for next time and it should be safe.
    #[tokio::test]
    async fn normalising_a_legacy_timestamp_is_idempotent() {
        let pool = crate::testing::make_test_pool().await;
        crate::testing::seed_profile(&pool, "a1", "Agent").await;
        crate::testing::seed_story(&pool, "s1", "Story", "done").await;
        sqlx::query(
            "INSERT INTO story_runs (id, story_id, agent_profile_id, status, started_at)
             VALUES ('r1', 's1', 'a1', 'done', '2026-05-22 20:58:30')",
        )
        .execute(&pool)
        .await
        .expect("seed a legacy row");

        const NORMALISE: &str = "
            UPDATE story_runs
               SET started_at = replace(started_at, ' ', 'T') || '.000Z'
             WHERE started_at IS NOT NULL
               AND length(started_at) = 19
               AND substr(started_at, 11, 1) = ' '";

        for _ in 0..2 {
            sqlx::query(NORMALISE).execute(&pool).await.expect("normalise");
        }

        let stored: String =
            sqlx::query_scalar("SELECT started_at FROM story_runs WHERE id = 'r1'")
                .fetch_one(&pool)
                .await
                .expect("read it back");

        assert_eq!(stored, "2026-05-22T20:58:30.000Z");
        assert!(parses_as_rfc3339(&stored));
    }
}
