# RUSTYAGE-29: Custom Shell Command Tools: Let users define runnable agent tools

- Story ID: dc3cffcf-4abe-43f2-8ec4-3372847cd45a
- Story Type: Story
- Status: done
- Priority: Medium
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: tools, shell, customization, db-migration
- Created At: 04/11/2026 00:17:54

## Description

## Problem
Agents are limited to built-in tools and MCP servers. Users cannot give agents the ability to run project-specific shell commands (e.g. `npm run dev`, `cargo test`, `python manage.py migrate`) without writing custom MCP servers.

## Goal
Let users define lightweight shell command tools that agents can call, without requiring external tooling infrastructure.

## User Story
As a user, I want to define custom shell command tools so that agents can run project-specific commands like `npm run dev` or `cargo test` as part of their workflow.

## Acceptance Criteria
- [ ] A new "Custom Tools" section exists in the Agents page (or MCP page) where users can create, edit, and delete shell command tool definitions
- [ ] Each tool definition has: `name` (used by the agent), `description` (sent to the LLM), `command` (the shell command to run), `working_dir` (relative to workspace root, defaults to `.`), and optional `timeout_secs` (default 30)
- [ ] Custom tools are stored in a `custom_tools` DB table and are bindable to agent profiles the same way MCP tools are (via the agent edit form)
- [ ] When an agent calls a custom tool, the backend executes the command in the workspace directory, captures stdout/stderr, and returns combined output as the tool result
- [ ] Security: commands are pre-defined by the user — the agent can only call the tool by name, it cannot modify or inject into the command string; parameterization (if any) uses a strict allowlist of named `{{args}}`
- [ ] A `max_output_bytes` cap (default 32 KB) truncates output with a notice to prevent context flooding
- [ ] Tool execution is non-interactive; if the command requires stdin it will fail cleanly with an error message
- [ ] Custom tool bindings respect the existing agent permission system (can be blocked by `PermissionPolicy`)

## Technical Notes
- New DB table: `custom_tools (id, name, description, command, working_dir, timeout_secs, created_at, workspace_id)`
- New tool type in the tools crate: `ShellCommandTool` implementing the `Tool` trait
- New junction table: `agent_custom_tool_bindings (agent_profile_id, custom_tool_id)`
- Frontend: form similar to `McpServerForm.tsx`, list similar to `McpServerList.tsx`; bind in `AgentProfileForm.tsx`
- Use `tokio::process::Command` for async execution with timeout via `tokio::time::timeout`
- Do NOT pass user-supplied strings directly to the shell — use `Command::new()` with split args, not `sh -c "…user_input…"`
