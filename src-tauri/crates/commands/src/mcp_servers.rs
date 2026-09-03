// CRUD Tauri commands for mcp_servers and agent_tool_bindings tables.

use db::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;
use db::timestamps::NOW_ISO8601;

// ---------------------------------------------------------------------------
// MCP Server types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServer {
    pub id: String,
    pub name: String,
    pub command: String,
    /// JSON array of strings, e.g. ["--port", "8080"]
    pub args: Vec<String>,
    /// JSON object of non-secret env vars
    pub env_vars: std::collections::HashMap<String, String>,
    pub auto_restart: bool,
    pub max_restart_attempts: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateMcpServerInput {
    pub name: String,
    pub command: String,
    pub args: Option<Vec<String>>,
    pub env_vars: Option<std::collections::HashMap<String, String>>,
    pub auto_restart: Option<bool>,
    pub max_restart_attempts: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMcpServerInput {
    pub name: Option<String>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub env_vars: Option<std::collections::HashMap<String, String>>,
    pub auto_restart: Option<bool>,
    pub max_restart_attempts: Option<i64>,
}

// ---------------------------------------------------------------------------
// Tool binding types
// ---------------------------------------------------------------------------

/// A binding between an agent profile and an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolBinding {
    pub id: String,
    pub agent_profile_id: String,
    pub mcp_server_id: String,
    /// Display name of the MCP server (from JOIN).
    pub mcp_server_name: Option<String>,
    /// JSON array of allowed tool names; None means all.
    pub allowed_tools: Option<Vec<String>>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateToolBindingInput {
    pub agent_profile_id: String,
    pub mcp_server_id: String,
    /// None = allow all tools from this server.
    pub allowed_tools: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_args(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

fn parse_env(json: &str) -> std::collections::HashMap<String, String> {
    serde_json::from_str(json).unwrap_or_default()
}

fn parse_allowed_tools(json: Option<&str>) -> Option<Vec<String>> {
    json.and_then(|s| serde_json::from_str(s).ok())
}

fn row_to_server(row: &sqlx::sqlite::SqliteRow) -> McpServer {
    let auto_restart: i64 = row.try_get("auto_restart").unwrap_or(1);
    let args_json: String = row.try_get("args").unwrap_or_else(|_| "[]".to_string());
    let env_json: String = row.try_get("env_vars").unwrap_or_else(|_| "{}".to_string());
    McpServer {
        id:                   row.try_get("id").unwrap_or_default(),
        name:                 row.try_get("name").unwrap_or_default(),
        command:              row.try_get("command").unwrap_or_default(),
        args:                 parse_args(&args_json),
        env_vars:             parse_env(&env_json),
        auto_restart:         auto_restart != 0,
        max_restart_attempts: row.try_get("max_restart_attempts").unwrap_or(3),
        created_at:           row.try_get("created_at").unwrap_or_default(),
        updated_at:           row.try_get("updated_at").unwrap_or_default(),
    }
}

fn row_to_binding(row: &sqlx::sqlite::SqliteRow) -> ToolBinding {
    let allowed_json: Option<String> = row.try_get("allowed_tools").ok().flatten();
    ToolBinding {
        id:                row.try_get("id").unwrap_or_default(),
        agent_profile_id:  row.try_get("agent_profile_id").unwrap_or_default(),
        mcp_server_id:     row.try_get("mcp_server_id").unwrap_or_default(),
        mcp_server_name:   row.try_get("server_name").ok().flatten(),
        allowed_tools:     parse_allowed_tools(allowed_json.as_deref()),
        created_at:        row.try_get("created_at").unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// get_mcp_servers
// ---------------------------------------------------------------------------

pub async fn get_mcp_servers(db: &DbPool) -> Result<Vec<McpServer>, String> {
    let rows = sqlx::query(
        "SELECT id, name, command, args, env_vars, auto_restart, max_restart_attempts,
                created_at, updated_at
         FROM mcp_servers
         ORDER BY name ASC",
    )
    .fetch_all(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    Ok(rows.iter().map(row_to_server).collect())
}

// ---------------------------------------------------------------------------
// get_mcp_server
// ---------------------------------------------------------------------------

pub async fn get_mcp_server(id: String, db: &DbPool) -> Result<McpServer, String> {
    let row = sqlx::query(
        "SELECT id, name, command, args, env_vars, auto_restart, max_restart_attempts,
                created_at, updated_at
         FROM mcp_servers WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?
    .ok_or_else(|| format!("MCP server '{id}' not found"))?;

    Ok(row_to_server(&row))
}

// ---------------------------------------------------------------------------
// create_mcp_server
// ---------------------------------------------------------------------------

pub async fn create_mcp_server(
    input: CreateMcpServerInput,
    db: &DbPool,
) -> Result<McpServer, String> {
    let id = Uuid::new_v4().to_string();
    let args_json =
        serde_json::to_string(&input.args.unwrap_or_default()).unwrap_or_else(|_| "[]".to_string());
    let env_json = serde_json::to_string(
        &input.env_vars.unwrap_or_default(),
    )
    .unwrap_or_else(|_| "{}".to_string());
    let auto_restart = input.auto_restart.unwrap_or(true) as i64;
    let max_restart = input.max_restart_attempts.unwrap_or(3);

    sqlx::query(
        "INSERT INTO mcp_servers
             (id, name, command, args, env_vars, auto_restart, max_restart_attempts)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&input.name)
    .bind(&input.command)
    .bind(&args_json)
    .bind(&env_json)
    .bind(auto_restart)
    .bind(max_restart)
    .execute(db)
    .await
    .map_err(|e| format!("DB insert error: {e}"))?;

    get_mcp_server(id, db).await
}

// ---------------------------------------------------------------------------
// update_mcp_server
// ---------------------------------------------------------------------------

pub async fn update_mcp_server(
    id: String,
    input: UpdateMcpServerInput,
    db: &DbPool,
) -> Result<McpServer, String> {
    // Load current values then merge.
    let current = get_mcp_server(id.clone(), db).await?;

    let name = input.name.unwrap_or(current.name);
    let command = input.command.unwrap_or(current.command);
    let args = input.args.unwrap_or(current.args);
    let env_vars = input.env_vars.unwrap_or(current.env_vars);
    let auto_restart = input.auto_restart.unwrap_or(current.auto_restart) as i64;
    let max_restart = input.max_restart_attempts.unwrap_or(current.max_restart_attempts);

    let args_json = serde_json::to_string(&args).unwrap_or_else(|_| "[]".to_string());
    let env_json = serde_json::to_string(&env_vars).unwrap_or_else(|_| "{}".to_string());

    sqlx::query(
        &format!("UPDATE mcp_servers
         SET name = ?, command = ?, args = ?, env_vars = ?,
             auto_restart = ?, max_restart_attempts = ?,
             updated_at = {NOW_ISO8601}
         WHERE id = ?"),
    )
    .bind(&name)
    .bind(&command)
    .bind(&args_json)
    .bind(&env_json)
    .bind(auto_restart)
    .bind(max_restart)
    .bind(&id)
    .execute(db)
    .await
    .map_err(|e| format!("DB update error: {e}"))?;

    get_mcp_server(id, db).await
}

// ---------------------------------------------------------------------------
// delete_mcp_server
// ---------------------------------------------------------------------------

pub async fn delete_mcp_server(id: String, db: &DbPool) -> Result<(), String> {
    sqlx::query("DELETE FROM mcp_servers WHERE id = ?")
        .bind(&id)
        .execute(db)
        .await
        .map_err(|e| format!("DB delete error: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// get_tool_bindings  (all bindings for a given agent, or all if None)
// ---------------------------------------------------------------------------

pub async fn get_tool_bindings(
    agent_profile_id: String,
    db: &DbPool,
) -> Result<Vec<ToolBinding>, String> {
    let rows = sqlx::query(
        "SELECT atb.id, atb.agent_profile_id, atb.mcp_server_id,
                atb.allowed_tools, atb.created_at,
                ms.name AS server_name
         FROM agent_tool_bindings atb
         LEFT JOIN mcp_servers ms ON ms.id = atb.mcp_server_id
         WHERE atb.agent_profile_id = ?
         ORDER BY ms.name ASC",
    )
    .bind(&agent_profile_id)
    .fetch_all(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    Ok(rows.iter().map(row_to_binding).collect())
}

// ---------------------------------------------------------------------------
// create_tool_binding
// ---------------------------------------------------------------------------

pub async fn create_tool_binding(
    input: CreateToolBindingInput,
    db: &DbPool,
) -> Result<ToolBinding, String> {
    let id = Uuid::new_v4().to_string();
    let allowed_json: Option<String> = input
        .allowed_tools
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_default());

    sqlx::query(
        "INSERT INTO agent_tool_bindings
             (id, agent_profile_id, mcp_server_id, allowed_tools)
         VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&input.agent_profile_id)
    .bind(&input.mcp_server_id)
    .bind(&allowed_json)
    .execute(db)
    .await
    .map_err(|e| format!("DB insert error: {e}"))?;

    let row = sqlx::query(
        "SELECT atb.id, atb.agent_profile_id, atb.mcp_server_id,
                atb.allowed_tools, atb.created_at,
                ms.name AS server_name
         FROM agent_tool_bindings atb
         LEFT JOIN mcp_servers ms ON ms.id = atb.mcp_server_id
         WHERE atb.id = ?",
    )
    .bind(&id)
    .fetch_one(db)
    .await
    .map_err(|e| format!("DB fetch error: {e}"))?;

    Ok(row_to_binding(&row))
}

// ---------------------------------------------------------------------------
// delete_tool_binding
// ---------------------------------------------------------------------------

pub async fn delete_tool_binding(id: String, db: &DbPool) -> Result<(), String> {
    sqlx::query("DELETE FROM agent_tool_bindings WHERE id = ?")
        .bind(&id)
        .execute(db)
        .await
        .map_err(|e| format!("DB delete error: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// update_tool_binding_allowed_tools
// ---------------------------------------------------------------------------

pub async fn update_tool_binding_allowed_tools(
    id: String,
    allowed_tools: Option<Vec<String>>,
    db: &DbPool,
) -> Result<ToolBinding, String> {
    let allowed_json: Option<String> = allowed_tools
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_default());

    sqlx::query(
        "UPDATE agent_tool_bindings SET allowed_tools = ? WHERE id = ?",
    )
    .bind(&allowed_json)
    .bind(&id)
    .execute(db)
    .await
    .map_err(|e| format!("DB update error: {e}"))?;

    let row = sqlx::query(
        "SELECT atb.id, atb.agent_profile_id, atb.mcp_server_id,
                atb.allowed_tools, atb.created_at,
                ms.name AS server_name
         FROM agent_tool_bindings atb
         LEFT JOIN mcp_servers ms ON ms.id = atb.mcp_server_id
         WHERE atb.id = ?",
    )
    .bind(&id)
    .fetch_one(db)
    .await
    .map_err(|e| format!("DB fetch error: {e}"))?;

    Ok(row_to_binding(&row))
}
