# RUSTYAGE-1: Database foundation & SQLite setup

- Story ID: f8b7071f-d333-4e47-b5da-0c6117ec8b1f
- Story Type: Story
- Status: Done
- Priority: Critical
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: phase-1, backend, infrastructure
- Created At: 04/09/2026 20:03:26

## Description

Set up SQLite with sqlx, WAL mode, and auto-run migrations. Create all core tables. Lives in the `db/` crate of the multi-crate workspace.

**Acceptance Criteria:**
- [ ] sqlx pool initialized at app startup with WAL mode enabled
- [ ] All migration files run automatically on startup via `sqlx::migrate!()`
- [ ] All core tables created with correct schema (see Architecture ADR): agent_profiles, agent_tool_bindings, stories, story_runs, run_events, agent_memory, mcp_servers
- [ ] UUID primary keys used throughout
- [ ] DB file stored in Tauri app data directory
- [ ] `db/` crate exposes pool handle and query helpers used by all other crates

**Technical Notes:**
- The `db/` crate is a workspace dependency imported by `runtime/`, `tools/`, `memory/`, and `commands/` crates
- WAL mode set via `PRAGMA journal_mode=WAL` after pool creation
- All queries use compile-time checked `sqlx::query!()` macros
