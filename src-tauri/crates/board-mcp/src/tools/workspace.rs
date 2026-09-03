//! Workspace selection and per-workspace settings.

use std::path::{Path, PathBuf};

use serde_json::json;
use tools::ToolOutput;

use crate::{
    ctx::PinScope,
    mcp_tool,
    registry::{json_ok, json_result, str_arg},
};

/// What a confined client would have to change to stop being confined.
///
/// Named per mechanism: telling an HTTP client to unset an environment
/// variable it never read is advice it cannot act on.
fn how_to_lift(scope: PinScope) -> String {
    match scope {
        PinScope::Process => format!(
            "unset {} to share the app's workspace",
            db::paths::WORKSPACE_ENV
        ),
        PinScope::Request => format!(
            "drop the {} header (or the '{}' query parameter) to follow the app's \
             active workspace",
            crate::WORKSPACE_HEADER,
            crate::WORKSPACE_QUERY_KEY
        ),
    }
}

/// Strip the Windows extended-length prefix so paths compare consistently with
/// what `db::normalize_workspace_path` stores.
pub(crate) fn normalize_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_string()
}

/// Resolve caller input to an existing, canonical directory.
fn resolve_workspace_input(path: &str) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("Missing required field: path".to_string());
    }

    let raw = PathBuf::from(trimmed);
    let candidate = if raw.is_absolute() {
        raw
    } else {
        std::env::current_dir()
            .map_err(|error| format!("Failed to resolve relative workspace path: {error}"))?
            .join(raw)
    };

    if !candidate.exists() {
        return Err(format!(
            "Workspace path does not exist: {}",
            candidate.display()
        ));
    }
    if !candidate.is_dir() {
        return Err(format!(
            "Workspace path is not a directory: {}",
            candidate.display()
        ));
    }

    candidate.canonicalize().map_err(|error| {
        format!(
            "Failed to normalize workspace path '{}': {error}",
            candidate.display()
        )
    })
}

mcp_tool! {
    pub ListWorkspacesTool,
    name        = "list_workspaces",
    description = "List RustyAgent workspaces the user has opened, most recently used first. \
                   Use this before switching board scope with use_workspace. A client \
                   confined to one workspace sees only that one, and cannot switch.",
    schema      = { "type": "object", "properties": {} },
    |input, ctx| {
        let current = ctx.workspace_root.as_deref().map(normalize_path);
        let workspaces = match db::list_workspaces(&ctx.db).await {
            Ok(rows) => rows,
            Err(error) => return ToolOutput::err(format!("Failed to list workspaces: {error}")),
        };

        // A pinned client sees only the workspace it is confined to. It cannot
        // switch to any of the others, so listing them would be an inventory of
        // the user's repositories handed to an agent that has no use for it —
        // and an invitation to keep trying `use_workspace` and being refused.
        let workspaces: Vec<_> = if ctx.pinned() {
            workspaces
                .into_iter()
                .filter(|workspace| current.as_deref() == Some(workspace.path.as_str()))
                .collect()
        } else {
            workspaces
        };

        json_ok(json!({
            "workspaces": workspaces
                .into_iter()
                .map(|workspace| json!({
                    "id": workspace.id,
                    "name": workspace.name,
                    "path": workspace.path,
                    "last_opened_at": workspace.last_opened_at,
                    "created_at": workspace.created_at,
                    "is_active": current.as_deref() == Some(workspace.path.as_str()),
                }))
                .collect::<Vec<_>>()
        }))
    }
}

mcp_tool! {
    pub GetActiveWorkspaceTool,
    name        = "get_active_workspace",
    description = "Read the workspace all other tools are currently scoped to.",
    schema      = { "type": "object", "properties": {} },
    |input, ctx| {
        let workspace = ctx.workspace_root.as_ref().map(|path| {
            let normalized = normalize_path(path);
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .filter(|value| !value.is_empty())
                .unwrap_or(&normalized)
                .to_string();
            json!({ "id": ctx.workspace_id, "name": name, "path": normalized })
        });

        json_ok(json!({ "workspace": workspace }))
    }
}

