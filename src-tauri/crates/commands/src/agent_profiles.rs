// CRUD Tauri commands for agent_profiles table.

use db::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub system_prompt: String,
    pub provider: String,
    pub model: String,
    pub context_strategy: String,
    pub persistent_memory: bool,
    pub max_input_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
    pub run_mode: String,
    pub cron_expression: Option<String>,
    pub continuous_poll_interval_secs: i64,
    pub max_iterations: i64,
    /// How many times a failed provider call may be retried inside a run.
    pub max_retries: i64,
    /// "global" or "workspace"
    pub scope: String,
    /// Absolute path to the source TOML file, if any.
    pub toml_path: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateProfileInput {
    pub name: String,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub provider: String,
    pub model: String,
    pub context_strategy: Option<String>,
    pub persistent_memory: Option<bool>,
    pub max_input_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
    pub run_mode: Option<String>,
    pub cron_expression: Option<String>,
    pub continuous_poll_interval_secs: Option<i64>,
    pub max_iterations: Option<i64>,
    pub max_retries: Option<i64>,
    /// "global" or "workspace"
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProfileInput {
    pub name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub context_strategy: Option<String>,
    pub persistent_memory: Option<bool>,
    pub max_input_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
    pub run_mode: Option<String>,
    pub cron_expression: Option<String>,
    pub continuous_poll_interval_secs: Option<i64>,
    pub max_iterations: Option<i64>,
    pub max_retries: Option<i64>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn row_to_profile(row: &sqlx::sqlite::SqliteRow) -> AgentProfile {
    let persistent_memory: i64 = row.try_get("persistent_memory").unwrap_or(0);
    AgentProfile {
        id:                           row.try_get("id").unwrap_or_default(),
        name:                         row.try_get("name").unwrap_or_default(),
        description:                  row.try_get("description").ok().flatten(),
        system_prompt:                row.try_get("system_prompt").unwrap_or_default(),
        provider:                     row.try_get("provider").unwrap_or_default(),
        model:                        row.try_get("model").unwrap_or_default(),
        context_strategy:             row.try_get("context_strategy").unwrap_or_default(),
        persistent_memory:            persistent_memory != 0,
        max_input_tokens:             row.try_get("max_input_tokens").ok().flatten(),
        max_output_tokens:            row.try_get("max_output_tokens").ok().flatten(),
        run_mode:                     row.try_get("run_mode").unwrap_or_default(),
        cron_expression:              row.try_get("cron_expression").ok().flatten(),
        continuous_poll_interval_secs: row.try_get("continuous_poll_interval_secs").unwrap_or(30),
        max_iterations:               row.try_get("max_iterations").unwrap_or(20),
        max_retries:                  row.try_get("max_retries").unwrap_or(2),
        scope:                        row.try_get("scope").unwrap_or_else(|_| "global".into()),
        toml_path:                    row.try_get("toml_path").ok().flatten(),
        created_at:                   row.try_get("created_at").unwrap_or_default(),
        updated_at:                   row.try_get("updated_at").unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

pub async fn get_profiles(
    db: &DbPool,
    workspace_id: Option<String>,
) -> Result<Vec<AgentProfile>, String> {
    let rows = match workspace_id {
        Some(ref ws_id) => {
            sqlx::query(
                "SELECT id, name, description, system_prompt, provider, model, context_strategy,
                        persistent_memory, max_input_tokens, max_output_tokens, run_mode,
                        cron_expression, continuous_poll_interval_secs, max_iterations,
                        max_retries, scope, toml_path, created_at, updated_at
                 FROM agent_profiles
                 WHERE scope = 'global' OR (scope = 'workspace' AND workspace_id = ?)
                 ORDER BY created_at ASC",
            )
            .bind(ws_id)
            .fetch_all(db)
            .await
            .map_err(|e| format!("DB error: {e}"))?
        }
        None => {
            sqlx::query(
                "SELECT id, name, description, system_prompt, provider, model, context_strategy,
                        persistent_memory, max_input_tokens, max_output_tokens, run_mode,
                        cron_expression, continuous_poll_interval_secs, max_iterations,
                        max_retries, scope, toml_path, created_at, updated_at
                 FROM agent_profiles
                 ORDER BY created_at ASC",
            )
            .fetch_all(db)
            .await
            .map_err(|e| format!("DB error: {e}"))?
        }
    };

    Ok(rows.iter().map(row_to_profile).collect())
}

pub async fn get_profile(id: String, db: &DbPool) -> Result<AgentProfile, String> {
    let row = sqlx::query(
        "SELECT id, name, description, system_prompt, provider, model, context_strategy,
                persistent_memory, max_input_tokens, max_output_tokens, run_mode,
                cron_expression, continuous_poll_interval_secs, max_iterations,
                max_retries, scope, toml_path, created_at, updated_at
         FROM agent_profiles WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?
    .ok_or_else(|| format!("Profile '{id}' not found"))?;

    Ok(row_to_profile(&row))
}

pub async fn create_profile(
    input: CreateProfileInput,
    db: &DbPool,
    workspace_id: Option<String>,
) -> Result<AgentProfile, String> {
    let id = Uuid::new_v4().to_string();
    let system_prompt = input.system_prompt.unwrap_or_default();
    let context_strategy = input.context_strategy.unwrap_or_else(|| "recent".into());
    let persistent_memory = input.persistent_memory.unwrap_or(false);
    let run_mode = input.run_mode.unwrap_or_else(|| "manual".into());
    let poll = input.continuous_poll_interval_secs.unwrap_or(30);
    let max_iter = input.max_iterations.unwrap_or(20);
    // Matches the column default in `20260410000021_agent_max_retries.sql`:
    // three attempts in total. A profile created through this API gets the
    // same budget as one created before the column existed.
    let max_retries = input.max_retries.unwrap_or(2);
    let scope = input.scope.unwrap_or_else(|| "global".into());
    // Only stamp workspace_id when creating a workspace-scoped profile.
    let effective_ws_id = if scope == "workspace" { workspace_id } else { None };

    sqlx::query(
        "INSERT INTO agent_profiles
             (id, name, description, system_prompt, provider, model, context_strategy,
              persistent_memory, max_input_tokens, max_output_tokens, run_mode,
              cron_expression, continuous_poll_interval_secs, max_iterations, max_retries,
              scope, workspace_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&input.name)
    .bind(&input.description)
    .bind(&system_prompt)
    .bind(&input.provider)
    .bind(&input.model)
    .bind(&context_strategy)
    .bind(persistent_memory as i64)
    .bind(input.max_input_tokens)
    .bind(input.max_output_tokens)
    .bind(&run_mode)
    .bind(&input.cron_expression)
    .bind(poll)
    .bind(max_iter)
    .bind(max_retries)
    .bind(&scope)
    .bind(&effective_ws_id)
    .execute(db)
    .await
    .map_err(|e| format!("DB insert error: {e}"))?;

    get_profile(id, db).await
}

pub async fn update_profile(
    id: String,
    input: UpdateProfileInput,
    db: &DbPool,
) -> Result<AgentProfile, String> {
    // Fetch current state so we only update provided fields.
    let current = get_profile(id.clone(), db).await?;

    let name               = input.name.unwrap_or(current.name);
    let description        = input.description.or(current.description);
    let system_prompt      = input.system_prompt.unwrap_or(current.system_prompt);
    let provider           = input.provider.unwrap_or(current.provider);
    let model              = input.model.unwrap_or(current.model);
    let context_strategy   = input.context_strategy.unwrap_or(current.context_strategy);
    let persistent_memory  = input.persistent_memory.unwrap_or(current.persistent_memory);
    let max_input_tokens   = input.max_input_tokens.or(current.max_input_tokens);
    let max_output_tokens  = input.max_output_tokens.or(current.max_output_tokens);
    let run_mode           = input.run_mode.unwrap_or(current.run_mode);
    let cron_expression    = input.cron_expression.or(current.cron_expression);
    let poll               = input.continuous_poll_interval_secs.unwrap_or(current.continuous_poll_interval_secs);
    let max_iter           = input.max_iterations.unwrap_or(current.max_iterations);
    let max_retries        = input.max_retries.unwrap_or(current.max_retries);

    sqlx::query(
        "UPDATE agent_profiles
         SET name = ?, description = ?, system_prompt = ?, provider = ?, model = ?,
             context_strategy = ?, persistent_memory = ?, max_input_tokens = ?,
             max_output_tokens = ?, run_mode = ?, cron_expression = ?,
             continuous_poll_interval_secs = ?, max_iterations = ?, max_retries = ?,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?",
    )
    .bind(&name)
    .bind(&description)
    .bind(&system_prompt)
    .bind(&provider)
    .bind(&model)
    .bind(&context_strategy)
    .bind(persistent_memory as i64)
    .bind(max_input_tokens)
    .bind(max_output_tokens)
    .bind(&run_mode)
    .bind(&cron_expression)
    .bind(poll)
    .bind(max_iter)
    .bind(max_retries)
    .bind(&id)
    .execute(db)
    .await
    .map_err(|e| format!("DB update error: {e}"))?;

    get_profile(id, db).await
}

pub async fn delete_profile(id: String, db: &DbPool) -> Result<(), String> {
    sqlx::query("DELETE FROM agent_profiles WHERE id = ?")
        .bind(&id)
        .execute(db)
        .await
        .map_err(|e| format!("DB delete error: {e}"))?;
    Ok(())
}

/// Write an agent profile to a `.rusty/agents/<slug>.toml` file and record
/// the `toml_path` in the DB row.
///
/// `workspace_root` must be provided when `scope == "workspace"`.
pub async fn save_profile_toml(
    id: String,
    scope: String,
    workspace_root: Option<String>,
    db: &DbPool,
) -> Result<String, String> {
    use workspace::{AgentToml, loader::write_profile_toml};
    use workspace::toml_profile::{BehaviorSection, LimitsSection, PermissionsSection, ProfileSection};

    let profile = get_profile(id.clone(), db).await?;

    // Fetch current permissions for this profile.
    let perms_row = sqlx::query(
        "SELECT allow_file_read_paths, allow_file_write_paths, allow_shell_commands, require_approval_on_write
         FROM agent_permissions WHERE profile_id = ?"
    )
    .bind(&id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    let (allow_read, allow_write, allow_shell, req_approval): (Vec<String>, Vec<String>, Vec<String>, bool) = match perms_row {
        Some(row) => {
            let r: String = row.try_get("allow_file_read_paths").unwrap_or_default();
            let w: String = row.try_get("allow_file_write_paths").unwrap_or_default();
            let s: String = row.try_get("allow_shell_commands").unwrap_or_default();
            let a: i64    = row.try_get("require_approval_on_write").unwrap_or(0);
            (
                serde_json::from_str(&r).unwrap_or_default(),
                serde_json::from_str(&w).unwrap_or_default(),
                serde_json::from_str(&s).unwrap_or_default(),
                a != 0,
            )
        }
        None => (vec![], vec![], vec![], true),
    };

    let agent = AgentToml {
        profile: ProfileSection {
            name:          profile.name.clone(),
            description:   profile.description.clone(),
            provider:      profile.provider.clone(),
            model:         profile.model.clone(),
            system_prompt: Some(profile.system_prompt.clone()),
        },
        behavior: BehaviorSection {
            context_strategy:              profile.context_strategy.clone(),
            persistent_memory:             profile.persistent_memory,
            max_iterations:                profile.max_iterations,
            run_mode:                      profile.run_mode.clone(),
            cron_expression:               profile.cron_expression.clone(),
            continuous_poll_interval_secs: profile.continuous_poll_interval_secs,
        },
        limits: LimitsSection {
            max_input_tokens:  profile.max_input_tokens,
            max_output_tokens: profile.max_output_tokens,
        },
        permissions: PermissionsSection {
            allow_read,
            allow_write,
            allow_shell,
            require_approval_on_write: req_approval,
        },
    };

    let ws_path = workspace_root.as_deref().map(std::path::Path::new);
    let toml_file = write_profile_toml(&agent, ws_path, &scope)
        .map_err(|e| format!("Failed to write TOML: {e}"))?;

    let toml_path_str = toml_file.to_string_lossy().to_string();

    // Record the toml_path and scope in the DB.
    sqlx::query("UPDATE agent_profiles SET toml_path = ?, scope = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?")
        .bind(&toml_path_str)
        .bind(&scope)
        .bind(&id)
        .execute(db)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    Ok(toml_path_str)
}

/// Sync TOML profiles from disk into SQLite.
/// Called on workspace open and by the live-reload watcher.
pub async fn sync_toml_profiles(
    workspace_root: Option<String>,
    db: &DbPool,
) -> Result<(), String> {
    let ws = workspace_root.as_deref().map(std::path::Path::new);
    workspace::loader::sync_profiles(db, ws)
        .await
        .map_err(|e| format!("sync_profiles error: {e}"))
}

