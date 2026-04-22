# RUSTYAGE-9: MCP server management & tool binding

- Story ID: 9bcd43c7-e1db-4753-a115-55a40a3e214d
- Story Type: Story
- Status: done
- Priority: High
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: phase-2, backend, frontend, mcp
- Created At: 04/09/2026 20:04:38

## Description

Implement MCP server management — user-defined external MCP servers that agents can use as tool groups. The Rust backend manages server lifecycles.

**Acceptance Criteria:**
- [ ] MCP Servers page: list servers with name, command, status (stopped/starting/running/error)
- [ ] Add/edit/remove MCP server config: name, command, args, env vars (non-secret)
- [ ] Rust backend spawns servers as child processes using Tokio; communicates via stdin/stdout JSON-RPC
- [ ] Health check every 30s; auto-restart on crash (max 3 retries)
- [ ] Server logs visible in UI (last N lines)
- [ ] Graceful shutdown on app exit
- [ ] Agent profiles can bind to one or more MCP servers (tool group binding UI)
- [ ] Permission layer: each binding has a custom allow-list for which MCP tools the agent can call
