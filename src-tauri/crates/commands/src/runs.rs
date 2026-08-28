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
    /// Input tokens billed at the full rate. Cached input is counted in the
    /// two cache columns instead, so the context a run read is the sum of all
    /// three.
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
    /// An estimate from the per-model price table, not a bill. Stays 0.0 when
    /// the model is not in the table.
    pub estimated_cost_usd: f64,
    pub iteration_count: i64,
    pub started_at: String,
    pub finished_at: Option<String>,
    /// Duration in seconds, computed from started_at/finished_at (None while running).
    pub duration_secs: Option<f64>,
    /// Git HEAD SHA at run start (None if workspace is not a git repo).
    pub before_sha: Option<String>,
    /// Absolute path of the isolated worktree the run executed in.
    ///
    /// Kept on the row after cleanup as part of the record; `isolation_status`
    /// says whether the directory still exists.
    pub worktree_path: Option<String>,
    /// Branch the run's worktree had checked out.
    pub branch_name: Option<String>,
    /// Commit made on that branch when the run finished, or None when the run
    /// changed nothing.
    pub after_sha: Option<String>,
    /// `isolated` | `not_a_git_repo` | `unavailable` | `no_workspace` |
    /// `accepted` | `reverted`. None for runs predating worktree isolation.
    pub isolation_status: Option<String>,
    /// Why a run was not isolated, or what was surprising about the one that
    /// was. Shown to the operator verbatim.
    pub isolation_note: Option<String>,
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
        cache_read_input_tokens:     row.try_get("cache_read_input_tokens").unwrap_or(0),
        cache_creation_input_tokens: row.try_get("cache_creation_input_tokens").unwrap_or(0),
        estimated_cost_usd:  row.try_get("estimated_cost_usd").unwrap_or(0.0),
        iteration_count:     row.try_get("iteration_count").unwrap_or(0),
        started_at,
        finished_at,
        duration_secs,
        before_sha:          row.try_get("before_sha").ok().flatten(),
        worktree_path:       row.try_get("worktree_path").ok().flatten(),
        branch_name:         row.try_get("branch_name").ok().flatten(),
        after_sha:           row.try_get("after_sha").ok().flatten(),
        isolation_status:    row.try_get("isolation_status").ok().flatten(),
        isolation_note:      row.try_get("isolation_note").ok().flatten(),
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
           r.cache_read_input_tokens, r.cache_creation_input_tokens,
           r.estimated_cost_usd, r.iteration_count,
           r.started_at, r.finished_at, r.before_sha,
           r.worktree_path, r.branch_name, r.after_sha,
           r.isolation_status, r.isolation_note
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

// ---------------------------------------------------------------------------
// Accept / revert
// ---------------------------------------------------------------------------

/// The isolation record of one run — what accept and revert operate on.
#[derive(Debug, Clone)]
struct RunIsolation {
    status: String,
    worktree_path: String,
    branch_name: String,
    run_status: String,
    story_id: String,
}

