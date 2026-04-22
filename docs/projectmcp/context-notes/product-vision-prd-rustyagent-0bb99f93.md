# Product Vision & PRD — RustyAgent

- Note ID: 0bb99f93-3a5a-4e34-850c-704bcdf14d45
- Project ID: 792eb04c-6091-419f-bfc2-dc573bef45d2
- Story ID: None
- Parent ID: None
- Order: 10
- Favorited: False
- Created At: 04/09/2026 20:02:30
- Updated At: 04/10/2026 14:06:37

---

# RustyAgent — Product Requirements Document

**Date**: April 9, 2026  
**Last updated**: April 10, 2026 (added editor/workspace, global vs workspace agents, permission system)  
**Stack**: Rust (Tauri backend) + React (frontend)  
**Status**: Draft

---

## Problem Statement

Developers and power users need a local-first, privacy-preserving platform to orchestrate AI agents across a variety of tasks — coding, research, brainstorming, tool use — with full control over agent behavior, permissions, scheduling, and collaboration. No existing tool combines native desktop performance, multi-agent coordination, a built-in task board, and an integrated code editor in a single privacy-first package.

---

## Goals & Success Metrics

| Goal | Metric |
|---|---|
| Agents can autonomously complete stories end-to-end | Stories completed without human re-prompting |
| Users trust agents with their file system and tools | Explicit permission model; 0 unauthorized actions |
| Agents collaborate on complex multi-step work | Parallel + handoff pipelines execute correctly |
| App is responsive even during long-running agent tasks | UI never blocks; streaming output always visible |
| Cost is visible and controllable | Per-agent token spend tracked; budget limits enforced |
| Workspace agents are portable and version-controllable | Agent profiles checked into `.rusty/` alongside project code |

---

## User Personas

- **Solo Developer**: Wants agents to write code, run tests, manage PRs — while they focus on higher-level decisions
- **Knowledge Worker**: Needs research, summarization, and brainstorming agents running on a schedule
- **Power User / Builder**: Wants to create complex multi-agent pipelines and share agent profiles

---

## LLM Provider Support

| Provider | Notes |
|---|---|
| **Anthropic** | Claude 3.x / 4.x via API |
| **OpenRouter** | Single API key, access to 100s of models |
| **Local (Ollama)** | No API key needed; fully private; auto-discover running models |

Each agent profile selects a provider + model independently. API keys stored in OS keychain (Windows Credential Manager / macOS Keychain) — never in config files.

---

## Core Feature Areas

### 1. Agent Profiles — Global vs Workspace

Agents exist in one of two scopes:

#### Global Agents
- Stored in `~/.rusty/agents/{agent-name}.toml`
- Available across all workspaces and sessions
- Typically general-purpose: "Researcher", "Code Reviewer", "Brainstormer"
- Managed from the app's global Agents page
- Default file system permission: no workspace path (must be explicitly granted)

#### Workspace Agents
- Stored in `{workspace}/.rusty/agents/{agent-name}.toml`
- Scoped to that specific project workspace
- Automatically available when the workspace is opened
- Can be committed to version control — teammates can share agent configs
- Default file system permission: workspace root (scoped to the project)
- Override or extend global agents within the workspace context

#### Agent Profile Fields (both scopes)
- Name, description, avatar/icon
- Base system prompt (markdown supported)
- Provider + model selection
- Assigned tool groups
- Permission set (see Permissions section)
- Memory settings: context window strategy (rolling/summarized/full), persistent memory on/off
- Cost limits: max tokens per run, max tokens per day
- Run mode: Manual / Continuous / Scheduled (cron expression)
- Scope: `global` | `workspace`

#### `.rusty/` Folder Structure
```
{workspace}/
└── .rusty/
    ├── config.toml          # workspace-level app config
    ├── agents/
    │   ├── dev-agent.toml   # workspace agent definition
    │   └── reviewer.toml
    └── memory/              # workspace-scoped agent memory (gitignored by default)

~/.rusty/
├── config.toml              # global app config
├── agents/
│   ├── researcher.toml      # global agent definitions
│   └── writer.toml
└── memory/                  # global agent memory
```

---

### 2. Permission System

A first-class permission model that governs what each agent is allowed to do. Permissions are defined per agent profile and enforced at the Rust runtime layer — the frontend cannot bypass them.

#### Permission Types

| Permission | Description | Granularity |
|---|---|---|
| **File system** | Read / write / delete files | Allow-list of paths/globs |
| **Shell** | Execute commands | Allow-list of exact commands or globs |
| **Network** | Outbound HTTP/HTTPS requests | Allow-list of hostnames |
| **MCP tools** | Call specific MCP tool names | Allow-list per MCP server binding |
| **Agent spawn** | Create sub-agent tasks | On/off per profile |
| **Story write** | Create/modify stories on the board | On/off per profile |

#### Permission Inheritance
- Workspace agents inherit the workspace root as the default file path allow-list
- Global agents have no file access by default — must be explicitly granted
- Workspace agent permissions can add to but not exceed what the workspace config allows (workspace-level ceiling)

#### Enforcement Flow
```
Agent calls tool →
  PermissionPolicy::check(agent_profile, tool_name, inputs) →
    Denied:           return error to agent
    Allowed:          execute tool
    Requires approval: emit ApprovalRequest → pause → wait for human →
      Approved: execute
      Rejected: return rejection message to agent
```

