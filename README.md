# RustyAgent

RustyAgent is a Tauri desktop app with a React frontend and a Rust backend for local agent execution, board management, runs, tools, and MCP integration.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/)
- [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode)
- [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## Development

```bash
npm install
npm run tauri dev
```

## MCP Server

RustyAgent exposes its board, run history, agent configuration, and workspace
files to an external MCP client (Claude Code, VS Code, …) over two transports.

| | HTTP | stdio |
|---|---|---|
| Endpoint | `http://127.0.0.1:8765/mcp` | the `rustyagent-board-mcp` binary |
| Needs the app running | yes | no |
| Authentication | bearer token | none needed (child process) |
| Tools | 35 | 31 |

The four extra tools on HTTP read live scheduler and pipeline state, which only
exists inside the running app. On stdio they are hidden from `tools/list` and
refused by `tools/call` — an explicit error rather than a stale `"idle"`.

### Connecting

The app logs a paste-ready command at startup with your token filled in. It
looks like:

```bash
claude mcp add --transport http rustyagent-board http://127.0.0.1:8765/mcp   --header "Authorization: Bearer $RUSTYAGENT_MCP_TOKEN"
```

Or in `.mcp.json` / `.vscode/mcp.json`:

```json
{
  "servers": {
    "rustyagent-board": {
      "type": "http",
      "url": "http://127.0.0.1:8765/mcp",
      "headers": { "Authorization": "Bearer ${env:RUSTYAGENT_MCP_TOKEN}" }
    }
  }
}
```

For the standalone binary, no token is required:

```bash
claude mcp add rustyagent-board-stdio --   cargo run --manifest-path src-tauri/Cargo.toml --bin rustyagent-board-mcp
```

### Authentication

The token is generated on first launch and persisted at
`%APPDATA%/com.rustyagent.dev/mcp-token`. To rotate it, delete that file and
restart. Resolution order is `RUSTYAGENT_MCP_TOKEN` → the token file →
generate-and-persist. Set `RUSTYAGENT_MCP_ALLOW_ANONYMOUS=1` to disable the
check (logged loudly at startup; intended for debugging).

Requests must also carry a localhost `Host`, and any `Origin` they send must be
a localhost or `tauri://` form. Together with the 127.0.0.1-only bind, that
blocks a web page from reaching the server via DNS rebinding.

### Tools

**Board** — `list_stories`, `get_story`, `create_story`, `update_story`,
`update_story_status`, `delete_story`, `reorder_stories`

**Workspace** — `list_workspaces`, `get_active_workspace`, `use_workspace`,
`get_workspace_settings`, `save_workspace_settings`

**Runs** — `list_runs`, `get_run`, `get_run_events`, `get_run_diff`

**Agents** — `list_agent_profiles`, `get_agent_profile`,
`get_agent_permissions`, `list_pending_human_requests`, `list_pending_approvals`

**Chat** — `list_chat_sessions`, `get_chat_session_messages`,
`create_chat_session`, `append_chat_message`

**Files (read only)** — `list_directory`, `read_file`

**Custom tools (read only)** — `list_custom_tools`, `get_custom_tool_bindings`

**Diagnostics** — `validate_pipeline`, `get_app_logs`

**Live state (HTTP only)** — `get_agent_runtime_status`,
`list_agent_runtime_statuses`, `get_pipeline_progress`, `list_active_pipelines`

### What is deliberately not exposed

An MCP client cannot start agents, read secrets, or destroy data it cannot
recreate:

- **Starting runs, pipelines, or schedulers.** These load your provider API keys
  and can execute the shell commands bound to an agent profile.
- **App settings.** `get_settings` returns all three API keys in cleartext.
- **Creating or binding custom shell tools.** Stored code execution: the command
  runs on the next agent run for the bound profile.
- **`list_mcp_servers`.** Its `env_vars` column is documented as non-secret but
  nothing enforces that, and tokens end up there in practice.
- **Writing or deleting files**, `remove_workspace` (cascades to every story,
  run, and event), `delete_profile`, `delete_run`, and `save_profile_toml`
  (writes to an unsandboxed path).

`delete_story` is the one deletion that is exposed: it predates this list, and a
story is cheap to recreate.

`use_workspace` can only select a workspace you have already opened in the app.
It cannot register a new one — otherwise it plus `read_file` would let a client
read any directory on the machine.
