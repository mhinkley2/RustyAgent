//! Tests for the pipeline engine's concurrency ceiling.
//!
//! `run_parallel` used to `tokio::spawn` every step of a pipeline at once. With
//! each step now holding a full git checkout and an independent stream of
//! provider calls, unbounded fan-out is unbounded disk and unbounded spend, so
//! the ceiling is the thing worth pinning down.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use db::testing::{make_test_pool, seed_profile, seed_run, seed_story, seed_workspace};

use crate::{
    resolve_parallel_limit, run_bounded, step_tip, workspace_parallel_limit,
    DEFAULT_MAX_PARALLEL_STEPS,
};

// ---------------------------------------------------------------------------
// Resolving the limit
// ---------------------------------------------------------------------------

#[test]
fn an_unconfigured_pipeline_gets_the_default_ceiling() {
    assert_eq!(resolve_parallel_limit(None, None), DEFAULT_MAX_PARALLEL_STEPS);
}

#[test]
fn the_global_setting_applies_when_the_workspace_says_nothing() {
    assert_eq!(resolve_parallel_limit(None, Some(2)), 2);
}

#[test]
fn the_workspace_override_beats_the_global_setting() {
    assert_eq!(resolve_parallel_limit(Some(1), Some(8)), 1);
}

#[test]
fn a_configured_zero_still_runs_one_step_at_a_time() {
    // "Run nothing" is never what anyone means by a concurrency limit, and a
    // zero-permit semaphore would hang the pipeline forever.
    assert_eq!(resolve_parallel_limit(Some(0), None), 1);
    assert_eq!(resolve_parallel_limit(None, Some(0)), 1);
}

// ---------------------------------------------------------------------------
// Reading the workspace override
// ---------------------------------------------------------------------------

async fn set_workspace_settings(db: &db::DbPool, workspace_id: &str, json: &str) {
    sqlx::query("INSERT INTO workspace_settings (workspace_id, settings_json) VALUES (?, ?)")
        .bind(workspace_id)
        .bind(json)
        .execute(db)
        .await
        .expect("seed workspace_settings");
}

