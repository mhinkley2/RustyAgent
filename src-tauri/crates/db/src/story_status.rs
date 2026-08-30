//! What happens to a story's card when its run ends.
//!
//! A story a run picks up is moved to `in_progress` and, until this module
//! existed, never moved off it. Nothing in the run lifecycle wrote a terminal
//! story status, so the board filled with work that looked permanently in
//! flight and a human had to clear every card by hand.
//!
//! The transitions live here rather than at each call site because there are
//! four of them — a single run finishing, a pipeline finishing, a run starting,
//! and the crash sweep — and the guards below are the whole substance of the
//! feature. Four copies of a `WHERE` clause is four chances to leave one out.
//!
//! # Who is protected from this
//!
//! Two guards ride on every statement:
//!
//! - **`status = 'in_progress'`** — an automatic transition may only move a
//!   card it can see was left mid-run. If a human dragged the card to
//!   `blocked`, or the agent called `update_story_status` deliberately, that
//!   is a decision and it outranks this one. The write becomes a no-op rather
//!   than a correction.
//! - **`story_type <> 'chat'`** — chat sessions are rows in `stories`
//!   (`commands/src/lib.rs` inserts them as `'chat'` / `'in_progress'`). They
//!   are not board work and have no card; without this guard every chat would
//!   surface on the board the moment it was answered.

use sqlx::{Executor, Sqlite};

use crate::DbPool;

/// The key this feature reads out of a workspace's `settings_json`.
///
/// Absent means on. The board being honest is the behaviour that should need
/// no configuration; the switch exists for someone who has built a habit
/// around moving cards themselves, and an automation that fights an
/// established habit is worse than none.
pub const AUTO_ADVANCE_SETTING: &str = "auto_advance_story_status";

/// An ISO-8601 timestamp in the spelling the schema's own defaults use.
///
/// `CURRENT_TIMESTAMP` emits `YYYY-MM-DD HH:MM:SS`, which is not RFC 3339 and
/// does not parse where the rest of the app parses timestamps.
const NOW_ISO8601: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

/// How a run ended, from the board's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// The run reached `done`.
    Succeeded,
    /// The run failed, and no retry is coming.
    Failed,
    /// The user stopped the run.
    Cancelled,
    /// The app exited mid-run and the startup sweep closed it out.
    Interrupted,
}

impl RunOutcome {
    /// The `stories.status` this outcome moves a card to.
    ///
    /// **Success is `review`, not `done`.** Agent output that lands in `done`
    /// with nobody looking asserts a confidence the app has not earned.
    /// `review` clears the in-progress queue and leaves `done` a human verb.
    ///
    /// **Everything else is `blocked`, never `ready`.** This is the
    /// load-bearing one. A failed story returned to `ready` while a
    /// continuous-mode profile is polling gets re-picked immediately, and the
    /// pair loops without bound against a story that just failed — burning API
    /// budget with nobody watching. `blocked` stops the cycle and makes the
    /// failure visible.
    /// Read a terminal `story_runs.status` as an outcome.
    ///
    /// One mapping for the two places a run can end — the agent loop and the
    /// pipeline engine — so the pair cannot come to disagree about what
    /// "finished" means. `done` is the vocabulary migration
    /// `20260410000016_normalize_run_status.sql` settled on; anything not
    /// recognised is treated as a failure, which is the safe direction: a
    /// story wrongly `blocked` is visible, a story wrongly `review` is a
    /// silent claim that work succeeded.
    pub fn from_run_status(status: &str) -> Self {
        match status {
            "done" => Self::Succeeded,
            "cancelled" => Self::Cancelled,
            _ => Self::Failed,
        }
    }

    pub fn story_status(self) -> &'static str {
        match self {
            Self::Succeeded => "review",
            Self::Failed | Self::Cancelled | Self::Interrupted => "blocked",
        }
    }
}

