//! The tool surface.
//!
//! Registration order is the order `tools/list` reports, so tools are grouped
//! by area to keep that listing readable.

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
