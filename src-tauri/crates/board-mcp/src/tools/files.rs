//! Workspace filesystem — reads only.
//!
//! Both tools delegate to `commands::filesystem`, which canonicalizes the path
//! and verifies it stays inside the active workspace root (resolving symlinks
//! and Windows junctions first). No write, rename, duplicate, create, or delete
//! command is exposed: those are available in the app, and an MCP client has no
//! need to reach for them.

use serde_json::json;
use tools::ToolOutput;

use crate::{
    mcp_tool,
    registry::{json_ok, json_result, str_arg},
};

mcp_tool! {
    pub ListDirectoryTool,
    name        = "list_directory",
    description = "List the immediate children of a directory inside the active workspace. \
                   Directories sort first. Build and VCS directories (.git, node_modules, \
                   target, dist, …) are skipped.",
    schema      = {
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Absolute path inside the workspace" }
        },
        "required": ["path"]
    },
    |input, ctx| {
        let Some(path) = str_arg(&input, "path") else {
            return ToolOutput::err("Missing required field: path");
        };
        json_result(commands::filesystem::list_directory(path, &ctx.db).await)
    }
}

mcp_tool! {
    pub ReadFileTool,
    name        = "read_file",
    description = "Read a UTF-8 text file inside the active workspace. Files over 10 MB are \
                   refused.",
    schema      = {
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Absolute path inside the workspace" }
        },
        "required": ["path"]
    },
    |input, ctx| {
        let Some(path) = str_arg(&input, "path") else {
            return ToolOutput::err("Missing required field: path");
        };
        match commands::filesystem::read_file_text(path.clone(), &ctx.db).await {
            Ok(content) => json_ok(json!({ "path": path, "content": content })),
            Err(error) => ToolOutput::err(error),
        }
    }
}

