//! Tests for [`crate::human`] — the two reads behind the board's
//! "something is waiting on you" markers.
//!
//! Both reads already knew which run was blocked. Neither passed on which
//! *story* that run belongs to, so the UI could only render a page-level count
//! and never point at a card. These tests pin the joins that carry the story
//! id through, including the cases where there is nothing to point at.

use db::testing::{make_test_pool, seed_profile, seed_run, seed_story};
use db::DbPool;

use crate::human::{get_pending_approvals, get_pending_human_requests};

const PROFILE: &str = "agent-1";

async fn pool() -> DbPool {
    let db = make_test_pool().await;
    seed_profile(&db, PROFILE, "An agent").await;
    db
}

/// Insert the synthetic story the `request_human_input` tool creates.
async fn seed_human_story(db: &DbPool, id: &str, parent_run_id: Option<&str>) {
    sqlx::query(
        "INSERT INTO stories (id, title, story_type, status, parent_run_id, human_question)
         VALUES (?, 'Which database?', 'human', 'ready', ?, 'Postgres or SQLite?')",
    )
    .bind(id)
    .bind(parent_run_id)
    .execute(db)
    .await
    .expect("seed a human story");
}

async fn seed_approval(db: &DbPool, id: &str, run_id: &str, status: &str) {
    sqlx::query(
        "INSERT INTO approval_requests (id, run_id, tool_name, tool_input, status)
         VALUES (?, ?, 'file_write', '{\"path\":\"a.txt\"}', ?)",
    )
    .bind(id)
    .bind(run_id)
    .bind(status)
    .execute(db)
    .await
    .expect("seed an approval request");
}

// ---------------------------------------------------------------------------
// Human input requests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_human_request_names_the_task_story_its_run_belongs_to() {
    let db = pool().await;
    seed_story(&db, "task-1", "Migrate the database", "in_progress").await;
    seed_run(&db, "run-1", "task-1", PROFILE).await;
    seed_human_story(&db, "human-1", Some("run-1")).await;

    let requests = get_pending_human_requests(&db).await.expect("read");

    assert_eq!(requests.len(), 1);
    // The request is still identified by the human story…
    assert_eq!(requests[0].story_id, "human-1");
    // …but the card a user would look at is the task behind it.
    assert_eq!(requests[0].task_story_id.as_deref(), Some("task-1"));
}

#[tokio::test]
async fn a_human_request_with_no_run_behind_it_names_no_task_story() {
    let db = pool().await;
    seed_human_story(&db, "human-1", None).await;

    let requests = get_pending_human_requests(&db).await.expect("read");

    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].task_story_id, None,
        "nothing to mark: this question was not raised by a run",
    );
}

#[tokio::test]
async fn a_human_request_whose_task_story_was_deleted_names_no_task_story() {
    let db = pool().await;
    seed_story(&db, "task-1", "Migrate the database", "in_progress").await;
    seed_run(&db, "run-1", "task-1", PROFILE).await;
    seed_human_story(&db, "human-1", Some("run-1")).await;

    sqlx::query("DELETE FROM stories WHERE id = 'task-1'")
        .execute(&db)
        .await
        .expect("delete the task story");

    let requests = get_pending_human_requests(&db).await.expect("read");

    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].task_story_id, None,
        "the id must be absent rather than dangling — there is no card to mark",
    );
}

#[tokio::test]
async fn answered_human_stories_are_not_pending() {
    let db = pool().await;
    seed_human_story(&db, "human-1", None).await;
    seed_human_story(&db, "human-2", None).await;
    sqlx::query("UPDATE stories SET status = 'done' WHERE id = 'human-2'")
        .execute(&db)
        .await
        .expect("answer the second one");

    let requests = get_pending_human_requests(&db).await.expect("read");

    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].story_id, "human-1");
}

// ---------------------------------------------------------------------------
// Approval requests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_pending_approval_names_the_story_whose_run_wants_the_tool_call() {
    let db = pool().await;
    seed_story(&db, "task-1", "Migrate the database", "in_progress").await;
    seed_run(&db, "run-1", "task-1", PROFILE).await;
    seed_approval(&db, "ap-1", "run-1", "pending").await;

    let approvals = get_pending_approvals(&db).await.expect("read");

    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].story_id.as_deref(), Some("task-1"));
    assert_eq!(
        approvals[0].story_title.as_deref(),
        Some("Migrate the database"),
    );
}

#[tokio::test]
async fn an_approval_whose_story_was_deleted_names_no_story() {
    let db = pool().await;
    seed_story(&db, "task-1", "Migrate the database", "in_progress").await;
    seed_run(&db, "run-1", "task-1", PROFILE).await;
    seed_approval(&db, "ap-1", "run-1", "pending").await;

    sqlx::query("DELETE FROM stories WHERE id = 'task-1'")
        .execute(&db)
        .await
        .expect("delete the task story");

    let approvals = get_pending_approvals(&db).await.expect("read");

    assert_eq!(approvals.len(), 1, "the approval is still pending");
    assert_eq!(approvals[0].story_id, None);
    assert_eq!(approvals[0].story_title, None);
}

#[tokio::test]
async fn decided_approvals_are_not_pending() {
    let db = pool().await;
    seed_story(&db, "task-1", "Migrate the database", "in_progress").await;
    seed_run(&db, "run-1", "task-1", PROFILE).await;
    seed_approval(&db, "ap-1", "run-1", "pending").await;
    seed_approval(&db, "ap-2", "run-1", "approved").await;
    seed_approval(&db, "ap-3", "run-1", "rejected").await;

    let approvals = get_pending_approvals(&db).await.expect("read");

    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].id, "ap-1");
}

#[tokio::test]
async fn two_approvals_on_one_story_both_name_it() {
    let db = pool().await;
    seed_story(&db, "task-1", "Migrate the database", "in_progress").await;
    seed_run(&db, "run-1", "task-1", PROFILE).await;
    seed_approval(&db, "ap-1", "run-1", "pending").await;
    seed_approval(&db, "ap-2", "run-1", "pending").await;

    let approvals = get_pending_approvals(&db).await.expect("read");

    assert_eq!(approvals.len(), 2);
    for approval in &approvals {
        assert_eq!(approval.story_id.as_deref(), Some("task-1"));
    }
}