mcp_tool! {
    pub UseWorkspaceTool,
    name        = "use_workspace",
    description = "Switch the workspace that all other tools are scoped to. The path must be \
                   a workspace already opened in the RustyAgent app — MCP clients cannot \
                   register new ones. Use list_workspaces to see the available paths. \
                   Refused when this client is confined to a single workspace.",
    mutation    = true,
    schema      = {
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Absolute path of an existing workspace" }
        },
        "required": ["path"]
    },
    |input, ctx| {
        // Refused before the argument is even read: a pinned client has no
        // valid target, including the workspace it is already on. Accepting
        // that one would make this a no-op that looks like a success, and the
        // model would have no way to learn the rule.
        if let Some(scope) = ctx.pin {
            let current = ctx
                .workspace_root
                .as_deref()
                .map(normalize_path)
                .unwrap_or_else(|| "its workspace".to_string());
            return ToolOutput::err(format!(
                "This client is confined to '{current}' and cannot switch workspaces. It named \
                 that project when it connected; connect a separate client for another project, \
                 or {}.",
                how_to_lift(scope)
            ));
        }

        let Some(path) = str_arg(&input, "path") else {
            return ToolOutput::err("Missing required field: path");
        };

        let resolved = match resolve_workspace_input(&path) {
            Ok(value) => value,
            Err(error) => return ToolOutput::err(error),
        };

        // Confinement: select from known workspaces only, never register a new
        // one. Without this an MCP client could point the workspace at any
        // directory on the machine and then read it through read_file.
        if db::find_workspace_by_path(&ctx.db, &resolved).await.is_none() {
            return ToolOutput::err(format!(
                "Unknown workspace '{}'. Open it in the RustyAgent app first; \
                 MCP clients cannot register new workspaces.",
                normalize_path(&resolved)
            ));
        }

        // Promote it to most-recently-opened, which is how both transports and
        // the app agree on which workspace is active.
        let workspace = match db::touch_workspace(&ctx.db, &resolved).await {
            Ok(value) => value,
            Err(error) => {
                return ToolOutput::err(format!(
                    "Failed to activate workspace '{}': {error}",
                    resolved.display()
                ))
            }
        };

        if let Some(host) = &ctx.host {
            host.workspace_changed(&workspace);
        }

        json_ok(json!({
            "workspace": {
                "id": workspace.id,
                "name": workspace.name,
                "path": workspace.path,
                "last_opened_at": workspace.last_opened_at,
                "created_at": workspace.created_at,
            }
        }))
    }
}

mcp_tool! {
    pub GetWorkspaceSettingsTool,
    name        = "get_workspace_settings",
    description = "Read the settings overrides stored for the active workspace.",
    schema      = { "type": "object", "properties": {} },
    |input, ctx| {
        let Some(workspace_id) = ctx.workspace_id.clone() else {
            return ToolOutput::err("No active workspace. Use use_workspace first.");
        };
        json_result(commands::settings::get_workspace_settings(workspace_id, &ctx.db).await)
    }
}

mcp_tool! {
    pub SaveWorkspaceSettingsTool,
    name        = "save_workspace_settings",
    description = "Write the settings overrides for the active workspace. Replaces the whole \
                   object; read it first if you intend to merge.",
    mutation    = true,
    schema      = {
        "type": "object",
        "properties": {
            "overrides": { "type": "object", "description": "Settings overrides to store" }
        },
        "required": ["overrides"]
    },
    |input, ctx| {
        let Some(workspace_id) = ctx.workspace_id.clone() else {
            return ToolOutput::err("No active workspace. Use use_workspace first.");
        };
        let Some(overrides) = input.get("overrides").cloned() else {
            return ToolOutput::err("Missing required field: overrides");
        };
        match commands::settings::save_workspace_settings(workspace_id, overrides, &ctx.db).await {
            Ok(()) => json_ok(json!({ "saved": true })),
            Err(error) => ToolOutput::err(error),
        }
    }
}

