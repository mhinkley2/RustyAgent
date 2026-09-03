//! JSON-RPC dispatch. Transport-free and pure, so the whole protocol surface
//! is testable without a socket or a pipe.

use serde_json::{json, Value};

use crate::{
    ctx::McpCtx,
    jsonrpc::{error_response, success_response, INVALID_PARAMS, METHOD_NOT_FOUND},
    registry::{tool_output_to_result, McpRegistry},
};

pub const SERVER_NAME: &str = "rustyagent-board-mcp";

/// Protocol revisions this server speaks, newest first.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Handle one JSON-RPC message.
///
/// `None` means "this was a notification — send nothing". Protocol failures
/// become JSON-RPC error objects rather than `Err`, so there is no `Result`.
pub async fn handle_message(
    ctx: &McpCtx,
    registry: &McpRegistry,
    message: &Value,
) -> Option<Value> {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));

    match method.as_str() {
        "initialize" => {
            let negotiated = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .filter(|version| SUPPORTED_PROTOCOL_VERSIONS.contains(version))
                .unwrap_or(SUPPORTED_PROTOCOL_VERSIONS[0]);

            Some(success_response(
                id,
                json!({
                    "protocolVersion": negotiated,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": {
                        "name": SERVER_NAME,
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "instructions": attachment(ctx),
                }),
            ))
        }

        "notifications/initialized" => None,

        "ping" => Some(success_response(id, json!({}))),

        "shutdown" => Some(success_response(id, Value::Null)),

        "tools/list" => Some(success_response(
            id,
            json!({ "tools": registry.definitions(ctx.host_available()) }),
        )),

        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error_response(
                    id,
                    INVALID_PARAMS,
                    "Missing required field: params.name",
                ));
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));

            match registry.get(name, ctx.host_available()) {
                Some(tool) => {
                    let output = tool.call(arguments, ctx).await;
                    Some(success_response(id, tool_output_to_result(output)))
                }
                None => Some(error_response(
                    id,
                    METHOD_NOT_FOUND,
                    unknown_tool_message(name, registry, ctx.host_available()),
                )),
            }
        }

        _ => {
            // A message with no id is a notification: never answer it, even to
            // report that the method is unknown.
            if id.is_null() {
                None
            } else {
                Some(error_response(
                    id,
                    METHOD_NOT_FOUND,
                    format!("Method not found: {method}"),
                ))
            }
        }
    }
}

/// Which board this client is attached to, and whether it can change that.
///
/// Reported in the `initialize` result because otherwise a client can only
/// infer its scope from the stories it gets back — and inferring it wrongly is
/// silent. The stdio binary prints the same answer on stderr, where it proved
/// immediately useful; a client connecting over HTTP has no stderr to read.
fn attachment(ctx: &McpCtx) -> String {
    let Some(root) = ctx.workspace_root.as_deref() else {
        return "No workspace is open in the RustyAgent app, so the board tools have \
                nothing to read. Open a folder in the app first."
            .to_string();
    };

    let path = root.display();
    if ctx.pinned() {
        format!(
            "Attached to the RustyAgent board for {path}. This client is confined to \
             that workspace and cannot switch."
        )
    } else {
        format!(
            "Attached to the RustyAgent board for {path}, following whichever workspace \
             the app has open. Use use_workspace to switch."
        )
    }
}

/// Distinguish "no such tool" from "that tool needs the desktop app running",
/// so a stdio client gets an actionable message instead of a bare not-found.
fn unknown_tool_message(name: &str, registry: &McpRegistry, host_available: bool) -> String {
    if !host_available && registry.get(name, true).is_some() {
        format!(
            "Tool '{name}' requires the RustyAgent desktop app to be running. \
             Connect over the HTTP transport instead."
        )
    } else {
        format!("Unknown tool: {name}")
    }
}

/// Refresh workspace scope, then dispatch. What both transports call.
pub async fn handle_message_refreshed(
    base: &McpCtx,
    registry: &McpRegistry,
    message: &Value,
) -> Option<Value> {
    let mut ctx = base.clone();
    ctx.refresh_workspace().await;
    handle_message(&ctx, registry, message).await
}
