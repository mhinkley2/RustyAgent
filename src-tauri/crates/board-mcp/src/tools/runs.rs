//! Run history — read only. Runs are started from the app, not over MCP.

use tools::ToolOutput;

use crate::{
    mcp_tool,
    paging::{paged_rows, page_request, NO_FULLER_FORM},
    registry::{json_ok, json_result, opt_str_arg, str_arg},
};

/// Events per page by default, and the ceiling on `limit`.
///
/// This is the largest unbounded response the MCP surface used to have: a
/// `run_events` row carries the full `tool_input` and `tool_output` of a tool
/// call, so one long autonomous run's log dwarfs any source file in the
/// repository. The row limit bounds the count, the per-field cap in
/// [`crate::paging`] bounds one enormous result, and the page byte budget
/// bounds the whole — all three are needed, because a hundred token events and
/// a single megabyte `tool_output` are both "one row".
const EVENT_DEFAULT_LIMIT: usize = 50;
const EVENT_MAX_LIMIT: usize = 200;

/// Columns of a run event that hold unbounded free text.
const EVENT_TEXT_FIELDS: &[&str] = &["content", "tool_input", "tool_output"];

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
    description = "Read the ordered event log for one run — assistant messages, tool calls, \
                   tool results, errors, and approval requests. This is the primary tool for \
                   diagnosing what an agent actually did. The reply is paged: it returns at \
                   most 50 events by default (200 with an explicit `limit`), stops early when \
                   the page would exceed 32 KB, and reports `total`, `complete` and a \
                   `next_offset` to continue from. Individual `content`, `tool_input` and \
                   `tool_output` values longer than 4 KB are cut and marked in place — a \
                   single tool result can be megabytes.",
    schema      = {
        "type": "object",
        "properties": {
            "run_id": { "type": "string", "description": "UUID of the run" },
            "offset": {
                "type": "integer",
                "minimum": 1,
                "description": "1-based index of the first event to return. Defaults to 1."
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "description": "Maximum events to return. Defaults to 50, clamped to 200."
            }
        },
        "required": ["run_id"]
    },
    |input, ctx| {
        let Some(run_id) = str_arg(&input, "run_id") else {
            return ToolOutput::err("Missing required field: run_id");
        };
        let request = match page_request(&input, EVENT_DEFAULT_LIMIT, EVENT_MAX_LIMIT) {
            Ok(request) => request,
            Err(error) => return ToolOutput::err(error),
        };
        // The command is left unpaged because the app's run detail view reads
        // it too and wants the whole log. The bound belongs at this boundary,
        // where the consumer is an agent whose context this process cannot see.
        let events = match commands::runs::get_run_events(run_id.clone(), &ctx.db).await {
            Ok(events) => events,
            Err(error) => return ToolOutput::err(error),
        };
        match paged_rows(
            events,
            request,
            "get_run_events",
            "events",
            &format!("run '{run_id}'"),
            EVENT_TEXT_FIELDS,
            NO_FULLER_FORM,
        ) {
            Ok(envelope) => json_ok(envelope),
            Err(error) => ToolOutput::err(error),
        }
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

