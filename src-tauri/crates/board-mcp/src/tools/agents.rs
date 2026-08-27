//! Agent profiles, permissions, and the human-in-the-loop queues. Read only.

use tools::ToolOutput;

use crate::{
    mcp_tool,
    registry::{json_result, str_arg},
};

mcp_tool! {
    pub ListAgentProfilesTool,
    name        = "list_agent_profiles",
    description = "List agent profiles available in the active workspace, including \
                   globally-scoped ones. Shows provider, model, run mode, and limits.",
    schema      = { "type": "object", "properties": {} },
    |input, ctx| {
        json_result(
            commands::agent_profiles::get_profiles(&ctx.db, ctx.workspace_id.clone()).await,
        )
    }
}

mcp_tool! {
    pub GetAgentProfileTool,
    name        = "get_agent_profile",
    description = "Read one agent profile in full: system prompt, provider, model, context \
                   strategy, token limits, run mode, and cron expression.",
    schema      = {
        "type": "object",
        "properties": { "profile_id": { "type": "string" } },
        "required": ["profile_id"]
    },
    |input, ctx| {
        let Some(profile_id) = str_arg(&input, "profile_id") else {
            return ToolOutput::err("Missing required field: profile_id");
        };
        json_result(commands::agent_profiles::get_profile(profile_id, &ctx.db).await)
    }
}

mcp_tool! {
    pub GetAgentPermissionsTool,
    name        = "get_agent_permissions",
    description = "Read a profile's sandbox: allowed tools, readable and writable path \
                   prefixes, permitted shell commands, network hosts, and whether writes \
                   require approval. Note an empty list means 'allow all', not 'deny all'.",
    schema      = {
        "type": "object",
        "properties": { "profile_id": { "type": "string" } },
        "required": ["profile_id"]
    },
    |input, ctx| {
        let Some(profile_id) = str_arg(&input, "profile_id") else {
            return ToolOutput::err("Missing required field: profile_id");
        };
        json_result(commands::permissions::get_agent_permissions(profile_id, &ctx.db).await)
    }
}

mcp_tool! {
    pub ListPendingHumanRequestsTool,
    name        = "list_pending_human_requests",
    description = "List questions agents have asked that are waiting on a human answer. \
                   Answering them is done in the RustyAgent app, not over MCP.",
    schema      = { "type": "object", "properties": {} },
    |input, ctx| {
        json_result(commands::human::get_pending_human_requests(&ctx.db).await)
    }
}

mcp_tool! {
    pub ListPendingApprovalsTool,
    name        = "list_pending_approvals",
    description = "List tool calls held for human approval, with the tool name and its input. \
                   Deciding them is done in the RustyAgent app, not over MCP.",
    schema      = { "type": "object", "properties": {} },
    |input, ctx| {
        json_result(commands::human::get_pending_approvals(&ctx.db).await)
    }
}

