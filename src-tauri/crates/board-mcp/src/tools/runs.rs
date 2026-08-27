//! Run history — read only. Runs are started from the app, not over MCP.

use tools::ToolOutput;

use crate::{
    mcp_tool,
    registry::{json_result, opt_str_arg, str_arg},
};

mcp_tool! {
    pub ListRunsTool,
    name        = "list_runs",
    description = "List agent runs in the active workspace, newest first. Chat sessions are \
                   excluded. Optionally filter by story, agent profile, or status \
                   (running, done, failed, cancelled).",
    schema      = {
        "type": "object",
        "properties": {
            "story_id":         { "type": "string" },
            "agent_profile_id": { "type": "string" },
            "status": {
                "type": "string",
                "enum": ["running", "done", "failed", "cancelled"]
            }
        }
    },
    |input, ctx| {
        let filters = commands::RunFilters {
            story_id:         opt_str_arg(&input, "story_id"),
            agent_profile_id: opt_str_arg(&input, "agent_profile_id"),
            status:           opt_str_arg(&input, "status"),
        };
        json_result(
            commands::runs::get_runs(Some(filters), ctx.workspace_id.clone(), &ctx.db).await,
        )
    }
}

mcp_tool! {
    pub GetRunTool,
    name        = "get_run",
    description = "Read one run's summary: status, token counts, estimated cost, iteration \
                   count, duration, and the git SHA captured before it started.",
    schema      = {
        "type": "object",
        "properties": { "run_id": { "type": "string", "description": "UUID of the run" } },
        "required": ["run_id"]
    },
    |input, ctx| {
        let Some(run_id) = str_arg(&input, "run_id") else {
            return ToolOutput::err("Missing required field: run_id");
        };
        json_result(commands::runs::get_run(run_id, &ctx.db).await)
    }
}

mcp_tool! {
    pub GetRunEventsTool,
    name        = "get_run_events",
    description = "Read the full ordered event log for one run — assistant messages, tool \
                   calls, tool results, errors, and approval requests. This is the primary \
                   tool for diagnosing what an agent actually did.",
    schema      = {
        "type": "object",
        "properties": { "run_id": { "type": "string", "description": "UUID of the run" } },
        "required": ["run_id"]
    },
    |input, ctx| {
        let Some(run_id) = str_arg(&input, "run_id") else {
            return ToolOutput::err("Missing required field: run_id");
        };
        json_result(commands::runs::get_run_events(run_id, &ctx.db).await)
    }
}

mcp_tool! {
    pub GetRunDiffTool,
    name        = "get_run_diff",
    description = "Read the git diff captured for one run — what the agent changed in the \
                   workspace. Null when the workspace is not a git repository.",
    schema      = {
        "type": "object",
        "properties": { "run_id": { "type": "string", "description": "UUID of the run" } },
        "required": ["run_id"]
    },
    |input, ctx| {
        let Some(run_id) = str_arg(&input, "run_id") else {
            return ToolOutput::err("Missing required field: run_id");
        };
        json_result(commands::runs::get_run_diff(run_id, &ctx.db).await)
    }
}

