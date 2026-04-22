use std::{
    convert::Infallible,
    env,
    net::{SocketAddr, TcpListener},
    path::{Path, PathBuf},
    sync::Arc,
};

use api::ToolDefinition;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{
    body::Incoming,
    header::{ALLOW, CONTENT_TYPE},
    server::conn::http1,
    service::service_fn,
    Method, Request, Response, StatusCode,
};
use hyper_util::rt::TokioIo;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::net::TcpListener as TokioTcpListener;
use tools::builtin::story::{
    CreateStoryTool, DeleteStoryTool, GetStoryTool, ListStoriesTool, UpdateStoryStatusTool,
    UpdateStoryTool,
};
use tools::{ToolContext, ToolOutput, ToolRegistry};
use tracing::{error, info, warn};

const JSON_RPC_VERSION: &str = "2.0";
const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const MCP_ENDPOINT_PATH: &str = "/mcp";
const DEFAULT_MCP_PORT: u16 = 8765;
const LIST_WORKSPACES_TOOL: &str = "list_workspaces";
const GET_ACTIVE_WORKSPACE_TOOL: &str = "get_active_workspace";
const USE_WORKSPACE_TOOL: &str = "use_workspace";

#[derive(Clone)]
struct BoardMcpHttpState {
    app: AppHandle,
    db: db::DbPool,
    registry: Arc<ToolRegistry>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

pub fn board_mcp_http_url() -> String {
    format!("http://127.0.0.1:{}/mcp", board_mcp_port())
}

pub fn spawn_board_mcp_http_server(app: AppHandle, db: db::DbPool) -> Result<(), String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], board_mcp_port()));
    let std_listener = TcpListener::bind(addr)
        .map_err(|error| format!("Failed to bind board MCP HTTP server to {addr}: {error}"))?;
    std_listener
        .set_nonblocking(true)
        .map_err(|error| format!("Failed to set board MCP HTTP listener non-blocking: {error}"))?;

    let state = BoardMcpHttpState {
        app,
        db,
        registry: Arc::new(build_registry()),
    };

    info!("Board MCP HTTP server listening at {}", board_mcp_http_url());

    tauri::async_runtime::spawn(async move {
        let listener = match TokioTcpListener::from_std(std_listener) {
            Ok(listener) => listener,
            Err(error) => {
                error!("Failed to create Tokio board MCP HTTP listener: {error}");
                return;
            }
        };
        let state = Arc::new(state);

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let io = TokioIo::new(stream);
                    let state = state.clone();

                    tokio::spawn(async move {
                        let service = service_fn(move |request| handle_http_request(request, state.clone()));
                        if let Err(error) = http1::Builder::new().serve_connection(io, service).await {
                            warn!("Board MCP HTTP connection failed: {error}");
                        }
                    });
                }
                Err(error) => {
                    error!("Board MCP HTTP accept failed: {error}");
                    break;
                }
            }
        }
    });

    Ok(())
}

fn board_mcp_port() -> u16 {
    env::var("RUSTYAGENT_BOARD_MCP_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(DEFAULT_MCP_PORT)
}

fn build_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ListStoriesTool));
    registry.register(Box::new(GetStoryTool));
    registry.register(Box::new(CreateStoryTool));
    registry.register(Box::new(UpdateStoryTool));
    registry.register(Box::new(UpdateStoryStatusTool));
    registry.register(Box::new(DeleteStoryTool));
    registry
}

fn tool_to_mcp(definition: ToolDefinition) -> Value {
    json!({
        "name": definition.name,
        "description": definition.description,
        "inputSchema": definition.input_schema,
    })
}

fn workspace_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": LIST_WORKSPACES_TOOL,
            "description": "List known RustyAgent workspaces ordered by most recently used. Use this before switching board scope.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": GET_ACTIVE_WORKSPACE_TOOL,
            "description": "Return the workspace currently used for board CRUD in this MCP server session.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": USE_WORKSPACE_TOOL,
            "description": "Switch board CRUD to a specific workspace path for the rest of this MCP server session.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative filesystem path to the workspace folder to use."
                    }
                },
                "required": ["path"]
            }
        }),
    ]
}

fn normalize_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_string()
}

fn resolve_workspace_input(path: &str) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("Missing required field: path".to_string());
    }

    let raw = PathBuf::from(trimmed);
    let candidate = if raw.is_absolute() {
        raw
    } else {
        env::current_dir()
            .map_err(|error| format!("Failed to resolve relative workspace path: {error}"))?
            .join(raw)
    };

    if !candidate.exists() {
        return Err(format!("Workspace path does not exist: {}", candidate.display()));
    }
    if !candidate.is_dir() {
        return Err(format!("Workspace path is not a directory: {}", candidate.display()));
    }

    candidate
        .canonicalize()
        .map_err(|error| format!("Failed to normalize workspace path '{}': {error}", candidate.display()))
}

