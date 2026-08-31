//! What a continuous-mode agent picks up next.
//!
//! This is the query the board's Ready column is *about*. Before this it
//! ordered by `created_at` alone, so dragging a card to the top — the board's
//! central gesture, persisted through `batch_update_story_order` — changed
//! nothing about what an agent took, and neither did marking a story
//! `critical`. The board presented a queue it did not control.
//!
//! The ordering is shared with `commands::stories`, so these tests and the
//! board's own ordering tests are asserting the same rule from both ends.

use db::testing::{make_test_pool, seed_profile};
use db::DbPool;

use crate::next_ready_story_sql;

const PROFILE: &str = "agent-1";

async fn pool() -> DbPool {
    let db = make_test_pool().await;
    seed_profile(&db, PROFILE, "An agent").await;
    db
}

async fn seed_story(
    db: &DbPool,
    id: &str,
    status: &str,
    priority: &str,
    sort_order: i64,
    created_at: &str,
    agent: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO stories
             (id, title, status, priority, sort_order, created_at, assigned_agent_id)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(format!("Story {id}"))
    .bind(status)
    .bind(priority)
    .bind(sort_order)
    .bind(created_at)
    .bind(agent)
    .execute(db)
    .await
    .expect("seed a story");
}

/// Run the real pick query and report which story it would take.
async fn next_pick(db: &DbPool) -> Option<String> {
    let row: Option<(String, String)> = sqlx::query_as(&next_ready_story_sql())
        .bind(PROFILE)
        .fetch_optional(db)
        .await
        .expect("run the pick query");
    row.map(|(id, _title)| id)
}

const AT: &str = "2026-04-13T00:00:00.000Z";

#[tokio::test]
async fn dragging_a_card_to_the_top_of_ready_decides_what_is_picked() {
    let db = pool().await;
    seed_story(&db, "was-first", "ready", "medium", 1, AT, Some(PROFILE)).await;
    seed_story(&db, "dragged-up", "ready", "medium", 0, AT, Some(PROFILE)).await;

    assert_eq!(next_pick(&db).await.as_deref(), Some("dragged-up"));
}

/// The other half of the same promise: marking a story critical moves it to
/// the front without also dragging it.
#[tokio::test]
async fn priority_outranks_the_position_a_card_sits_in() {
    let db = pool().await;
    seed_story(&db, "top-of-column", "ready", "low", 0, AT, Some(PROFILE)).await;
    seed_story(&db, "urgent", "ready", "critical", 99, AT, Some(PROFILE)).await;

    assert_eq!(next_pick(&db).await.as_deref(), Some("urgent"));
}

/// A lexical sort would answer `critical` here too — by accident, since
/// `critical` sorts before `low` alphabetically. `high` is the case that
/// separates a real ranking from a lucky one.
#[tokio::test]
async fn high_beats_low_even_though_it_sorts_after_it_alphabetically() {
    let db = pool().await;
    seed_story(&db, "low", "ready", "low", 0, AT, Some(PROFILE)).await;
    seed_story(&db, "high", "ready", "high", 1, AT, Some(PROFILE)).await;

    assert_eq!(next_pick(&db).await.as_deref(), Some("high"));
}

#[tokio::test]
async fn age_still_decides_when_priority_and_position_tie() {
    let db = pool().await;
    seed_story(&db, "newer", "ready", "medium", 0, "2026-04-13T02:00:00.000Z", Some(PROFILE)).await;
    seed_story(&db, "older", "ready", "medium", 0, "2026-04-13T01:00:00.000Z", Some(PROFILE)).await;

    assert_eq!(next_pick(&db).await.as_deref(), Some("older"));
}

#[tokio::test]
async fn only_ready_stories_are_picked() {
    let db = pool().await;
    seed_story(&db, "in-progress", "in_progress", "critical", 0, AT, Some(PROFILE)).await;
    seed_story(&db, "backlog", "backlog", "critical", 0, AT, Some(PROFILE)).await;
    seed_story(&db, "ready", "ready", "low", 9, AT, Some(PROFILE)).await;

    assert_eq!(next_pick(&db).await.as_deref(), Some("ready"));
}

/// The pick is per profile: an agent must not take work assigned to another,
/// or to nobody.
#[tokio::test]
async fn another_agents_work_is_not_picked_up() {
    let db = pool().await;
    seed_story(&db, "someone-else", "ready", "critical", 0, AT, Some("agent-2")).await;
    seed_story(&db, "unassigned", "ready", "critical", 0, AT, None).await;
    seed_story(&db, "mine", "ready", "low", 9, AT, Some(PROFILE)).await;

    assert_eq!(next_pick(&db).await.as_deref(), Some("mine"));
}

#[tokio::test]
async fn an_empty_queue_picks_nothing() {
    let db = pool().await;

    assert_eq!(next_pick(&db).await, None);
}
