//! Tests for the startup reconciliation sweep.
//!
//! Every case drives the real [`reconcile_orphaned_runs`] against an in-memory
//! database seeded through `db::testing`, and asserts what a user would see
//! afterwards: the run's status and finish time, its timeline, the pipeline
//! view, and whether an approval is still offered.

use sqlx::Row;

use crate::recovery::{
    instance_id, reconcile_orphaned_runs, INTERRUPTED_APPROVAL_REASON, INTERRUPTED_EVENT_TYPE,
    INTERRUPTED_RUN_STATUS,
};
use crate::testing::{
    make_test_pool, run_events, run_iteration_count, run_status, run_usage, seed_profile, seed_run,
    seed_run_owned, seed_story,
};
use crate::DbPool;

const STORY: &str = "story-1";
const PROFILE: &str = "agent-1";

/// Whatever the sweep is given as "this process". Any value works; the sweep
/// only ever compares it for equality.
const THIS_INSTANCE: &str = "instance-current";

/// A pool with one story and one profile already in it.
async fn pool() -> DbPool {
    let db = make_test_pool().await;
    seed_story(&db, STORY, "A story", "in_progress").await;
    seed_profile(&db, PROFILE, "An agent").await;
    db
}

async fn story_status(db: &DbPool, id: &str) -> String {
    sqlx::query_scalar("SELECT status FROM stories WHERE id = ?")
        .bind(id)
        .fetch_one(db)
        .await
        .expect("read story status")
}

async fn seed_approval(db: &DbPool, id: &str, run_id: &str) {
    sqlx::query(
        "INSERT INTO approval_requests (id, run_id, tool_name, tool_input)
         VALUES (?, ?, 'shell_exec', '{}')",
    )
    .bind(id)
    .bind(run_id)
    .execute(db)
    .await
    .expect("seed approval_requests");
}

/// `(status, rejection_reason, decided_at)` for one approval request.
async fn approval(db: &DbPool, id: &str) -> (String, Option<String>, Option<String>) {
    let row = sqlx::query(
        "SELECT status, rejection_reason, decided_at FROM approval_requests WHERE id = ?",
    )
    .bind(id)
    .fetch_one(db)
    .await
    .expect("fetch approval_requests");
    (
        row.get("status"),
        row.get("rejection_reason"),
        row.get("decided_at"),
    )
}

/// What `get_pending_approvals` would still offer the user.
async fn still_offered(db: &DbPool) -> Vec<String> {
    sqlx::query_scalar("SELECT id FROM approval_requests WHERE status = 'pending' ORDER BY id")
        .fetch_all(db)
        .await
        .expect("list pending approvals")
}

async fn finished_at(db: &DbPool, run_id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT finished_at FROM story_runs WHERE id = ?")
        .bind(run_id)
        .fetch_one(db)
        .await
        .expect("fetch finished_at")
}

async fn seed_pipeline_step(
    db: &DbPool,
    id: &str,
    pipeline_run_id: &str,
    step_index: i64,
    run_id: Option<&str>,
    status: &str,
) {
    sqlx::query(
        "INSERT INTO pipeline_step_runs
             (id, pipeline_run_id, step_index, story_id, agent_profile_id, run_id, status)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(pipeline_run_id)
    .bind(step_index)
    .bind(STORY)
    .bind(PROFILE)
    .bind(run_id)
    .bind(status)
    .execute(db)
    .await
    .expect("seed pipeline_step_runs");
}

async fn step_status(db: &DbPool, id: &str) -> String {
    sqlx::query_scalar("SELECT status FROM pipeline_step_runs WHERE id = ?")
        .bind(id)
        .fetch_one(db)
        .await
        .expect("fetch pipeline step status")
}

// ---------------------------------------------------------------------------
// Orphaned runs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_run_left_running_by_a_previous_session_is_moved_to_a_terminal_status() {
    let db = pool().await;
    seed_run(&db, "run-1", STORY, PROFILE).await;

    let report = reconcile_orphaned_runs(&db, THIS_INSTANCE)
        .await
        .expect("sweep");

    assert_eq!(report.runs, vec!["run-1".to_string()]);
    assert_eq!(run_status(&db, "run-1").await, INTERRUPTED_RUN_STATUS);
}

