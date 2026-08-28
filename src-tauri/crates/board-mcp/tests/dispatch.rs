//! Protocol-level behaviour: initialize, negotiation, notifications, errors,
//! the result envelope, and host gating.

mod common;

use board_mcp::{handle_message, SUPPORTED_PROTOCOL_VERSIONS};
use common::*;
use serde_json::{json, Value};

// -- initialize --------------------------------------------------------------

#[tokio::test]
async fn initialize_reports_capabilities_and_server_info() {
    let (ctx, registry) = (ctx().await, registry());

    let response = send(&ctx, &registry, request(json!(1), "initialize", json!({}))).await;

    let result = &response["result"];
    assert_eq!(result["capabilities"]["tools"]["listChanged"], json!(false));
    assert_eq!(result["serverInfo"]["name"], json!("rustyagent-board-mcp"));
    assert!(
        result["serverInfo"]["version"]
            .as_str()
            .is_some_and(|v| !v.is_empty()),
        "serverInfo.version must be populated"
    );
}

#[tokio::test]
async fn initialize_echoes_a_supported_client_protocol_version() {
    let (ctx, registry) = (ctx().await, registry());

    for version in SUPPORTED_PROTOCOL_VERSIONS {
        let response = send(
            &ctx,
            &registry,
            request(json!(1), "initialize", json!({ "protocolVersion": version })),
        )
        .await;
        assert_eq!(response["result"]["protocolVersion"], json!(version));
    }
}

#[tokio::test]
async fn initialize_falls_back_to_the_newest_version_for_an_unknown_request() {
    let (ctx, registry) = (ctx().await, registry());

    let response = send(
        &ctx,
        &registry,
        request(json!(1), "initialize", json!({ "protocolVersion": "1999-01-01" })),
    )
    .await;

    assert_eq!(
        response["result"]["protocolVersion"],
        json!(SUPPORTED_PROTOCOL_VERSIONS[0])
    );
}

// -- notifications and lifecycle --------------------------------------------

