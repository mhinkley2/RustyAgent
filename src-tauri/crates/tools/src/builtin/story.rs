use async_trait::async_trait;
use serde_json::json;
use sqlx::Row;
use std::path::Path;
use crate::{Tool, ToolContext, ToolOutput};

async fn resolve_workspace_id(ctx: &ToolContext) -> Option<String> {
    let root = ctx.workspace_root.as_ref()?;
    let raw = root.to_string_lossy();
    let normalized = raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_string();

    let existing = sqlx::query(
        "SELECT id FROM workspaces WHERE path = ? ORDER BY last_opened_at DESC LIMIT 1"
    )
    .bind(&normalized)
    .fetch_optional(&ctx.db)
    .await
    .ok()?;

    if let Some(row) = existing {
        return row.try_get::<String, _>("id").ok();
    }

    let workspace_id = uuid::Uuid::new_v4().to_string();
    let workspace_name = Path::new(&normalized)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(&normalized)
        .to_string();

    let upserted = sqlx::query(
        "INSERT INTO workspaces (id, path, name) VALUES (?, ?, ?)
         ON CONFLICT(path) DO UPDATE SET
            name = excluded.name,
            last_opened_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')"
    )
    .bind(&workspace_id)
    .bind(&normalized)
    .bind(&workspace_name)
    .execute(&ctx.db)
    .await
    .ok()?;

    if upserted.rows_affected() == 0 {
        return None;
    }

    let row = sqlx::query(
        "SELECT id FROM workspaces WHERE path = ? ORDER BY last_opened_at DESC LIMIT 1"
    )
    .bind(&normalized)
    .fetch_optional(&ctx.db)
    .await
    .ok()??;

    row.try_get::<String, _>("id").ok()
}

// ---------------------------------------------------------------------------
// get_story
// ---------------------------------------------------------------------------

pub struct GetStoryTool;

#[async_trait]
impl Tool for GetStoryTool {
    fn name(&self) -> &str { "get_story" }