async fn current_workspace_root(db: &db::DbPool) -> Option<PathBuf> {
    db::get_active_workspace_path(db).await
}

async fn list_workspaces_result(
    db: &db::DbPool,
    current_workspace_root: Option<&PathBuf>,
) -> Result<Value, String> {
    let current_path = current_workspace_root.map(|path| normalize_path(path));
    let workspaces = db::list_workspaces(db)
        .await
        .map_err(|error| format!("Failed to list workspaces: {error}"))?;

    Ok(tool_output_to_result(ToolOutput::ok(
        serde_json::to_string(&json!({
            "workspaces": workspaces
                .into_iter()
                .map(|workspace| {
                    json!({
                        "id": workspace.id,
                        "name": workspace.name,
                        "path": workspace.path,
                        "last_opened_at": workspace.last_opened_at,
                        "created_at": workspace.created_at,
                        "is_active": current_path.as_deref() == Some(workspace.path.as_str()),
                    })
                })
                .collect::<Vec<_>>()
        }))
        .unwrap_or_else(|_| "{\"workspaces\":[]}".to_string()),
    )))
}

fn get_active_workspace_result(current_workspace_root: Option<&PathBuf>) -> Value {
    let workspace = current_workspace_root.map(|path| {
        let normalized = normalize_path(path);
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or(&normalized)
            .to_string();
        json!({
            "name": name,
            "path": normalized,
        })
    });

    tool_output_to_result(ToolOutput::ok(
        serde_json::to_string(&json!({ "workspace": workspace }))
            .unwrap_or_else(|_| "{\"workspace\":null}".to_string()),
    ))
}

async fn use_workspace_result(
    app: &AppHandle,
    db: &db::DbPool,
    params: &Value,
) -> Result<Value, String> {
    let path = params
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "Missing required field: path".to_string())?;
    let resolved = resolve_workspace_input(path)?;
    let workspace = db::touch_workspace(db, &resolved)
        .await
        .map_err(|error| format!("Failed to activate workspace '{}': {error}", resolved.display()))?;

    let active_workspace = app.state::<commands::ActiveWorkspace>();
    active_workspace.set(Some(workspace.id.clone()));

    let payload = commands::Workspace {
        id: workspace.id.clone(),
        name: workspace.name.clone(),
        path: workspace.path.clone(),
        last_opened_at: workspace.last_opened_at.clone(),
        created_at: workspace.created_at.clone(),
    };
    if let Err(error) = app.emit("workspace-changed", &payload) {
        warn!("Failed to emit workspace-changed from board MCP HTTP server: {error}");
    }

    Ok(tool_output_to_result(ToolOutput::ok(
        serde_json::to_string(&json!({
            "workspace": {
                "id": workspace.id,
                "name": workspace.name,
                "path": workspace.path,
                "last_opened_at": workspace.last_opened_at,
                "created_at": workspace.created_at,
            }
        }))
        .unwrap_or_else(|_| "{\"workspace\":null}".to_string()),
    )))
}

fn success_response(id: Value, result: Value) -> Value {
    serde_json::to_value(JsonRpcResponse {
        jsonrpc: JSON_RPC_VERSION,
        id,
        result: Some(result),
        error: None,
    })
    .unwrap_or_else(|_| json!({
        "jsonrpc": JSON_RPC_VERSION,
        "id": null,
        "error": { "code": -32603, "message": "Failed to serialize response" }
    }))
}

fn error_response(id: Value, code: i32, message: impl Into<String>) -> Value {
    serde_json::to_value(JsonRpcResponse {
        jsonrpc: JSON_RPC_VERSION,
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.into(),
        }),
    })
    .unwrap_or_else(|_| json!({
        "jsonrpc": JSON_RPC_VERSION,
        "id": null,
        "error": { "code": -32603, "message": "Failed to serialize response" }
    }))
}

fn tool_output_to_result(output: ToolOutput) -> Value {
    let parsed = serde_json::from_str::<Value>(&output.content).ok();
    let text = if let Some(value) = parsed.as_ref() {
        serde_json::to_string_pretty(value).unwrap_or_else(|_| output.content.clone())
    } else {
        output.content.clone()
    };

    let mut result = json!({
        "content": [
            {
                "type": "text",
                "text": text,
            }
        ],
        "isError": output.is_error,
    });

    if let Some(value) = parsed {
        result["structuredContent"] = value;
    }

    result
}

