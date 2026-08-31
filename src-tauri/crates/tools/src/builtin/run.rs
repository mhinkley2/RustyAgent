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

// ---------------------------------------------------------------------------
// wait_for_subtask
// ---------------------------------------------------------------------------

/// Seconds to wait when the caller does not say.
///
/// Long enough for a real subtask, short enough that a parent which asked for
/// something that will never finish gets its turn back eventually rather than
/// never.
const DEFAULT_WAIT_SECS: u64 = 300;

/// The longest wait this tool will hold a run open for.
const MAX_WAIT_SECS: u64 = 3600;

/// How often the wait re-reads the runs it is waiting on.
///
/// A subtask takes tens of seconds at the very least, so half a second of
/// latency on noticing one finished is free. What this interval really governs
/// is how quickly a cancelled parent stops waiting.
const POLL_INTERVAL_MS: u64 = 500;

/// How many runs one call may wait on.
const MAX_WAITED_RUNS: usize = 32;

/// Wait for spawned subtasks and collect what they produced.
///
/// The wait is a poll of `story_runs` rather than a completion signal, and that
/// is a deliberate trade. The database is where a run's terminal state actually
/// lands — `get_run` reads the same column — so polling it works for any run
/// however it was started, and cannot disagree with what a second mechanism
/// believed. A broadcast channel would react faster, but would only cover runs
/// started through the spawn path and would introduce a second answer to "is it
/// done". Half a second of latency on a task measured in minutes is not worth
/// that.
///
/// What the story asked to avoid was a *model* poll loop: an orchestrator
/// burning one round-trip of a twenty-iteration budget per check. This costs
/// one tool call however long the children take.
pub struct WaitForSubtaskTool;

#[async_trait]
impl Tool for WaitForSubtaskTool {
    fn name(&self) -> &str { "wait_for_subtask" }

    fn description(&self) -> &str {
        "Wait for one or more subtasks to finish, then return what each produced: its final \
         status, its last message, and whatever the group left in shared_scratchpad. Blocks \
         inside a single tool call, so waiting costs nothing from your iteration budget \
         however long the subtasks take. Give it the run_ids spawn_subtask returned. Returns \
         when every run has finished or the timeout expires, whichever is first; a run still \
         going at the timeout is reported as such rather than waited on forever."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "run_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": format!(
                        "Run ids to wait for, as returned by spawn_subtask. At most \
                         {MAX_WAITED_RUNS}."
                    )
                },
                "timeout_secs": {
                    "type": "integer",
                    "minimum": 1,
                    "description": format!(
                        "How long to wait before returning with whatever has finished. \
                         Defaults to {DEFAULT_WAIT_SECS}, clamped to {MAX_WAIT_SECS}."
                    )
                }
            },
            "required": ["run_ids"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolOutput {
        let run_ids = match waited_ids(&input) {
            Ok(ids) => ids,
            Err(error) => return ToolOutput::err(error),
        };

        let timeout = match input.get("timeout_secs") {
            None => DEFAULT_WAIT_SECS,
            Some(value) => match value.as_u64() {
                Some(secs) if secs > 0 => secs.min(MAX_WAIT_SECS),
                _ => {
                    return ToolOutput::err(
                        "Parameter 'timeout_secs' must be a positive integer number of seconds",
                    )
                }
            },
        };

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);
        let mut cancelled = false;

        loop {
            let statuses = match statuses_of(&ctx.db, &run_ids).await {
                Ok(statuses) => statuses,
                Err(error) => return ToolOutput::err(error),
            };
            let all_done = statuses.iter().all(|(_, status)| match status {
                // A run that does not exist resolves rather than blocks: a typo
                // or a deleted row would otherwise be waited on forever, which
                // is the hang this tool exists to avoid. The result says so.
                None => true,
                Some(status) => db::story_status::is_terminal_run_status(status),
            });
            if all_done {
                break;
            }

            // Checked every tick rather than once at the top. For the length of
            // this call nothing else in the parent is running, so this is the
            // only thing in a position to notice the run was stopped.
            if ctx.run_control.as_ref().is_some_and(|c| c.is_cancelled()) {
                cancelled = true;
                break;
            }

            if std::time::Instant::now() >= deadline {
                break;
            }

            tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
        }

        // A parent that stopped waiting leaves children doing work nobody will
        // read. Only the ones still going: cancelling a finished run is a no-op,
        // but asking is still a lock and a lookup.
        if cancelled {
            if let Some(control) = ctx.run_control.as_ref() {
                for (run_id, status) in statuses_of(&ctx.db, &run_ids).await.unwrap_or_default() {
                    if status.as_deref() == Some("running") {
                        control.cancel_run(&run_id);
                    }
                }
            }
        }

        let mut results = Vec::with_capacity(run_ids.len());
        for run_id in &run_ids {
            results.push(subtask_result(&ctx.db, run_id).await);
        }

        let finished = results
            .iter()
            .filter(|r| r["is_terminal"] == json!(true))
            .count();
        let complete = finished == results.len();

        // One scratchpad for the call, not one per child: the scope is shared
        // by construction. `spawn_subtask` hands a child the parent's
        // `pipeline_run_id`, falling back to the parent's own run id outside a
        // pipeline, and `memory_write` scopes a `shared_scratchpad` entry by
        // exactly that — so this is the same key the children wrote under.
        let scratchpad_scope = ctx
            .pipeline_run_id
            .clone()
            .unwrap_or_else(|| ctx.run_id.clone());

        let mut payload = json!({
            "results": results,
            "scratchpad": scratchpad(&ctx.db, &scratchpad_scope).await,
            "waited_for": run_ids.len(),
            "finished": finished,
            "complete": complete,
            "cancelled": cancelled,
        });

        if cancelled {
            payload["notice"] = json!(
                "[wait_for_subtask CANCELLED: this run was stopped while waiting. Any subtask \
                 still running has been asked to stop. Do not start more work.]"
            );
        } else if !complete {
            payload["notice"] = json!(format!(
                "[wait_for_subtask TIMED OUT after {timeout}s: {finished} of {} subtasks \
                 finished. The rest are still running — call wait_for_subtask again with \
                 their run_ids to keep waiting, or get_run to check one.]",
                run_ids.len()
            ));
        }

        ToolOutput::ok(serde_json::to_string(&payload).unwrap_or_default())
    }
}

