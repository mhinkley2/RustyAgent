//! The tool surface.
//!
//! Registration order is the order `tools/list` reports, so tools are grouped
//! by area to keep that listing readable.
//!
//! ## Response size
//!
//! Every tool here answers into an external agent's context. The four that
//! could return unbounded text are bounded: `read_file` by the shared
//! [`tools::read_cap`] cap, and `get_run_events`, `get_chat_session_messages`
//! and `list_directory` by [`crate::paging`]. `get_app_logs` returns a bounded
//! tail by default.
//!
//! Still unbounded, audited and knowingly left alone rather than overlooked:
//!
//! * `get_run_diff` — one `diff_output` blob, which `commands::runs` itself
//!   notes "can be arbitrarily large". The largest remaining hole on this
//!   surface.
//! * `list_stories` — the shared agent tool, which returns every story's full
//!   `description`. Capping it changes the internal agent tool too, so it is
//!   not a board-mcp-local fix.
//! * `list_runs`, `list_agent_profiles`, `list_workspaces`,
//!   `list_pending_approvals`, `list_pending_human_requests`,
//!   `list_custom_tools`, `get_custom_tool_bindings` — fixed-shape rows with no
//!   free-text column, bounded in practice by how many of each thing a user
//!   creates.

pub mod agents;
pub mod board;
pub mod chat;
pub mod custom_tools;
pub mod files;
pub mod host;
pub mod runs;
pub mod workspace;

use crate::registry::McpRegistry;

/// Build the full registry. Host-only tools are included here unconditionally;
/// [`McpRegistry::definitions`] and [`McpRegistry::get`] filter them out when no
/// [`crate::HostBridge`] is present.
pub fn build_registry() -> McpRegistry {
    let mut registry = McpRegistry::new();

    // Board — the six story tools are existing agent tools, adapted unchanged.
    registry.register_agent_tool(tools::builtin::story::ListStoriesTool, false);
    registry.register_agent_tool(tools::builtin::story::GetStoryTool, false);
    registry.register_agent_tool(tools::builtin::story::CreateStoryTool, true);
    registry.register_agent_tool(tools::builtin::story::UpdateStoryTool, true);
    registry.register_agent_tool(tools::builtin::story::UpdateStoryStatusTool, true);
    registry.register_agent_tool(tools::builtin::story::DeleteStoryTool, true);
    registry.register(board::ReorderStoriesTool);

    // Workspace
    registry.register(workspace::ListWorkspacesTool);
    registry.register(workspace::GetActiveWorkspaceTool);
    registry.register(workspace::UseWorkspaceTool);
    registry.register(workspace::GetWorkspaceSettingsTool);
    registry.register(workspace::SaveWorkspaceSettingsTool);

    // Run history
    registry.register(runs::ListRunsTool);
    registry.register(runs::GetRunTool);
    registry.register(runs::GetRunEventsTool);
    registry.register(runs::GetRunDiffTool);

    // Agents
    registry.register(agents::ListAgentProfilesTool);
    registry.register(agents::GetAgentProfileTool);
    registry.register(agents::GetAgentPermissionsTool);
    registry.register(agents::ListPendingHumanRequestsTool);
    registry.register(agents::ListPendingApprovalsTool);

    // Chat
    registry.register(chat::ListChatSessionsTool);
    registry.register(chat::GetChatSessionMessagesTool);
    registry.register(chat::CreateChatSessionTool);
    registry.register(chat::AppendChatMessageTool);

    // Files (read only)
    registry.register(files::ListDirectoryTool);
    registry.register(files::ReadFileTool);

    // Custom tools (read only)
    registry.register(custom_tools::ListCustomToolsTool);
    registry.register(custom_tools::GetCustomToolBindingsTool);

    // Diagnostics, and the live-state tools that need the running app.
    registry.register(host::ValidatePipelineTool);
    registry.register(host::GetAppLogsTool);
    registry.register(host::GetAgentRuntimeStatusTool);
    registry.register(host::ListAgentRuntimeStatusesTool);
    registry.register(host::GetPipelineProgressTool);
    registry.register(host::ListActivePipelinesTool);

    registry
}
