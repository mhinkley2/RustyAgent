# Architecture Decisions — RustyAgent

- Note ID: d486dd7b-dcaa-4d17-b95e-4e63c3e27c59
- Project ID: 792eb04c-6091-419f-bfc2-dc573bef45d2
- Story ID: None
- Parent ID: None
- Order: 11
- Favorited: False
- Created At: 04/09/2026 20:03:15
- Updated At: 04/10/2026 14:08:11

---

# Architecture Decisions — RustyAgent

**Date**: April 9, 2026  
**Last updated**: April 10, 2026 (added .rusty folder, Monaco editor, workspace crate, permission policy engine, approval queue pattern)

---

## Stack

| Layer | Technology | Rationale |
|---|---|---|
| Desktop shell | Tauri 2.x | Rust backend + web frontend; smaller bundle than Electron; OS API access |
| Frontend | React + TypeScript | Component ecosystem; strong typing |
| Styling | Tailwind CSS + shadcn/ui | Utility-first; accessible component primitives |
| Code editor | Monaco Editor (`@monaco-editor/react`) | MIT licensed; same engine as VS Code; built-in diff view |
| Backend | Rust (Tokio async runtime) | Non-blocking execution; safe concurrency; no GC pauses during agent runs |
| Database | SQLite via `sqlx` | Local-first; zero config; ACID; supports WAL mode for concurrent reads |
| Vector store | LanceDB (Rust-native) | Semantic memory retrieval; embedded, no daemon required |
| Embeddings | `fastembed-rs` | Local ONNX embedding models; no API call; works offline |
| Agent config format | TOML | Human-readable; committable to version control; serde-compatible |
| File watching | `notify` Rust crate | Cross-platform FS events for live tree refresh |
| Secrets | OS keychain via `keyring` crate | Windows Credential Manager / macOS Keychain; never flat files |

---

## Workspace Layout (multi-crate)

```
src-tauri/
└── crates/
    ├── api/           # LLM provider clients, SSE streaming, request/response types, MockLlmProvider
    ├── runtime/       # ConversationRuntime, PermissionPolicy, MCP lifecycle, usage tracking
    ├── tools/         # Tool trait, dispatch, built-in tool implementations
    ├── memory/        # Unified memory abstraction (SQLite episodic + LanceDB semantic)
    ├── db/            # sqlx pool, migrations, query helpers
    ├── workspace/     # .rusty folder loading, agent config discovery, file tree, FS watcher
    ├── scheduler/     # Cron + continuous mode polling (background Tokio tasks)
    ├── pipeline/      # Multi-agent orchestration, cycle detection, shared scratchpad
    └── commands/      # Tauri command handlers (thin layer — delegates to crates above)
```

---

## `.rusty` Folder Convention

Agent profiles are plain TOML files stored in `.rusty/` folders. Two scopes:

### Global scope — `~/.rusty/`
```
~/.rusty/
├── config.toml              # Global app preferences
├── agents/
│   ├── researcher.toml      # General-purpose agents available everywhere
│   └── writer.toml
└── memory/                  # Global agent memory (LanceDB + episodic store)
```

### Workspace scope — `{workspace}/.rusty/`
```
{workspace}/
└── .rusty/
    ├── config.toml          # Workspace-level overrides (ceiling permissions, default model, etc.)
    ├── agents/
    │   ├── dev-agent.toml   # Project-specific agents
    │   └── reviewer.toml
    └── memory/              # Workspace-scoped memory (gitignored by default via .gitignore entry)
```

### Agent TOML format
```toml
[agent]
name = "Dev Agent"
description = "Implements code changes within the workspace"
scope = "workspace"            # global | workspace
icon = "code"

[llm]
provider = "anthropic"         # anthropic | openrouter | ollama
model = "claude-opus-4-6"
context_strategy = "rolling"   # rolling | summarized | full
max_tokens_per_run = 50000
max_tokens_per_day = 500000

[run]
mode = "manual"                # manual | continuous | scheduled
cron = ""                      # only used when mode = "scheduled"

[permissions]
# File system: allow-list of paths/globs relative to workspace root
file_read = ["**/*"]
file_write = ["src/**", "tests/**"]
file_delete = []               # empty = denied

# Network: hostname allow-list
network = ["api.anthropic.com", "registry.npmjs.org"]

# Shell: exact command or glob allow-list
shell = ["cargo build", "cargo test", "npm run *"]

# Feature flags
agent_spawn = true             # can create sub-agent tasks
story_write = true             # can create/modify stories
require_approval_on_write = false   # true = every file write needs human approval

[memory]
persistent = true
```

### Loading Priority
When the app resolves agent profiles, workspace agents take priority over globals for the same name. The `workspace/` crate discovers both sets at workspace open time.

### Live Reload
The `workspace/` crate uses `notify` to watch `.rusty/agents/` for TOML changes. Edits made in the Monaco editor or externally are reflected in the app immediately without restart.

---

## Permission System

