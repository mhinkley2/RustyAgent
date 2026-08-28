//! JSON-RPC 2.0 envelope helpers shared by both transports.

use serde::Serialize;
use serde_json::{json, Value};

pub const JSON_RPC_VERSION: &str = "2.0";

// Standard JSON-RPC codes.
pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

// Implementation-defined range (-32000..=-32099), reserved for servers.
pub const UNAUTHORIZED: i32 = -32001;
pub const FORBIDDEN: i32 = -32002;

#[derive(Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

#[derive(Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

fn serialization_fallback() -> Value {
    json!({
        "jsonrpc": JSON_RPC_VERSION,
        "id": null,
        "error": { "code": INTERNAL_ERROR, "message": "Failed to serialize response" }
    })
}

pub fn success_response(id: Value, result: Value) -> Value {
    serde_json::to_value(JsonRpcResponse {
        jsonrpc: JSON_RPC_VERSION,
        id,
        result: Some(result),
        error: None,
    })
    .unwrap_or_else(|_| serialization_fallback())
}

pub fn error_response(id: Value, code: i32, message: impl Into<String>) -> Value {
    serde_json::to_value(JsonRpcResponse {
        jsonrpc: JSON_RPC_VERSION,
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.into(),
        }),
    })
    .unwrap_or_else(|_| serialization_fallback())
}
