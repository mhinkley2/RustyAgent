use async_trait::async_trait;
use serde_json::json;
use sqlx::Row;
use crate::{Tool, ToolContext, ToolOutput};

/// Memory read/write tools backed by the agent_memory SQLite table.

pub struct MemoryReadTool;

#[async_trait]
impl Tool for MemoryReadTool {
    fn name(&self) -> &str { "memory_read" }

    fn description(&self) -> &str {
        "Read a value from the agent's persistent memory by key."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "The memory key to read" },
                "scope": {
                    "type": "string",
                    "enum": ["session", "persistent", "shared_scratchpad"],
                    "default": "persistent"
                }
            },
            "required": ["key"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolOutput {
        let key = match input.get("key").and_then(|v| v.as_str()) {
            Some(k) => k.to_string(),
            None => return ToolOutput::err("Missing required field: key"),
        };
        let scope = input.get("scope").and_then(|v| v.as_str()).unwrap_or("persistent");

        // For shared_scratchpad scope, filter by the pipeline_run_id if present.
        let result = if scope == "shared_scratchpad" {
            match &ctx.pipeline_run_id {
                Some(pid) => sqlx::query(
                    "SELECT value FROM agent_memory WHERE scope = 'shared_scratchpad' AND key = ? AND pipeline_run_id = ?"
                )
                .bind(&key)
                .bind(pid)
                .fetch_optional(&ctx.db)
                .await,
                None => return ToolOutput::err("shared_scratchpad requires a pipeline context"),
            }
        } else {
            sqlx::query(
                "SELECT value FROM agent_memory WHERE agent_profile_id = ? AND scope = ? AND key = ? AND pipeline_run_id IS NULL"
            )
            .bind(&ctx.agent_profile_id)
            .bind(scope)
            .bind(&key)
            .fetch_optional(&ctx.db)
            .await
        };

        match result {
            Ok(Some(row)) => {
                let value: String = row.try_get("value").unwrap_or_default();
                ToolOutput::ok(value)
            }
            Ok(None) => ToolOutput::err(format!("No memory found for key '{key}'")),
            Err(e) => ToolOutput::err(format!("DB error: {e}")),
        }
    }
}

pub struct MemoryWriteTool;

#[async_trait]
impl Tool for MemoryWriteTool {
    fn name(&self) -> &str { "memory_write" }