### PermissionPolicy
Constructed from the agent's TOML config when a `ConversationRuntime` is created. Immutable for the duration of a run — profile changes don't affect in-progress runs.

```rust
struct PermissionPolicy {
    file_read: Vec<GlobPattern>,
    file_write: Vec<GlobPattern>,
    file_delete: Vec<GlobPattern>,
    network_hostnames: Vec<String>,
    shell_commands: Vec<GlobPattern>,
    agent_spawn: bool,
    story_write: bool,
    require_approval_on_write: bool,
    workspace_root: Option<PathBuf>,  // None for global agents without workspace context
}

enum PolicyDecision {
    Allow,
    Deny(String),          // reason string returned to agent
    RequiresApproval(ApprovalRequest),
}

impl PermissionPolicy {
    fn check(&self, tool: &str, inputs: &ToolInputs) -> PolicyDecision { ... }
}
```

### Enforcement Points
- Called inside `ConversationRuntime` before every tool execution
- Runs in Rust — frontend cannot bypass
- Every check (allow, deny, approval request) recorded as a `run_event`
- Path checking uses workspace-relative resolution — agents cannot path-traverse outside allowed roots using `../`

### Approval Queue (not modals)
- Pending approvals emitted as Tauri events
- Stored in `approval_queue` Zustand store in frontend
- Dedicated **Approvals** panel in the UI sidebar — badge count shows pending items
- User can batch-review: see tool name, inputs, and file diff (for file writes) then approve/reject
- Rejected tool: runtime receives rejection message and continues the agent loop without executing

---

## Key Architectural Pattern: `ConversationRuntime`

Each story run is an owned `ConversationRuntime` struct. Owns everything for one agent conversation from start to finish.

```rust
struct ConversationRuntime {
    run_id: Uuid,
    story_id: Uuid,
    profile: AgentProfile,
    provider: Box<dyn LlmProvider>,
    tools: ToolRegistry,
    permissions: PermissionPolicy,      // immutable for run duration
    messages: Vec<Message>,             // working memory (in-process)
    mcp_clients: Vec<McpClient>,
    usage: UsageTracker,
    event_tx: EventSender,             // Tauri event channel → frontend
    cancel: CancellationToken,         // for stop_run()
    workspace_root: Option<PathBuf>,   // active workspace context
}
```

---

## Monaco Editor Integration

### Frontend Components
```
components/
  editor/
    WorkspaceExplorer.tsx    # File tree sidebar (recursive, icons by type)
    EditorPanel.tsx          # Monaco instance, tab bar, unsaved indicators
    EditorTabs.tsx           # Open file tabs with close/dirty state
    DiffViewer.tsx           # Monaco diff editor for agent file change approvals
```

### Hooks / State
```
hooks/
  useWorkspace.ts            # Open folder, file tree state, FS watcher events
  useEditor.ts               # Open files, active tab, unsaved changes
stores/
  workspaceStore.ts          # Active workspace path, file tree nodes
  editorStore.ts             # Open tabs, file contents cache
```

### Agent → Editor Integration
- File system tool writes → Tauri event → `useWorkspace` updates file tree
- Approval for file write → `DiffViewer` shows before/after in approval panel
- `editor_focus` built-in tool → agent can surface a specific file to the user
- Monaco configured read-only for files in pending approval state

### File Watching (Rust)
```
workspace/
  watcher.rs    # notify-based watcher; debounced 200ms; emits file_changed events
  tree.rs       # file tree builder, respects .gitignore
  loader.rs     # .rusty/ config discovery and TOML parsing
```

---

## Session Persistence: Append-Only Event Log

Run events stored as append-only rows in `run_events` — one row per event, never updated. Crash-safe.

```rust
enum EventType {
    UserMessage,
    AssistantMessage,
    ToolCall { tool: String, inputs: Json },
    ToolResult { tool: String, output: Json, duration_ms: u64 },
    PermissionDenied { tool: String, reason: String },
    PermissionApprovalRequested { tool: String, inputs: Json },
    PermissionApprovalGranted,
    PermissionApprovalRejected,
    Thought,
    Error,
    RunCompleted { reason: StopReason },
}
```

Runs exportable as `.jsonl` (one JSON event object per line).

---

## Mock LLM Provider (for testing)

`MockLlmProvider` implements `LlmProvider` with scripted deterministic responses:

```rust
let provider = MockLlmProvider::script(vec![
    Response::text("I will read the file."),
    Response::tool_call("read_file", json!({ "path": "src/main.rs" })),
    Response::text("Done."),
]);
```

Also runnable as a local HTTP server for full integration tests.

---

## Memory Architecture (Three-Tier)

| Tier | Storage | Retrieval | Use case |
|---|---|---|---|
| **Working memory** | `Vec<Message>` in `ConversationRuntime` | Sequential, always in context | Current conversation buffer |
| **Episodic memory** | SQLite `agent_memory` | Exact / filtered lookup by key | Key-value facts, agent notes |
| **Semantic memory** | LanceDB (local vector store) | Similarity search over embeddings | Past run summaries, retrieved context |

