# RUSTYAGE-23: Permission system — policy engine & approval queue UI

- Story ID: 0caabdfb-3be4-4b12-be4b-c3d484550b1e
- Story Type: Story
- Status: done
- Priority: Critical
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: phase-1, backend, frontend, security, permissions
- Created At: 04/10/2026 14:09:03

## Description

Implement the full permission system — the `PermissionPolicy` Rust engine that enforces what each agent can do, plus the UI for managing permissions and reviewing approval requests.

**Acceptance Criteria:**
- [ ] `PermissionPolicy` struct constructed from agent TOML config at start of each `ConversationRuntime`; immutable for run duration
- [ ] `PermissionPolicy::check(tool, inputs) -> PolicyDecision` called before every tool execution
- [ ] Policy decisions: `Allow`, `Deny(reason)`, `RequiresApproval(request)`
- [ ] Path traversal protection: agents cannot escape allowed roots via `../` or symlinks
- [ ] All policy decisions (allow, deny, approval request + response) recorded as `run_events`
- [ ] Permission editor in agent profile form: visual allow-list builder for file paths, shell commands, network hostnames — no raw TOML editing required
- [ ] Per-permission-type "require approval" toggle (e.g., writes always approved, reads auto-allowed)
- [ ] Workspace config ceiling: workspace `config.toml` can define max permissions; workspace agents cannot exceed them
- [ ] **Approval queue panel** in sidebar with badge count of pending items
- [ ] Approval item shows: agent name, tool name, full inputs, and inline Monaco diff for file writes
- [ ] User can approve or reject each item individually; rejection sends message back to agent
- [ ] Batch approve/reject all pending items in one action

**Technical Notes:**
- Enforcement lives in `crates/runtime/` — frontend cannot bypass
- Approval queue state managed in `approvalsStore.ts` (Zustand) fed by Tauri events
- `require_approval_on_write = true` in agent TOML triggers per-write approval gate
- Permission denied → agent receives structured error and continues its loop
