//! Shared fixtures for the MCP integration tests.
//!
//! Compiled separately into each test binary, so any given binary uses only
//! part of it.
#![allow(dead_code)]

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use board_mcp::{build_registry, handle_message, HostBridge, McpCtx, McpRegistry};
use serde_json::{json, Value};

/// A [`HostBridge`] that records calls, standing in for the desktop app.
#[derive(Default)]
pub struct FakeHost {
    pub workspace_changes: AtomicUsize,
}

impl FakeHost {
    pub fn workspace_change_count(&self) -> usize {
        self.workspace_changes.load(Ordering::SeqCst)
    }
}

impl HostBridge for FakeHost {
    fn workspace_changed(&self, _workspace: &db::WorkspaceRecord) {
        self.workspace_changes.fetch_add(1, Ordering::SeqCst);
    }
    fn agent_runtime_status(&self, profile_id: &str) -> Value {
        json!({ "profile_id": profile_id, "state": "running" })
    }
    fn agent_runtime_statuses(&self) -> Value {
        json!([{ "profile_id": "agent-1", "state": "running" }])
    }
    fn pipeline_progress(&self, pipeline_run_id: &str) -> Option<Value> {
        Some(json!({ "pipeline_run_id": pipeline_run_id, "status": "running" }))
    }
    fn active_pipelines(&self) -> Value {
        json!([])
    }
}

/// A context over a fresh in-memory database, with no host.
pub async fn ctx() -> McpCtx {
    McpCtx::new(db::testing::make_test_pool().await)
}

/// Same, but with a [`FakeHost`] attached.
pub async fn ctx_with_host() -> (McpCtx, Arc<FakeHost>) {
    let host = Arc::new(FakeHost::default());
    let ctx = McpCtx::new(db::testing::make_test_pool().await).with_host(Some(host.clone()));
    (ctx, host)
}

pub fn registry() -> McpRegistry {
    build_registry()
}

// -- request helpers --------------------------------------------------------

pub fn request(id: Value, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

pub fn call(name: &str, arguments: Value) -> Value {
    request(json!(1), "tools/call", json!({ "name": name, "arguments": arguments }))
}

/// Dispatch one message and require a response.
pub async fn send(ctx: &McpCtx, registry: &McpRegistry, message: Value) -> Value {
    handle_message(ctx, registry, &message)
        .await
        .expect("expected a response")
}

/// Dispatch a `tools/call` and return its parsed `structuredContent`.
///
/// Panics when the tool reported an error, so a failing call surfaces its own
/// message rather than a confusing assertion further down.
pub async fn call_ok(ctx: &McpCtx, registry: &McpRegistry, name: &str, args: Value) -> Value {
    let response = send(ctx, registry, call(name, args)).await;
    let result = &response["result"];
    assert_eq!(
        result["isError"],
        json!(false),
        "{name} failed: {}",
        result["content"][0]["text"]
    );
    result["structuredContent"].clone()
}

/// Dispatch a `tools/call` expected to fail, returning the error text.
pub async fn call_err(ctx: &McpCtx, registry: &McpRegistry, name: &str, args: Value) -> String {
    let response = send(ctx, registry, call(name, args)).await;
    let result = &response["result"];
    assert_eq!(
        result["isError"],
        json!(true),
        "{name} unexpectedly succeeded: {result}"
    );
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// The JSON-RPC error object from a response.
pub fn rpc_error(response: &Value) -> (i64, String) {
    let error = &response["error"];
    (
        error["code"].as_i64().expect("error code"),
        error["message"].as_str().unwrap_or_default().to_string(),
    )
}