#[tokio::test]
async fn an_initialized_notification_gets_no_response() {
    let (ctx, registry) = (ctx().await, registry());

    let response = handle_message(
        &ctx,
        &registry,
        &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await;

    assert!(response.is_none());
}

#[tokio::test]
async fn ping_and_shutdown_are_answered() {
    let (ctx, registry) = (ctx().await, registry());

    let ping = send(&ctx, &registry, request(json!(7), "ping", json!({}))).await;
    assert_eq!(ping["result"], json!({}));
    assert_eq!(ping["id"], json!(7));

    let shutdown = send(&ctx, &registry, request(json!(8), "shutdown", json!({}))).await;
    assert_eq!(shutdown["result"], Value::Null);
}

#[tokio::test]
async fn an_unknown_method_with_an_id_is_a_method_not_found_error() {
    let (ctx, registry) = (ctx().await, registry());

    let response = send(&ctx, &registry, request(json!(1), "resources/list", json!({}))).await;

    let (code, message) = rpc_error(&response);
    assert_eq!(code, -32601);
    assert!(message.contains("resources/list"), "got {message}");
}

#[tokio::test]
async fn an_unknown_method_without_an_id_is_silently_ignored() {
    // A message with no id is a notification: never answer it, not even to
    // report that the method is unknown.
    let (ctx, registry) = (ctx().await, registry());

    let response = handle_message(
        &ctx,
        &registry,
        &json!({ "jsonrpc": "2.0", "method": "notifications/cancelled" }),
    )
    .await;

    assert!(response.is_none());
}

#[tokio::test]
async fn the_request_id_is_preserved_across_shapes() {
    let (ctx, registry) = (ctx().await, registry());

    for id in [json!("abc"), json!(42), Value::Null] {
        let response = send(&ctx, &registry, request(id.clone(), "ping", json!({}))).await;
        assert_eq!(response["id"], id);
    }
}

// -- tools/list --------------------------------------------------------------

#[tokio::test]
async fn tools_list_hides_host_only_tools_when_there_is_no_host() {
    let (ctx, registry) = (ctx().await, registry());

    let response = send(&ctx, &registry, request(json!(1), "tools/list", json!({}))).await;
    let names: Vec<&str> = response["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();

    assert!(names.contains(&"list_stories"));
    assert!(names.contains(&"get_run_events"));
    for host_only in [
        "get_agent_runtime_status",
        "list_agent_runtime_statuses",
        "get_pipeline_progress",
        "list_active_pipelines",
    ] {
        assert!(!names.contains(&host_only), "{host_only} should be hidden");
    }
}

#[tokio::test]
async fn tools_list_includes_host_only_tools_when_a_host_is_present() {
    let (ctx, _host) = ctx_with_host().await;
    let registry = registry();

    let response = send(&ctx, &registry, request(json!(1), "tools/list", json!({}))).await;
    let names: Vec<&str> = response["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();

    for host_only in [
        "get_agent_runtime_status",
        "list_agent_runtime_statuses",
        "get_pipeline_progress",
        "list_active_pipelines",
    ] {
        assert!(names.contains(&host_only), "{host_only} should be listed");
    }
}

#[tokio::test]
async fn every_listed_tool_has_a_description_and_an_object_schema() {
    let (ctx, _host) = ctx_with_host().await;
    let registry = registry();

    let response = send(&ctx, &registry, request(json!(1), "tools/list", json!({}))).await;

    for tool in response["result"]["tools"].as_array().expect("tools array") {
        let name = tool["name"].as_str().expect("name");
        assert!(!name.is_empty(), "a tool has an empty name");
        assert!(
            tool["description"].as_str().is_some_and(|d| d.len() > 20),
            "{name} needs a usable description"
        );
        assert_eq!(
            tool["inputSchema"]["type"],
            json!("object"),
            "{name} schema must be an object"
        );
        assert!(
            tool["inputSchema"].get("properties").is_some(),
            "{name} schema must declare properties"
        );
    }
}

#[tokio::test]
async fn tool_names_are_unique() {
    // Guards against a copy-paste slip in the mcp_tool! invocations, which
    // would otherwise silently shadow one tool with another.
    let registry = registry();
    let mut names = registry.all_names();
    let total = names.len();
    names.sort_unstable();
    names.dedup();

    assert_eq!(names.len(), total, "duplicate tool name registered");
}

// -- tools/call --------------------------------------------------------------

#[tokio::test]
async fn calling_without_a_name_is_an_invalid_params_error() {
    let (ctx, registry) = (ctx().await, registry());

    let response = send(
        &ctx,
        &registry,
        request(json!(1), "tools/call", json!({ "arguments": {} })),
    )
    .await;

    assert_eq!(rpc_error(&response).0, -32602);
}

#[tokio::test]
async fn calling_an_unknown_tool_is_a_method_not_found_error() {
    let (ctx, registry) = (ctx().await, registry());

    let response = send(&ctx, &registry, call("no_such_tool", json!({}))).await;

    let (code, message) = rpc_error(&response);
    assert_eq!(code, -32601);
    assert!(message.contains("no_such_tool"), "got {message}");
}

#[tokio::test]
async fn a_host_only_tool_is_refused_rather_than_answered_with_a_default() {
    // The important case for the stdio transport: returning a plausible
    // "idle" would be worse for a client than an explicit error.
    let (ctx, registry) = (ctx().await, registry());

    let response = send(
        &ctx,
        &registry,
        call("get_agent_runtime_status", json!({ "profile_id": "agent-1" })),
    )
    .await;

    let (code, message) = rpc_error(&response);
    assert_eq!(code, -32601);
    assert!(
        message.contains("desktop app"),
        "the message should explain why: {message}"
    );
}

#[tokio::test]
async fn a_host_only_tool_works_when_a_host_is_present() {
    let (ctx, _host) = ctx_with_host().await;
    let registry = registry();

    let structured = call_ok(
        &ctx,
        &registry,
        "get_agent_runtime_status",
        json!({ "profile_id": "agent-1" }),
    )
    .await;

    assert_eq!(structured["state"], json!("running"));
}

#[tokio::test]
async fn a_json_result_is_returned_as_both_text_and_structured_content() {
    let (ctx, registry) = (ctx().await, registry());

    let response = send(&ctx, &registry, call("list_workspaces", json!({}))).await;
    let result = &response["result"];

    assert_eq!(result["content"][0]["type"], json!("text"));
    assert_eq!(result["isError"], json!(false));
    assert!(
        result["structuredContent"].get("workspaces").is_some(),
        "structuredContent should carry the parsed payload"
    );
    // The text form is the same data, pretty-printed.
    let text = result["content"][0]["text"].as_str().expect("text");
    assert!(text.contains("workspaces"));
}

#[tokio::test]
async fn a_tool_failure_is_a_successful_response_flagged_is_error() {
    // Tool failures are in-band results, not JSON-RPC protocol errors.
    let (ctx, registry) = (ctx().await, registry());

    let response = send(&ctx, &registry, call("get_run", json!({ "run_id": "nope" }))).await;

    assert!(response.get("error").is_none(), "should not be an RPC error");
    assert_eq!(response["result"]["isError"], json!(true));
}

#[tokio::test]
async fn a_missing_required_argument_is_reported_by_the_tool() {
    let (ctx, registry) = (ctx().await, registry());

    let message = call_err(&ctx, &registry, "get_run", json!({})).await;

    assert!(message.contains("run_id"), "got {message}");
}
