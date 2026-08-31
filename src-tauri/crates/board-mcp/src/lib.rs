//! MCP server for RustyAgent.
//!
//! Exposes the board, run history, agent configuration, and workspace files to
//! an external MCP client. Two transports share one dispatch and one tool
//! registry:
//!
//! - **stdio** (newline-delimited JSON) — a standalone binary that works with
//!   the desktop app closed. Serves everything backed by the database.
//! - **HTTP** — runs inside the desktop app, so it can also serve live
//!   scheduler and pipeline state. Requires a bearer token.
//!
//! Tools that depend on the running app declare `requires_host()`. On stdio
//! they are hidden from `tools/list` and refused by `tools/call`, rather than
//! answered with a stale default.
//!
//! Deliberately **not** exposed: starting runs or schedulers (they load API keys
//! and spawn user-defined shell commands), reading or writing app settings
//! (plaintext API keys), creating custom shell tools (stored code execution),
//! and destructive deletes beyond `delete_story`.

pub mod ctx;
pub mod jsonrpc;
pub mod protocol;
pub mod registry;
pub mod tools;
pub mod transport;

#[cfg(feature = "http")]
pub mod auth;

/// Bounding list-shaped responses.
///
/// Lives in `tools` because the agent tools this surface adapts need the same
/// bound: an internal agent has the same finite context an external one does,
/// and `list_stories` is read by both. Re-exported here because every tool in
/// `crate::tools` reaches for it as `crate::paging`.
///
/// `::tools` and not `tools`: this crate has a module of that name too, and the
/// local one wins.
pub use ::tools::paging;

pub use ctx::{HostBridge, McpCtx};
pub use protocol::{handle_message, handle_message_refreshed, SUPPORTED_PROTOCOL_VERSIONS};
pub use registry::{McpRegistry, McpTool};
pub use tools::build_registry;

/// Default port for the HTTP transport.
pub const DEFAULT_MCP_PORT: u16 = 8765;

/// Port override, read fresh so a restart picks up a change.
pub fn mcp_port() -> u16 {
    std::env::var("RUSTYAGENT_BOARD_MCP_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(DEFAULT_MCP_PORT)
}
