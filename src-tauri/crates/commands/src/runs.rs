// Tauri commands for run history, event log, and export.

use db::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::Row;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Summary of one story run — returned by get_runs / get_run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryRun {
    pub id: String,
    pub story_id: String,
    pub story_title: Option<String>,
    pub agent_profile_id: String,
    pub agent_name: Option<String>,
    pub status: String,             // 'running' | 'done' | 'failed' | 'cancelled'
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub estimated_cost_usd: f64,
    pub iteration_count: i64,
    pub started_at: String,
    pub finished_at: Option<String>,
    /// Duration in seconds, computed from started_at/finished_at (None while running).
    pub duration_secs: Option<f64>,
    /// Git HEAD SHA at run start (None if workspace is not a git repo).
    pub before_sha: Option<String>,
}

/// Git diff payload — fetched separately from StoryRun due to potentially large size.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunDiff {
    pub run_id: String,
    pub before_sha: Option<String>,
    pub diff_output: Option<String>,
}

/// A single event in a run's event log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEvent {
    pub id: String,
    pub run_id: String,
    pub event_type: String,          // 'message' | 'tool_call' | 'tool_result' | 'thought' | 'error' | 'approval_request' | 'approval_response'
    pub role: Option<String>,        // 'user' | 'assistant' | 'tool'
    pub content: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<String>,  // JSON
    pub tool_output: Option<String>, // JSON
    pub is_error: bool,
    pub sequence_num: i64,
    pub created_at: String,
}

/// Filter parameters for listing runs.
#[derive(Debug, Deserialize)]
pub struct RunFilters {
    pub story_id: Option<String>,
    pub agent_profile_id: Option<String>,
    pub status: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn row_to_run(row: &sqlx::sqlite::SqliteRow) -> StoryRun {
    let started_at: String = row.try_get("started_at").unwrap_or_default();
    let finished_at: Option<String> = row.try_get("finished_at").ok().flatten();
    let duration_secs: Option<f64> = finished_at.as_deref().and_then(|end| {
        let start = chrono::DateTime::parse_from_rfc3339(&started_at).ok()?;
        let finish = chrono::DateTime::parse_from_rfc3339(end).ok()?;
        Some((finish - start).num_milliseconds() as f64 / 1000.0)
    });
    StoryRun {
        id:                  row.try_get("id").unwrap_or_default(),
        story_id:            row.try_get("story_id").unwrap_or_default(),
        story_title:         row.try_get("story_title").ok().flatten(),
        agent_profile_id:    row.try_get("agent_profile_id").unwrap_or_default(),
        agent_name:          row.try_get("agent_name").ok().flatten(),
        status:              row.try_get("status").unwrap_or_default(),
        input_tokens:        row.try_get("input_tokens").unwrap_or(0),
        output_tokens:       row.try_get("output_tokens").unwrap_or(0),
        estimated_cost_usd:  row.try_get("estimated_cost_usd").unwrap_or(0.0),
        iteration_count:     row.try_get("iteration_count").unwrap_or(0),
        started_at,
        finished_at,
        duration_secs,
        before_sha:          row.try_get("before_sha").ok().flatten(),
    }
}

fn row_to_event(row: &sqlx::sqlite::SqliteRow) -> RunEvent {
    let is_error: i64 = row.try_get("is_error").unwrap_or(0);
    RunEvent {
        id:           row.try_get("id").unwrap_or_default(),
        run_id:       row.try_get("run_id").unwrap_or_default(),
        event_type:   row.try_get("event_type").unwrap_or_default(),
        role:         row.try_get("role").ok().flatten(),
        content:      row.try_get("content").ok().flatten(),
        tool_name:    row.try_get("tool_name").ok().flatten(),
        tool_input:   row.try_get("tool_input").ok().flatten(),
        tool_output:  row.try_get("tool_output").ok().flatten(),
        is_error:     is_error != 0,
        sequence_num: row.try_get("sequence_num").unwrap_or(0),
        created_at:   row.try_get("created_at").unwrap_or_default(),
    }
}

const SELECT_RUNS: &str = "
    SELECT r.id, r.story_id, s.title AS story_title,
           r.agent_profile_id, a.name AS agent_name,
           r.status, r.input_tokens, r.output_tokens,
           r.estimated_cost_usd, r.iteration_count,
           r.started_at, r.finished_at, r.before_sha
    FROM story_runs r
    LEFT JOIN stories s ON s.id = r.story_id
    LEFT JOIN agent_profiles a ON a.id = r.agent_profile_id";

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// List runs, optionally filtered by story, agent, status, and active workspace.
pub async fn get_runs(
    filters: Option<RunFilters>,
    workspace_id: Option<String>,
    db: &DbPool,
) -> Result<Vec<StoryRun>, String> {
    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref f) = filters {
        if let Some(ref sid) = f.story_id {
            conditions.push("r.story_id = ?".to_string());
            binds.push(sid.clone());
        }
        if let Some(ref aid) = f.agent_profile_id {
            conditions.push("r.agent_profile_id = ?".to_string());
            binds.push(aid.clone());
        }
        if let Some(ref st) = f.status {
            conditions.push("r.status = ?".to_string());
            binds.push(st.clone());
        }
    }