async fn handle_json_rpc_message(state: &BoardMcpHttpState, message: Value) -> Result<Option<Value>, String> {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let current_workspace_root = current_workspace_root(&state.db).await;

    let response = match method.as_str() {
        "initialize" => Some(success_response(id, json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {
                "tools": {
                    "listChanged": false
                }
            },
            "serverInfo": {
                "name": "rustyagent-board-mcp",
                "version": env!("CARGO_PKG_VERSION")
            }
        }))),
        "notifications/initialized" => None,
        "ping" => Some(success_response(id, json!({}))),
        "shutdown" => Some(success_response(id, Value::Null)),
        "tools/list" => {
            let mut tools = state
                .registry
                .all_definitions()
                .into_iter()
                .map(tool_to_mcp)
                .collect::<Vec<_>>();
            tools.extend(workspace_tool_definitions());
            Some(success_response(id, json!({ "tools": tools })))
        }
        "tools/call" => {
            let tool_name = params
                .get("name")
                .and_then(Value::as_str)
                .map(|value| value.to_string());
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));

            match tool_name {
                Some(name) if name == LIST_WORKSPACES_TOOL => {
                    match list_workspaces_result(&state.db, current_workspace_root.as_ref()).await {
                        Ok(result) => Some(success_response(id, result)),
                        Err(message) => Some(error_response(id, -32603, message)),
                    }
                }
                Some(name) if name == GET_ACTIVE_WORKSPACE_TOOL => {
                    Some(success_response(id, get_active_workspace_result(current_workspace_root.as_ref())))
                }
                Some(name) if name == USE_WORKSPACE_TOOL => {
                    match use_workspace_result(&state.app, &state.db, &arguments).await {
                        Ok(result) => Some(success_response(id, result)),
                        Err(message) => Some(error_response(id, -32602, message)),
                    }
                }
                Some(name) => {
                    if let Some(tool) = state.registry.get_arc(&name) {
                        let context = ToolContext {
                            db: state.db.clone(),
                            agent_profile_id: "rustyagent-board-mcp".to_string(),
                            run_id: "rustyagent-board-mcp".to_string(),
                            pipeline_run_id: None,
                            pipeline_depth: 0,
                            spawn_subtask: None,
                            workspace_root: current_workspace_root,
                        };
                        let output = tool.execute(arguments, &context).await;
                        Some(success_response(id, tool_output_to_result(output)))
                    } else {
                        Some(error_response(id, -32601, format!("Unknown tool: {name}")))
                    }
                }
                None => Some(error_response(id, -32602, "Missing required field: params.name")),
            }
        }
        _ if id.is_null() => None,
        _ => Some(error_response(id, -32601, format!("Method not found: {method}"))),
    };

    Ok(response)
}

async fn handle_http_request(
    request: Request<Incoming>,
    state: Arc<BoardMcpHttpState>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let response = match (request.method(), request.uri().path()) {
        (_, path) if path != MCP_ENDPOINT_PATH => empty_response(StatusCode::NOT_FOUND),
        (&Method::GET, _) => method_not_allowed_response(),
        (&Method::POST, _) => match process_post_request(request, &state).await {
            Ok(response) => response,
            Err(error) => {
                warn!("Board MCP HTTP request failed: {error}");
                json_error_response(StatusCode::BAD_REQUEST, &json!({
                    "jsonrpc": JSON_RPC_VERSION,
                    "error": {
                        "code": -32600,
                        "message": error,
                    }
                }))
            }
        },
        _ => method_not_allowed_response(),
    };

    Ok(response)
}

async fn process_post_request(
    request: Request<Incoming>,
    state: &BoardMcpHttpState,
) -> Result<Response<Full<Bytes>>, String> {
    let body = request
        .into_body()
        .collect()
        .await
        .map_err(|error| format!("Failed to read request body: {error}"))?
        .to_bytes();

    let message: Value = serde_json::from_slice(&body)
        .map_err(|error| format!("Invalid JSON-RPC payload: {error}"))?;
    if message.is_array() {
        return Err("JSON-RPC batches are not supported by rustyagent-board HTTP MCP".to_string());
    }
    if !message.is_object() {
        return Err("JSON-RPC payload must be an object".to_string());
    }

    match handle_json_rpc_message(state, message).await? {
        Some(payload) => Ok(json_success_response(StatusCode::OK, &payload)),
        None => Ok(empty_response(StatusCode::ACCEPTED)),
    }
}

fn empty_response(status: StatusCode) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::new()))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
}

fn method_not_allowed_response() -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::METHOD_NOT_ALLOWED)
        .header(ALLOW, "POST")
        .body(Full::new(Bytes::new()))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
}

fn json_success_response(status: StatusCode, body: &Value) -> Response<Full<Bytes>> {
    json_response(status, body).unwrap_or_else(|| empty_response(StatusCode::INTERNAL_SERVER_ERROR))
}

fn json_error_response(status: StatusCode, body: &Value) -> Response<Full<Bytes>> {
    json_response(status, body).unwrap_or_else(|| empty_response(StatusCode::INTERNAL_SERVER_ERROR))
}

fn json_response(status: StatusCode, body: &Value) -> Option<Response<Full<Bytes>>> {
    let payload = serde_json::to_vec(body).ok()?;
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(payload)))
        .ok()
}