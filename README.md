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

## Data directory

Everything RustyAgent writes lives under one directory: `rustyagent.db`, `logs/`,
`settings.json`, the MCP auth token, and the per-run git worktrees. By default it
is the Tauri app-data directory named after the bundle identifier — on Windows
that is `%APPDATA%/com.rustyagent.app`; on macOS and Linux the exact location is
whatever Tauri resolves for the platform (typically
`~/Library/Application Support/com.rustyagent.app` and
`~/.local/share/com.rustyagent.app` respectively, though `XDG_DATA_HOME` and
similar can move it).

The `rustyagent-board-mcp` binary has no Tauri handle and so approximates that
default rather than reproducing it. If the two ever disagree on your machine,
set `RUSTYAGENT_DATA_DIR` to make both explicit.

Two environment variables move it. Most specific wins:

| Variable | Moves | Notes |
|---|---|---|
| `RUSTYAGENT_DATA_DIR` | the whole directory — database, logs, worktrees, settings, MCP token | what you want per branch |
| `RUSTYAGENT_DB_PATH` | the database file only | pre-existing, honoured by both binaries; everything else stays with the data directory |

Both the desktop app and the `rustyagent-board-mcp` binary resolve them through
the same helper (`src-tauri/crates/db/src/paths.rs`), so the two never disagree
about which database they opened. Each logs the directory and database it
resolved at startup — the app to its log file, the stdio binary to stderr.

If the directory cannot be created or written to, startup fails with a message
naming the path and the variable. It never falls back to the default: a silent
fallback is indistinguishable from the override having worked.

### Per-branch databases

Migrations only move forward, and sqlx refuses to start when a migration is
recorded in `_sqlx_migrations` but missing from the build. So running a branch
that adds a migration against the default database leaves every other build
unable to launch, with no forward path. Give each branch its own directory:

```bash
# bash / Git Bash
RUSTYAGENT_DATA_DIR="$HOME/.rustyagent/$(git branch --show-current)" npm run tauri dev
```

```powershell
# PowerShell
$env:RUSTYAGENT_DATA_DIR = "$HOME\.rustyagent\$(git branch --show-current)"
npm run tauri dev
```

The directory is created on first use, and the app logs that it is starting an
empty database so a fresh board is never mistaken for lost data. Separating dev
from release builds works the same way — point each profile at its own
directory.

The variable is read from the launching environment, so `npm run tauri dev`
picks up a shell export but a release build started from the desktop will not.
That is deliberate: the default has to stay correct for normal users.

## Unattended runs

A run is meant to survive you walking away from it, which takes two things.

**It calls you back.** Desktop notifications are raised once when a run
finishes, once when it fails, and once when a gated tool call is waiting on
your decision — never per token or per tool call. Agents can also raise one
through the `send_notification` tool. Every category has its own switch under
Settings → Notifications, beneath a master toggle. A notification that could
not be delivered — permission refused, or a category switched off — is
reported to the agent as a failure rather than as success, so a model is never
told you know something you do not.

**It waits rather than fail-closing.** A tool call that needs approval parks
the run until you answer, for as long as that takes; the timeline, the run
detail view and a notification all say the run is parked. Set an *approval
timeout* in Settings if you would rather a run end than sit waiting. Expiry is
recorded on the request as `expired`, never as `rejected`: only a decision you
actually made is written down as one.

The run detail view follows a run live, so an autonomous run can be watched
while it works instead of only after it stops.

## When a provider call fails

A rate limit or a dropped connection no longer ends a run. The failed call is
retried **inside** the run, so the conversation, the tool work already done and
the run's worktree all survive — a rate limit fifteen iterations in costs the
wait and nothing else, rather than discarding those fifteen iterations and
paying for them again.

What gets retried is decided by the error, not by a counter:

| Failure | Retried? |
|---|---|
| Rate limited | yes, after the delay the provider asked for |
| Network error, dropped stream | yes, with capped backoff |
| HTTP 5xx, 408, 429 | yes, with capped backoff |
| HTTP 400/401/403/404 and other 4xx | no — a second identical request is refused identically |
| Serialization, missing API key, opaque provider error | no |

