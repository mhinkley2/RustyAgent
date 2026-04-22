# RUSTYAGE-5: Agent execution loop & streaming output

- Story ID: 3185fdb1-eece-45cb-942a-7e1dc847a8ed
- Story Type: Story
- Status: Done
- Priority: Critical
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: phase-1, backend, execution
- Created At: 04/09/2026 20:03:58

## Description

Build the core `ConversationRuntime` in the `runtime/` crate. Each story run is an owned `ConversationRuntime` instance that manages the full execution lifecycle — LLM calls, tool dispatch, permission enforcement, memory, streaming, and event persistence.

**Acceptance Criteria:**
- [ ] `ConversationRuntime` struct owns: run_id, story, profile, provider (Box<dyn LlmProvider>), tool registry, PermissionPolicy, working memory (Vec<Message>), MCP clients, usage tracker, Tauri event sender
- [ ] `start_run(story_id, profile_id)` Tauri command creates and launches a ConversationRuntime in a Tokio task
- [ ] Execution loop: query semantic memory (LanceDB) for relevant context → inject top-K into system prompt → call LLM → handle tool_use → loop until end_turn or limit hit
- [ ] `PermissionPolicy::check()` called before every tool execution; enforced in Rust runtime, not in the tool itself
- [ ] All events persisted append-only to run_events table (message, tool_call, tool_result, thought, error, approval_request)
- [ ] Story status auto-updated (→ In Progress on start, → Done or Failed on end)
- [ ] Budget check before each LLM call; halt if exceeded
- [ ] Max iteration limit per run (configurable per profile, default 20)
- [ ] `stop_run(run_id)` command for user-initiated cancellation via Tokio CancellationToken
- [ ] Streaming tokens emitted via Tauri events to frontend in real time
- [ ] On run end: summarize run → embed via fastembed-rs → store in LanceDB for future semantic retrieval
- [ ] RunPanel component displays live streaming output
- [ ] Built-in tools available: story CRUD, memory read/write, send notification

**Technical Notes:**
- Lives in `crates/runtime/`; depends on `api/`, `tools/`, `memory/`, `db/`
- Permission enforcement at runtime layer — tools declare needs, policy decides
- Working memory is in-process Vec<Message>; flushed/summarized to LanceDB at run end
- Fully testable using `MockLlmProvider` from `api/` crate without real API calls
