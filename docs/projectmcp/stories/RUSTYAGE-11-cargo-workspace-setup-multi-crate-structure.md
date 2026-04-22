# RUSTYAGE-11: Cargo workspace setup (multi-crate structure)

- Story ID: b4d80ca1-625c-46c6-84a5-523e7b388cb3
- Story Type: Story
- Status: done
- Priority: Critical
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: phase-1, backend, infrastructure
- Created At: 04/09/2026 20:22:08

## Description

Set up the Rust Cargo workspace with all crates scaffolded and cross-crate dependencies wired. This is the foundational build structure all other backend stories depend on.

**Acceptance Criteria:**
- [ ] Workspace `Cargo.toml` at `src-tauri/` root declares all member crates
- [ ] Crates scaffolded with empty `lib.rs`: `api`, `runtime`, `tools`, `memory`, `db`, `scheduler`, `pipeline`, `commands`
- [ ] Cross-crate dependency declarations correct: `runtime` depends on `api`, `tools`, `memory`, `db`; `commands` depends on `runtime`, `db`; `scheduler` depends on `runtime`, `db`; `pipeline` depends on `runtime`
- [ ] Workspace compiles cleanly (`cargo build --workspace`)
- [ ] Shared dependencies (tokio, serde, uuid, etc.) declared at workspace level to avoid version conflicts
- [ ] Tauri binary crate wired to import from `commands/` crate

**Technical Notes:**
- Use `[workspace.dependencies]` in root Cargo.toml for shared dep versions
- All crates are `lib` crates except the Tauri binary entry point
- This story is a blocker for all other backend stories (RUSTYAGE-1, 4, 5)
