//! Crash recovery — making the database honest again after the app dies.
//!
//! Every terminal run status is written in-process, at the end of the run
//! loop. Close the app mid-run and nothing ever writes one: the `story_runs`
//! row reads `running` for good, it counts as active everywhere status is
//! filtered, and no user action can clear it. The `approval_requests` rows a
//! run was waiting on are worse — `ApprovalGate` is an in-memory map, so after
//! a restart the UI still offers them and deciding one resolves nothing.
//!
//! [`reconcile_orphaned_runs`] is the startup pass that settles both. It is
//! deliberately *not* a resume: the conversation itself lives only in the run
//! task's message list and is gone. What this restores is the honesty of the
//! record — the run ended, here is when, and here is why.
//!
//! Nothing is deleted. An interrupted run really did work and really spent
//! tokens; its usage, its `iteration_count`, its events and its isolated
//! worktree are all left exactly as they were, so the user can still read the
//! run back and accept or revert what it wrote.
//!
//! # Who may call this
//!
//! Only the desktop app, from `setup()`, before the scheduler restores
//! continuous and scheduled profiles. Never the `rustyagent-board-mcp` stdio
//! binary: that process opens the same database while the app may be running,
//! and a sweep from there would mark live runs failed. The
//! `owner_instance_id` guard makes that a bounded mistake rather than an
//! unbounded one — a sweep can never touch a run started by the process
//! calling it — but the binary simply does not call it.

use std::sync::OnceLock;

use anyhow::{Context, Result};

use crate::DbPool;
use crate::timestamps::NOW_ISO8601;

/// The terminal status given to a run that was still `running` when the app
/// exited.
///
/// `failed`, not a new `interrupted` spelling. Migration
/// `20260410000016_normalize_run_status.sql` already had to repair one drift
/// between the runtime, the pipeline engine and the frontend's `RunStatus`
/// union — a fifth status would render as a blank badge in every filter and
/// list that has not learned it. The *reason* is not lost: it goes into the
/// run's own timeline as an [`INTERRUPTED_EVENT_TYPE`] event.
pub const INTERRUPTED_RUN_STATUS: &str = "failed";

/// `run_events.event_type` appended to each reconciled run.
///
/// A type of its own rather than a generic `error`, so the timeline can say
/// "the app went away" instead of implying the agent or a tool broke.
pub const INTERRUPTED_EVENT_TYPE: &str = "interrupted";

/// What that event says, verbatim, to whoever opens the run afterwards.
pub const INTERRUPTED_EVENT_MESSAGE: &str =
    "RustyAgent exited while this run was still executing, so it was marked failed on the \
     next startup. Its recorded token usage, iteration count and any committed work are kept; \
     the conversation itself was not saved and the run cannot be resumed.";

/// `approval_requests.rejection_reason` for a request nothing can answer.
pub const INTERRUPTED_APPROVAL_REASON: &str =
    "Denied automatically: RustyAgent restarted while this request was pending. The run \
     waiting on the decision is gone, so approving it would have executed nothing.";

/// What a swept run's `story_status` event gives as its reason.
pub const INTERRUPTED_TRANSITION_REASON: &str =
    "RustyAgent exited while this run was still executing, so its story was moved out of \
     in_progress on the next startup rather than left claiming to be in flight";

/// The status a `pipeline_step_runs` row is moved to alongside its run.
const INTERRUPTED_STEP_STATUS: &str = "failed";


/// Identifies this launch of the application.
///
/// Written to `story_runs.owner_instance_id` when a run starts, and the only
/// thing that separates "orphaned by a crash" from "running right now". A
/// fresh value per process is exactly the property the sweep needs: a row
/// carrying somebody else's id cannot be live *in this process*, and this
/// process is the one doing the sweeping.
///
/// The value is generated once and never changes, so nothing here races —
/// unlike a process-global that tests mutate.
///
/// Its limit is worth stating plainly: it identifies a process, not a machine.
/// Two desktop apps opened on the same data directory at the same time would
/// each treat the other's runs as orphans. That is out of scope here; the
/// multi-process case the story guards is the stdio MCP binary, which never
/// calls the sweep at all.
pub fn instance_id() -> &'static str {
    static INSTANCE_ID: OnceLock<String> = OnceLock::new();
    INSTANCE_ID.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

/// What one reconciliation pass changed.
///
/// Empty on every pass after the first — the sweep is idempotent, and this is
/// how a caller can say so without re-querying.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReconcileReport {
    /// Ids of the runs moved out of `running`.
    pub runs: Vec<String>,
    /// `pipeline_step_runs` rows moved off `pending` / `running`.
    pub pipeline_steps: u64,
    /// `approval_requests` rows denied.
    pub approvals: u64,
    /// Stories moved off `in_progress` because the run holding them is gone.
    pub stories: u64,
}

impl ReconcileReport {
    /// True when the pass found nothing to do.
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
            && self.pipeline_steps == 0
            && self.approvals == 0
            && self.stories == 0
    }
}

