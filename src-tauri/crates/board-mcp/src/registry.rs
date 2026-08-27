//! The MCP tool abstraction, the registry, and helpers every tool uses.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};
use tools::ToolOutput;

use crate::ctx::McpCtx;

/// One callable MCP tool.
///
/// Distinct from `tools::Tool` (which agents call during a run) because MCP
/// tools take an [`McpCtx`] rather than a `ToolContext`, and because they need
/// to declare whether they depend on the running app.
#[async_trait]
pub trait McpTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;

    /// True when this tool needs [`McpCtx::host`]. Host-only tools are hidden
    /// from `tools/list` and rejected by `tools/call` when no host is present.
    fn requires_host(&self) -> bool {
        false
    }

    /// True when this tool writes. Informational — used for logging.
    fn is_mutation(&self) -> bool {
        false
    }

    async fn call(&self, input: Value, ctx: &McpCtx) -> ToolOutput;
}

// ---------------------------------------------------------------------------
// Helpers used by tool bodies
// ---------------------------------------------------------------------------

/// Read a required string argument.
pub fn str_arg(input: &Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Read an optional string argument (absent and blank both yield `None`).
pub fn opt_str_arg(input: &Value, key: &str) -> Option<String> {
    str_arg(input, key)
}

pub fn opt_i64_arg(input: &Value, key: &str) -> Option<i64> {
    input.get(key).and_then(Value::as_i64)
}

/// Turn a command's `Result<T, String>` into a tool output, serializing `T`.
pub fn json_result<T: Serialize>(result: Result<T, String>) -> ToolOutput {
    match result {
        Ok(value) => match serde_json::to_string(&value) {
            Ok(text) => ToolOutput::ok(text),
            Err(error) => ToolOutput::err(format!("Failed to serialize result: {error}")),
        },
        Err(message) => ToolOutput::err(message),
    }
}

/// Wrap an already-built JSON value as a successful tool output.
pub fn json_ok(value: Value) -> ToolOutput {
    match serde_json::to_string(&value) {
        Ok(text) => ToolOutput::ok(text),
        Err(error) => ToolOutput::err(format!("Failed to serialize result: {error}")),
    }
}

/// Convert a tool's output into MCP's `tools/call` result envelope.
///
/// When the content parses as JSON it is emitted twice: pretty-printed as text
/// (for models) and raw in `structuredContent` (for programs).
pub fn tool_output_to_result(output: ToolOutput) -> Value {
    let parsed = serde_json::from_str::<Value>(&output.content).ok();
    let text = if let Some(value) = parsed.as_ref() {
        serde_json::to_string_pretty(value).unwrap_or_else(|_| output.content.clone())
    } else {
        output.content.clone()
    };

    let mut result = json!({
        "content": [ { "type": "text", "text": text } ],
        "isError": output.is_error,
    });

    if let Some(value) = parsed {
        result["structuredContent"] = value;
    }

    result
}

// ---------------------------------------------------------------------------
// Adapter for the existing agent-facing tools
// ---------------------------------------------------------------------------

/// Exposes an existing `tools::Tool` over MCP unchanged.
///
/// Used for the six story tools, which already carry their own name,
/// description, and JSON schema.
pub struct AgentToolAdapter<T: tools::Tool> {
    inner: T,
    mutation: bool,
}

impl<T: tools::Tool> AgentToolAdapter<T> {
    pub fn new(inner: T, mutation: bool) -> Self {
        Self { inner, mutation }
    }
}

#[async_trait]
impl<T: tools::Tool + 'static> McpTool for AgentToolAdapter<T> {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn input_schema(&self) -> Value {
        self.inner.input_schema()
    }
    fn is_mutation(&self) -> bool {
        self.mutation
    }
    async fn call(&self, input: Value, ctx: &McpCtx) -> ToolOutput {
        self.inner.execute(input, &ctx.tool_ctx()).await
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct McpRegistry {
    tools: Vec<Arc<dyn McpTool>>,
}

impl McpRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: impl McpTool + 'static) {
        self.tools.push(Arc::new(tool));
    }

    /// Register an existing agent tool with no rewriting.
    pub fn register_agent_tool(&mut self, tool: impl tools::Tool + 'static, mutation: bool) {
        self.tools
            .push(Arc::new(AgentToolAdapter::new(tool, mutation)));
    }

    /// Tool definitions visible to this session.
    pub fn definitions(&self, host_available: bool) -> Vec<Value> {
        self.tools
            .iter()
            .filter(|tool| host_available || !tool.requires_host())
            .map(|tool| {
                json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "inputSchema": tool.input_schema(),
                })
            })
            .collect()
    }

    /// Look up a callable tool, honouring host availability.
    pub fn get(&self, name: &str, host_available: bool) -> Option<Arc<dyn McpTool>> {
        self.tools
            .iter()
            .find(|tool| tool.name() == name && (host_available || !tool.requires_host()))
            .cloned()
    }

    /// Every registered name, including host-only ones. Used by tests.
    pub fn all_names(&self) -> Vec<&str> {
        self.tools.iter().map(|tool| tool.name()).collect()
    }
}

/// Define an [`McpTool`] with its schema and body inline.
///
/// The schema is the irreducible part; everything else is boilerplate this
/// removes. Bodies are plain `async` — no boxed futures.
#[macro_export]
macro_rules! mcp_tool {
    (
        $vis:vis $struct:ident,
        name        = $name:literal,
        description = $desc:literal,
        $(host_only  = $host:literal,)?
        $(mutation   = $mutation:literal,)?
        schema      = $schema:tt,
        |$input:ident, $ctx:ident| $body:block
    ) => {
        $vis struct $struct;

        #[::async_trait::async_trait]
        impl $crate::McpTool for $struct {
            fn name(&self) -> &str { $name }
            fn description(&self) -> &str { $desc }
            fn input_schema(&self) -> ::serde_json::Value { ::serde_json::json!($schema) }
            $(fn requires_host(&self) -> bool { $host })?
            $(fn is_mutation(&self) -> bool { $mutation })?

            #[allow(unused_variables)]
            async fn call(
                &self,
                $input: ::serde_json::Value,
                $ctx: &$crate::McpCtx,
            ) -> ::tools::ToolOutput $body
        }
    };
}
