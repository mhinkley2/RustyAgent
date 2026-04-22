# RUSTYAGE-24: Workspace explorer: context menu & diagnostic colors

- Story ID: 373580be-fcf4-4c65-9621-0717d4506cc4
- Story Type: Story
- Status: done
- Priority: High
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: phase-1, frontend, editor, ux
- Created At: 04/10/2026 14:53:07

## Description

Enhance the workspace file tree with a right-click context menu and Monaco diagnostic-driven color indicators on file and folder nodes.

---

## Part 1 — Right-Click Context Menu

A positioned context menu appears on right-click of any file or folder node in `WorkspaceExplorer`. Menu items vary by node type.

**File menu items:**
- **Rename** — inline rename input replaces the label; confirms on Enter, cancels on Escape
- **Duplicate** — copies file to same directory with ` copy` suffix (e.g., `main copy.rs`); opens in editor
- **Delete** — confirmation prompt then `fs::remove_file`; closes open tab if file is currently open
- **New File** — creates a new empty file in the same directory; opens inline rename input
- **Copy Path** — copies absolute path to clipboard
- **Copy Relative Path** — copies path relative to workspace root
- **Reveal in Explorer** — opens the OS file explorer to the file's location via `shell::open`

**Folder menu items:**
- **New File** — creates new empty file inside the folder; inline rename input
- **New Folder** — creates new subfolder inside the folder; inline rename input
- **Rename** — inline rename of the folder
- **Delete** — confirmation: "Delete folder and all contents?" then `fs::remove_dir_all`
- **Copy Path**
- **Reveal in Explorer**

**Acceptance Criteria:**
- [ ] Right-click on any file or folder node opens the context menu positioned at mouse coords
- [ ] Click outside or press Escape closes the menu
- [ ] Rename shows inline input in place of the label; saves on Enter, cancels on Escape; validates non-empty and no path separator characters
- [ ] Duplicate creates the copy file, tree refreshes, new file opens in editor
- [ ] Delete shows confirmation dialog with file/folder name before proceeding
- [ ] New File / New Folder immediately shows inline rename input for the new node
- [ ] Copy Path / Copy Relative Path writes to clipboard (`navigator.clipboard.writeText`)
- [ ] Reveal in Explorer calls Tauri `shell::open` with the parent directory path
- [ ] All file system operations use Tauri `fs` plugin commands (not raw JS)
- [ ] Tree refreshes after every mutating operation (rename, duplicate, delete, new)

**Technical Notes:**
- `ContextMenu` is a portal-rendered absolutely-positioned component (z-index above editor)
- Use `onContextMenu` on tree node elements; `e.preventDefault()` to suppress browser default
- Tauri commands needed: `rename_path`, `duplicate_file`, `delete_path`, `create_file`, `create_dir`

---

## Part 2 — Diagnostic Color Indicators

File and folder labels in the tree are colorized based on Monaco language diagnostic severity, mirroring VS Code's explorer behavior.

**Color scheme:**
- **Error** (severity 8): label text → `var(--color-error)` (red)
- **Warning** (severity 4): label text → `var(--color-warning)` (amber)
- **No diagnostics**: label text → default color

**Folder propagation**: A folder inherits the worst severity of all its descendants. A folder with one errored child file shows red, even if it has no direct errors itself.

**Acceptance Criteria:**
- [ ] `useFileDiagnostics` hook subscribes to `monaco.editor.onDidChangeMarkers`
- [ ] Maintains a `Map<absolutePath, 'error' | 'warning' | 'none'>` updated on every marker change
- [ ] File tree nodes receive the correct severity class: `.tree-node--error`, `.tree-node--warning`
- [ ] Folder severity is computed as max severity of all descendant files (error > warning > none)
- [ ] Colors update in real time as the user edits and errors appear/resolve in open files
- [ ] Severity classes applied only to the label text span, not the icon or expand chevron
- [ ] Works correctly for files not currently open in the editor (severity = none until opened)

**Technical Notes:**
- `monaco.editor.getModelMarkers({ resource: monaco.Uri.file(path) })` for per-file markers
- Folder severity walk runs on every marker change — debounced 150ms to avoid thrash
- CSS variables `--color-error` and `--color-warning` should already exist in the app theme; use them for consistency
