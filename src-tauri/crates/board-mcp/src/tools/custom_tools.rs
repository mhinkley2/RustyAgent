//! Custom shell tools — read only.
//!
//! Creating or binding a custom tool is stored arbitrary code execution: the
//! command runs on the next agent run for the bound profile. Those stay in the
//! app deliberately; only inspection is exposed here.

use tools::ToolOutput;

use crate::{
    mcp_tool,
    registry::{json_result, str_arg},
};

mcp_tool! {
    pub ListCustomToolsTool,
    name        = "list_custom_tools",
    description = "List custom shell-command tools defined in the active workspace, with the \
                   command each would run.",
    schema      = { "type": "object", "properties": {} },
    |input, ctx| {
        json_result(
            commands::custom_tools::get_custom_tools(ctx.workspace_id.clone(), &ctx.db).await,
        )
    }
}

mcp_tool! {
    pub GetCustomToolBindingsTool,
    name        = "get_custom_tool_bindings",
    description = "List which custom shell tools are bound to an agent profile — that is, \
                   which commands that agent is able to run.",
    schema      = {
        "type": "object",
        "properties": { "profile_id": { "type": "string" } },
        "required": ["profile_id"]
    },
    |input, ctx| {
        let Some(profile_id) = str_arg(&input, "profile_id") else {
            return ToolOutput::err("Missing required field: profile_id");
        };
        json_result(
            commands::custom_tools::get_custom_tool_bindings(profile_id, &ctx.db).await,
        )
    }
}