#[tokio::test]
async fn the_limit_is_read_from_the_active_workspaces_settings() {
    let db = make_test_pool().await;
    seed_workspace(&db, "ws-1", "/tmp/ws-1").await;
    set_workspace_settings(&db, "ws-1", r#"{"max_parallel_steps": 3}"#).await;

    assert_eq!(workspace_parallel_limit(&db).await, Some(3));
}

#[tokio::test]
async fn a_workspace_without_the_key_falls_through_to_the_global_setting() {
    let db = make_test_pool().await;
    seed_workspace(&db, "ws-1", "/tmp/ws-1").await;
    set_workspace_settings(&db, "ws-1", r#"{"something_else": true}"#).await;

    assert_eq!(workspace_parallel_limit(&db).await, None);
}

#[tokio::test]
async fn malformed_workspace_settings_do_not_break_the_pipeline() {
    let db = make_test_pool().await;
    seed_workspace(&db, "ws-1", "/tmp/ws-1").await;
    set_workspace_settings(&db, "ws-1", "not json at all").await;

    assert_eq!(workspace_parallel_limit(&db).await, None);
}

#[tokio::test]
async fn a_non_numeric_limit_is_ignored_rather_than_guessed_at() {
    let db = make_test_pool().await;
    seed_workspace(&db, "ws-1", "/tmp/ws-1").await;
    set_workspace_settings(&db, "ws-1", r#"{"max_parallel_steps": "lots"}"#).await;

    assert_eq!(workspace_parallel_limit(&db).await, None);
}

#[tokio::test]
async fn with_no_workspace_at_all_there_is_no_override() {
    let db = make_test_pool().await;

    assert_eq!(workspace_parallel_limit(&db).await, None);
}

// ---------------------------------------------------------------------------
// Enforcing the ceiling
// ---------------------------------------------------------------------------

/// Tracks how many tasks were running at once.
#[derive(Default)]
struct Concurrency {
    in_flight: AtomicUsize,
    peak: AtomicUsize,
}

impl Concurrency {
    fn enter(&self) {
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
    }
    fn leave(&self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
    fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }
}

/// Run `count` no-op tasks through the real fan-out and report the peak
/// concurrency observed.
///
/// Each task yields several times while "working", so a ceiling that was not
/// enforced would show up as a peak above `limit`. The default `#[tokio::test]`
/// runtime is single-threaded, which makes the interleaving — and therefore the
/// peak — deterministic rather than a race.
async fn peak_concurrency(limit: usize, count: usize) -> (usize, usize) {
    let seen = Arc::new(Concurrency::default());
    let results = run_bounded(limit, count, |index| {
        let seen = seen.clone();
        async move {
            seen.enter();
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
            seen.leave();
            index
        }
    })
    .await;

    let completed = results
        .into_iter()
        .enumerate()
        .filter(|(index, r)| r.as_ref().is_ok_and(|got| got == index))
        .count();
    (seen.peak(), completed)
}

#[tokio::test]
async fn no_more_than_the_limit_run_at_once() {
    let (peak, completed) = peak_concurrency(2, 8).await;

    assert_eq!(peak, 2, "the ceiling was not enforced");
    assert_eq!(completed, 8, "every step must still run");
}

#[tokio::test]
async fn a_limit_of_one_serialises_the_steps_completely() {
    let (peak, completed) = peak_concurrency(1, 5).await;

    assert_eq!(peak, 1);
    assert_eq!(completed, 5);
}

#[tokio::test]
async fn a_limit_wider_than_the_pipeline_lets_every_step_run() {
    let (peak, completed) = peak_concurrency(10, 3).await;

    assert_eq!(peak, 3);
    assert_eq!(completed, 3);
}

#[tokio::test]
async fn results_come_back_in_step_order_not_completion_order() {
    // The caller maps results back onto step indices positionally, so a
    // reordering here would report the wrong step as failed.
    let results = run_bounded(4, 5, |index| async move {
        // Later steps finish first.
        for _ in 0..(5 - index) {
            tokio::task::yield_now().await;
        }
        index * 10
    })
    .await;

    let values: Vec<usize> = results.into_iter().map(|r| r.expect("no panic")).collect();
    assert_eq!(values, vec![0, 10, 20, 30, 40]);
}

#[tokio::test]
async fn one_panicking_step_does_not_take_the_others_with_it() {
    let results = run_bounded(2, 4, |index| async move {
        assert_ne!(index, 2, "step 2 panics on purpose");
        index
    })
    .await;

    assert!(results[0].is_ok());
    assert!(results[1].is_ok());
    assert!(results[2].is_err(), "the panicking step should surface as a JoinError");
    assert!(results[3].is_ok());
}

#[tokio::test]
async fn an_empty_pipeline_completes_without_work() {
    let results = run_bounded::<usize, _, _>(4, 0, |index| async move { index }).await;

    assert!(results.is_empty());
}

// ---------------------------------------------------------------------------
// Chaining sequential steps
// ---------------------------------------------------------------------------

async fn seeded_step(db: &db::DbPool, run_id: &str) {
    seed_profile(db, "agent-1", "Agent").await;
    seed_story(db, "story-1", "Story", "ready").await;
    seed_run(db, run_id, "story-1", "agent-1").await;
}

#[tokio::test]
async fn the_next_step_branches_from_the_commit_the_last_one_made() {
    let db = make_test_pool().await;
    seeded_step(&db, "run-1").await;
    sqlx::query(
        "UPDATE story_runs SET isolation_status = 'isolated', before_sha = 'base', \
         after_sha = 'made-by-step-one' WHERE id = ?",
    )
    .bind("run-1")
    .execute(&db)
    .await
    .expect("update");

    assert_eq!(step_tip("run-1", &db).await.as_deref(), Some("made-by-step-one"));
}

#[tokio::test]
async fn a_step_that_changed_nothing_hands_on_the_commit_it_started_from() {
    // No commit of its own, so the next step branches from the same place —
    // which is exactly the state of the tree it would have inherited.
    let db = make_test_pool().await;
    seeded_step(&db, "run-1").await;
    sqlx::query(
        "UPDATE story_runs SET isolation_status = 'isolated', before_sha = 'base' WHERE id = ?",
    )
    .bind("run-1")
    .execute(&db)
    .await
    .expect("update");

    assert_eq!(step_tip("run-1", &db).await.as_deref(), Some("base"));
}

#[tokio::test]
async fn an_un_isolated_step_starts_no_chain() {
    let db = make_test_pool().await;
    seeded_step(&db, "run-1").await;
    sqlx::query(
        "UPDATE story_runs SET isolation_status = 'not_a_git_repo', before_sha = 'base' \
         WHERE id = ?",
    )
    .bind("run-1")
    .execute(&db)
    .await
    .expect("update");

    assert_eq!(step_tip("run-1", &db).await, None);
}

#[tokio::test]
async fn an_unknown_step_starts_no_chain() {
    let db = make_test_pool().await;

    assert_eq!(step_tip("nope", &db).await, None);
}

// ---------------------------------------------------------------------------
// The sequential handoff
// ---------------------------------------------------------------------------

mod handoff {
    use crate::{handoff_output, HANDOFF_MAX_BYTES};

    #[test]
    fn a_short_message_is_carried_whole() {
        assert_eq!(handoff_output("done".into()), "done");
    }

    #[test]
    fn a_message_exactly_at_the_cap_is_left_alone() {
        let message = "x".repeat(HANDOFF_MAX_BYTES);
        assert_eq!(handoff_output(message.clone()), message);
    }

    #[test]
    fn a_long_message_is_cut_and_marked() {
        let out = handoff_output("x".repeat(HANDOFF_MAX_BYTES * 2));
        assert!(out.ends_with('…'), "a shortened handoff should say so");
        assert!(out.starts_with("xxxx"));
    }

    #[test]
    fn the_cap_includes_the_ellipsis_it_appends() {
        // Otherwise the constant is not the ceiling it claims: the marker was
        // added on top of the budget rather than taken out of it.
        let out = handoff_output("x".repeat(HANDOFF_MAX_BYTES * 2));
        assert!(out.len() <= HANDOFF_MAX_BYTES, "carried {} bytes", out.len());
    }

    #[test]
    fn cutting_never_splits_a_codepoint() {
        // The panic this function exists for. A reply full of box-drawing
        // characters is ordinary, and slicing at a fixed byte offset inside one
        // brings the whole pipeline step down.
        let out = handoff_output("│".repeat(HANDOFF_MAX_BYTES));
        assert!(out.ends_with('…'));
        assert!(out.len() <= HANDOFF_MAX_BYTES);
        // Round-trips as UTF-8, which is the whole point.
        assert!(out.chars().all(|c| c == '│' || c == '…'));
    }

    #[test]
    fn an_emoji_at_the_boundary_does_not_panic() {
        let mut message = "a".repeat(HANDOFF_MAX_BYTES - 2);
        message.push_str("🙂🙂🙂");
        let out = handoff_output(message);
        assert!(out.len() <= HANDOFF_MAX_BYTES);
    }
}
