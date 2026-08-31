//! Tests for [`crate::stories`] — the board's read.
//!
//! Chiefly the latest-run join. The board renders every story at once, so the
//! run summary on a card has to arrive with the stories rather than as a query
//! per card.

use db::testing::{make_test_pool, seed_profile, seed_story};
use db::DbPool;

use crate::stories::{get_stories, Story};

const PROFILE: &str = "agent-1";

async fn pool() -> DbPool {
    let db = make_test_pool().await;
    seed_profile(&db, PROFILE, "An agent").await;
    db
}

/// A run with an explicit timestamp, so a test can order two of them.
async fn seed_run_at(db: &DbPool, id: &str, story_id: &str, started_at: &str, status: &str) {
    sqlx::query(
        "INSERT INTO story_runs
             (id, story_id, agent_profile_id, status, started_at,
              iteration_count, input_tokens, output_tokens, estimated_cost_usd)
         VALUES (?, ?, ?, ?, ?, 3, 100, 50, 0.25)",
    )
    .bind(id)
    .bind(story_id)
    .bind(PROFILE)
    .bind(status)
    .bind(started_at)
    .execute(db)
    .await
    .expect("seed a run");
}

async fn stories_of(db: &DbPool) -> Vec<Story> {
    get_stories(db, None).await.expect("get_stories")
}

#[tokio::test]
async fn a_story_that_has_never_run_carries_no_run() {
    let db = pool().await;
    seed_story(&db, "s1", "Never run", "ready").await;

    let stories = stories_of(&db).await;

    assert_eq!(stories.len(), 1);
    assert!(
        stories[0].latest_run.is_none(),
        "the card must render as it did before, not with an empty run slot"
    );
}

#[tokio::test]
async fn a_story_carries_its_most_recent_run() {
    let db = pool().await;
    seed_story(&db, "s1", "Has runs", "in_progress").await;
    seed_run_at(&db, "old", "s1", "2026-04-13T00:00:00.000Z", "failed").await;
    seed_run_at(&db, "new", "s1", "2026-04-13T01:00:00.000Z", "running").await;

    let latest = stories_of(&db).await.remove(0).latest_run.expect("a run");

    assert_eq!(latest.id, "new");
    assert_eq!(latest.status, "running");
}

/// `story_runs.started_at` is written with `CURRENT_TIMESTAMP`, whose
/// resolution is one second. Two runs of a quickly-retried story share a
/// timestamp, and without the `rowid` tiebreak "latest" would be arbitrary —
/// the card could show the earlier attempt.
#[tokio::test]
async fn runs_started_in_the_same_second_are_ordered_by_insertion() {
    let db = pool().await;
    seed_story(&db, "s1", "Retried fast", "in_progress").await;
    let same_second = "2026-04-13T00:00:00.000Z";
    seed_run_at(&db, "first", "s1", same_second, "failed").await;
    seed_run_at(&db, "second", "s1", same_second, "running").await;

    let latest = stories_of(&db).await.remove(0).latest_run.expect("a run");

    assert_eq!(latest.id, "second", "the later insertion is the later run");
}

/// One story's runs must not appear on another's card.
#[tokio::test]
async fn each_story_gets_its_own_latest_run() {
    let db = pool().await;
    seed_story(&db, "s1", "First", "in_progress").await;
    seed_story(&db, "s2", "Second", "in_progress").await;
    seed_run_at(&db, "r1", "s1", "2026-04-13T00:00:00.000Z", "done").await;
    seed_run_at(&db, "r2", "s2", "2026-04-13T00:00:01.000Z", "failed").await;

    let stories = stories_of(&db).await;
    let by_id = |id: &str| {
        stories
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("story {id}"))
    };

    assert_eq!(by_id("s1").latest_run.as_ref().expect("run").id, "r1");
    assert_eq!(by_id("s2").latest_run.as_ref().expect("run").id, "r2");
}

/// A story with many runs must not multiply into many rows — the join picks
/// one run, it does not fan the story out.
#[tokio::test]
async fn many_runs_do_not_duplicate_the_story() {
    let db = pool().await;
    seed_story(&db, "s1", "Much retried", "in_progress").await;
    for i in 0..5 {
        seed_run_at(
            &db,
            &format!("r{i}"),
            "s1",
            &format!("2026-04-13T00:00:0{i}.000Z"),
            "failed",
        )
        .await;
    }

    let stories = stories_of(&db).await;

    assert_eq!(stories.len(), 1, "one story, however many runs it has");
    assert_eq!(stories[0].latest_run.as_ref().expect("run").id, "r4");
}

#[tokio::test]
async fn the_run_summary_carries_what_the_card_shows() {
    let db = pool().await;
    seed_story(&db, "s1", "Finished", "review").await;
    seed_run_at(&db, "r1", "s1", "2026-04-13T00:00:00.000Z", "done").await;
    sqlx::query("UPDATE story_runs SET finished_at = '2026-04-13T00:05:00.000Z' WHERE id = 'r1'")
        .execute(&db)
        .await
        .expect("finish the run");

    let latest = stories_of(&db).await.remove(0).latest_run.expect("a run");

    assert_eq!(latest.status, "done");
    assert_eq!(latest.started_at, "2026-04-13T00:00:00.000Z");
    assert_eq!(latest.finished_at.as_deref(), Some("2026-04-13T00:05:00.000Z"));
    assert_eq!(latest.iteration_count, 3);
    assert_eq!(latest.input_tokens, 100);
    assert_eq!(latest.output_tokens, 50);
    assert!((latest.estimated_cost_usd - 0.25).abs() < f64::EPSILON);
}