#### Permission UI
- Permission editor in agent profile form — visual allow-list builder, not raw config
- "Requires approval" toggle per permission type (e.g., writes always require approval, reads don't)
- Approval queue panel: dedicated UI area listing pending approvals (not modal interrupts)
- Per-run permission audit log: every permission check recorded in run_events

---

### 3. Integrated Code Editor & Workspace

An embedded Monaco-based code editor so agents and users work in the same environment.

#### Workspace
- Open any local folder as the active workspace
- File tree explorer in sidebar
- Recent workspaces list
- The active workspace defines the default file scope for workspace agents

#### Editor
- Monaco Editor (MIT licensed — same engine as VS Code)
- Tab-based file management with unsaved change indicators
- Language auto-detection from file extension
- Theme synced with app theme (dark/light)
- Read-only view for agent-generated files before approval

#### Agent Integration
- When an agent writes or modifies a file via its file system tool → change reflected live in the editor
- Diff view (Monaco built-in diff editor) shown in the approval panel when an agent modifies an existing file
- Agents can surface a specific file to the user as part of a human input request
- File tree refreshes in real time via file system watcher

#### Phased Delivery
- **Phase 1**: Open workspace, file tree, Monaco editor with tabs
- **Phase 2**: File watcher → live tree refresh; diff view on agent edits; recent workspaces
- **Phase 3**: Multi-root workspaces; embedded terminal; Git status panel

---

### 4. Story Board (Built-in)
A first-class task management system built into the app.

**Story fields:**
- Title, description (markdown)
- Type: `Code | Research | Brainstorm | Tool | Pipeline | Human`
- Status: `Backlog → Ready → In Progress → Blocked → Review → Done`
- Assignee: an Agent Profile OR a human user (you)
- Priority: Critical / High / Medium / Low
- Labels / tags
- Subtasks (checklist)
- Linked stories (dependencies)
- Attachments (file paths)
- Agent output log (append-only transcript)
- Workspace context (which workspace was open when the story was created)
- Created / updated timestamps

**Human assignee**: When a story is assigned to "Human", the app surfaces it as a notification/prompt for the user to respond. Agents can create these stories to gather information they need. This is the primary human-in-the-loop mechanism.

**Future integrations (not in scope for v1):** Azure DevOps, GitHub Issues, ProjectMCP

---

### 5. Tool System

**Built-in App Tools (always available, no MCP needed):**
- Story CRUD — agents can create, update, query, close stories
- Human input request — create a "Human" story to gather info
- Agent spawn — orchestrator agents can create sub-tasks assigned to other agents
- Memory read/write — access to the agent's persistent memory store
- Notification — send a desktop notification to the user
- Editor focus — surface a specific file in the editor (for human review)

**MCP Tool Groups (user-defined, bound to profiles):**
- File system server (read/write within allowed paths)
- Shell/command server (allowed command list)
- Web search / fetch server
- Custom MCP servers (user adds their own)

MCP servers are managed as child processes by the Rust backend. The UI shows server health, start/stop controls, and logs.

---

### 6. Execution Engine

**Modes:**
- **Manual**: User clicks "Run" on a story or agent
- **Continuous**: Agent monitors its assigned story queue and processes sequentially
- **Scheduled**: Cron expression on the profile (e.g., `0 8 * * *` = daily at 8am)
- **Event-triggered** (Phase 3): File change, new story created, webhook

**Safety:**
- Approval queue: pending approvals listed in dedicated panel — never blocking modal
- Max iteration limit per story run (prevents infinite loops)
- Agent budget limit enforced before each LLM call

---

### 7. Multi-Agent Collaboration

| Pattern | Description |
|---|---|
| **Sequential handoff** | Agent A completes → output passed as context to Agent B |
| **Parallel fan-out** | Orchestrator splits work → multiple agents run concurrently → results merged |
| **Supervisor** | Manager agent delegates sub-stories to specialists, reviews outputs |
| **Shared scratchpad** | All agents in a pipeline can read/write a shared memory space |

Cycle detection halts circular delegation (A → B → A) with a clear error.

---

### 8. Observability & Debugging

- Live token-by-token streaming panel per story run
- Full tool call log (tool name, inputs, outputs, duration)
- Thought chain display (for models that support extended thinking)
- Cost breakdown per run (input tokens, output tokens, estimated cost)
- Run history — replay inputs, compare outputs across runs
- Agent timeline view — who ran what and when
- Permission audit log — every permission check recorded per run

---

## Non-Functional Requirements

| Category | Requirement |
|---|---|
| **Performance** | UI never blocks during agent execution (Tokio async in Rust) |
| **Privacy** | All data local by default; no telemetry |
| **Security** | OS keychain for secrets; explicit permission allow-lists enforced in Rust |
| **Reliability** | Agent state persisted in SQLite; resume after crash |
| **Accessibility** | WCAG 2.1 AA; keyboard navigable story board and editor |
| **Platform** | Windows first; macOS and Linux via Tauri |
| **Portability** | Workspace agent profiles are plain TOML — committable to version control |

---

## Out of Scope (v1)

- Cloud sync or multi-device support
- External issue tracker integrations (ADO, GitHub, Linear)
- Agent marketplace / profile sharing
- Web-based UI (desktop only)
- Multi-user / team collaboration
- Embedded terminal (Phase 3)
- Git status panel (Phase 3)

---

## Open Questions

- [x] ~~What's the right UI pattern for approvals?~~ → Dedicated approval queue panel, not modal
- [ ] Should agents share a global memory pool, or is memory strictly per-profile? (leaning: per-profile + shared scratchpad for pipelines)
- [ ] How should the app handle Ollama model discovery — auto-detect from running Ollama, or user-configured?
- [ ] Should there be a "meta-agent" concept — an agent that reads the story board and auto-assigns stories?
- [ ] Should `.rusty/agents/*.toml` be watched for changes so edits in the editor are reflected live in the app?
- [ ] What happens to workspace agent memory when a workspace is closed — preserved, paused, or cleared?
