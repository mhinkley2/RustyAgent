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

## Board MCP Server

RustyAgent now includes a local app-hosted HTTP MCP server for board CRUD access:

- Endpoint: `http://127.0.0.1:8765/mcp`
- Requirement: the RustyAgent desktop app must be running
- Default scope: the active RustyAgent workspace inside the app
- Standalone fallback binary: `cargo run --manifest-path src-tauri/Cargo.toml --bin rustyagent-board-mcp`
- Default database: `%APPDATA%/com.rustyagent.dev/rustyagent.db`, or `RUSTYAGENT_DB_PATH` when set

Exposed tools:

- `list_stories`
- `get_story`
- `create_story`
- `update_story`
- `update_story_status`
- `delete_story`
- `list_workspaces`
- `get_active_workspace`
- `use_workspace`

The repository includes a workspace MCP configuration in `.vscode/mcp.json` named `rustyagent-board` that connects to the local app-hosted endpoint. Start the RustyAgent app first, then use `MCP: List Servers` in VS Code. Switch workspaces with `use_workspace` when you need to target a different board.