/// Load and validate the isolation record, or explain why the run cannot be
/// accepted or reverted.
async fn load_isolation(run_id: &str, db: &DbPool) -> Result<RunIsolation, String> {
    let row = sqlx::query(
        "SELECT status, story_id, worktree_path, branch_name, isolation_status \
         FROM story_runs WHERE id = ?",
    )
    .bind(run_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?
    .ok_or_else(|| format!("Run '{run_id}' not found"))?;

    let run_status: String = row.try_get("status").unwrap_or_default();
    if run_status == "running" {
        return Err(
            "This run is still going. Stop it first — its changes are not finished or committed \
             yet."
                .to_string(),
        );
    }

    let status: Option<String> = row.try_get("isolation_status").ok().flatten();
    let worktree_path: Option<String> = row.try_get("worktree_path").ok().flatten();
    let branch_name: Option<String> = row.try_get("branch_name").ok().flatten();

    match (status.as_deref(), worktree_path, branch_name) {
        (Some(runtime::worktree::STATUS_ISOLATED), Some(path), Some(branch)) => Ok(RunIsolation {
            status: runtime::worktree::STATUS_ISOLATED.to_string(),
            worktree_path: path,
            branch_name: branch,
            run_status,
            story_id: row.try_get("story_id").unwrap_or_default(),
        }),
        (Some("accepted"), ..) => Err(
            "This run has already been accepted; its worktree and branch are gone.".to_string(),
        ),
        (Some("reverted"), ..) => {
            Err("This run has already been reverted; there is nothing left to undo.".to_string())
        }
        _ => Err(
            "This run was not isolated, so RustyAgent has nothing of its own to accept or throw \
             away. Its changes — if any — are already in your working tree, and reverting them is \
             yours to do with git."
                .to_string(),
        ),
    }
}

/// The repository the run's worktree belongs to.
///
/// Asked of git first, since the worktree knows its own main tree. Falls back
/// to the run's workspace record for the case where the directory is gone.
async fn main_repo_for(iso: &RunIsolation, db: &DbPool) -> Result<std::path::PathBuf, String> {
    let worktree = std::path::Path::new(&iso.worktree_path);
    if let Some(main) = runtime::worktree::main_worktree_root(worktree) {
        return Ok(main);
    }

    let row = sqlx::query(
        "SELECT w.path FROM stories s JOIN workspaces w ON w.id = s.workspace_id WHERE s.id = ?",
    )
    .bind(&iso.story_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    let from_story: Option<String> = row.and_then(|r| r.try_get("path").ok());
    from_story
        .map(std::path::PathBuf::from)
        .or(db::get_active_workspace_path(db).await)
        .ok_or_else(|| {
            "Could not work out which repository this run belongs to — its worktree is gone and \
             its workspace is no longer registered."
                .to_string()
        })
}

async fn set_isolation_status(run_id: &str, status: &str, note: &str, db: &DbPool) {
    let _ = sqlx::query("UPDATE story_runs SET isolation_status = ?, isolation_note = ? WHERE id = ?")
        .bind(status)
        .bind(note)
        .bind(run_id)
        .execute(db)
        .await;
}

/// Bring a finished run's changes into the user's working tree.
///
/// The merge is a `git merge --squash`, so the changes land staged and
/// uncommitted for the user to review, and git aborts rather than overwriting
/// uncommitted local work. Nothing is cleaned up unless the merge succeeded:
/// a failed accept leaves the worktree and branch exactly where they were, so
/// it can be retried or reverted instead.
pub async fn accept_run(run_id: String, db: &DbPool) -> Result<String, String> {
    let iso = load_isolation(&run_id, db).await?;
    let main = main_repo_for(&iso, db).await?;

    runtime::worktree::apply_to_main(&main, &iso.branch_name)?;

    // Only now that the changes are safely in the user's tree.
    let cleanup = cleanup_worktree(&main, &iso);
    let note = format!(
        "Accepted into {} from branch '{}'. The changes are staged but not committed.{}",
        main.display(),
        iso.branch_name,
        cleanup
            .as_ref()
            .map(|e| format!(" Cleanup warning: {e}"))
            .unwrap_or_default()
    );
    set_isolation_status(&run_id, "accepted", &note, db).await;
    Ok(note)
}

/// Throw a finished run's changes away.
///
/// This deletes the run's own worktree and its own branch, and nothing else.
/// The user's working tree is never read, written, reset, or cleaned — the run
/// never wrote there in the first place, which is what makes the undo exact
/// rather than best-effort.
pub async fn revert_run(run_id: String, db: &DbPool) -> Result<String, String> {
    let iso = load_isolation(&run_id, db).await?;
    let main = main_repo_for(&iso, db).await?;

    if let Some(error) = cleanup_worktree(&main, &iso) {
        return Err(error);
    }

    let note = format!(
        "Reverted: worktree '{}' and branch '{}' were deleted. Your working tree was not touched.",
        iso.worktree_path, iso.branch_name
    );
    set_isolation_status(&run_id, "reverted", &note, db).await;
    Ok(note)
}

/// Remove the run's worktree and branch. Returns the first error, if any.
fn cleanup_worktree(main: &std::path::Path, iso: &RunIsolation) -> Option<String> {
    debug_assert_eq!(iso.status, runtime::worktree::STATUS_ISOLATED);
    debug_assert_ne!(iso.run_status, "running");
    let worktree = std::path::Path::new(&iso.worktree_path);
    runtime::worktree::remove(main, worktree)
        .err()
        .or_else(|| runtime::worktree::delete_branch(main, &iso.branch_name).err())
}

// ---------------------------------------------------------------------------
// Startup sweep
// ---------------------------------------------------------------------------

/// Delete worktree directories that no run in the database claims.
///
/// Called once at startup. A run that finished but has not been accepted or
/// reverted still claims its worktree and is left alone — the user has not
/// decided about it yet, and the whole point of keeping it is that they can.
pub async fn sweep_orphaned_worktrees(
    worktrees_dir: &std::path::Path,
    db: &DbPool,
) -> Result<usize, String> {
    // Only a run still in the `isolated` state claims its directory. Accepted
    // and reverted runs keep `worktree_path` for the record, but the directory
    // it names is meant to be gone — if a cleanup half-failed, the sweep is
    // what finishes the job.
    let rows = sqlx::query(
        "SELECT worktree_path FROM story_runs          WHERE worktree_path IS NOT NULL AND isolation_status = 'isolated'",
    )
        .fetch_all(db)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    let claimed: std::collections::HashSet<String> = rows
        .iter()
        .filter_map(|r| r.try_get::<String, _>("worktree_path").ok())
        .collect();

    let swept = runtime::worktree::sweep_orphans(worktrees_dir, &claimed);
    for entry in &swept {
        match &entry.error {
            Some(e) => tracing::warn!("Could not sweep worktree '{}': {e}", entry.path.display()),
            None => tracing::info!("Swept orphaned worktree '{}'", entry.path.display()),
        }
    }
    Ok(swept.iter().filter(|e| e.error.is_none()).count())
}