    fn description(&self) -> &str {
        "Write a value to the agent's persistent memory under a key."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "The memory key" },
                "value": { "type": "string", "description": "The value to store" },
                "scope": {
                    "type": "string",
                    "enum": ["session", "persistent", "shared_scratchpad"],
                    "default": "persistent"
                }
            },
            "required": ["key", "value"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolOutput {
        let key = match input.get("key").and_then(|v| v.as_str()) {
            Some(k) => k.to_string(),
            None => return ToolOutput::err("Missing required field: key"),
        };
        let value = match input.get("value").and_then(|v| v.as_str()) {
            Some(v) => v.to_string(),
            None => return ToolOutput::err("Missing required field: value"),
        };
        let scope = input.get("scope").and_then(|v| v.as_str()).unwrap_or("persistent");
        let id = uuid::Uuid::new_v4().to_string();

        // Resolve pipeline_run_id for shared_scratchpad scope.
        let opt_pipeline_run_id: Option<String> = if scope == "shared_scratchpad" {
            match &ctx.pipeline_run_id {
                Some(pid) => Some(pid.clone()),
                None => return ToolOutput::err("shared_scratchpad requires a pipeline context"),
            }
        } else {
            None
        };

        // SQLite's UNIQUE constraint treats NULL != NULL, so a plain UPSERT
        // ON CONFLICT never fires when pipeline_run_id IS NULL.  Use an
        // explicit UPDATE-then-INSERT to achieve correct upsert semantics.
        let update_result = if let Some(ref pid) = opt_pipeline_run_id {
            sqlx::query(
                "UPDATE agent_memory
                 SET value = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE scope = 'shared_scratchpad' AND key = ? AND pipeline_run_id = ?",
            )
            .bind(&value)
            .bind(&key)
            .bind(pid)
            .execute(&ctx.db)
            .await
        } else {
            sqlx::query(
                "UPDATE agent_memory
                 SET value = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE agent_profile_id = ? AND scope = ? AND key = ? AND pipeline_run_id IS NULL",
            )
            .bind(&value)
            .bind(&ctx.agent_profile_id)
            .bind(scope)
            .bind(&key)
            .execute(&ctx.db)
            .await
        };

        match update_result {
            Err(e) => return ToolOutput::err(format!("DB error: {e}")),
            Ok(r) if r.rows_affected() > 0 => {
                return ToolOutput::ok(format!("Stored '{key}' = '{value}'"))
            }
            Ok(_) => {} // Row didn't exist yet — fall through to INSERT.
        }

        let insert_result = sqlx::query(
            "INSERT INTO agent_memory (id, agent_profile_id, scope, key, value, pipeline_run_id)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&ctx.agent_profile_id)
        .bind(scope)
        .bind(&key)
        .bind(&value)
        .bind(opt_pipeline_run_id)
        .execute(&ctx.db)
        .await;

        match insert_result {
            Ok(_) => ToolOutput::ok(format!("Stored '{key}' = '{value}'")),
            Err(e) => ToolOutput::err(format!("DB error: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{make_ctx, make_test_pool};

    #[tokio::test]
    async fn write_and_read_roundtrip() {
        let db = make_test_pool().await;
        let ctx = make_ctx(db);

        let w = MemoryWriteTool
            .execute(serde_json::json!({"key": "greeting", "value": "hello"}), &ctx)
            .await;
        assert!(!w.is_error, "Write failed: {}", w.content);

        let r = MemoryReadTool
            .execute(serde_json::json!({"key": "greeting"}), &ctx)
            .await;
        assert!(!r.is_error, "Read failed: {}", r.content);
        assert_eq!(r.content, "hello");
    }

    #[tokio::test]
    async fn write_upserts_existing_key() {
        let db = make_test_pool().await;
        let ctx = make_ctx(db);

        MemoryWriteTool
            .execute(serde_json::json!({"key": "counter", "value": "1"}), &ctx)
            .await;
        MemoryWriteTool
            .execute(serde_json::json!({"key": "counter", "value": "2"}), &ctx)
            .await;

        let r = MemoryReadTool
            .execute(serde_json::json!({"key": "counter"}), &ctx)
            .await;
        assert_eq!(r.content, "2", "Expected upserted value");
    }

    #[tokio::test]
    async fn read_missing_key_returns_error() {
        let db = make_test_pool().await;
        let ctx = make_ctx(db);

        let r = MemoryReadTool
            .execute(serde_json::json!({"key": "nonexistent"}), &ctx)
            .await;
        assert!(r.is_error);
        assert!(r.content.contains("nonexistent"));
    }

    #[tokio::test]
    async fn read_missing_key_field_returns_error() {
        let db = make_test_pool().await;
        let ctx = make_ctx(db);

        let r = MemoryReadTool.execute(serde_json::json!({}), &ctx).await;
        assert!(r.is_error);
    }

    #[tokio::test]
    async fn write_missing_key_field_returns_error() {
        let db = make_test_pool().await;
        let ctx = make_ctx(db);

        let r = MemoryWriteTool
            .execute(serde_json::json!({"value": "v"}), &ctx)
            .await;
        assert!(r.is_error);
    }

    #[tokio::test]
    async fn write_missing_value_field_returns_error() {
        let db = make_test_pool().await;
        let ctx = make_ctx(db);

        let r = MemoryWriteTool
            .execute(serde_json::json!({"key": "k"}), &ctx)
            .await;
        assert!(r.is_error);
    }

    #[tokio::test]
    async fn different_scopes_are_isolated() {
        let db = make_test_pool().await;
        let ctx = make_ctx(db);

        // Write "v1" under "session" scope
        MemoryWriteTool
            .execute(
                serde_json::json!({"key": "x", "value": "session_val", "scope": "session"}),
                &ctx,
            )
            .await;

        // Reading the same key under "persistent" scope should return error
        let r = MemoryReadTool
            .execute(
                serde_json::json!({"key": "x", "scope": "persistent"}),
                &ctx,
            )
            .await;
        assert!(r.is_error, "Scopes should be isolated");
    }
}
