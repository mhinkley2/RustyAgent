// Per-profile permission settings stored in the `agent_permissions` table.

use db::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use db::timestamps::NOW_ISO8601;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentPermissions {
    pub profile_id: String,
    /// Exact tool names the profile may call (empty = allow all).
    pub allowed_tools: Vec<String>,
    /// Absolute path prefixes for allowed file reads (empty = no restriction).
    pub allow_file_read_paths: Vec<String>,
    /// Absolute path prefixes for allowed file writes (empty = no restriction).
    pub allow_file_write_paths: Vec<String>,
    /// Program names permitted for shell tools (empty = no restriction).
    pub allow_shell_commands: Vec<String>,
    /// When true, every write-tool call requires human approval.
    pub require_approval_on_write: bool,
}

// ---------------------------------------------------------------------------
// get_agent_permissions
// ---------------------------------------------------------------------------

/// Load the permission settings for a profile.  If no row exists the caller
/// receives a default `AgentPermissions` with all-allow semantics.
pub async fn get_agent_permissions(
    profile_id: String,
    db: &DbPool,
) -> Result<AgentPermissions, String> {
    let row = sqlx::query(
        "SELECT allowed_tools, allow_file_read_paths, allow_file_write_paths,
                allow_shell_commands, require_approval_on_write
         FROM agent_permissions WHERE profile_id = ?",
    )
    .bind(&profile_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    let parse_list = |s: &str| -> Vec<String> {
        serde_json::from_str::<Vec<String>>(s).unwrap_or_default()
    };

    match row {
        Some(r) => {
            let tools: String = r.try_get("allowed_tools").unwrap_or_else(|_| "[]".into());
            let reads: String = r.try_get("allow_file_read_paths").unwrap_or_else(|_| "[]".into());
            let writes: String = r.try_get("allow_file_write_paths").unwrap_or_else(|_| "[]".into());
            let cmds: String = r.try_get("allow_shell_commands").unwrap_or_else(|_| "[]".into());
            let approval: i64 = r.try_get("require_approval_on_write").unwrap_or(0);
            Ok(AgentPermissions {
                profile_id,
                allowed_tools: parse_list(&tools),
                allow_file_read_paths: parse_list(&reads),
                allow_file_write_paths: parse_list(&writes),
                allow_shell_commands: parse_list(&cmds),
                require_approval_on_write: approval != 0,
            })
        }
        None => Ok(AgentPermissions {
            profile_id,
            ..Default::default()
        }),
    }
}

// ---------------------------------------------------------------------------
// upsert_agent_permissions
// ---------------------------------------------------------------------------

/// Insert or replace the permission row for the given profile.
pub async fn upsert_agent_permissions(
    perms: AgentPermissions,
    db: &DbPool,
) -> Result<(), String> {
    let to_json = |v: &[String]| serde_json::to_string(v).unwrap_or_else(|_| "[]".into());

    sqlx::query(
        &format!("INSERT INTO agent_permissions
             (profile_id, allowed_tools, allow_file_read_paths, allow_file_write_paths,
              allow_shell_commands, require_approval_on_write,
              updated_at)
         VALUES (?, ?, ?, ?, ?, ?, {NOW_ISO8601})
         ON CONFLICT(profile_id) DO UPDATE SET
             allowed_tools           = excluded.allowed_tools,
             allow_file_read_paths   = excluded.allow_file_read_paths,
             allow_file_write_paths  = excluded.allow_file_write_paths,
             allow_shell_commands    = excluded.allow_shell_commands,
             require_approval_on_write = excluded.require_approval_on_write,
             updated_at              = excluded.updated_at"),
    )
    .bind(&perms.profile_id)
    .bind(to_json(&perms.allowed_tools))
    .bind(to_json(&perms.allow_file_read_paths))
    .bind(to_json(&perms.allow_file_write_paths))
    .bind(to_json(&perms.allow_shell_commands))
    .bind(perms.require_approval_on_write as i64)
    .execute(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    Ok(())
}
