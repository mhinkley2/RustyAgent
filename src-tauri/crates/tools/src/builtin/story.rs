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
                    "description": "Optional filter by status (backlog, ready, in_progress, blocked, review, done, failed). Omit for all.",
                    "enum": ["backlog", "ready", "in_progress", "blocked", "review", "done", "failed"]
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
                    "enum": ["backlog", "ready", "in_progress", "blocked", "review", "done"]
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
                    "enum": ["backlog", "ready", "in_progress", "blocked", "review", "done", "failed"]
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
        let status = input.get("status").and_then(|v| v.as_str()).map(|value| value.to_string()).unwrap_or(current_status);
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
        "Update the status of a story. Valid statuses: ready, in_progress, done, failed, blocked."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "story_id": { "type": "string", "description": "UUID of the story" },
                "status": {
                    "type": "string",
                    "enum": ["ready", "in_progress", "done", "failed", "blocked"]
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
