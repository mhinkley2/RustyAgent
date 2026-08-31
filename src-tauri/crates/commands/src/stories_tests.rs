//! Tests for [`crate::stories`] — the board's read.
//!
//! Chiefly the latest-run join. The board renders every story at once, so the
//! run summary on a card has to arrive with the stories rather than as a query
//! per card.

use db::testing::{make_test_pool, run_status, seed_profile, seed_run, seed_story};
use db::DbPool;
use sqlx::Row;

use crate::stories::{get_stories, update_story, Story, UpdateStoryInput};

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


// ---------------------------------------------------------------------------
// Timestamp shape
// ---------------------------------------------------------------------------

/// `CURRENT_TIMESTAMP` writes `YYYY-MM-DD HH:MM:SS`, which JavaScript parses as
/// *local* time — so a card built on the raw value shows every elapsed time
/// shifted by the reader's UTC offset, and a run a minute old can appear to
/// start in the future. Every run in a real database is stored this way.
#[tokio::test]
async fn a_current_timestamp_run_comes_back_as_rfc3339() {
    let db = pool().await;
    seed_story(&db, "s1", "Legacy timestamps", "in_progress").await;
    sqlx::query(
        "INSERT INTO story_runs (id, story_id, agent_profile_id, status, started_at, finished_at)
         VALUES ('r1', 's1', ?, 'done', '2026-05-22 20:58:30', '2026-05-22 20:59:12')",
    )
    .bind(PROFILE)
    .execute(&db)
    .await
    .expect("seed a run the way CURRENT_TIMESTAMP would");

    let latest = stories_of(&db).await.remove(0).latest_run.expect("a run");

    assert_eq!(latest.started_at, "2026-05-22T20:58:30.000Z");
    assert_eq!(latest.finished_at.as_deref(), Some("2026-05-22T20:59:12.000Z"));
}

/// A value already stored in the ISO shape must come back unchanged in
/// meaning, not double-converted.
#[tokio::test]
async fn an_already_iso_timestamp_survives_the_conversion() {
    let db = pool().await;
    seed_story(&db, "s1", "ISO timestamps", "in_progress").await;
    seed_run_at(&db, "r1", "s1", "2026-05-22T20:58:30.000Z", "done").await;

    let latest = stories_of(&db).await.remove(0).latest_run.expect("a run");

    assert_eq!(latest.started_at, "2026-05-22T20:58:30.000Z");
}

/// A run still going has no finish time, and the conversion must leave that
/// absence alone rather than inventing an epoch.
#[tokio::test]
async fn a_running_run_still_has_no_finish_time() {
    let db = pool().await;
    seed_story(&db, "s1", "Running", "in_progress").await;
    seed_run_at(&db, "r1", "s1", "2026-05-22T20:58:30.000Z", "running").await;

    let latest = stories_of(&db).await.remove(0).latest_run.expect("a run");

    assert_eq!(latest.finished_at, None);
}


// ---------------------------------------------------------------------------
// Queue order
// ---------------------------------------------------------------------------

async fn seed_ready(db: &DbPool, id: &str, priority: &str, sort_order: i64, created_at: &str) {
    sqlx::query(
        "INSERT INTO stories (id, title, status, priority, sort_order, created_at)
         VALUES (?, ?, 'ready', ?, ?, ?)",
    )
    .bind(id)
    .bind(format!("Story {id}"))
    .bind(priority)
    .bind(sort_order)
    .bind(created_at)
    .execute(db)
    .await
    .expect("seed a ready story");
}

async fn ordered_ids(db: &DbPool) -> Vec<String> {
    stories_of(db).await.into_iter().map(|s| s.id).collect()
}

/// `priority` is a text column, so ordering by it directly sorts lexically:
/// `critical` after `low`, `high` after `critical`. The rank has to be
/// explicit, and this is the test that would catch losing it.
#[tokio::test]
async fn priority_ranks_by_urgency_not_alphabetically() {
    let db = pool().await;
    let at = "2026-04-13T00:00:00.000Z";
    seed_ready(&db, "low", "low", 0, at).await;
    seed_ready(&db, "critical", "critical", 1, at).await;
    seed_ready(&db, "medium", "medium", 2, at).await;
    seed_ready(&db, "high", "high", 3, at).await;

    assert_eq!(
        ordered_ids(&db).await,
        vec!["critical", "high", "medium", "low"],
        "alphabetical order would put critical after low"
    );
}

/// Within one priority band, the position a user dragged a card to decides —
/// which is the gesture the board is built around.
#[tokio::test]
async fn manual_position_decides_within_a_priority() {
    let db = pool().await;
    let at = "2026-04-13T00:00:00.000Z";
    seed_ready(&db, "third", "high", 2, at).await;
    seed_ready(&db, "first", "high", 0, at).await;
    seed_ready(&db, "second", "high", 1, at).await;

    assert_eq!(ordered_ids(&db).await, vec!["first", "second", "third"]);
}

/// Marking something critical should not also require dragging it.
#[tokio::test]
async fn priority_outranks_the_position_a_card_was_dragged_to() {
    let db = pool().await;
    let at = "2026-04-13T00:00:00.000Z";
    seed_ready(&db, "dragged-to-top", "low", 0, at).await;
    seed_ready(&db, "urgent-at-bottom", "critical", 99, at).await;

    assert_eq!(
        ordered_ids(&db).await,
        vec!["urgent-at-bottom", "dragged-to-top"]
    );
}

