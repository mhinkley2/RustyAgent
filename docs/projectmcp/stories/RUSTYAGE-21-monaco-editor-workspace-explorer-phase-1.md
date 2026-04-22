# RUSTYAGE-21: Monaco editor & workspace explorer (Phase 1)

- Story ID: 2474373e-37d2-4bac-84cc-e6c43a9a17b0
- Story Type: Story
- Status: Done
- Priority: High
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: phase-1, frontend, backend, editor
- Created At: 04/10/2026 14:08:32

## Description

Embed Monaco Editor and a file tree explorer so users and agents work in the same environment. Workspace is a locally opened folder.

**Acceptance Criteria:**
- [ ] "Open Workspace" action opens a native OS folder picker; selected path stored in `workspaces` table
- [ ] Recent workspaces list on the Editor page and in a quick-open menu
- [ ] File tree explorer in sidebar: recursive directory listing, icons by file type, expand/collapse folders, respects `.gitignore`
- [ ] Files open in Monaco editor on click; tab-based with unsaved change indicators (dot on tab)
- [ ] Language auto-detected from file extension (syntax highlighting)
- [ ] App theme (dark/light) synced to Monaco theme
- [ ] Monaco configured read-only for files with a pending approval
- [ ] `editor_focus` built-in agent tool surfaces a specific file in the editor
- [ ] `workspace/` crate exposes open folder path to `ConversationRuntime` as `workspace_root`

**Technical Notes:**
- `@monaco-editor/react` (MIT licensed)
- File tree uses `workspace/tree.rs` which calls Tauri `fs` plugin under the hood
- Active workspace path injected into workspace-scoped agent `PermissionPolicy` as default file allow-list root
- Phase 2 additions: live tree refresh via file watcher, diff view in approval panel
