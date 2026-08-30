//! Tests for what a finished pipeline does to its story card.
//!
//! `run_pipeline`'s completion runs inside a task spawned with a concrete
//! `tauri::AppHandle`, which cannot be built here — so `settle_pipeline_story`
//! is split out of that task deliberately, leaving the part with behaviour
//! worth pinning reachable without an app. What stays untested is the wiring:
//! that the spawned task calls it, with the pipeline's own story id, once.

use db::testing::make_test_pool;
use db::DbPool;

use crate::settle_pipeline_story;

const PIPELINE_STORY: &str = "pipeline-story";
const STEP_STORY: &str = "step-story";

async fn seed_story(db: &DbPool, id: &str, status: &str) {
    sqlx::query("INSERT INTO stories (id, title, status) VALUES (?, ?, ?)")
        .bind(id)
        .bind("A story")
        .bind(status)
        .execute(db)
        .await
        .expect("seed story");
}

async fn status_of(db: &DbPool, id: &str) -> String {
    sqlx::query_scalar("SELECT status FROM stories WHERE id = ?")
        .bind(id)
        .fetch_one(db)
        .await
        .expect("read status")
}

#[tokio::test]
async fn a_completed_pipeline_sends_its_card_to_review() {
    let db = make_test_pool().await;
    seed_story(&db, PIPELINE_STORY, "in_progress").await;

    settle_pipeline_story(&db, PIPELINE_STORY, "done").await;

    assert_eq!(status_of(&db, PIPELINE_STORY).await, "review");
}

#[tokio::test]
async fn a_failed_pipeline_blocks_its_card_rather_than_readying_it() {
    let db = make_test_pool().await;
    seed_story(&db, PIPELINE_STORY, "in_progress").await;

    settle_pipeline_story(&db, PIPELINE_STORY, "failed").await;

    let status = status_of(&db, PIPELINE_STORY).await;
    assert_eq!(status, "blocked");
    assert_ne!(status, "ready", "a continuous profile would re-pick this");
}

/// The card that moves is the pipeline's own. A step's story is settled by the
/// step's own run, so a multi-step pipeline must not reach across and move one
/// on its behalf.
#[tokio::test]
async fn a_finishing_pipeline_touches_only_its_own_card() {
    let db = make_test_pool().await;
    seed_story(&db, PIPELINE_STORY, "in_progress").await;
    seed_story(&db, STEP_STORY, "in_progress").await;

    settle_pipeline_story(&db, PIPELINE_STORY, "done").await;

    assert_eq!(status_of(&db, PIPELINE_STORY).await, "review");
    assert_eq!(
        status_of(&db, STEP_STORY).await,
        "in_progress",
        "a step's card is the step's to settle"
    );
}

/// Steps finish one after another, and the pipeline's own completion runs once
/// at the end. Should the parent path ever be reached more than once, the card
/// must still move a single time rather than being re-stamped.
#[tokio::test]
async fn settling_the_same_pipeline_twice_moves_the_card_once() {
    let db = make_test_pool().await;
    seed_story(&db, PIPELINE_STORY, "in_progress").await;

    settle_pipeline_story(&db, PIPELINE_STORY, "done").await;
    // A second, contradictory outcome must not overwrite the first: after the
    // first move the card is no longer `in_progress`, so the guard holds.
    settle_pipeline_story(&db, PIPELINE_STORY, "failed").await;

    assert_eq!(status_of(&db, PIPELINE_STORY).await, "review");
}

#[tokio::test]
async fn a_card_someone_moved_during_the_pipeline_is_left_alone() {
    let db = make_test_pool().await;
    seed_story(&db, PIPELINE_STORY, "blocked").await;

    settle_pipeline_story(&db, PIPELINE_STORY, "done").await;

    assert_eq!(status_of(&db, PIPELINE_STORY).await, "blocked");
}

#[tokio::test]
async fn the_workspace_setting_switches_the_pipeline_side_off_too() {
    let db = make_test_pool().await;
    seed_story(&db, PIPELINE_STORY, "in_progress").await;
    sqlx::query("INSERT INTO workspaces (id, name, path) VALUES ('w1', 'W', '/tmp/w')")
        .execute(&db)
        .await
        .expect("seed workspace");
    sqlx::query(
        "INSERT INTO workspace_settings (workspace_id, settings_json) \
         VALUES ('w1', '{\"auto_advance_story_status\": false}')",
    )
    .execute(&db)
    .await
    .expect("seed workspace settings");

    settle_pipeline_story(&db, PIPELINE_STORY, "done").await;

    assert_eq!(status_of(&db, PIPELINE_STORY).await, "in_progress");
}
