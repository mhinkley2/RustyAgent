//! Chat sessions.

use serde_json::{json, Value};
use tools::ToolOutput;

use crate::{
    mcp_tool,
    registry::{json_ok, json_result, opt_i64_arg, opt_str_arg, str_arg},
};

mcp_tool! {
    pub ListChatSessionsTool,
    name        = "list_chat_sessions",
    description = "List chat sessions in the active workspace, most recently updated first.",
    schema      = {
        "type": "object",
        "properties": {
            "limit": { "type": "integer", "description": "Maximum sessions to return" }
        }
    },
    |input, ctx| {
        json_result(
            commands::chat_sessions::list_chat_sessions(
                ctx.workspace_id.clone(),
                opt_i64_arg(&input, "limit"),
                &ctx.db,
            )
            .await,
        )
    }
}

mcp_tool! {
    pub GetChatSessionMessagesTool,
    name        = "get_chat_session_messages",
    description = "Read every message in a chat session, oldest first.",
    schema      = {
        "type": "object",
        "properties": { "session_id": { "type": "string" } },
        "required": ["session_id"]
    },
    |input, ctx| {
        let Some(session_id) = str_arg(&input, "session_id") else {
            return ToolOutput::err("Missing required field: session_id");
        };
        json_result(
            commands::chat_sessions::get_chat_session_messages(session_id, &ctx.db).await,
        )
    }
}

mcp_tool! {
    pub CreateChatSessionTool,
    name        = "create_chat_session",
    description = "Create an empty chat session in the active workspace.",
    mutation    = true,
    schema      = {
        "type": "object",
        "properties": {
            "title": { "type": "string", "description": "Optional title; defaults to 'New Chat'" }
        }
    },
    |input, ctx| {
        json_result(
            commands::chat_sessions::create_chat_session(
                ctx.workspace_id.clone(),
                opt_str_arg(&input, "title"),
                &ctx.db,
            )
            .await,
        )
    }
}

mcp_tool! {
    pub AppendChatMessageTool,
    name        = "append_chat_message",
    description = "Append a message to a chat session. Records the message only — it does not \
                   run an agent.",
    mutation    = true,
    schema      = {
        "type": "object",
        "properties": {
            "session_id":       { "type": "string" },
            "role":             { "type": "string", "enum": ["user", "assistant"] },
            "content":          { "type": "string" },
            "agent_profile_id": { "type": "string" }
        },
        "required": ["session_id", "role", "content"]
    },
    |input, ctx| {
        let (Some(session_id), Some(role), Some(content)) = (
            str_arg(&input, "session_id"),
            str_arg(&input, "role"),
            input.get("content").and_then(Value::as_str).map(str::to_string),
        ) else {
            return ToolOutput::err("Missing required field: session_id, role, or content");
        };

        match commands::chat_sessions::append_chat_session_message(
            session_id,
            role,
            content,
            opt_str_arg(&input, "agent_profile_id"),
            &ctx.db,
        )
        .await
        {
            Ok(()) => json_ok(json!({ "appended": true })),
            Err(error) => ToolOutput::err(error),
        }
    }
}
