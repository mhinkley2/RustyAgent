//! Pipeline validation, app logs, and the live-state tools that need the app.
//!
//! The four `host_only` tools read in-memory state (scheduler task maps,
//! pipeline progress) that exists only inside the running desktop process. They
//! are hidden from `tools/list` and rejected by `tools/call` on the stdio
//! transport rather than answered with a plausible-looking default — a stale
//! `"idle"` is worse for a client than an error.

use serde_json::json;
use tools::ToolOutput;

use crate::{
    mcp_tool,
    registry::{json_ok, opt_i64_arg, str_arg},
};

mcp_tool! {
    pub ValidatePipelineTool,
    name        = "validate_pipeline",
    description = "Check a pipeline story's configuration without running it: rejects an \
                   empty step list, a step referencing the pipeline story itself, and \
                   duplicate step stories.",
    schema      = {
        "type": "object",
        "properties": {
            "story_id": { "type": "string", "description": "UUID of the pipeline story" }
        },
        "required": ["story_id"]
    },
    |input, ctx| {
        let Some(story_id) = str_arg(&input, "story_id") else {
            return ToolOutput::err("Missing required field: story_id");
        };

        let config = match pipeline::load_pipeline_config(&story_id, &ctx.db).await {
            Ok(config) => config,
            Err(error) => return ToolOutput::err(format!("{error}")),
        };

        match pipeline::validate_pipeline_config(&story_id, &config) {
            Ok(()) => json_ok(json!({
                "valid": true,
                "mode": config.mode,
                "step_count": config.steps.len(),
            })),
            Err(error) => ToolOutput::err(format!("{error}")),
        }
    }
}

mcp_tool! {
    pub GetAppLogsTool,
    name        = "get_app_logs",
    description = "Read the tail of RustyAgent's application log. Defaults to the last 500 \
                   lines; raise tail_lines to see more.",
    schema      = {
        "type": "object",
        "properties": {
            "tail_lines": {
                "type": "integer",
                "description": "How many trailing lines to return (default 500)"
            }
        }
    },
    |input, ctx| {
        let Some(dir) = ctx.app_data_dir.as_ref() else {
            return ToolOutput::err(
                "App data directory is unknown, so the log file cannot be located.",
            );
        };

        let path = dir.join("logs").join("rustyagent.log");
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                return ToolOutput::err(format!(
                    "Failed to read '{}': {error}",
                    path.display()
                ))
            }
        };

        // Bounded by default — the log is unrotated and can reach many MB.
        let requested = opt_i64_arg(&input, "tail_lines").unwrap_or(500).max(1) as usize;
        let lines: Vec<&str> = content.lines().collect();
        let start = lines.len().saturating_sub(requested);

        json_ok(json!({
            "path": path.display().to_string(),
            "total_lines": lines.len(),
            "returned_lines": lines.len() - start,
            "content": lines[start..].join("\n"),
        }))
    }
}

mcp_tool! {
    pub GetAgentRuntimeStatusTool,
    name        = "get_agent_runtime_status",
    description = "Live scheduler state for one agent profile: whether it is idle, polling, \
                   running, or waiting on a human, plus its next scheduled run.",
    host_only   = true,
    schema      = {
        "type": "object",
        "properties": { "profile_id": { "type": "string" } },
        "required": ["profile_id"]
    },
    |input, ctx| {
        let Some(profile_id) = str_arg(&input, "profile_id") else {
            return ToolOutput::err("Missing required field: profile_id");
        };
        let Some(host) = ctx.host.as_ref() else {
            return ToolOutput::err("This tool requires the RustyAgent desktop app.");
        };
        json_ok(host.agent_runtime_status(&profile_id))
    }
}

mcp_tool! {
    pub ListAgentRuntimeStatusesTool,
    name        = "list_agent_runtime_statuses",
    description = "Live scheduler state for every agent profile at once.",
    host_only   = true,
    schema      = { "type": "object", "properties": {} },
    |input, ctx| {
        let Some(host) = ctx.host.as_ref() else {
            return ToolOutput::err("This tool requires the RustyAgent desktop app.");
        };
        json_ok(host.agent_runtime_statuses())
    }
}

mcp_tool! {
    pub GetPipelineProgressTool,
    name        = "get_pipeline_progress",
    description = "Step-by-step progress for one in-flight pipeline run.",
    host_only   = true,
    schema      = {
        "type": "object",
        "properties": { "pipeline_run_id": { "type": "string" } },
        "required": ["pipeline_run_id"]
    },
    |input, ctx| {
        let Some(pipeline_run_id) = str_arg(&input, "pipeline_run_id") else {
            return ToolOutput::err("Missing required field: pipeline_run_id");
        };
        let Some(host) = ctx.host.as_ref() else {
            return ToolOutput::err("This tool requires the RustyAgent desktop app.");
        };
        json_ok(json!({ "progress": host.pipeline_progress(&pipeline_run_id) }))
    }
}

mcp_tool! {
    pub ListActivePipelinesTool,
    name        = "list_active_pipelines",
    description = "List pipeline runs currently in flight, with per-step progress.",
    host_only   = true,
    schema      = { "type": "object", "properties": {} },
    |input, ctx| {
        let Some(host) = ctx.host.as_ref() else {
            return ToolOutput::err("This tool requires the RustyAgent desktop app.");
        };
        json_ok(host.active_pipelines())
    }
}