The two other ways a run can fail — exhausting `max_iterations`, and exceeding
the context budget under `context_strategy = "full"` — are never retried. Both
are deterministic: a fresh attempt meets the same ceiling with the same prompt.

Backoff is exponential and capped at 30 seconds, scaled by ±25% derived from
the run id so that an outage does not have every run in flight return at the
same instant and reproduce it. A retry that is waiting still answers the stop
button: cancelling interrupts the wait rather than finishing it.

Each retry is written to the run's timeline saying which attempt it was, what
failed, how long it waited, and whether that delay was the provider's number or
ours — so a run that appears to sit still for thirty seconds says why while it
is happening.

The budget is `max_retries` on the agent profile, alongside `max_iterations`.
It defaults to **2** — three attempts in total. Set it to `0` to switch retries
off for a profile.

## The board and the run lifecycle

A story a run picks up moves to `in_progress` when the run starts, and off it
when the run ends:

| The run | The card |
|---|---|
| finished | `review` |
| failed | `blocked` |
| was cancelled | `blocked` |
| was interrupted by a restart | `blocked` |

Two of those are deliberate choices rather than obvious ones.

**Success lands in `review`, not `done`.** Agent output nobody has looked at
should not claim to be finished work. `review` clears the in-progress queue and
leaves `done` a human verb.

**Failure lands in `blocked`, never back in `ready`.** A failed story returned
to `ready` while a continuous-mode profile is polling gets re-picked
immediately, and the two loop without bound against work that just failed —
spending API budget with nobody watching.

The board follows these moves as they happen: every in-process writer
announces a change and the open board refetches, debounced so a pipeline
settling several cards causes one fetch. The `rustyagent-board-mcp` binary runs
as a separate process and cannot emit an event at all, so the board also polls
every 15 seconds as a floor, and the header says how stale the view is with a
button to refetch now.

An automatic transition only ever moves a card it can see nobody has decided
about: `ready` to claim it, `in_progress` to settle it. If you moved it
yourself, or the agent called `update_story_status`, that is a decision and it
stands — a run started against a card sitting in `blocked` leaves it there, at
both ends. Chat sessions are rows in the same table but are never
moved, so conversations stay off the board.

Every move *off* `in_progress` is recorded as a `story_status` event on the
timeline of the run that caused it — a run finishing, a pipeline finishing, or
the startup sweep — so a card that moves says where to and why. A card left
alone writes nothing, because nothing happened to it. The move *onto*
`in_progress` needs no event of its own: a card in progress with a run
attached to it is already the record of what claimed it, and when.

To switch the whole behaviour off, set `auto_advance_story_status` to `false`
in the active workspace's settings:

```json
{ "auto_advance_story_status": false }
```

It defaults to on — a board that tells the truth about what is in flight is
the behaviour that should need no configuration.

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

### Response size

These tools answer into your agent's context window, not RustyAgent's, so the
ones that could return unbounded text are capped and pageable.

`read_file` returns at most 32 KB of file text — the same cap, the same 1-based
`offset` / `limit` parameters, and the same truncation marker as the app's
internal `file_read` tool. Over the cap, the `content` ends with a
`[read_file TRUNCATED: …]` line stating the file's real size, the line range
returned, and the `offset` to call again with; the reply also carries
`truncated`, `complete` and `next_offset` fields. The separate 10 MB refusal is
a memory guard and still applies.

`get_run_events`, `get_chat_session_messages` and `list_directory` return a
paged envelope — `{ <items>, offset, returned, total, complete, next_offset }`
— rather than a bare array. They take `offset` and `limit`, stop early when a
page would exceed 32 KB, and cap any single `content`, `tool_input` or
`tool_output` value at 4 KB with a marker in place. A run's event log is the
largest response on this surface: one autonomous run carries the full input and
output of every tool call it made.

`get_run_diff` is **not** capped — a single diff blob can be arbitrarily large.

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