/// Validate `run_ids` into a deduplicated list.
///
/// Deduplicated because waiting on the same run twice would report it twice and
/// make `finished` disagree with how many subtasks there were.
fn waited_ids(input: &serde_json::Value) -> Result<Vec<String>, String> {
    let Some(raw) = input.get("run_ids") else {
        return Err("Missing required field: run_ids (array of run ids)".to_string());
    };
    let Some(array) = raw.as_array() else {
        return Err("Parameter 'run_ids' must be an array of run ids".to_string());
    };

    let mut ids: Vec<String> = Vec::with_capacity(array.len());
    for value in array {
        let Some(id) = value.as_str() else {
            return Err("Every entry in 'run_ids' must be a string".to_string());
        };
        if !ids.iter().any(|seen| seen == id) {
            ids.push(id.to_string());
        }
    }

    if ids.is_empty() {
        return Err("Parameter 'run_ids' must name at least one run".to_string());
    }
    if ids.len() > MAX_WAITED_RUNS {
        return Err(format!(
            "Parameter 'run_ids' names {} runs; at most {MAX_WAITED_RUNS} may be waited on at \
             once.",
            ids.len()
        ));
    }
    Ok(ids)
}

/// Each named run's status, or `None` where no such run exists.
async fn statuses_of(
    db: &db::DbPool,
    run_ids: &[String],
) -> Result<Vec<(String, Option<String>)>, String> {
    let mut out = Vec::with_capacity(run_ids.len());
    for run_id in run_ids {
        let status = sqlx::query("SELECT status FROM story_runs WHERE id = ?")
            .bind(run_id)
            .fetch_optional(db)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .and_then(|row| row.try_get::<String, _>("status").ok());
        out.push((run_id.clone(), status));
    }
    Ok(out)
}

/// What one subtask produced: how it ended, and what it said.
///
/// A status transition alone is not an answer. An orchestrator that delegated
/// work wants the work back.
async fn subtask_result(db: &db::DbPool, run_id: &str) -> serde_json::Value {
    let row = sqlx::query(
        "SELECT r.status, r.story_id, r.iteration_count, s.status AS story_status
         FROM story_runs r
         LEFT JOIN stories s ON s.id = r.story_id
         WHERE r.id = ?",
    )
    .bind(run_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();

    let Some(row) = row else {
        return json!({
            "run_id": run_id,
            "status": serde_json::Value::Null,
            "is_terminal": true,
            "error": "No such run — it was never started, or has been deleted.",
        });
    };

    let status: String = row.try_get("status").unwrap_or_default();

    let mut result = json!({
        "run_id": run_id,
        "story_id": row.try_get::<String, _>("story_id").unwrap_or_default(),
        "story_status": row.try_get::<Option<String>, _>("story_status").ok().flatten(),
        "status": status,
        "is_terminal": db::story_status::is_terminal_run_status(&status),
        "iteration_count": row.try_get::<i64, _>("iteration_count").unwrap_or_default(),
        "output": last_assistant_message(db, run_id).await,
        "error": if status == "failed" { last_error(db, run_id).await } else { None },
    });

    cap_text_fields(
        &mut result,
        &["output", "error"],
        "wait_for_subtask",
        FULL_ERROR_VIA_RUN_EVENTS,
    );
    result
}

/// The last thing the subtask's agent said — its answer, in its own words.
async fn last_assistant_message(db: &db::DbPool, run_id: &str) -> Option<String> {
    sqlx::query(
        "SELECT content FROM run_events
         WHERE run_id = ? AND role = 'assistant' AND content IS NOT NULL
         ORDER BY sequence_num DESC LIMIT 1",
    )
    .bind(run_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .and_then(|row| row.try_get::<String, _>("content").ok())
}

/// Everything written to the scratchpad this group of runs shares.
async fn scratchpad(db: &db::DbPool, pipeline_run_id: &str) -> serde_json::Value {
    let rows = sqlx::query(
        "SELECT key, value FROM agent_memory
         WHERE scope = 'shared_scratchpad' AND pipeline_run_id = ?
         ORDER BY key ASC",
    )
    .bind(pipeline_run_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    let mut map = serde_json::Map::new();
    for row in rows {
        let key: String = row.try_get("key").unwrap_or_default();
        let value: String = row.try_get("value").unwrap_or_default();
        map.insert(key, json!(value));
    }
    serde_json::Value::Object(map)
}