    // Scope by active workspace through the stories join.
    match &workspace_id {
        Some(ws_id) => {
            conditions.push("s.workspace_id = ?".to_string());
            binds.push(ws_id.clone());
        }
        None => {
            conditions.push("s.workspace_id IS NULL".to_string());
        }
    }

    let where_clause = if conditions.is_empty() {
        " WHERE s.story_type != 'chat'".to_string()
    } else {
        format!(" WHERE s.story_type != 'chat' AND {}", conditions.join(" AND "))
    };

    let sql = format!(
        "{}{} ORDER BY r.started_at DESC",
        SELECT_RUNS, where_clause
    );

    let mut q = sqlx::query(&sql);
    for b in &binds {
        q = q.bind(b);
    }

    let rows = q
        .fetch_all(db)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    Ok(rows.iter().map(row_to_run).collect())
}

/// Get a single run by ID.
pub async fn get_run(id: String, db: &DbPool) -> Result<StoryRun, String> {
    let sql = format!("{} WHERE r.id = ?", SELECT_RUNS);
    let row = sqlx::query(&sql)
        .bind(&id)
        .fetch_optional(db)
        .await
        .map_err(|e| format!("DB error: {e}"))?
        .ok_or_else(|| format!("Run '{id}' not found"))?;
    Ok(row_to_run(&row))
}

/// Get all events for a run, ordered by sequence.
pub async fn get_run_events(run_id: String, db: &DbPool) -> Result<Vec<RunEvent>, String> {
    let rows = sqlx::query(
        "SELECT id, run_id, event_type, role, content, tool_name, tool_input, tool_output,
                is_error, sequence_num, created_at
         FROM run_events
         WHERE run_id = ?
         ORDER BY sequence_num ASC",
    )
    .bind(&run_id)
    .fetch_all(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    Ok(rows.iter().map(row_to_event).collect())
}

/// Delete a run and all its events (cascade).
pub async fn delete_run(id: String, db: &DbPool) -> Result<(), String> {
    sqlx::query("DELETE FROM story_runs WHERE id = ?")
        .bind(&id)
        .execute(db)
        .await
        .map_err(|e| format!("DB delete error: {e}"))?;
    Ok(())
}

/// Export a run's events as a JSON array string (one object per event).
/// The caller can write this to a .jsonl file on the frontend.
pub async fn export_run_events(run_id: String, db: &DbPool) -> Result<String, String> {
    let events = get_run_events(run_id, db).await?;
    serde_json::to_string(&events).map_err(|e| format!("Serialization error: {e}"))
}

/// Fetch the git diff for a single run.
/// `diff_output` is stored separately and excluded from `get_runs`/`get_run`
/// because it can be arbitrarily large.
pub async fn get_run_diff(run_id: String, db: &DbPool) -> Result<RunDiff, String> {
    let row = sqlx::query(
        "SELECT id, before_sha, diff_output FROM story_runs WHERE id = ?"
    )
    .bind(&run_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?
    .ok_or_else(|| format!("Run '{run_id}' not found"))?;

    Ok(RunDiff {
        run_id,
        before_sha:  row.try_get("before_sha").ok().flatten(),
        diff_output: row.try_get("diff_output").ok().flatten(),
    })
}