/// Move a story off `in_progress` to reflect how its run ended.
///
/// Returns whether a row moved, so a caller can record the transition against
/// the run that caused it — and so a caller can tell "left alone deliberately"
/// from "written" rather than assuming.
///
/// Generic over the executor because the crash sweep calls this inside its own
/// transaction, where a run and its story must settle together or not at all.
pub async fn settle_story<'e, E>(
    executor: E,
    story_id: &str,
    outcome: RunOutcome,
) -> sqlx::Result<bool>
where
    E: Executor<'e, Database = Sqlite>,
{
    let result = sqlx::query(&format!(
        "UPDATE stories SET status = ?, updated_at = {NOW_ISO8601} \
         WHERE id = ? AND status = 'in_progress' AND story_type <> 'chat'"
    ))
    .bind(outcome.story_status())
    .bind(story_id)
    .execute(executor)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Move a `ready` story to `in_progress` because a run just started on it.
///
/// The counterpart to [`settle_story`], so the two ends of a run are
/// symmetric. Without it a manually started run leaves its card looking
/// untouched while it executes, and the completion side then has nothing in
/// `in_progress` to move.
///
/// Only `ready` is claimed. A story already `in_progress` is left alone (a
/// second run on the same story does not re-stamp it), and one sitting in
/// `backlog`, `blocked` or `review` is somewhere a person put it.
pub async fn claim_story<'e, E>(executor: E, story_id: &str) -> sqlx::Result<bool>
where
    E: Executor<'e, Database = Sqlite>,
{
    let result = sqlx::query(&format!(
        "UPDATE stories SET status = 'in_progress', updated_at = {NOW_ISO8601} \
         WHERE id = ? AND status = 'ready' AND story_type <> 'chat'"
    ))
    .bind(story_id)
    .execute(executor)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// The Tauri event announcing that the board has changed underneath whoever is
/// looking at it.
///
/// Named here rather than in the frontend or in `commands` because the writers
/// are spread across four crates and one of them — the crash sweep — lives in
/// this one. The string is the contract; `useStories` listens for it.
///
/// It carries no payload on purpose. A board refetch is one cheap query, and a
/// payload would be a second representation of a story to keep in step with
/// the first.
pub const STORIES_CHANGED_EVENT: &str = "stories-changed";

/// The `run_events.event_type` written when a card moves on its own.
pub const TRANSITION_EVENT_TYPE: &str = "story_status";

/// The body of a [`TRANSITION_EVENT_TYPE`] event.
///
/// Built here rather than at each of the three call sites so a reader of the
/// timeline sees one shape whichever path moved the card.
pub fn transition_payload(story_id: &str, to: &str, reason: &str) -> serde_json::Value {
    serde_json::json!({
        "storyId": story_id,
        "from": "in_progress",
        "to": to,
        "reason": reason,
    })
}

/// Record a card's move on the timeline of the run that caused it.
///
/// For callers with no in-memory sequence counter — the pipeline engine and
/// the crash sweep. `sequence_num` continues the run's own numbering with a
/// `MAX(...) + 1` subquery, the same shape `recovery` uses for its
/// `interrupted` event, so the entry sorts after whatever the run wrote before
/// it. `MAX` over an empty set still yields one row, so a run that recorded
/// nothing gets sequence 0.
///
/// `ConversationRuntime` does not use this: it holds an `AtomicU32` counter
/// for the run it is executing, and a `MAX`-based insert would collide with
/// it. It writes the same event type and the same [`transition_payload`].
pub async fn record_transition<'e, E>(
    executor: E,
    run_id: &str,
    story_id: &str,
    to: &str,
    reason: &str,
) -> sqlx::Result<()>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO run_events (id, run_id, event_type, content, sequence_num) \
         SELECT ?, ?, ?, ?, COALESCE(MAX(sequence_num), -1) + 1 \
         FROM run_events WHERE run_id = ?",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(run_id)
    .bind(TRANSITION_EVENT_TYPE)
    .bind(transition_payload(story_id, to, reason).to_string())
    .bind(run_id)
    .execute(executor)
    .await?;

    Ok(())
}

/// Whether automatic story transitions are switched on for the active
/// workspace.
///
/// Reads the most recently opened workspace's override, mirroring the shape
/// `pipeline::workspace_parallel_limit` uses. Anything unreadable — no
/// workspace, no override, malformed JSON, a non-boolean — is treated as
/// absent, and absent is on.
pub async fn auto_advance_enabled(db: &DbPool) -> bool {
    workspace_flag(db, AUTO_ADVANCE_SETTING).await.unwrap_or(true)
}

