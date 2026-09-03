// .rusty/ directory discovery and TOML → SQLite synchronisation.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use db::DbPool;
use tracing::{info, warn};
use uuid::Uuid;

use crate::toml_profile::AgentToml;
use db::timestamps::NOW_ISO8601;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialise the `.rusty/` directory structure under `workspace_root`:
/// - `{workspace_root}/.rusty/agents/`  — workspace-scoped agent TOML files
/// - `{workspace_root}/.rusty/memory/`  — agent memory store (gitignored)
///
/// Also ensures `{workspace_root}/.gitignore` contains an entry for
/// `.rusty/memory/` so the volatile memory directory is never committed.
pub fn ensure_rusty_dir(workspace_root: &Path) -> Result<()> {
    let agents_dir = workspace_root.join(".rusty").join("agents");
    let memory_dir = workspace_root.join(".rusty").join("memory");

    std::fs::create_dir_all(&agents_dir)
        .with_context(|| format!("create {}", agents_dir.display()))?;
    std::fs::create_dir_all(&memory_dir)
        .with_context(|| format!("create {}", memory_dir.display()))?;

    // Ensure .rusty/memory/ is gitignored.
    let gitignore_path = workspace_root.join(".gitignore");
    let entry = ".rusty/memory/\n";
    if gitignore_path.exists() {
        let content = std::fs::read_to_string(&gitignore_path).unwrap_or_default();
        if !content.contains(".rusty/memory/") {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&gitignore_path)?;
            use std::io::Write;
            if !content.ends_with('\n') {
                f.write_all(b"\n")?;
            }
            f.write_all(entry.as_bytes())?;
        }
    } else {
        std::fs::write(&gitignore_path, entry)?;
    }

    Ok(())
}

/// Discover all TOML agent profiles and upsert them into SQLite.
///
/// Discovery order (later wins on name collision):
/// 1. `~/.rusty/agents/*.toml`           → scope = `global`
/// 2. `{workspace_root}/.rusty/agents/*.toml` → scope = `workspace`
///
/// Profiles that exist only in the DB (no TOML file) are left untouched.
pub async fn sync_profiles(
    db: &DbPool,
    workspace_root: Option<&Path>,
) -> Result<()> {
    sync_profiles_inner(db, None, workspace_root).await
}

/// Like `sync_profiles` but also stamps workspace-scoped profiles with `workspace_id`.
/// Called from `open_workspace` so agents from `.rusty/agents/` get the correct FK.
pub async fn sync_profiles_for_workspace(
    db: &DbPool,
    workspace_id: &str,
    workspace_root: Option<&Path>,
) -> Result<()> {
    sync_profiles_inner(db, Some(workspace_id), workspace_root).await
}