/// Age is the last word, so two cards never sit in an order that depends on
/// which row the database happened to return first.
#[tokio::test]
async fn age_breaks_a_tie_on_priority_and_position() {
    let db = pool().await;
    seed_ready(&db, "newer", "medium", 0, "2026-04-13T02:00:00.000Z").await;
    seed_ready(&db, "older", "medium", 0, "2026-04-13T01:00:00.000Z").await;

    assert_eq!(ordered_ids(&db).await, vec!["older", "newer"]);
}

/// A typo in the priority column should not jump the queue.
#[tokio::test]
async fn an_unrecognised_priority_sorts_last_rather_than_first() {
    let db = pool().await;
    let at = "2026-04-13T00:00:00.000Z";
    seed_ready(&db, "nonsense", "urgent-ish", 0, at).await;
    seed_ready(&db, "known", "low", 1, at).await;

    assert_eq!(ordered_ids(&db).await, vec!["known", "nonsense"]);
}

// ---------------------------------------------------------------------------
// Assignment
// ---------------------------------------------------------------------------
//
// Assignment gates both actions that start work, and the board now writes it
// from three places rather than only from the edit form. These pin what those
// writes mean — chiefly the third state of `assigned_agent_id`, which is easy
// to send by accident and impossible to see going wrong.

/// A blank update, so a test can name only the field it is exercising.
fn no_change() -> UpdateStoryInput {
    UpdateStoryInput {
        title: None,
        description: None,
        story_type: None,
        status: None,
        priority: None,
        assigned_agent_id: None,
        requires_approval: None,
        track_history: None,
        labels: None,
    }
}

fn assign_to(agent: &str) -> UpdateStoryInput {
    UpdateStoryInput {
        assigned_agent_id: Some(agent.to_string()),
        ..no_change()
    }
}

async fn run_profile(db: &DbPool, run_id: &str) -> String {
    sqlx::query("SELECT agent_profile_id FROM story_runs WHERE id = ?")
        .bind(run_id)
        .fetch_one(db)
        .await
        .expect("fetch the run")
        .get::<String, _>("agent_profile_id")
}

#[tokio::test]
async fn assigning_sets_the_agent_and_reports_its_name() {
    let db = pool().await;
    seed_story(&db, "s1", "Migrate the database", "ready").await;

    let updated = update_story("s1".into(), assign_to(PROFILE), &db, None)
        .await
        .expect("assign");

    assert_eq!(updated.assigned_agent_id.as_deref(), Some(PROFILE));
    // The name comes from a JOIN, so the board can render the change without
    // refetching the profile list.
    assert_eq!(updated.assigned_agent_name.as_deref(), Some("An agent"));
}

#[tokio::test]
async fn an_empty_agent_id_clears_the_assignment() {
    let db = pool().await;
    seed_story(&db, "s1", "Migrate the database", "ready").await;
    update_story("s1".into(), assign_to(PROFILE), &db, None)
        .await
        .expect("assign");

    let cleared = update_story(
        "s1".into(),
        UpdateStoryInput {
            assigned_agent_id: Some(String::new()),
            ..no_change()
        },
        &db,
        None,
    )
    .await
    .expect("unassign");

    assert_eq!(cleared.assigned_agent_id, None);
    assert_eq!(cleared.assigned_agent_name, None);
}

#[tokio::test]
async fn an_absent_agent_id_keeps_the_assignment() {
    // The distinction the frontend has to get right: omitting the field is not
    // how you unassign, it is how you edit a title without touching the agent.
    let db = pool().await;
    seed_story(&db, "s1", "Migrate the database", "ready").await;
    update_story("s1".into(), assign_to(PROFILE), &db, None)
        .await
        .expect("assign");

    let renamed = update_story(
        "s1".into(),
        UpdateStoryInput {
            title: Some("Migrate the database, carefully".into()),
            ..no_change()
        },
        &db,
        None,
    )
    .await
    .expect("rename");

    assert_eq!(renamed.title, "Migrate the database, carefully");
    assert_eq!(renamed.assigned_agent_id.as_deref(), Some(PROFILE));
}

#[tokio::test]
async fn reassigning_during_a_run_leaves_that_run_on_the_agent_it_started_with() {
    // The panel tells the user a change applies to the next run. This is why
    // that is true rather than a hope: a run records its profile on its own row
    // when it starts, and nothing about the story reaches back into it.
    let db = pool().await;
    seed_profile(&db, "agent-2", "Another agent").await;
    seed_story(&db, "s1", "Migrate the database", "in_progress").await;
    update_story("s1".into(), assign_to(PROFILE), &db, None)
        .await
        .expect("assign");
    seed_run(&db, "run-1", "s1", PROFILE).await;

    let reassigned = update_story("s1".into(), assign_to("agent-2"), &db, None)
        .await
        .expect("reassign mid-run");

    assert_eq!(reassigned.assigned_agent_id.as_deref(), Some("agent-2"));
    assert_eq!(
        run_profile(&db, "run-1").await,
        PROFILE,
        "the live run keeps the agent it started with",
    );
    assert_eq!(run_status(&db, "run-1").await, "running", "and keeps running");
}

#[tokio::test]
async fn unassigning_during_a_run_does_not_strip_the_run_of_its_agent() {
    let db = pool().await;
    seed_story(&db, "s1", "Migrate the database", "in_progress").await;
    update_story("s1".into(), assign_to(PROFILE), &db, None)
        .await
        .expect("assign");
    seed_run(&db, "run-1", "s1", PROFILE).await;

    update_story(
        "s1".into(),
        UpdateStoryInput {
            assigned_agent_id: Some(String::new()),
            ..no_change()
        },
        &db,
        None,
    )
    .await
    .expect("unassign mid-run");

    assert_eq!(run_profile(&db, "run-1").await, PROFILE);
}
