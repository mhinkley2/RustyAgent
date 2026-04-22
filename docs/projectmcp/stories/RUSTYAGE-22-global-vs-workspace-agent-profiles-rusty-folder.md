# RUSTYAGE-22: Global vs workspace agent profiles (.rusty folder)

- Story ID: cfd94820-93af-4fc6-a302-ff5a3d3ed370
- Story Type: Story
- Status: Done
- Priority: High
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: phase-1, backend, frontend, agents
- Created At: 04/10/2026 14:08:45

## Description

Implement the two-scope agent profile system. Global agents live in `~/.rusty/agents/` and are available everywhere. Workspace agents live in `{workspace}/.rusty/agents/` and are scoped to their project. Both are plain TOML files — committable to version control.

**Acceptance Criteria:**
- [ ] `workspace/loader.rs` discovers and parses all TOML agent profiles from both `~/.rusty/agents/` and `{workspace}/.rusty/agents/` on workspace open
- [ ] Workspace agents take priority over globals with the same name
- [ ] Agents page shows both scopes with a visual scope badge (Global / Workspace)
- [ ] Agent profile form has a Scope selector; saving a workspace-scoped agent writes the TOML to `{workspace}/.rusty/agents/{slug}.toml`
- [ ] Agent TOML format fully defined (see Architecture ADR) and round-trips correctly through serde
- [ ] `agent_profiles` SQLite table auto-synced from TOML on load (TOML is source of truth)
- [ ] `.rusty/memory/` created and `.gitignore`-appended automatically on workspace open
- [ ] Global agents without a workspace open have no file system permissions by default
- [ ] Workspace agents default to workspace root as file read allow-list
- [ ] Live reload: editing a `.rusty/agents/*.toml` in the Monaco editor updates the in-app profile immediately (via `notify` watcher)

**Technical Notes:**
- `workspace/` crate handles all `.rusty/` discovery, parsing, and watching
- TOML parsed with `toml` + `serde` crates
- Profile changes during an active run do NOT affect that run's `PermissionPolicy` (immutable per run)