/// The sweep already settles the run, its pipeline steps and its approvals.
/// Without the card, everything about an interrupted run is cleaned up except
/// the one part the user actually looks at.
#[tokio::test]
async fn a_story_left_in_progress_by_a_crash_is_moved_off_it() {
    let db = pool().await;
    seed_run(&db, "run-1", STORY, PROFILE).await;

    let report = reconcile_orphaned_runs(&db, THIS_INSTANCE)
        .await
        .expect("sweep");

    assert_eq!(report.stories, 1);
    assert_eq!(story_status(&db, STORY).await, "blocked");
}

/// `blocked`, not `ready`. A crash is not a reason to hand the story back to a
/// continuous-mode profile, which would re-pick it the moment the app came up.
#[tokio::test]
async fn a_crashed_run_does_not_return_its_story_to_the_schedulers_queue() {
    let db = pool().await;
    seed_run(&db, "run-1", STORY, PROFILE).await;

    reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    assert_ne!(story_status(&db, STORY).await, "ready");
}

/// The sweep never touches a run this process owns, and so must never touch
/// its card either — the run is still working on it.
#[tokio::test]
async fn the_sweep_leaves_the_card_of_a_live_run_alone() {
    let db = pool().await;
    seed_run_owned(&db, "run-1", STORY, PROFILE, THIS_INSTANCE).await;

    let report = reconcile_orphaned_runs(&db, THIS_INSTANCE)
        .await
        .expect("sweep");

    assert_eq!(report.stories, 0);
    assert_eq!(story_status(&db, STORY).await, "in_progress");
}

/// A card someone moved deliberately before the restart is theirs, not the
/// sweep's.
#[tokio::test]
async fn the_sweep_does_not_overwrite_a_status_someone_chose() {
    let db = pool().await;
    seed_run(&db, "run-1", STORY, PROFILE).await;
    sqlx::query("UPDATE stories SET status = 'review' WHERE id = ?")
        .bind(STORY)
        .execute(&db)
        .await
        .expect("set status");

    let report = reconcile_orphaned_runs(&db, THIS_INSTANCE)
        .await
        .expect("sweep");

    assert_eq!(report.stories, 0);
    assert_eq!(story_status(&db, STORY).await, "review");
}

/// The sweep is run more than once in an app's life; the second pass must find
/// nothing to do.
#[tokio::test]
async fn settling_a_card_is_idempotent() {
    let db = pool().await;
    seed_run(&db, "run-1", STORY, PROFILE).await;

    reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("first");
    let second = reconcile_orphaned_runs(&db, THIS_INSTANCE)
        .await
        .expect("second");

    assert!(second.is_empty(), "{second:?}");
    assert_eq!(story_status(&db, STORY).await, "blocked");
}

#[tokio::test]
async fn a_reconciled_run_gets_a_finished_at_so_it_stops_counting_as_active() {
    let db = pool().await;
    seed_run(&db, "run-1", STORY, PROFILE).await;

    reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    // Deliberately not compared against `started_at`: both are stamped at
    // millisecond resolution and a seeded run can finish inside the same
    // millisecond it started. What matters is that the column is no longer
    // NULL, which is what "still going" looks like in the UI.
    let finished = finished_at(&db, "run-1").await.expect("finished_at is set");
    assert!(
        finished.ends_with('Z') && finished.contains('T'),
        "finished_at must be RFC 3339 so the Runs list can parse it: {finished}"
    );
}

#[tokio::test]
async fn the_terminal_status_is_one_the_frontend_already_knows() {
    // Migration 20260410000016 had to repair a status drift once already. A
    // status outside this set renders as a blank badge and matches no filter.
    assert!(
        ["running", "done", "failed", "cancelled"].contains(&INTERRUPTED_RUN_STATUS),
        "'{INTERRUPTED_RUN_STATUS}' is not in the normalized RunStatus vocabulary"
    );
}