async fn workspace_flag(db: &DbPool, key: &str) -> Option<bool> {
    let row: (String,) = sqlx::query_as(
        "SELECT ws.settings_json FROM workspace_settings ws \
         JOIN workspaces w ON w.id = ws.workspace_id \
         ORDER BY w.last_opened_at DESC LIMIT 1",
    )
    .fetch_optional(db)
    .await
    .ok()??;

    serde_json::from_str::<serde_json::Value>(&row.0)
        .ok()?
        .get(key)?
        .as_bool()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::make_test_pool;

    async fn seed(db: &DbPool, id: &str, status: &str, story_type: &str) {
        sqlx::query("INSERT INTO stories (id, title, status, story_type) VALUES (?, ?, ?, ?)")
            .bind(id)
            .bind("A story")
            .bind(status)
            .bind(story_type)
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

    /// The status the scheduler's pick query selects on
    /// (`scheduler/src/lib.rs:285`). A terminal automatic transition must
    /// never produce this value, or a continuous-mode profile re-picks the
    /// story it just failed, without bound.
    const SCHEDULER_PICKS: &str = "ready";

    #[test]
    fn a_terminal_run_status_maps_to_the_outcome_it_describes() {
        assert_eq!(RunOutcome::from_run_status("done"), RunOutcome::Succeeded);
        assert_eq!(RunOutcome::from_run_status("failed"), RunOutcome::Failed);
        assert_eq!(
            RunOutcome::from_run_status("cancelled"),
            RunOutcome::Cancelled
        );
    }

    /// The safe direction. A story wrongly `blocked` is visible and a person
    /// fixes it; a story wrongly `review` silently claims work succeeded.
    #[test]
    fn an_unrecognised_run_status_is_treated_as_a_failure() {
        assert_eq!(RunOutcome::from_run_status("completed"), RunOutcome::Failed);
        assert_eq!(RunOutcome::from_run_status(""), RunOutcome::Failed);
    }

    #[test]
    fn a_finished_run_sends_its_story_to_review_not_done() {
        assert_eq!(RunOutcome::Succeeded.story_status(), "review");
    }

    #[test]
    fn no_outcome_ever_returns_a_story_to_the_schedulers_queue() {
        for outcome in [
            RunOutcome::Succeeded,
            RunOutcome::Failed,
            RunOutcome::Cancelled,
            RunOutcome::Interrupted,
        ] {
            assert_ne!(
                outcome.story_status(),
                SCHEDULER_PICKS,
                "{outcome:?} would let a continuous-mode profile re-pick the story"
            );
        }
    }

    #[test]
    fn every_written_status_is_one_the_board_can_render() {
        // `src/types/board.ts` — a value outside this set lands a card in a
        // column the UI does not draw.
        const RENDERABLE: [&str; 6] = [
            "backlog",
            "ready",
            "in_progress",
            "blocked",
            "review",
            "done",
        ];
        for outcome in [
            RunOutcome::Succeeded,
            RunOutcome::Failed,
            RunOutcome::Cancelled,
            RunOutcome::Interrupted,
        ] {
            assert!(
                RENDERABLE.contains(&outcome.story_status()),
                "{outcome:?} writes {} which the board cannot render",
                outcome.story_status()
            );
        }
    }

    #[tokio::test]
    async fn a_succeeded_run_moves_its_story_to_review() {
        let db = make_test_pool().await;
        seed(&db, "s1", "in_progress", "task").await;

        assert!(settle_story(&db, "s1", RunOutcome::Succeeded).await.unwrap());

        assert_eq!(status_of(&db, "s1").await, "review");
    }

    #[tokio::test]
    async fn a_failed_run_blocks_its_story_rather_than_readying_it() {
        let db = make_test_pool().await;
        seed(&db, "s1", "in_progress", "task").await;

        assert!(settle_story(&db, "s1", RunOutcome::Failed).await.unwrap());

        assert_eq!(status_of(&db, "s1").await, "blocked");
    }

    #[tokio::test]
    async fn a_cancelled_run_blocks_its_story() {
        let db = make_test_pool().await;
        seed(&db, "s1", "in_progress", "task").await;

        settle_story(&db, "s1", RunOutcome::Cancelled).await.unwrap();

        assert_eq!(status_of(&db, "s1").await, "blocked");
    }

    /// The guard that keeps this from being a correction of the user.
    #[tokio::test]
    async fn a_status_someone_set_during_the_run_is_left_alone() {
        let db = make_test_pool().await;
        seed(&db, "s1", "blocked", "task").await;

        let moved = settle_story(&db, "s1", RunOutcome::Succeeded).await.unwrap();

        assert!(!moved, "a deliberate status outranks an automatic one");
        assert_eq!(status_of(&db, "s1").await, "blocked");
    }

    #[tokio::test]
    async fn a_story_the_agent_already_marked_done_is_not_pulled_back_to_review() {
        let db = make_test_pool().await;
        seed(&db, "s1", "done", "task").await;

        settle_story(&db, "s1", RunOutcome::Succeeded).await.unwrap();

        assert_eq!(status_of(&db, "s1").await, "done");
    }

    /// Chat sessions are rows in `stories`. They have no card, and moving them
    /// would put every conversation on the board.
    #[tokio::test]
    async fn a_chat_session_is_never_moved() {
        let db = make_test_pool().await;
        seed(&db, "chat-1", "in_progress", "chat").await;

        let moved = settle_story(&db, "chat-1", RunOutcome::Succeeded)
            .await
            .unwrap();

        assert!(!moved);
        assert_eq!(status_of(&db, "chat-1").await, "in_progress");
    }

    #[tokio::test]
    async fn starting_a_run_claims_a_ready_story() {
        let db = make_test_pool().await;
        seed(&db, "s1", "ready", "task").await;

        assert!(claim_story(&db, "s1").await.unwrap());

        assert_eq!(status_of(&db, "s1").await, "in_progress");
    }

    #[tokio::test]
    async fn starting_a_run_does_not_drag_a_story_out_of_backlog() {
        let db = make_test_pool().await;
        seed(&db, "s1", "backlog", "task").await;

        assert!(!claim_story(&db, "s1").await.unwrap());

        assert_eq!(status_of(&db, "s1").await, "backlog");
    }

    #[tokio::test]
    async fn starting_a_chat_does_not_put_it_on_the_board() {
        let db = make_test_pool().await;
        seed(&db, "chat-1", "ready", "chat").await;

        assert!(!claim_story(&db, "chat-1").await.unwrap());

        assert_eq!(status_of(&db, "chat-1").await, "ready");
    }

    #[tokio::test]
    async fn settling_a_story_stamps_updated_at() {
        let db = make_test_pool().await;
        seed(&db, "s1", "in_progress", "task").await;
        sqlx::query("UPDATE stories SET updated_at = '2020-01-01T00:00:00.000Z' WHERE id = ?")
            .bind("s1")
            .execute(&db)
            .await
            .unwrap();

        settle_story(&db, "s1", RunOutcome::Succeeded).await.unwrap();

        let updated: String = sqlx::query_scalar("SELECT updated_at FROM stories WHERE id = ?")
            .bind("s1")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_ne!(updated, "2020-01-01T00:00:00.000Z", "the move must be dated");
    }

    // ---------------------------------------------------------------------
    // The setting
    // ---------------------------------------------------------------------

    async fn seed_workspace_with(db: &DbPool, settings_json: &str) {
        sqlx::query("INSERT INTO workspaces (id, name, path) VALUES ('w1', 'W', '/tmp/w')")
            .execute(db)
            .await
            .expect("seed workspace");
        sqlx::query("INSERT INTO workspace_settings (workspace_id, settings_json) VALUES ('w1', ?)")
            .bind(settings_json)
            .execute(db)
            .await
            .expect("seed workspace settings");
    }

    #[tokio::test]
    async fn the_board_is_kept_honest_by_default() {
        let db = make_test_pool().await;

        assert!(auto_advance_enabled(&db).await, "no workspace, no override");
    }

    #[tokio::test]
    async fn a_workspace_with_other_settings_still_defaults_to_on() {
        let db = make_test_pool().await;
        seed_workspace_with(&db, "{\"max_parallel_steps\": 3}").await;

        assert!(auto_advance_enabled(&db).await);
    }

    #[tokio::test]
    async fn a_workspace_can_switch_the_behaviour_off() {
        let db = make_test_pool().await;
        seed_workspace_with(&db, "{\"auto_advance_story_status\": false}").await;

        assert!(!auto_advance_enabled(&db).await);
    }

    #[tokio::test]
    async fn unreadable_settings_do_not_silently_disable_the_feature() {
        let db = make_test_pool().await;
        seed_workspace_with(&db, "not json at all").await;

        assert!(auto_advance_enabled(&db).await);
    }
}