Two memory namespaces:
- **Profile-scoped**: each agent profile has its own episodic + semantic memory
- **Shared scratchpad**: pipeline-run-scoped memory shared across all agents in one pipeline

Memory stored alongside the workspace in `.rusty/memory/` (workspace scope) or `~/.rusty/memory/` (global scope).

---

## Data Model (Core Entities)

### `agent_profiles` (SQLite — mirrors loaded TOML, auto-synced)
```
id (uuid)
name, description, icon
scope (global | workspace)
workspace_path (nullable string)
toml_path (string)             -- source TOML file path
system_prompt (text)
provider, model
context_strategy
persistent_memory (bool)
max_tokens_per_run, max_tokens_per_day
run_mode, cron_expression
permissions_json (json)        -- serialized PermissionPolicy
created_at, updated_at
```

### `stories`
```
id (uuid)
title, description (markdown)
story_type (code | research | brainstorm | tool | pipeline | human)
status (backlog | ready | in_progress | blocked | review | done)
assignee_type (agent | human)
assignee_id (nullable uuid)
priority (critical | high | medium | low)
labels (json array)
requires_approval (bool)
parent_story_id (nullable uuid)
workspace_path (nullable string)   -- which workspace this story belongs to
created_at, updated_at
```

### `story_runs`
```
id (uuid)
story_id (fk)
agent_profile_id (fk)
status (running | completed | failed | paused_for_approval)
started_at, ended_at
input_tokens, output_tokens
cost_usd (decimal)
workspace_path (nullable string)
```

### `run_events` (append-only)
```
id (uuid)
run_id (fk)
event_type (see EventType enum)
role (user | assistant | tool | system)
content (text)
metadata (json)
created_at
```

### `agent_memory`
```
id (uuid)
profile_id (fk)
scope (session | persistent | shared_scratchpad)
pipeline_run_id (nullable uuid)   -- set for shared_scratchpad entries
key (string, nullable)
content (text)
created_at, updated_at
```

### `mcp_servers`
```
id (uuid)
name, description
command (string)
args (json array)
env (json -- non-secret env vars only)
status (stopped | starting | running | error)
scope (global | workspace)
workspace_path (nullable string)
```

### `workspaces` (recent workspace list)
```
id (uuid)
path (string, unique)
name (string)
last_opened_at
```

---

## Frontend Architecture (src/)

```
components/
  editor/
    WorkspaceExplorer.tsx    # File tree with icons, expand/collapse
    EditorPanel.tsx          # Monaco editor, tabs, language detection
    DiffViewer.tsx           # Monaco diff editor (for approval flow)
  stories/                   # StoryBoard, StoryCard, StoryDetail, StoryForm
  agents/                    # AgentList, AgentProfileForm, AgentStatusBadge
  runs/                      # RunPanel, StreamingOutput, ToolCallLog, CostSummary
  approvals/                 # ApprovalQueue, ApprovalItem (with diff inline)
  mcp/                       # McpServerList, McpServerCard
  shared/                    # Layout, Sidebar, Notifications
pages/
  Board.tsx                  # Story board (kanban + list); primary UI
  Editor.tsx                 # Monaco editor + workspace explorer
  Agents.tsx                 # Global + workspace agent management
  Runs.tsx                   # Run history; jsonl export
  Settings.tsx               # API keys, preferences
  McpServers.tsx             # MCP server management
hooks/
  useAgent.ts, useStories.ts, useRun.ts, useMcp.ts
  useWorkspace.ts            # Open folder, file tree, FS watcher events
  useEditor.ts               # Open tabs, unsaved state
  useApprovals.ts            # Approval queue state + response handlers
stores/
  workspaceStore.ts, editorStore.ts, approvalsStore.ts
  agentStore.ts, storyStore.ts, runStore.ts
```

---

## Technology Decisions

| Decision | Choice | Rationale |
|---|---|---|
| DB migrations | `sqlx` compile-time checked queries | Type safety; no ORM overhead |
| Cron scheduling | `cron` crate | Lightweight; standard expression format |
| HTTP client | `reqwest` with streaming | Supports SSE/chunked streams |
| Vector store | LanceDB | Rust-native, embedded, no daemon |
| Embeddings | `fastembed-rs` | Local ONNX; offline; no API cost |
| Agent config format | TOML + `serde` | Human-readable; version-controllable |
| File watching | `notify` crate | Cross-platform; debounced events |
| Code editor | `@monaco-editor/react` | MIT; VS Code engine; built-in diff |
| State management | Zustand | Lightweight; no Redux boilerplate |
| UI components | shadcn/ui + Tailwind | Accessible primitives |
| Markdown rendering | `react-markdown` + `remark-gfm` | Story descriptions + agent output |
| Drag-and-drop | `dnd-kit` | Accessible; well-maintained |
| Testing | `MockLlmProvider` + mock HTTP service | Deterministic; no token spend |