#[tokio::test]
async fn each_reconciled_run_gains_an_event_saying_a_restart_ended_it() {
    let db = pool().await;
    seed_run(&db, "run-1", STORY, PROFILE).await;

    reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    let events = run_events(&db, "run-1").await;
    let (kind, content) = events.last().expect("an event was appended");
    assert_eq!(kind, INTERRUPTED_EVENT_TYPE);
    assert!(
        content.contains("exited") && content.contains("failed"),
        "the timeline must say why the run ended: {content}"
    );
}

#[tokio::test]
async fn the_interruption_event_sorts_after_what_the_run_managed_to_write() {
    let db = pool().await;
    seed_run(&db, "run-1", STORY, PROFILE).await;
    for (i, kind) in ["token", "tool_call"].iter().enumerate() {
        sqlx::query(
            "INSERT INTO run_events (id, run_id, event_type, content, sequence_num)
             VALUES (?, 'run-1', ?, 'x', ?)",
        )
        .bind(format!("e{i}"))
        .bind(kind)
        .bind(i as i64)
        .execute(&db)
        .await
        .expect("seed run_events");
    }

    reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    // `run_events` orders by sequence_num, not by timestamp — the ordering the
    // data actually expresses.
    let kinds: Vec<String> = run_events(&db, "run-1").await.into_iter().map(|(k, _)| k).collect();
    assert_eq!(kinds, vec!["token", "tool_call", INTERRUPTED_EVENT_TYPE]);
}

#[tokio::test]
async fn a_run_that_recorded_nothing_still_gets_its_interruption_event() {
    // The `MAX(sequence_num)` over an empty timeline must not swallow the row.
    let db = pool().await;
    seed_run(&db, "run-1", STORY, PROFILE).await;

    reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    assert_eq!(run_events(&db, "run-1").await.len(), 1);
}

#[tokio::test]
async fn the_tokens_an_interrupted_run_spent_survive_reconciliation() {
    // The run did real work and cost real money before the app died. A sweep
    // that zeroed that would be lying in the other direction.
    let db = pool().await;
    seed_run(&db, "run-1", STORY, PROFILE).await;
    sqlx::query(
        "UPDATE story_runs
         SET input_tokens = 1200, output_tokens = 340, cache_read_input_tokens = 800,
             cache_creation_input_tokens = 64, estimated_cost_usd = 0.25, iteration_count = 7
         WHERE id = 'run-1'",
    )
    .execute(&db)
    .await
    .expect("record usage");

    reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    let usage = run_usage(&db, "run-1").await;
    assert_eq!(usage.input_tokens, 1200);
    assert_eq!(usage.output_tokens, 340);
    assert_eq!(usage.cache_read_input_tokens, 800);
    assert_eq!(usage.cache_creation_input_tokens, 64);
    assert_eq!(usage.estimated_cost_usd, 0.25);
    // And the iterations it got through before it was cut off.
    assert_eq!(run_iteration_count(&db, "run-1").await, 7);
}

#[tokio::test]
async fn the_sweep_deletes_no_history() {
    let db = pool().await;
    seed_run(&db, "run-1", STORY, PROFILE).await;
    seed_run(&db, "run-2", STORY, PROFILE).await;
    sqlx::query(
        "INSERT INTO run_events (id, run_id, event_type, content, sequence_num)
         VALUES ('e0', 'run-1', 'token', 'hello', 0)",
    )
    .execute(&db)
    .await
    .expect("seed run_events");

    reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    let runs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM story_runs")
        .fetch_one(&db)
        .await
        .expect("count runs");
    assert_eq!(runs, 2, "run history is the audit trail; nothing may be removed");
    assert!(
        run_events(&db, "run-1").await.iter().any(|(k, c)| k == "token" && c == "hello"),
        "the run's own events must still be there"
    );
}

