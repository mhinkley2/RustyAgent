use std::env;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::path::PathBuf;

use api::ToolDefinition;
use serde::Serialize;
use serde_json::{json, Value};
use tools::builtin::story::{
    CreateStoryTool, DeleteStoryTool, GetStoryTool, ListStoriesTool, UpdateStoryStatusTool,
    UpdateStoryTool,
};
use tools::{ToolContext, ToolOutput, ToolRegistry};

const JSON_RPC_VERSION: &str = "2.0";
const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const LIST_WORKSPACES_TOOL: &str = "list_workspaces";
const GET_ACTIVE_WORKSPACE_TOOL: &str = "get_active_workspace";
const USE_WORKSPACE_TOOL: &str = "use_workspace";

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

fn default_db_path() -> Result<PathBuf, String> {
    if let Ok(path) = env::var("RUSTYAGENT_DB_PATH") {
        if !path.trim().is_empty() {
            return Ok(PathBuf::from(path));
        }
    }

    if let Ok(appdata) = env::var("APPDATA") {
        return Ok(PathBuf::from(appdata).join("com.rustyagent.dev").join("rustyagent.db"));
    }

    if let Ok(home) = env::var("HOME") {
        return Ok(PathBuf::from(home).join(".local").join("share").join("com.rustyagent.dev").join("rustyagent.db"));
    }

    Err("Unable to determine RustyAgent database path. Set RUSTYAGENT_DB_PATH.".to_string())
}

async fn resolve_workspace_root(db: &db::DbPool) -> Option<PathBuf> {
    if let Ok(path) = env::var("RUSTYAGENT_WORKSPACE_PATH") {
        if !path.trim().is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    db::get_active_workspace_path(db).await
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

fn list_workspaces_result(
    runtime: &tokio::runtime::Runtime,
    db: &db::DbPool,
    current_workspace_root: Option<&PathBuf>,
) -> Result<Value, String> {
    let current_path = current_workspace_root.map(|path| normalize_path(path));
    let workspaces = runtime
        .block_on(db::list_workspaces(db))
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

fn use_workspace_result(
    runtime: &tokio::runtime::Runtime,
    db: &db::DbPool,
    params: &Value,
    current_workspace_root: &mut Option<PathBuf>,
) -> Result<Value, String> {
    let path = params
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "Missing required field: path".to_string())?;
    let resolved = resolve_workspace_input(path)?;
    let workspace = runtime
        .block_on(db::touch_workspace(db, &resolved))
        .map_err(|error| format!("Failed to activate workspace '{}': {error}", resolved.display()))?;

    *current_workspace_root = Some(PathBuf::from(&workspace.path));

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

fn read_message<R: BufRead>(reader: &mut R) -> io::Result<Option<Value>> {
    let mut header_bytes = Vec::new();
    let mut byte = [0_u8; 1];

    loop {
        let bytes = reader.read(&mut byte)?;
        if bytes == 0 {
            if header_bytes.is_empty() {
                return Ok(None);
            }

            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Unexpected EOF while reading MCP headers",
            ));
        }

        header_bytes.push(byte[0]);

        if header_bytes.ends_with(b"\r\n\r\n") || header_bytes.ends_with(b"\n\n") {
            break;
        }
    }

    let header_text = String::from_utf8(header_bytes).map_err(|error| {
        io::Error::new(io::ErrorKind::InvalidData, format!("Invalid MCP header encoding: {error}"))
    })?;

    let mut content_length: Option<usize> = None;
    for line in header_text.lines() {
        let trimmed = line.trim_matches(['\r', '\n']);
        if trimmed.is_empty() {
            continue;
        }

        if let Some((name, value)) = trimmed.split_once(':') {
            if !name.trim().eq_ignore_ascii_case("Content-Length") {
                continue;
            }

            let parsed = value.trim().parse::<usize>().map_err(|error| {
                io::Error::new(io::ErrorKind::InvalidData, format!("Invalid Content-Length: {error}"))
            })?;
            content_length = Some(parsed);
        }
    }

    let length = content_length.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "Missing Content-Length header")
    })?;

    let mut payload = vec![0_u8; length];
    reader.read_exact(&mut payload)?;
    let value = serde_json::from_slice::<Value>(&payload).map_err(|error| {
        io::Error::new(io::ErrorKind::InvalidData, format!("Invalid JSON payload: {error}"))
    })?;

    Ok(Some(value))
}

fn write_message<W: Write>(writer: &mut W, body: &Value) -> io::Result<()> {
    let payload = serde_json::to_vec(body)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("Failed to serialize JSON: {error}")))?;
    write!(writer, "Content-Length: {}\r\n\r\n", payload.len())?;
    writer.write_all(&payload)?;
    writer.flush()
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

fn main() {
    if let Err(error) = run() {
        eprintln!("rustyagent-board-mcp: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let runtime = tokio::runtime::Runtime::new().map_err(|error| format!("Failed to create runtime: {error}"))?;
    let db_path = default_db_path()?;
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create database directory '{}': {error}", parent.display()))?;
    }
    let db = runtime
        .block_on(db::init_db(&db_path.to_string_lossy()))
        .map_err(|error| format!("Failed to initialize database: {error}"))?;
    let mut current_workspace_root = runtime.block_on(resolve_workspace_root(&db));
    let registry = build_registry();

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();

    loop {
        let Some(message) = read_message(&mut reader).map_err(|error| format!("Failed to read MCP message: {error}"))? else {
            break;
        };

        let id = message.get("id").cloned().unwrap_or(Value::Null);
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let params = message.get("params").cloned().unwrap_or(Value::Null);

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
            "exit" => break,
            "tools/list" => {
                let mut tools = registry
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
                        match list_workspaces_result(&runtime, &db, current_workspace_root.as_ref()) {
                            Ok(result) => Some(success_response(id, result)),
                            Err(message) => Some(error_response(id, -32603, message)),
                        }
                    }
                    Some(name) if name == GET_ACTIVE_WORKSPACE_TOOL => {
                        Some(success_response(id, get_active_workspace_result(current_workspace_root.as_ref())))
                    }
                    Some(name) if name == USE_WORKSPACE_TOOL => {
                        match use_workspace_result(&runtime, &db, &arguments, &mut current_workspace_root) {
                            Ok(result) => Some(success_response(id, result)),
                            Err(message) => Some(error_response(id, -32602, message)),
                        }
                    }
                    Some(name) => {
                        if let Some(tool) = registry.get_arc(&name) {
                            let context = ToolContext {
                                db: db.clone(),
                                agent_profile_id: "rustyagent-board-mcp".to_string(),
                                run_id: "rustyagent-board-mcp".to_string(),
                                pipeline_run_id: None,
                                pipeline_depth: 0,
                                spawn_subtask: None,
                                workspace_root: current_workspace_root.clone(),
                            };
                            let output = runtime.block_on(tool.execute(arguments, &context));
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

        if let Some(payload) = response {
            write_message(&mut writer, &payload)
                .map_err(|error| format!("Failed to write MCP response: {error}"))?;
        }
    }

    Ok(())
}