/// Settle every run and approval a previous session left mid-flight.
///
/// A run is orphaned when its status is `running` and its `owner_instance_id`
/// is not `instance_id` — either a previous launch's id, or NULL for a row
/// written before the column existed. A run this process started is never
/// touched, which is what makes the pass safe to invoke more than once and at
/// any point in the app's life rather than only at boot.
///
/// For each orphan the pass:
///
/// 1. moves the run to [`INTERRUPTED_RUN_STATUS`] and stamps `finished_at`,
///    leaving the token, cost and `iteration_count` columns alone;
/// 2. appends an [`INTERRUPTED_EVENT_TYPE`] event to the run's timeline;
/// 3. fails the `pipeline_step_runs` rows still `pending` or `running` that
///    belong to it — as the pipeline's root run *or* as one step's own run, so
///    a parent and its children come out consistent whichever the pass
///    reaches first.
///
/// Then every `pending` approval request that is not held by a run live in
/// this process is denied with [`INTERRUPTED_APPROVAL_REASON`], which is what
/// takes it out of `get_pending_approvals` and off the Approvals UI.
///
/// The whole pass is one transaction: a run never ends up failed without the
/// event that explains why.
pub async fn reconcile_orphaned_runs(db: &DbPool, instance_id: &str) -> Result<ReconcileReport> {
    let auto_advance = crate::story_status::auto_advance_enabled(db).await;

    let mut tx = db
        .begin()
        .await
        .context("Failed to open the startup reconciliation transaction")?;

    let orphans: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, story_id FROM story_runs \
         WHERE status = 'running' \
           AND (owner_instance_id IS NULL OR owner_instance_id <> ?)",
    )
    .bind(instance_id)
    .fetch_all(&mut *tx)
    .await
    .context("Failed to list runs left running by a previous session")?;

    let mut pipeline_steps = 0u64;
    let mut stories = 0u64;

    // Read before the transaction: the setting is the user's, not part of the
    // state being reconciled.
    let stories_settled_allowed = auto_advance;

    for (run_id, story_id) in &orphans {
        sqlx::query(&format!(
            "UPDATE story_runs SET status = ?, finished_at = {NOW_ISO8601} WHERE id = ?"
        ))
        .bind(INTERRUPTED_RUN_STATUS)
        .bind(run_id)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("Failed to reconcile run '{run_id}'"))?;

        // The event continues the run's own numbering rather than starting a
        // new one, so it sorts after whatever the run managed to write before
        // the process went away. `MAX` over an empty set still yields one row,
        // so a run that recorded nothing gets sequence 0.
        sqlx::query(
            "INSERT INTO run_events (id, run_id, event_type, content, sequence_num) \
             SELECT ?, ?, ?, ?, COALESCE(MAX(sequence_num), -1) + 1 \
             FROM run_events WHERE run_id = ?",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(run_id)
        .bind(INTERRUPTED_EVENT_TYPE)
        .bind(INTERRUPTED_EVENT_MESSAGE)
        .bind(run_id)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("Failed to record the interruption of run '{run_id}'"))?;

        // The card too, or the sweep leaves a story claiming to be in flight
        // after everything else about the run has been cleaned up. `blocked`,
        // not `ready`: nothing was concluded about the work, and a story
        // returned to `ready` is one a continuous-mode profile picks straight
        // back up.
        if stories_settled_allowed
            && crate::story_status::settle_story(
                &mut *tx,
                story_id,
                crate::story_status::RunOutcome::Interrupted,
            )
            .await
            .with_context(|| format!("Failed to settle the story of run '{run_id}'"))?
        {
            // Attributed like any other automatic move. The `interrupted`
            // event above says the run ended; this says what became of the
            // card, which is a different fact and the one a user looking at
            // the board is asking about.
            crate::story_status::record_transition(
                &mut *tx,
                run_id,
                story_id,
                crate::story_status::RunOutcome::Interrupted.story_status(),
                INTERRUPTED_TRANSITION_REASON,
            )
            .await
            .with_context(|| format!("Failed to record the story move of run '{run_id}'"))?;
            stories += 1;
        }

        let steps = sqlx::query(&format!(
            "UPDATE pipeline_step_runs SET status = ?, updated_at = {NOW_ISO8601} \
             WHERE status IN ('pending', 'running') \
               AND (pipeline_run_id = ? OR run_id = ?)"
        ))
        .bind(INTERRUPTED_STEP_STATUS)
        .bind(run_id)
        .bind(run_id)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("Failed to reconcile the pipeline steps of run '{run_id}'"))?;
        pipeline_steps += steps.rows_affected();
    }

    // Deliberately not restricted to the runs swept above. An approval whose
    // run already reached a terminal status some other way is just as
    // undeliverable — the gate that would have carried the decision died with
    // the process either way.
    let approvals = sqlx::query(&format!(
        "UPDATE approval_requests \
         SET status = 'rejected', rejection_reason = ?, decided_at = {NOW_ISO8601} \
         WHERE status = 'pending' \
           AND run_id NOT IN ( \
               SELECT id FROM story_runs WHERE status = 'running' AND owner_instance_id = ? \
           )"
    ))
    .bind(INTERRUPTED_APPROVAL_REASON)
    .bind(instance_id)
    .execute(&mut *tx)
    .await
    .context("Failed to deny approval requests left pending by a previous session")?
    .rows_affected();

    tx.commit()
        .await
        .context("Failed to commit the startup reconciliation")?;

    Ok(ReconcileReport {
        runs: orphans.into_iter().map(|(run_id, _)| run_id).collect(),
        pipeline_steps,
        approvals,
        stories,
    })
}
