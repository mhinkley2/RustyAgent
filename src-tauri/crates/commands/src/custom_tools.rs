// CRUD Tauri commands for custom_tools and agent_custom_tool_bindings tables.

use db::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;
use db::timestamps::NOW_ISO8601;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomTool {
    pub id: String,
    pub name: String,
    pub description: String,
    pub command: String,
    pub working_dir: String,
    pub timeout_secs: i64,
    pub workspace_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateCustomToolInput {
    pub name: String,
    pub description: Option<String>,
    pub command: String,
    pub working_dir: Option<String>,
    pub timeout_secs: Option<i64>,
    pub workspace_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCustomToolInput {
    pub name: Option<String>,
    pub description: Option<String>,
    pub command: Option<String>,
    pub working_dir: Option<String>,
    pub timeout_secs: Option<i64>,
}

/// A custom tool that is bound to an agent profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomToolBinding {
    pub agent_profile_id: String,
    pub custom_tool_id: String,
    pub tool_name: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn row_to_custom_tool(row: &sqlx::sqlite::SqliteRow) -> CustomTool {
    CustomTool {
        id:           row.try_get("id").unwrap_or_default(),
        name:         row.try_get("name").unwrap_or_default(),
        description:  row.try_get("description").unwrap_or_default(),
        command:      row.try_get("command").unwrap_or_default(),
        working_dir:  row.try_get("working_dir").unwrap_or_else(|_| ".".to_string()),
        timeout_secs: row.try_get("timeout_secs").unwrap_or(30),
        workspace_id: row.try_get("workspace_id").ok().flatten(),
        created_at:   row.try_get("created_at").unwrap_or_default(),
        updated_at:   row.try_get("updated_at").unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// get_custom_tools
// ---------------------------------------------------------------------------

pub async fn get_custom_tools(
    workspace_id: Option<String>,
    db: &DbPool,
) -> Result<Vec<CustomTool>, String> {
    let rows = match &workspace_id {
        Some(ws_id) => sqlx::query(
            "SELECT id, name, description, command, working_dir, timeout_secs,
                    workspace_id, created_at, updated_at
             FROM custom_tools
             WHERE workspace_id = ? OR workspace_id IS NULL
             ORDER BY name ASC",
        )
        .bind(ws_id)
        .fetch_all(db)
        .await
        .map_err(|e| format!("DB error: {e}"))?,
        None => sqlx::query(
            "SELECT id, name, description, command, working_dir, timeout_secs,
                    workspace_id, created_at, updated_at
             FROM custom_tools
             WHERE workspace_id IS NULL
             ORDER BY name ASC",
        )
        .fetch_all(db)
        .await
        .map_err(|e| format!("DB error: {e}"))?,
    };

    Ok(rows.iter().map(row_to_custom_tool).collect())
}

// ---------------------------------------------------------------------------
// get_custom_tool
// ---------------------------------------------------------------------------

pub async fn get_custom_tool(id: String, db: &DbPool) -> Result<CustomTool, String> {
    let row = sqlx::query(
        "SELECT id, name, description, command, working_dir, timeout_secs,
                workspace_id, created_at, updated_at
         FROM custom_tools WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?
    .ok_or_else(|| format!("Custom tool '{id}' not found"))?;

    Ok(row_to_custom_tool(&row))
}

// ---------------------------------------------------------------------------
// create_custom_tool
// ---------------------------------------------------------------------------

pub async fn create_custom_tool(
    input: CreateCustomToolInput,
    db: &DbPool,
) -> Result<CustomTool, String> {
    let id = Uuid::new_v4().to_string();
    let description = input.description.unwrap_or_default();
    let working_dir = input.working_dir.unwrap_or_else(|| ".".to_string());
    let timeout_secs = input.timeout_secs.unwrap_or(30);

    sqlx::query(
        "INSERT INTO custom_tools
             (id, name, description, command, working_dir, timeout_secs, workspace_id)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&input.name)
    .bind(&description)
    .bind(&input.command)
    .bind(&working_dir)
    .bind(timeout_secs)
    .bind(&input.workspace_id)
    .execute(db)
    .await
    .map_err(|e| format!("DB insert error: {e}"))?;

    get_custom_tool(id, db).await
}

// ---------------------------------------------------------------------------
// update_custom_tool
// ---------------------------------------------------------------------------

pub async fn update_custom_tool(
    id: String,
    input: UpdateCustomToolInput,
    db: &DbPool,
) -> Result<CustomTool, String> {
    let current = get_custom_tool(id.clone(), db).await?;

    let name = input.name.unwrap_or(current.name);
    let description = input.description.unwrap_or(current.description);
    let command = input.command.unwrap_or(current.command);
    let working_dir = input.working_dir.unwrap_or(current.working_dir);
    let timeout_secs = input.timeout_secs.unwrap_or(current.timeout_secs);

    sqlx::query(
        &format!("UPDATE custom_tools
         SET name = ?, description = ?, command = ?, working_dir = ?, timeout_secs = ?,
             updated_at = {NOW_ISO8601}
         WHERE id = ?"),
    )
    .bind(&name)
    .bind(&description)
    .bind(&command)
    .bind(&working_dir)
    .bind(timeout_secs)
    .bind(&id)
    .execute(db)
    .await
    .map_err(|e| format!("DB update error: {e}"))?;

    get_custom_tool(id, db).await
}

// ---------------------------------------------------------------------------
// delete_custom_tool
// ---------------------------------------------------------------------------

pub async fn delete_custom_tool(id: String, db: &DbPool) -> Result<(), String> {
    sqlx::query("DELETE FROM custom_tools WHERE id = ?")
        .bind(&id)
        .execute(db)
        .await
        .map_err(|e| format!("DB delete error: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// get_custom_tool_bindings
// ---------------------------------------------------------------------------

pub async fn get_custom_tool_bindings(
    agent_profile_id: String,
    db: &DbPool,
) -> Result<Vec<CustomToolBinding>, String> {
    let rows = sqlx::query(
        "SELECT actb.agent_profile_id, actb.custom_tool_id, ct.name AS tool_name
         FROM agent_custom_tool_bindings actb
         LEFT JOIN custom_tools ct ON ct.id = actb.custom_tool_id
         WHERE actb.agent_profile_id = ?
         ORDER BY ct.name ASC",
    )
    .bind(&agent_profile_id)
    .fetch_all(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    Ok(rows
        .iter()
        .map(|row| CustomToolBinding {
            agent_profile_id: row.try_get("agent_profile_id").unwrap_or_default(),
            custom_tool_id:   row.try_get("custom_tool_id").unwrap_or_default(),
            tool_name:        row.try_get("tool_name").ok().flatten(),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// create_custom_tool_binding
// ---------------------------------------------------------------------------

pub async fn create_custom_tool_binding(
    agent_profile_id: String,
    custom_tool_id: String,
    db: &DbPool,
) -> Result<CustomToolBinding, String> {
    sqlx::query(
        "INSERT OR IGNORE INTO agent_custom_tool_bindings
             (agent_profile_id, custom_tool_id)
         VALUES (?, ?)",
    )
    .bind(&agent_profile_id)
    .bind(&custom_tool_id)
    .execute(db)
    .await
    .map_err(|e| format!("DB insert error: {e}"))?;

    let row = sqlx::query(
        "SELECT actb.agent_profile_id, actb.custom_tool_id, ct.name AS tool_name
         FROM agent_custom_tool_bindings actb
         LEFT JOIN custom_tools ct ON ct.id = actb.custom_tool_id
         WHERE actb.agent_profile_id = ? AND actb.custom_tool_id = ?",
    )
    .bind(&agent_profile_id)
    .bind(&custom_tool_id)
    .fetch_one(db)
    .await
    .map_err(|e| format!("DB fetch error: {e}"))?;

    Ok(CustomToolBinding {
        agent_profile_id: row.try_get("agent_profile_id").unwrap_or_default(),
        custom_tool_id:   row.try_get("custom_tool_id").unwrap_or_default(),
        tool_name:        row.try_get("tool_name").ok().flatten(),
    })
}

// ---------------------------------------------------------------------------
// delete_custom_tool_binding
// ---------------------------------------------------------------------------

pub async fn delete_custom_tool_binding(
    agent_profile_id: String,
    custom_tool_id: String,
    db: &DbPool,
) -> Result<(), String> {
    sqlx::query(
        "DELETE FROM agent_custom_tool_bindings
         WHERE agent_profile_id = ? AND custom_tool_id = ?",
    )
    .bind(&agent_profile_id)
    .bind(&custom_tool_id)
    .execute(db)
    .await
    .map_err(|e| format!("DB delete error: {e}"))?;
    Ok(())
}