async fn sync_profiles_inner(
    db: &DbPool,
    workspace_id: Option<&str>,
    workspace_root: Option<&Path>,
) -> Result<()> {
    let global_agents_dir = global_agents_dir()?;
    let mut sources: Vec<(PathBuf, &'static str)> = Vec::new();

    if global_agents_dir.is_dir() {
        sources.push((global_agents_dir, "global"));
    }

    if let Some(ws) = workspace_root {
        let ws_agents_dir = ws.join(".rusty").join("agents");
        if ws_agents_dir.is_dir() {
            sources.push((ws_agents_dir, "workspace"));
        }
    }

    for (dir, scope) in sources {
        // Only stamp workspace_id for workspace-scoped profiles.
        let effective_ws_id = if scope == "workspace" { workspace_id } else { None };

        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                warn!("Cannot read agents dir {}: {e}", dir.display());
                continue;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            match load_and_upsert(db, &path, scope, effective_ws_id).await {
                Ok(_) => info!("Synced agent profile from {}", path.display()),
                Err(e) => warn!("Skipping {}: {e}", path.display()),
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Returns `~/.rusty/agents/`, creating the directory if needed.
fn global_agents_dir() -> Result<PathBuf> {
    let home = dirs_home()?;
    let dir = home.join(".rusty").join("agents");
    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create {}", dir.display()))?;
    }
    Ok(dir)
}

/// Cross-platform home directory (without pulling in the full `dirs` crate).
fn dirs_home() -> Result<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .with_context(|| "Cannot determine home directory")
}

/// Parse one TOML file and upsert the profile into `agent_profiles`.
async fn load_and_upsert(db: &DbPool, path: &Path, scope: &str, workspace_id: Option<&str>) -> Result<()> {
    let src = std::fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;
    let agent: AgentToml = AgentToml::from_str(&src)
        .with_context(|| format!("parse TOML {}", path.display()))?;

    let toml_path = path.to_string_lossy().to_string();
    let name      = &agent.profile.name;

    // Check if a profile with this toml_path already exists.
    let existing_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM agent_profiles WHERE toml_path = ? LIMIT 1"
    )
    .bind(&toml_path)
    .fetch_optional(db)
    .await
    .context("DB lookup")?;

    let id = existing_id.unwrap_or_else(|| Uuid::new_v4().to_string());

    let allow_read  = serde_json::to_string(&agent.permissions.allow_read)?;
    let allow_write = serde_json::to_string(&agent.permissions.allow_write)?;
    let allow_shell = serde_json::to_string(&agent.permissions.allow_shell)?;
    let require_approval = agent.permissions.require_approval_on_write as i64;

    sqlx::query(
        &format!("INSERT INTO agent_profiles (
            id, name, description, system_prompt, provider, model,
            context_strategy, persistent_memory, max_input_tokens, max_output_tokens,
            run_mode, cron_expression, continuous_poll_interval_secs, max_iterations,
            scope, toml_path, workspace_id
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(id) DO UPDATE SET
            name                          = excluded.name,
            description                   = excluded.description,
            system_prompt                 = excluded.system_prompt,
            provider                      = excluded.provider,
            model                         = excluded.model,
            context_strategy              = excluded.context_strategy,
            persistent_memory             = excluded.persistent_memory,
            max_input_tokens              = excluded.max_input_tokens,
            max_output_tokens             = excluded.max_output_tokens,
            run_mode                      = excluded.run_mode,
            cron_expression               = excluded.cron_expression,
            continuous_poll_interval_secs = excluded.continuous_poll_interval_secs,
            max_iterations                = excluded.max_iterations,
            scope                         = excluded.scope,
            toml_path                     = excluded.toml_path,
            workspace_id                  = excluded.workspace_id,
            updated_at                    = {NOW_ISO8601}")
    )
    .bind(&id)
    .bind(name)
    .bind(&agent.profile.description)
    .bind(agent.profile.system_prompt.as_deref().unwrap_or(""))
    .bind(&agent.profile.provider)
    .bind(&agent.profile.model)
    .bind(&agent.behavior.context_strategy)
    .bind(agent.behavior.persistent_memory as i64)
    .bind(agent.limits.max_input_tokens)
    .bind(agent.limits.max_output_tokens)
    .bind(&agent.behavior.run_mode)
    .bind(&agent.behavior.cron_expression)
    .bind(agent.behavior.continuous_poll_interval_secs)
    .bind(agent.behavior.max_iterations)
    .bind(scope)
    .bind(&toml_path)
    .bind(workspace_id)
    .execute(db)
    .await
    .context("DB upsert")?;

    // Upsert permissions into agent_permissions table.
    upsert_permissions(
        db, &id, &allow_read, &allow_write, &allow_shell, require_approval,
    ).await?;

    Ok(())
}

async fn upsert_permissions(
    db: &DbPool,
    profile_id: &str,
    allow_read: &str,
    allow_write: &str,
    allow_shell: &str,
    require_approval_on_write: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO agent_permissions (
            profile_id, allow_file_read_paths, allow_file_write_paths, allow_shell_commands, require_approval_on_write
        ) VALUES (?, ?, ?, ?, ?)
        ON CONFLICT(profile_id) DO UPDATE SET
            allow_file_read_paths     = excluded.allow_file_read_paths,
            allow_file_write_paths    = excluded.allow_file_write_paths,
            allow_shell_commands      = excluded.allow_shell_commands,
            require_approval_on_write = excluded.require_approval_on_write"
    )
    .bind(profile_id)
    .bind(allow_read)
    .bind(allow_write)
    .bind(allow_shell)
    .bind(require_approval_on_write)
    .execute(db)
    .await
    .context("DB upsert permissions")?;
    Ok(())
}

/// Write an [`AgentToml`] to the appropriate `.rusty/agents/` directory.
/// Returns the path written.
pub fn write_profile_toml(
    agent: &AgentToml,
    workspace_root: Option<&Path>,
    scope: &str,
) -> Result<PathBuf> {
    let dir = match scope {
        "workspace" => {
            let ws = workspace_root.context("No workspace open for workspace-scoped agent")?;
            ensure_rusty_dir(ws)?;
            ws.join(".rusty").join("agents")
        }
        _ => {
            let dir = global_agents_dir()?;
            std::fs::create_dir_all(&dir)?;
            dir
        }
    };

    let slug = AgentToml::slug(&agent.profile.name);
    let file = dir.join(format!("{slug}.toml"));
    let content = agent.to_toml_string()?;
    std::fs::write(&file, content)?;
    Ok(file)
}