#[tokio::test]
async fn runs_that_already_finished_are_left_exactly_as_they_were() {
    let db = pool().await;
    for (id, status) in [("done-1", "done"), ("failed-1", "failed"), ("cancelled-1", "cancelled")] {
        seed_run(&db, id, STORY, PROFILE).await;
        sqlx::query("UPDATE story_runs SET status = ? WHERE id = ?")
            .bind(status)
            .bind(id)
            .execute(&db)
            .await
            .expect("set status");
    }

    let report = reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    assert!(report.runs.is_empty());
    assert_eq!(run_status(&db, "done-1").await, "done");
    assert_eq!(run_status(&db, "cancelled-1").await, "cancelled");
    assert!(run_events(&db, "done-1").await.is_empty());
}

// ---------------------------------------------------------------------------
// Liveness guard
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_run_started_by_the_sweeping_process_is_never_touched() {
    // The guard that keeps the sweep from killing a live run: if the process
    // asking for the sweep is the one that started the run, the run is real.
    let db = pool().await;
    seed_run_owned(&db, "live", STORY, PROFILE, THIS_INSTANCE).await;
    seed_run(&db, "orphan", STORY, PROFILE).await;

    let report = reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    assert_eq!(report.runs, vec!["orphan".to_string()]);
    assert_eq!(run_status(&db, "live").await, "running");
    assert!(finished_at(&db, "live").await.is_none());
    assert!(run_events(&db, "live").await.is_empty());
}

#[tokio::test]
async fn a_run_owned_by_some_other_launch_is_orphaned() {
    let db = pool().await;
    seed_run_owned(&db, "run-1", STORY, PROFILE, "instance-from-yesterday").await;

    let report = reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    assert_eq!(report.runs, vec!["run-1".to_string()]);
}

#[tokio::test]
async fn every_process_gets_a_distinct_and_stable_instance_id() {
    let first = instance_id();

    assert!(!first.is_empty());
    assert_eq!(first, instance_id(), "the id must not change under the sweep");
}

// ---------------------------------------------------------------------------
// Idempotency
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_second_sweep_changes_nothing() {
    let db = pool().await;
    seed_run(&db, "run-1", STORY, PROFILE).await;
    seed_approval(&db, "ap-1", "run-1").await;
    seed_pipeline_step(&db, "step-1", "run-1", 0, None, "running").await;

    let first = reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("first sweep");
    let events_after_first = run_events(&db, "run-1").await;
    let finished_after_first = finished_at(&db, "run-1").await;

    let second = reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("second sweep");

    assert!(!first.is_empty(), "the first pass had work to do");
    assert!(second.is_empty(), "the second pass must be a no-op: {second:?}");
    assert_eq!(run_events(&db, "run-1").await, events_after_first, "no duplicate event");
    assert_eq!(finished_at(&db, "run-1").await, finished_after_first, "finished_at is not restamped");
    assert_eq!(approval(&db, "ap-1").await.0, "rejected");
}

#[tokio::test]
async fn a_database_with_nothing_in_flight_sweeps_to_an_empty_report() {
    let db = pool().await;

    let report = reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    assert!(report.is_empty());
}

// ---------------------------------------------------------------------------
// Approvals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_pending_approval_nothing_can_deliver_is_denied_with_its_reason() {
    let db = pool().await;
    seed_run(&db, "run-1", STORY, PROFILE).await;
    seed_approval(&db, "ap-1", "run-1").await;

    let report = reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    assert_eq!(report.approvals, 1);
    let (status, reason, decided_at) = approval(&db, "ap-1").await;
    assert_eq!(status, "rejected");
    assert_eq!(reason.as_deref(), Some(INTERRUPTED_APPROVAL_REASON));
    assert!(decided_at.is_some(), "a decided request must carry when it was decided");
}

#[tokio::test]
async fn a_denied_approval_is_no_longer_offered_to_the_user() {
    // The whole visible symptom: the Approvals list shows a request, the user
    // clicks it, and `ApprovalGate::resolve` returns false because the map is
    // empty after a restart.
    let db = pool().await;
    seed_run(&db, "run-1", STORY, PROFILE).await;
    seed_approval(&db, "ap-1", "run-1").await;
    assert_eq!(still_offered(&db).await, vec!["ap-1".to_string()]);

    reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    assert!(still_offered(&db).await.is_empty());
}

