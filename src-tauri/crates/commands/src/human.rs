// Tauri commands for the human-in-the-loop feature.
//
// Covers two flows:
//   1. Human input requests — agent creates a 'human' story and waits for a
//      text response from the user.
//   2. Approval gates — runs with `requires_approval=true` pause before each
//      tool call; the user approves or rejects via `decide_approval`.

use db::DbPool;
use runtime::ApprovalGate;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A pending human input request (story_type = 'human').
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanRequest {
    pub id: String,
    /// The human-type story id.
    pub story_id: String,
    /// Title of the human story (also shown as the subject line).
    pub story_title: String,
    /// Which run is paused waiting for this input.
    pub run_id: Option<String>,
    /// The agent's question / context for the user.
    pub question: Option<String>,
    /// 'backlog' | 'ready' | 'in_progress' — anything except done/failed means pending.
    pub status: String,
    pub created_at: String,
}

/// A pending tool-call approval request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub run_id: String,
    /// Friendly story title for context (from JOIN).
    pub story_title: Option<String>,
    pub tool_name: String,
    /// JSON object of tool inputs.
    pub tool_input: String,
    /// 'pending' | 'approved' | 'rejected'
    pub status: String,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// get_pending_human_requests
// ---------------------------------------------------------------------------

/// Return all human-type stories that have not yet been answered.
pub async fn get_pending_human_requests(
    db: State<'_, DbPool>,
) -> Result<Vec<HumanRequest>, String> {
    let rows = sqlx::query(
        "SELECT s.id, s.title, s.status, s.parent_run_id, s.human_question, s.created_at
         FROM stories s
         WHERE s.story_type = 'human'
           AND s.status NOT IN ('done', 'failed')
         ORDER BY s.created_at DESC",
    )
    .fetch_all(db.inner())
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    Ok(rows
        .iter()
        .map(|r| HumanRequest {
            id: r.try_get("id").unwrap_or_default(),
            story_id: r.try_get("id").unwrap_or_default(),
            story_title: r.try_get("title").unwrap_or_default(),
            run_id: r.try_get("parent_run_id").ok().flatten(),
            question: r.try_get("human_question").ok().flatten(),
            status: r.try_get("status").unwrap_or_default(),
            created_at: r.try_get("created_at").unwrap_or_default(),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// respond_to_human_request
// ---------------------------------------------------------------------------

/// Submit the user's text reply to a pending human-type story.
///
/// Side effects:
/// - Sets `human_response` on the story and moves it to `done`.
/// - If a `parent_run_id` exists, emits `human-response` on the Tauri event
///   bus so the runtime can resume the stalled run.
pub async fn respond_to_human_request(
    story_id: String,
    response: String,
    app: AppHandle,
    db: State<'_, DbPool>,
) -> Result<(), String> {
    // Fetch the parent_run_id before updating.
    let row = sqlx::query(
        "SELECT parent_run_id FROM stories WHERE id = ? AND story_type = 'human'",
    )
    .bind(&story_id)
    .fetch_optional(db.inner())
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    let parent_run_id: Option<String> = row
        .as_ref()
        .and_then(|r| r.try_get("parent_run_id").ok().flatten());

    sqlx::query(
        "UPDATE stories
         SET human_response = ?,
             status = 'done',
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ? AND story_type = 'human'",
    )
    .bind(&response)
    .bind(&story_id)
    .execute(db.inner())
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    // Notify the runtime so it can resume the paused run.
    if let Some(run_id) = &parent_run_id {
        let payload = serde_json::json!({ "runId": run_id, "response": response });
        app.emit("human-response", payload)
            .map_err(|e| format!("Event emit error: {e}"))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// create_human_request
// ---------------------------------------------------------------------------

/// Create a new human-type story (called by the agent runtime via the
/// `request_human_input` built-in tool).
///
/// Returns the story id.
pub async fn create_human_request(
    run_id: String,
    question: String,
    context: Option<String>,
    app: AppHandle,
    db: State<'_, DbPool>,
) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let title = if question.len() > 80 {
        format!("{}…", &question[..77])
    } else {
        question.clone()
    };
    let description = context;

    sqlx::query(
        "INSERT INTO stories
             (id, title, description, story_type, status, priority,
              parent_run_id, human_question)
         VALUES (?, ?, ?, 'human', 'ready', 'critical', ?, ?)",
    )
    .bind(&id)
    .bind(&title)
    .bind(&description)
    .bind(&run_id)
    .bind(&question)
    .execute(db.inner())
    .await
    .map_err(|e| format!("DB insert error: {e}"))?;

    // Notify the frontend so it can show a badge / notification.
    let payload = serde_json::json!({ "storyId": id, "runId": run_id, "question": question });
    app.emit("human-request-created", payload)
        .map_err(|e| format!("Event emit error: {e}"))?;

    Ok(id)
}

// ---------------------------------------------------------------------------
// get_pending_approvals
// ---------------------------------------------------------------------------

/// Return approval requests that are still pending.
pub async fn get_pending_approvals(
    db: State<'_, DbPool>,
) -> Result<Vec<ApprovalRequest>, String> {
    let rows = sqlx::query(
        "SELECT ar.id, ar.run_id, ar.tool_name, ar.tool_input, ar.status, ar.created_at,
                s.title AS story_title
         FROM approval_requests ar
         LEFT JOIN story_runs sr ON sr.id = ar.run_id
         LEFT JOIN stories s    ON s.id  = sr.story_id
         WHERE ar.status = 'pending'
         ORDER BY ar.created_at ASC",
    )
    .fetch_all(db.inner())
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    Ok(rows
        .iter()
        .map(|r| ApprovalRequest {
            id: r.try_get("id").unwrap_or_default(),
            run_id: r.try_get("run_id").unwrap_or_default(),
            story_title: r.try_get("story_title").ok().flatten(),
            tool_name: r.try_get("tool_name").unwrap_or_default(),
            tool_input: r
                .try_get("tool_input")
                .unwrap_or_else(|_| "{}".to_string()),
            status: r.try_get("status").unwrap_or_default(),
            created_at: r.try_get("created_at").unwrap_or_default(),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// create_approval_request
// ---------------------------------------------------------------------------

/// Called by the agent runtime before executing a tool when
/// `requires_approval=true`.  Returns the approval_request id.
pub async fn create_approval_request(
    run_id: String,
    tool_name: String,
    tool_input: String,
    app: AppHandle,
    db: State<'_, DbPool>,
) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO approval_requests (id, run_id, tool_name, tool_input)
         VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&run_id)
    .bind(&tool_name)
    .bind(&tool_input)
    .execute(db.inner())
    .await
    .map_err(|e| format!("DB insert error: {e}"))?;

    let payload = serde_json::json!({
        "approvalRequestId": id,
        "runId": run_id,
        "toolName": tool_name,
    });
    app.emit("approval-request-created", payload)
        .map_err(|e| format!("Event emit error: {e}"))?;

    Ok(id)
}

// ---------------------------------------------------------------------------
// decide_approval
// ---------------------------------------------------------------------------

/// Record the user's approval or rejection of a tool-call gate.
///
/// Emits `approval-decision` on the Tauri event bus and wakes the runtime
/// task that is waiting on the `ApprovalGate` channel.
pub async fn decide_approval(
    id: String,
    approved: bool,
    rejection_reason: Option<String>,
    app: AppHandle,
    db: State<'_, DbPool>,
    gate: State<'_, std::sync::Arc<ApprovalGate>>,
) -> Result<(), String> {
    let status = if approved { "approved" } else { "rejected" };

    sqlx::query(
        "UPDATE approval_requests
         SET status = ?,
             rejection_reason = ?,
             decided_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?",
    )
    .bind(status)
    .bind(&rejection_reason)
    .bind(&id)
    .execute(db.inner())
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    // Fetch run_id so the runtime knows which run to resume.
    let row = sqlx::query("SELECT run_id FROM approval_requests WHERE id = ?")
        .bind(&id)
        .fetch_optional(db.inner())
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    if let Some(r) = row {
        let run_id: String = r.try_get("run_id").unwrap_or_default();
        let payload = serde_json::json!({
            "approvalRequestId": id,
            "runId": run_id,
            "approved": approved,
            "rejectionReason": rejection_reason,
        });
        app.emit("approval-decision", payload)
            .map_err(|e| format!("Event emit error: {e}"))?;
    }

    // Wake the runtime task waiting on the gate channel.
    gate.resolve(&id, approved);

    Ok(())
}