    fn description(&self) -> &str {
        "Read a story by ID. Returns title, description, status, and priority."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "story_id": { "type": "string", "description": "UUID of the story to retrieve" }
            },
            "required": ["story_id"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolOutput {
        let story_id = match input.get("story_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => return ToolOutput::err("Missing required field: story_id"),
        };

        let result = sqlx::query(
            "SELECT id, title, description, status, priority, story_type, labels, assigned_agent_id \
             FROM stories WHERE id = ?"
        )
        .bind(&story_id)
        .fetch_optional(&ctx.db)
        .await;

        match result {
            Ok(Some(row)) => {
                let id: String = row.try_get("id").unwrap_or_default();
                let title: String = row.try_get("title").unwrap_or_default();
                let description: Option<String> = row.try_get("description").ok().flatten();
                let status: String = row.try_get("status").unwrap_or_default();
                let priority: String = row.try_get("priority").unwrap_or_default();
                let story_type: String = row.try_get("story_type").unwrap_or_default();
                let labels_json: String = row.try_get("labels").unwrap_or_else(|_| "[]".to_string());
                let labels: Vec<String> = serde_json::from_str(&labels_json).unwrap_or_default();
                let assigned_agent_id: Option<String> = row.try_get("assigned_agent_id").ok().flatten();
                ToolOutput::ok(serde_json::to_string(&json!({
                    "id": id,
                    "title": title,
                    "description": description,
                    "status": status,
                    "priority": priority,
                    "story_type": story_type,
                    "labels": labels,
                    "assigned_agent_id": assigned_agent_id,
                })).unwrap_or_default())
            }
            Ok(None) => ToolOutput::err(format!("Story {story_id} not found")),
            Err(e) => ToolOutput::err(format!("DB error: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// list_stories
// ---------------------------------------------------------------------------

pub struct ListStoriesTool;

#[async_trait]
impl Tool for ListStoriesTool {
    fn name(&self) -> &str { "list_stories" }

    fn description(&self) -> &str {
        "List all stories on the board. Returns id, title, status, priority, and assigned agent for each story. \
         Use this to discover story IDs before calling get_story or update_story_status."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "description": format!(
                        "Optional filter by status ({}). Omit for all.",
                        db::story_status::status_list_prose()
                    ),
                    "enum": db::story_status::status_enum_json()
                }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolOutput {
        let status_filter = input.get("status").and_then(|v| v.as_str()).map(|s| s.to_string());
        let workspace_id = resolve_workspace_id(ctx).await;

        let result = match (status_filter.as_ref(), workspace_id.as_ref()) {
            (Some(status), Some(ws_id)) => {
                sqlx::query(
                    "SELECT s.id, s.title, s.status, s.priority, s.story_type, s.description, \
                            s.labels, a.name AS agent_name \
                     FROM stories s \
                     LEFT JOIN agent_profiles a ON a.id = s.assigned_agent_id \
                     WHERE s.story_type != 'chat' AND s.status = ? \
                       AND (s.workspace_id = ? OR s.workspace_id IS NULL) \
                     ORDER BY s.sort_order ASC, s.created_at ASC",
                )
                .bind(status)
                .bind(ws_id)
                .fetch_all(&ctx.db)
                .await
            }
            (Some(status), None) => {
                sqlx::query(
                    "SELECT s.id, s.title, s.status, s.priority, s.story_type, s.description, \
                            s.labels, a.name AS agent_name \
                     FROM stories s \
                     LEFT JOIN agent_profiles a ON a.id = s.assigned_agent_id \
                     WHERE s.story_type != 'chat' AND s.status = ? \
                       AND s.workspace_id IS NULL \
                     ORDER BY s.sort_order ASC, s.created_at ASC",
                )
                .bind(status)
                .fetch_all(&ctx.db)
                .await
            }
            (None, Some(ws_id)) => {
                sqlx::query(
                    "SELECT s.id, s.title, s.status, s.priority, s.story_type, s.description, \
                            s.labels, a.name AS agent_name \
                     FROM stories s \
                     LEFT JOIN agent_profiles a ON a.id = s.assigned_agent_id \
                     WHERE s.story_type != 'chat' \
                       AND (s.workspace_id = ? OR s.workspace_id IS NULL) \
                     ORDER BY s.sort_order ASC, s.created_at ASC",
                )
                .bind(ws_id)
                .fetch_all(&ctx.db)
                .await
            }
            (None, None) => {
                sqlx::query(
                    "SELECT s.id, s.title, s.status, s.priority, s.story_type, s.description, \
                            s.labels, a.name AS agent_name \
                     FROM stories s \
                     LEFT JOIN agent_profiles a ON a.id = s.assigned_agent_id \
                     WHERE s.story_type != 'chat' \
                       AND s.workspace_id IS NULL \
                     ORDER BY s.sort_order ASC, s.created_at ASC",
                )
                .fetch_all(&ctx.db)
                .await
            }
        };

        match result {
            Ok(rows) => {
                let stories: Vec<serde_json::Value> = rows.iter().map(|row| {
                    let labels_json: String = row.try_get("labels").unwrap_or_else(|_| "[]".to_string());
                    let labels: Vec<String> = serde_json::from_str(&labels_json).unwrap_or_default();
                    json!({
                        "id": row.try_get::<String, _>("id").unwrap_or_default(),
                        "title": row.try_get::<String, _>("title").unwrap_or_default(),
                        "status": row.try_get::<String, _>("status").unwrap_or_default(),
                        "priority": row.try_get::<String, _>("priority").unwrap_or_default(),
                        "story_type": row.try_get::<String, _>("story_type").unwrap_or_default(),
                        "description": row.try_get::<Option<String>, _>("description").ok().flatten(),
                        "labels": labels,
                        "assigned_agent": row.try_get::<Option<String>, _>("agent_name").ok().flatten(),
                    })
                }).collect();
                ToolOutput::ok(serde_json::to_string(&json!({ "stories": stories, "count": stories.len() })).unwrap_or_default())
            }
            Err(e) => ToolOutput::err(format!("DB error: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// create_story
// ---------------------------------------------------------------------------

pub struct CreateStoryTool;

#[async_trait]
impl Tool for CreateStoryTool {
    fn name(&self) -> &str { "create_story" }

    fn description(&self) -> &str {
        "Create a new story on the board. Returns the created story's id and title."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Story title (required)" },
                "description": { "type": "string", "description": "Detailed description (optional)" },
                "story_type": {
                    "type": "string",
                    "description": "Story type. Defaults to 'task'.",
                    "enum": ["task", "human", "pipeline"]
                },
                "status": {
                    "type": "string",
                    "description": "Initial status. Defaults to 'backlog'.",
                    "enum": db::story_status::status_enum_json()
                },
                "priority": {
                    "type": "string",
                    "description": "Priority level. Defaults to 'medium'.",
                    "enum": ["low", "medium", "high", "critical"]
                },
                "labels": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of label strings."
                },
                "assigned_agent_id": {
                    "type": "string",
                    "description": "Optional agent profile UUID to assign the story to."
                },
                "requires_approval": {
                    "type": "boolean",
                    "description": "Whether write actions for this story require approval."
                },
                "track_history": {
                    "type": "boolean",
                    "description": "Whether to retain run history for the story."
                }
            },
            "required": ["title"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolOutput {
        let title = match input.get("title").and_then(|v| v.as_str()) {
            Some(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => return ToolOutput::err("Missing required field: title"),
        };
        let description = input.get("description").and_then(|v| v.as_str()).map(|s| s.to_string());
        let story_type = input.get("story_type").and_then(|v| v.as_str()).unwrap_or("task").to_string();
        let status   = input.get("status").and_then(|v| v.as_str()).unwrap_or("backlog").to_string();
        if let Err(e) = db::story_status::validate_status(&status) {
            return ToolOutput::err(e);
        }
        let priority = input.get("priority").and_then(|v| v.as_str()).unwrap_or("medium").to_string();
        let labels: Vec<String> = input.get("labels")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect())
            .unwrap_or_default();
        let labels_json = serde_json::to_string(&labels).unwrap_or_else(|_| "[]".to_string());
        let assigned_agent_id = input
            .get("assigned_agent_id")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let requires_approval = input.get("requires_approval").and_then(|v| v.as_bool()).unwrap_or(false);
        let track_history = input.get("track_history").and_then(|v| v.as_bool()).unwrap_or(true);
        let workspace_id = resolve_workspace_id(ctx).await;

        let id = uuid::Uuid::new_v4().to_string();

        // Compute max sort_order so the new story lands at the bottom for this workspace.
        let max_sort: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sort_order), -1) FROM stories WHERE story_type != 'chat' AND workspace_id IS ?"
        )
            .bind(&workspace_id)
            .fetch_one(&ctx.db)
            .await
            .unwrap_or(-1);
        let sort_order = max_sort + 1;

        let result = sqlx::query(
            "INSERT INTO stories
                (id, title, description, story_type, status, priority, assigned_agent_id,
                 requires_approval, track_history, labels, sort_order, workspace_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&id)
        .bind(&title)
        .bind(&description)
        .bind(&story_type)
        .bind(&status)
        .bind(&priority)
        .bind(&assigned_agent_id)
        .bind(requires_approval as i64)
        .bind(track_history as i64)
        .bind(&labels_json)
        .bind(sort_order)
        .bind(&workspace_id)
        .execute(&ctx.db)
        .await;

        match result {
            Ok(_) => ToolOutput::ok(serde_json::to_string(&json!({
                "id": id,
                "title": title,
                "story_type": story_type,
                "status": status,
                "priority": priority,
            })).unwrap_or_default()),
            Err(e) => ToolOutput::err(format!("DB error creating story: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// update_story
// ---------------------------------------------------------------------------

pub struct UpdateStoryTool;

#[async_trait]
impl Tool for UpdateStoryTool {
    fn name(&self) -> &str { "update_story" }

    fn description(&self) -> &str {
        "Update a story's title, description, priority, or labels. \
         To change only the status use update_story_status instead."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "story_id": { "type": "string", "description": "UUID of the story to update" },
                "title": { "type": "string" },
                "description": { "type": "string" },
                "story_type": {
                    "type": "string",
                    "enum": ["task", "human", "pipeline"]
                },
                "status": {
                    "type": "string",
                    "enum": db::story_status::status_enum_json()
                },
                "priority": {
                    "type": "string",
                    "enum": ["low", "medium", "high", "critical"]
                },
                "labels": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "assigned_agent_id": {
                    "type": ["string", "null"],
                    "description": "Set to a UUID string to assign, or null / empty string to clear."
                },
                "requires_approval": {
                    "type": "boolean"
                },
                "track_history": {
                    "type": "boolean"
                }
            },
            "required": ["story_id"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolOutput {
        let story_id = match input.get("story_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => return ToolOutput::err("Missing required field: story_id"),
        };

        let current = match sqlx::query(
            "SELECT title, description, story_type, status, priority, assigned_agent_id,
                    requires_approval, track_history, labels
             FROM stories WHERE id = ?"
        )
        .bind(&story_id)
        .fetch_optional(&ctx.db)
        .await {
            Ok(Some(row)) => row,
            Ok(None) => return ToolOutput::err(format!("Story {story_id} not found")),
            Err(e) => return ToolOutput::err(format!("DB error: {e}")),
        };

        let current_title: String = current.try_get("title").unwrap_or_default();
        let current_description: Option<String> = current.try_get("description").ok().flatten();
        let current_story_type: String = current.try_get("story_type").unwrap_or_else(|_| "task".to_string());
        let current_status: String = current.try_get("status").unwrap_or_else(|_| "backlog".to_string());
        let current_priority: String = current.try_get("priority").unwrap_or_else(|_| "medium".to_string());
        let current_assigned_agent_id: Option<String> = current.try_get("assigned_agent_id").ok().flatten();
        let current_requires_approval = current.try_get::<i64, _>("requires_approval").unwrap_or(0) != 0;
        let current_track_history = current.try_get::<i64, _>("track_history").unwrap_or(1) != 0;
        let current_labels_json: String = current.try_get("labels").unwrap_or_else(|_| "[]".to_string());

        let title = input.get("title").and_then(|v| v.as_str()).map(|value| value.to_string()).unwrap_or(current_title);
        let description = match input.get("description") {
            Some(value) if value.is_null() => None,
            Some(value) => value.as_str().map(|text| text.to_string()),
            None => current_description,
        };
        let story_type = input.get("story_type").and_then(|v| v.as_str()).map(|value| value.to_string()).unwrap_or(current_story_type);
        // Validate exactly what the caller supplied, then fall back — rather
        // than falling back first and validating whatever came out.
        //
        // The earlier form gated on `input.get("status").is_some()`, which is
        // true for `"status": null` as well: `as_str()` then yields `None`,
        // the fallback substitutes the row's *current* status, and that gets
        // validated. On a row carrying a value from before the vocabulary was
        // settled, an update that never mentioned the status would be refused
        // because of it.
        //
        // A status already in the row is not the caller's doing, and refusing
        // to change a title because of it would be the wrong trade.
        // Validate exactly what the caller supplied, then fall back — rather
        // than falling back first and validating whatever came out.
        //
        // The earlier form gated on `input.get("status").is_some()`, which is
        // true for `"status": null` as well: `as_str()` then yields `None`,
        // the fallback substitutes the row's *current* status, and that gets
        // validated. On a row carrying a value from before the vocabulary was
        // settled, an update that never mentioned the status would be refused
        // because of it.
        //
        // A status already in the row is not the caller's doing, and refusing
        // to change a title because of it would be the wrong trade.
        let supplied_status = input.get("status").and_then(|v| v.as_str());
        if let Some(supplied) = supplied_status {
            if let Err(e) = db::story_status::validate_status(supplied) {
                return ToolOutput::err(e);
            }
        }
        let status = supplied_status.map(|value| value.to_string()).unwrap_or(current_status);
        let priority = input.get("priority").and_then(|v| v.as_str()).map(|value| value.to_string()).unwrap_or(current_priority);
        let labels_json = match input.get("labels") {
            Some(value) => {
                let labels: Vec<String> = value
                    .as_array()
                    .map(|items| items.iter().filter_map(|item| item.as_str()).map(|text| text.to_string()).collect())
                    .unwrap_or_default();
                serde_json::to_string(&labels).unwrap_or_else(|_| "[]".to_string())
            }
            None => current_labels_json,
        };
        let assigned_agent_id = match input.get("assigned_agent_id") {
            Some(value) if value.is_null() => None,
            Some(value) => value
                .as_str()
                .map(|text| text.trim().to_string())
                .filter(|text| !text.is_empty()),
            None => current_assigned_agent_id,
        };
        let requires_approval = input
            .get("requires_approval")
            .and_then(|v| v.as_bool())
            .unwrap_or(current_requires_approval);
        let track_history = input
            .get("track_history")
            .and_then(|v| v.as_bool())
            .unwrap_or(current_track_history);

        let has_changes = input.get("title").is_some()
            || input.get("description").is_some()
            || input.get("story_type").is_some()
            || input.get("status").is_some()
            || input.get("priority").is_some()
            || input.get("labels").is_some()
            || input.get("assigned_agent_id").is_some()
            || input.get("requires_approval").is_some()
            || input.get("track_history").is_some();

        if !has_changes {
            return ToolOutput::err("No fields to update — provide at least one mutable story field");
        }

        let workspace_id = resolve_workspace_id(ctx).await;

        match sqlx::query(
            "UPDATE stories SET
                 title = ?,
                 description = ?,
                 story_type = ?,
                 status = ?,
                 priority = ?,
                 assigned_agent_id = ?,
                 requires_approval = ?,
                 track_history = ?,
                 labels = ?,
                 workspace_id = CASE WHEN workspace_id IS NULL THEN ? ELSE workspace_id END,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?"
        )
        .bind(&title)
        .bind(&description)
        .bind(&story_type)
        .bind(&status)
        .bind(&priority)
        .bind(&assigned_agent_id)
        .bind(requires_approval as i64)
        .bind(track_history as i64)
        .bind(&labels_json)
        .bind(&workspace_id)
        .bind(&story_id)
        .execute(&ctx.db)
        .await {
            Ok(r) if r.rows_affected() > 0 => ToolOutput::ok(format!("Story {story_id} updated")),
            Ok(_) => ToolOutput::err(format!("Story {story_id} not found")),
            Err(e) => ToolOutput::err(format!("DB error: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// update_story_status
// ---------------------------------------------------------------------------

pub struct UpdateStoryStatusTool;

#[async_trait]
impl Tool for UpdateStoryStatusTool {
    fn name(&self) -> &str { "update_story_status" }

    fn description(&self) -> &str {
        // Derived, not written out. This string is prompt text the model
        // reasons from: the version it replaces named `failed` — which is now
        // refused — and omitted `review` and `backlog`, so an agent reading it
        // would try the one status that cannot work and never learn about the
        // column every finished run lands in.
        static DESCRIPTION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        DESCRIPTION.get_or_init(|| {
            format!(
                "Update the status of a story. Valid statuses: {}.",
                db::story_status::status_list_prose()
            )
        })
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "story_id": { "type": "string", "description": "UUID of the story" },
                "status": {
                    "type": "string",
                    "enum": db::story_status::status_enum_json()
                }
            },
            "required": ["story_id", "status"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolOutput {
        let story_id = match input.get("story_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => return ToolOutput::err("Missing required field: story_id"),
        };
        let status = match input.get("status").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return ToolOutput::err("Missing required field: status"),
        };
        // The schema's `enum` is advisory — nothing validates a tool call
        // against it — so the check that actually holds is this one.
        if let Err(e) = db::story_status::validate_status(&status) {
            return ToolOutput::err(e);
        }
        let workspace_id = resolve_workspace_id(ctx).await;

        let result = sqlx::query(
            "UPDATE stories SET
                 status = ?,
                 workspace_id = CASE WHEN workspace_id IS NULL THEN ? ELSE workspace_id END,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?"
        )
        .bind(&status)
        .bind(&workspace_id)
        .bind(&story_id)
        .execute(&ctx.db)
        .await;

        match result {
            Ok(r) if r.rows_affected() > 0 => ToolOutput::ok(format!("Story {story_id} status updated to {status}")),
            Ok(_) => ToolOutput::err(format!("Story {story_id} not found")),
            Err(e) => ToolOutput::err(format!("DB error: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// delete_story
// ---------------------------------------------------------------------------

pub struct DeleteStoryTool;

#[async_trait]
impl Tool for DeleteStoryTool {
    fn name(&self) -> &str { "delete_story" }

    fn description(&self) -> &str {
        "Delete a story from the board by ID."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "story_id": { "type": "string", "description": "UUID of the story to delete" }
            },
            "required": ["story_id"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolOutput {
        let story_id = match input.get("story_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => return ToolOutput::err("Missing required field: story_id"),
        };

        match sqlx::query("DELETE FROM stories WHERE id = ?")
            .bind(&story_id)
            .execute(&ctx.db)
            .await {
            Ok(result) if result.rows_affected() > 0 => ToolOutput::ok(format!("Story {story_id} deleted")),
            Ok(_) => ToolOutput::err(format!("Story {story_id} not found")),
            Err(e) => ToolOutput::err(format!("DB error: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{make_ctx, make_test_pool};

    // -----------------------------------------------------------------------
    // The status vocabulary
    // -----------------------------------------------------------------------

    async fn seed_story(db: &db::DbPool, id: &str) {
        sqlx::query("INSERT INTO stories (id, title, status) VALUES (?, 'A story', 'ready')")
            .bind(id)
            .execute(db)
            .await
            .expect("seed story");
    }

    async fn status_of(db: &db::DbPool, id: &str) -> String {
        sqlx::query_scalar("SELECT status FROM stories WHERE id = ?")
            .bind(id)
            .fetch_one(db)
            .await
            .expect("read status")
    }

    /// The defect this story exists for: every finished run lands its card in
    /// `review`, and no agent could move one out of there — or into it —
    /// because the tool's vocabulary did not have the word.
    #[tokio::test]
    async fn an_agent_can_move_a_story_to_review() {
        let db = make_test_pool().await;
        seed_story(&db, "s1").await;
        let ctx = make_ctx(db.clone());

        let r = UpdateStoryStatusTool
            .execute(json!({"story_id": "s1", "status": "review"}), &ctx)
            .await;

        assert!(!r.is_error, "{}", r.content);
        assert_eq!(status_of(&db, "s1").await, "review");
    }

    #[tokio::test]
    async fn an_agent_can_send_a_story_back_to_the_backlog() {
        let db = make_test_pool().await;
        seed_story(&db, "s1").await;
        let ctx = make_ctx(db.clone());

        let r = UpdateStoryStatusTool
            .execute(json!({"story_id": "s1", "status": "backlog"}), &ctx)
            .await;

        assert!(!r.is_error, "{}", r.content);
        assert_eq!(status_of(&db, "s1").await, "backlog");
    }

    /// `failed` was accepted here and rendered by no column, so a card set to
    /// it left the board entirely.
    #[tokio::test]
    async fn a_status_the_board_cannot_draw_is_refused() {
        let db = make_test_pool().await;
        seed_story(&db, "s1").await;
        let ctx = make_ctx(db.clone());

        let r = UpdateStoryStatusTool
            .execute(json!({"story_id": "s1", "status": "failed"}), &ctx)
            .await;

        assert!(r.is_error, "a card in no column is worse than a refused call");
        assert!(r.content.contains("blocked"), "the error should name the alternatives: {}", r.content);
        assert_eq!(status_of(&db, "s1").await, "ready", "and nothing was written");
    }

    /// A JSON-schema `enum` is advisory. Nothing validates a call against it,
    /// so the refusal has to happen in the tool.
    #[tokio::test]
    async fn a_nonsense_status_is_refused_rather_than_stored() {
        let db = make_test_pool().await;
        seed_story(&db, "s1").await;
        let ctx = make_ctx(db.clone());

        let r = UpdateStoryStatusTool
            .execute(json!({"story_id": "s1", "status": "whatever"}), &ctx)
            .await;

        assert!(r.is_error);
        assert_eq!(status_of(&db, "s1").await, "ready");
    }

    #[tokio::test]
    async fn creating_a_story_with_an_unknown_status_is_refused() {
        let db = make_test_pool().await;
        let ctx = make_ctx(db.clone());

        let r = CreateStoryTool
            .execute(json!({"title": "New", "status": "failed"}), &ctx)
            .await;

        assert!(r.is_error, "{}", r.content);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stories")
            .fetch_one(&db)
            .await
            .expect("count");
        assert_eq!(count, 0, "nothing should have been created");
    }

    #[tokio::test]
    async fn updating_a_story_with_an_unknown_status_is_refused() {
        let db = make_test_pool().await;
        seed_story(&db, "s1").await;
        let ctx = make_ctx(db.clone());

        let r = UpdateStoryTool
            .execute(json!({"story_id": "s1", "status": "failed"}), &ctx)
            .await;

        assert!(r.is_error, "{}", r.content);
        assert_eq!(status_of(&db, "s1").await, "ready");
    }

    /// `"status": null` is not the caller supplying a status, and must not be
    /// treated as one. The earlier guard checked for the *key*, so a null
    /// fell through to validating the row's own stored value — refusing an
    /// unrelated edit on a legacy row for a status the caller never sent.
    #[tokio::test]
    async fn an_explicit_null_status_is_not_a_supplied_status() {
        let db = make_test_pool().await;
        sqlx::query("INSERT INTO stories (id, title, status) VALUES ('s1', 'A story', 'failed')")
            .execute(&db)
            .await
            .expect("seed");
        let ctx = make_ctx(db.clone());

        let r = UpdateStoryTool
            .execute(
                json!({"story_id": "s1", "title": "Renamed", "status": serde_json::Value::Null}),
                &ctx,
            )
            .await;

        assert!(!r.is_error, "a null status must not refuse the edit: {}", r.content);
        assert_eq!(
            status_of(&db, "s1").await,
            "failed",
            "and the stored value is left as it was"
        );
    }

    /// An update that does not mention the status must still work on a row
    /// whose stored status predates the vocabulary being settled.
    #[tokio::test]
    async fn a_legacy_status_does_not_block_an_unrelated_update() {
        let db = make_test_pool().await;
        sqlx::query("INSERT INTO stories (id, title, status) VALUES ('s1', 'A story', 'failed')")
            .execute(&db)
            .await
            .expect("seed");
        let ctx = make_ctx(db.clone());

        let r = UpdateStoryTool
            .execute(json!({"story_id": "s1", "title": "Renamed"}), &ctx)
            .await;

        assert!(!r.is_error, "{}", r.content);
    }

    /// A tool description is prompt text the model reasons from, so a stale
    /// one does not merely mislead a reader — it teaches the agent a
    /// vocabulary that will be refused. The version this replaced named
    /// `failed` and omitted `review`, which is the one status every finished
    /// run lands in.
    #[test]
    fn the_status_tools_description_names_the_statuses_that_work() {
        let description = UpdateStoryStatusTool.description();

        for status in db::story_status::STORY_STATUSES {
            assert!(
                description.contains(status),
                "the description should name {status}: {description}"
            );
        }
        assert!(
            !description.contains("failed"),
            "the description must not offer a status that is refused: {description}"
        );
    }

    /// Every tool that names statuses names the same ones.
    #[test]
    fn every_story_tool_advertises_one_vocabulary() {
        let expected = db::story_status::status_enum_json();

        for (name, schema) in [
            ("create_story", CreateStoryTool.input_schema()),
            ("update_story", UpdateStoryTool.input_schema()),
            ("update_story_status", UpdateStoryStatusTool.input_schema()),
            ("list_stories", ListStoriesTool.input_schema()),
        ] {
            assert_eq!(
                schema["properties"]["status"]["enum"], expected,
                "{name} advertises a different set of statuses"
            );
        }
    }

    // -----------------------------------------------------------------------
    // GetStoryTool
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_story_missing_field_returns_error() {
        let db = make_test_pool().await;
        let ctx = make_ctx(db);
        let result = GetStoryTool.execute(serde_json::json!({}), &ctx).await;
        assert!(result.is_error);
        assert!(result.content.contains("story_id"));
    }

    #[tokio::test]
    async fn get_story_not_found_returns_error() {
        let db = make_test_pool().await;
        let ctx = make_ctx(db);
        let result = GetStoryTool
            .execute(serde_json::json!({"story_id": "does-not-exist"}), &ctx)
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn get_story_returns_data_for_existing_story() {
        let db = make_test_pool().await;
        let ctx = make_ctx(db.clone());

        sqlx::query(
            "INSERT INTO stories (id, title, status, priority) VALUES ('s1', 'Sprint Goal', 'ready', 'high')",
        )
        .execute(&db)
        .await
        .unwrap();

        let result = GetStoryTool
            .execute(serde_json::json!({"story_id": "s1"}), &ctx)
            .await;

        assert!(!result.is_error, "Unexpected error: {}", result.content);
        assert!(result.content.contains("Sprint Goal"));
        assert!(result.content.contains("ready"));
        assert!(result.content.contains("high"));
    }

    // -----------------------------------------------------------------------
    // UpdateStoryStatusTool
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn update_story_status_missing_fields_returns_error() {
        let db = make_test_pool().await;
        let ctx = make_ctx(db);
        let result = UpdateStoryStatusTool
            .execute(serde_json::json!({"story_id": "s1"}), &ctx)
            .await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn update_story_status_not_found_returns_error() {
        let db = make_test_pool().await;
        let ctx = make_ctx(db);
        let result = UpdateStoryStatusTool
            .execute(serde_json::json!({"story_id": "no-such-story", "status": "done"}), &ctx)
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn update_story_status_persists_new_status() {
        let db = make_test_pool().await;
        let ctx = make_ctx(db.clone());

        sqlx::query(
            "INSERT INTO stories (id, title, status, priority) VALUES ('s2', 'My Task', 'ready', 'medium')",
        )
        .execute(&db)
        .await
        .unwrap();

        let result = UpdateStoryStatusTool
            .execute(serde_json::json!({"story_id": "s2", "status": "done"}), &ctx)
            .await;

        assert!(!result.is_error, "Unexpected error: {}", result.content);

        let status: String =
            sqlx::query_scalar("SELECT status FROM stories WHERE id = 's2'")
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(status, "done");
    }

    #[tokio::test]
    async fn delete_story_removes_existing_story() {
        let db = make_test_pool().await;
        let ctx = make_ctx(db.clone());

        sqlx::query(
            "INSERT INTO stories (id, title, status, priority) VALUES ('s3', 'Disposable', 'ready', 'medium')",
        )
        .execute(&db)
        .await
        .unwrap();

        let result = DeleteStoryTool
            .execute(serde_json::json!({"story_id": "s3"}), &ctx)
            .await;

        assert!(!result.is_error, "Unexpected error: {}", result.content);

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stories WHERE id = 's3'")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}