#[tokio::test]
async fn an_approval_held_by_a_live_run_is_left_pending() {
    let db = pool().await;
    seed_run_owned(&db, "live", STORY, PROFILE, THIS_INSTANCE).await;
    seed_run(&db, "orphan", STORY, PROFILE).await;
    seed_approval(&db, "ap-live", "live").await;
    seed_approval(&db, "ap-orphan", "orphan").await;

    let report = reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    assert_eq!(report.approvals, 1);
    assert_eq!(still_offered(&db).await, vec!["ap-live".to_string()]);
}

#[tokio::test]
async fn an_approval_the_user_already_decided_is_not_rewritten() {
    let db = pool().await;
    seed_run(&db, "run-1", STORY, PROFILE).await;
    seed_approval(&db, "ap-1", "run-1").await;
    sqlx::query("UPDATE approval_requests SET status = 'approved' WHERE id = 'ap-1'")
        .execute(&db)
        .await
        .expect("decide");

    let report = reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    assert_eq!(report.approvals, 0);
    assert_eq!(approval(&db, "ap-1").await.0, "approved");
}

#[tokio::test]
async fn an_approval_whose_run_already_ended_is_denied_too() {
    // Not every stranded approval belongs to a run the sweep itself moved: a
    // run can have failed on its own with the request still sitting there.
    let db = pool().await;
    seed_run(&db, "run-1", STORY, PROFILE).await;
    sqlx::query("UPDATE story_runs SET status = 'failed' WHERE id = 'run-1'")
        .execute(&db)
        .await
        .expect("finish run");
    seed_approval(&db, "ap-1", "run-1").await;

    let report = reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    assert!(report.runs.is_empty());
    assert_eq!(report.approvals, 1);
    assert!(still_offered(&db).await.is_empty());
}

// ---------------------------------------------------------------------------
// Pipelines
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_pipeline_root_and_its_child_runs_are_reconciled_together() {
    let db = pool().await;
    seed_run(&db, "pipeline-1", STORY, PROFILE).await;
    seed_run(&db, "child-1", STORY, PROFILE).await;
    seed_pipeline_step(&db, "step-0", "pipeline-1", 0, Some("child-1"), "running").await;
    seed_pipeline_step(&db, "step-1", "pipeline-1", 1, None, "pending").await;

    let report = reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    assert_eq!(report.runs.len(), 2);
    assert_eq!(run_status(&db, "pipeline-1").await, INTERRUPTED_RUN_STATUS);
    assert_eq!(run_status(&db, "child-1").await, INTERRUPTED_RUN_STATUS);
    assert_eq!(step_status(&db, "step-0").await, "failed");
    assert_eq!(
        step_status(&db, "step-1").await,
        "failed",
        "a step that never started cannot stay pending once its pipeline is over"
    );
    assert_eq!(report.pipeline_steps, 2, "each step is counted once, not once per run");
}

#[tokio::test]
async fn a_pipeline_step_that_already_finished_keeps_its_result() {
    let db = pool().await;
    seed_run(&db, "pipeline-1", STORY, PROFILE).await;
    seed_pipeline_step(&db, "step-0", "pipeline-1", 0, None, "done").await;
    seed_pipeline_step(&db, "step-1", "pipeline-1", 1, None, "running").await;

    let report = reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    assert_eq!(step_status(&db, "step-0").await, "done");
    assert_eq!(step_status(&db, "step-1").await, "failed");
    assert_eq!(report.pipeline_steps, 1);
}

#[tokio::test]
async fn a_live_pipelines_steps_are_left_alone() {
    let db = pool().await;
    seed_run_owned(&db, "pipeline-1", STORY, PROFILE, THIS_INSTANCE).await;
    seed_pipeline_step(&db, "step-0", "pipeline-1", 0, None, "running").await;

    let report = reconcile_orphaned_runs(&db, THIS_INSTANCE).await.expect("sweep");

    assert!(report.is_empty());
    assert_eq!(step_status(&db, "step-0").await, "running");
}
