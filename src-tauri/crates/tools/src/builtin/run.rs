//! Reading a run's state — the other half of `spawn_subtask`.
//!
//! `spawn_subtask` hands back a `run_id`, and until this module existed not one
//! of the thirteen registered tools accepted one. The orchestrator was given an
//! identifier its whole toolbox could not consume, and told to poll `get_story`
//! instead — a card whose movement depends on the child agent choosing to call
//! `update_story_status`, and on a workspace setting about board tidiness.
//!
//! A run's own status depends on neither. It is written by the run lifecycle
//! whether the child cooperates, fails, is cancelled, or exhausts its
//! iterations, so it is the thing worth polling.

use async_trait::async_trait;
use serde_json::json;
use sqlx::Row;

use crate::paging::cap_text_fields;
use crate::{Tool, ToolContext, ToolOutput};

/// Where a reader whose error text was cut can find the rest.
const FULL_ERROR_VIA_RUN_EVENTS: &str =
    "Read the run's timeline in the RustyAgent app for the full error.";

pub struct GetRunTool;

#[async_trait]
impl Tool for GetRunTool {
    fn name(&self) -> &str { "get_run" }

    fn description(&self) -> &str {
        "Read one run's state by id — the run_id spawn_subtask returns. Reports status \
         ('running', 'done', 'failed' or 'cancelled'), whether it has finished, how many \
         iterations it took, and the last error if it failed. A run's status is written by \
         the run itself, so it is reliable even when the agent that ran it did not update \
         its story."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": {
                    "type": "string",
                    "description": "UUID of the run, as returned by spawn_subtask"
                }
            },
            "required": ["run_id"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolOutput {
        let Some(run_id) = input.get("run_id").and_then(|v| v.as_str()) else {
            return ToolOutput::err("Missing required field: run_id");
        };

        // The story's own status comes along because an orchestrator wants both
        // and should not need two calls: the run says whether the work stopped,
        // the card says what a human would see.
        let row = sqlx::query(
            "SELECT r.id, r.story_id, r.agent_profile_id, r.status, r.iteration_count,
                    strftime('%Y-%m-%dT%H:%M:%fZ', r.started_at)  AS started_at,
                    strftime('%Y-%m-%dT%H:%M:%fZ', r.finished_at) AS finished_at,
                    s.status AS story_status, s.title AS story_title
             FROM story_runs r
             LEFT JOIN stories s ON s.id = r.story_id
             WHERE r.id = ?",
        )
        .bind(run_id)
        .fetch_optional(&ctx.db)
        .await;

        let row = match row {
            Ok(Some(row)) => row,
            Ok(None) => return ToolOutput::err(format!("Run {run_id} not found")),
            Err(e) => return ToolOutput::err(format!("DB error: {e}")),
        };

        let status: String = row.try_get("status").unwrap_or_default();
        let is_terminal = db::story_status::is_terminal_run_status(&status);

        // Only for a run that failed. A `done` run's timeline can still hold an
        // error event — a tool call that went wrong and was recovered from —
        // and surfacing that as "the run's error" would report a success as a
        // failure.
        let error = if status == "failed" {
            last_error(&ctx.db, run_id).await
        } else {
            None
        };

        let mut payload = json!({
            "run_id": row.try_get::<String, _>("id").unwrap_or_default(),
            "story_id": row.try_get::<String, _>("story_id").unwrap_or_default(),
            "story_title": row.try_get::<Option<String>, _>("story_title").ok().flatten(),
            "story_status": row.try_get::<Option<String>, _>("story_status").ok().flatten(),
            "agent_profile_id": row.try_get::<String, _>("agent_profile_id").unwrap_or_default(),
            "status": status,
            "is_terminal": is_terminal,
            "iteration_count": row.try_get::<i64, _>("iteration_count").unwrap_or_default(),
            "started_at": row.try_get::<Option<String>, _>("started_at").ok().flatten(),
            "finished_at": row.try_get::<Option<String>, _>("finished_at").ok().flatten(),
            "error": error,
        });

        // A provider's error body can be enormous, and this reply is read into
        // an orchestrator's context on every poll.
        cap_text_fields(&mut payload, &["error"], "get_run", FULL_ERROR_VIA_RUN_EVENTS);

        ToolOutput::ok(serde_json::to_string(&payload).unwrap_or_default())
    }
}

/// The newest `error` event on a run, if it has one.
///
/// Newest rather than first: a run that retried and then failed for a different
/// reason should report the reason it actually stopped for.
async fn last_error(db: &db::DbPool, run_id: &str) -> Option<String> {
    sqlx::query(
        "SELECT content FROM run_events
         WHERE run_id = ? AND event_type = 'error'
         ORDER BY sequence_num DESC LIMIT 1",
    )
    .bind(run_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .and_then(|row| row.try_get::<Option<String>, _>("content").ok().flatten())
    // Returned whole, and cut by `cap_text_fields` alone.
    //
    // A pre-trim here looked like a cheap saving and was a silent truncation:
    // trimming to `MAX_FIELD_BYTES` hands the capper a value exactly at its
    // limit, which it leaves alone — so the text was shortened and the marker
    // saying so never appeared. The row is already a `String` in memory by this
    // point, so the saving was imaginary anyway.
}
