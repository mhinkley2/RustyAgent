# RUSTYAGE-25: App shell redesign: activity bar, resizable side panel & status bar

- Story ID: ddf5f93e-82d7-42a6-aa2a-993fc0765e56
- Story Type: Story
- Status: Done
- Priority: High
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: ux, design, frontend, phase-1, layout
- Created At: 04/10/2026 17:43:32

## Description

Rework the app shell to follow the VS Code spatial model: a compact 48px icon-only activity bar, a resizable per-page primary side panel, and a persistent 28px status bar. This recovers ~172px of horizontal space compared to the current 220px labeled sidebar and moves global status out of the navigation chrome.

## User Problem
The current 220px sidebar takes up significant horizontal real estate on every screen. The labeled nav items consume space that could be given to page content, the kanban board, or run output. There is also no persistent place to surface global app state (agents running, HiTL awaiting) without navigating to a specific page.

---

## 1. Activity Bar (replaces the 220px Sidebar)

**File:** `Sidebar.tsx` → refactor in place, rename component to `ActivityBar`
**File:** `App.css` — replace `.sidebar` block with `.activity-bar` block

### Layout
```
┌────┐
│ 🤖 │  ← app icon (top, no wordmark — clicking navigates to /agents)
├────┤
│ 🤖 │  Agents       Ctrl+1
│ 🗂 │  Board        Ctrl+2
│ 📜 │  Runs         Ctrl+3
│ 💬 │  Chat         Ctrl+4
│ ✏️  │  Editor       Ctrl+5
│ ⚙  │  MCP Servers  Ctrl+6
│    │
│    │  (flex spacer)
│    │
│ ⚙  │  Settings     (pinned to bottom)
└────┘
```

### Specs
- **Width:** 48px, fixed, no collapse
- **Item height:** 48px, full width
- **Icon size:** 20px
- **No labels visible** — label shown only in tooltip on hover (200ms delay)
- **Tooltip format:** `"Agents   Ctrl+1"` — name left, shortcut right, separated by spaces
- **Active state:** 2px left accent border + `bg-accent-subtle` background fill
- **Hover state:** `bg-subtle`, no border
- **Settings** pinned to bottom with a `margin-top: auto` spacer above it — separated visually from nav items
- **App icon** at top: `Bot` icon in accent color, 22px, links to `/agents` — replaces the current logo wordmark block
- **Badges:** Red dot only (no count number) positioned top-right of the icon. A dot is sufficient — detail is in the status bar. Use `aria-label` to communicate count to screen readers.
- **Wordmark** "RustyAgent" moves to the titlebar left side (see section 4)

### Component changes
- Rename `Sidebar` → `ActivityBar` in `Sidebar.tsx` and update `AppShell.tsx` import
- Remove `sidebar__logo`, `sidebar__logo-name`, `sidebar__label` elements
- Tooltip: use a CSS `::after` pseudo-element or a small `<span role="tooltip">` — no third-party library required
- Badge: change from count pill to a simple 8px dot (`.activity-bar__badge`)

---

## 2. Resizable Primary Side Panel

**New file:** `src/components/layout/SidePanel.tsx`
**New CSS:** `.side-panel` block in `App.css`

The primary side panel sits between the activity bar and the main content area. It is:
- **Always structurally present** in the DOM (for clean keyboard/focus management)
- **Toggled open/closed** by clicking the currently-active activity bar item again
- **Resizable** via a drag handle on its right edge
- **Per-page content** is passed in by each page via a slot/prop

### Layout
```
┌────┬──────────────────┬──────────────────────────────────────┐
│    │                  │                                      │
│ A  │  Side Panel      │   Main Content  <Outlet />           │
│ c  │  240px default   │   (fills remaining width)            │
│ t  │                  │                                      │
│ B  │  Page-specific   │                                      │
│ a  │  list/tree/nav   │                                      │
│ r  │                  │                                      │
│    │       ↕          │                                      │
│    │  drag to resize  │                                      │
└────┴──────────────────┴──────────────────────────────────────┘
```

### Spec
- **Default width:** 240px
- **Min width:** 160px (content becomes unusable below this)
- **Max width:** 480px
- **Snap points:** closed (0px / hidden), 240px (default), 360px (wide)
- **Snap behavior:** when dragged within 20px of closed, snap closed; otherwise snap to nearest named width on mouse-up
- **Drag handle:** 4px wide, full height, `cursor: col-resize`, visible on hover as `--border-strong`
- **Persist width:** store in `localStorage` key `rustyagent.sidepanel.width` — restore on load
- **Toggle:** clicking the active activity bar item toggles the panel open/closed. Persist open/closed state in `localStorage` key `rustyagent.sidepanel.open`
- **Animation:** width transition `150ms ease` when toggling open/closed (not while dragging — disable transition during drag for performance)

### `AppShell` changes
```tsx
// AppShell passes the open/width state down to SidePanel.
// Each page renders its side panel content via a React context or outlet mechanism.
// Simplest approach: a SidePanelContext that pages write into via a useEffect.
```

Recommended pattern — `SidePanelContext`:
- Context exposes: `setPanelContent(node: React.ReactNode)` 
- Pages call `setPanelContent(<MyPagePanel />)` in a `useEffect` on mount, clear on unmount
- `AppShell` renders `<SidePanel>{panelContent}</SidePanel>` between activity bar and `<Outlet />`

### Per-page panel content (initial scope — stubs are acceptable)

| Page | Panel content |
|---|---|
| Agents | Scrollable list of agent profile cards (name + status badge); clicking one selects it in the main area |
| Board | The existing `FilterBar` + a label/assignee quick-nav tree |
| Runs | Scrollable list of recent runs (story title + status + timestamp); clicking one opens run detail in main |
| Chat | Conversation list |
| Editor | File/note tree |
| MCP Servers | Server list + status dots |
| Settings | Category list (General, API Keys, Appearance…) |

**For this story: implement the infrastructure (SidePanelContext, SidePanel component, drag-to-resize, toggle). Per-page panel content can be a `<p className="side-panel__placeholder">Coming soon</p>` stub and filled in alongside each page's own story.**

---

## 3. Status Bar

**New file:** `src/components/layout/StatusBar.tsx`
**New CSS:** `.status-bar` block in `App.css`

A 28px full-width strip pinned to the bottom of the app shell. Replaces sidebar badges as the primary location for global app state.

### Layout
```
┌──────────────────────────────────────────────────────────────────┐
│ ● 2 running   ⚑ 1 awaiting input   ⚡ 3 MCP connected           │  ← left-aligned items
│                                                    v0.1.0  Dark  │  ← right-aligned items
└──────────────────────────────────────────────────────────────────┘
```

### Spec
- **Height:** 28px fixed
- **Background:** `--bg-surface` default; changes to `--warning-subtle` when HiTL is pending
- **Font size:** `--text-xs` (11px)
- **Left items** (spacer-separated):
  - `● N running` — shown only when N > 0; click navigates to `/runs`
  - `⚑ N awaiting input` — shown only when N > 0; click navigates to `/board` and opens HumanInputDialog
  - `⚡ N MCP connected` — shown only when N > 0; click navigates to `/mcp`
- **Right items:**
  - App version `v0.1.0`
  - Current theme toggle: `Dark` / `Light` — clicking toggles `data-theme` on `<html>` and persists to `localStorage`
- **No border-top** — use the background color contrast to separate it from content
- **Clickable items:** subtle hover highlight (`--bg-subtle`), `cursor: pointer`, `border-radius: 3px`

### Data sourcing
- Running agents count: same `useHumanRequests` + a new `useRunningAgents` hook (or extend `useHumanRequests` to also return active run count from a Tauri command `get_active_run_count`)
- MCP connected count: new `useMcpServers` hook (or reuse existing) — count servers with status "running"
- HiTL pending: existing `humanRequests` + `approvalRequests` from `useHumanRequests`

For this story: wire up HiTL count (already available in `AppShell`). Running agents and MCP counts can be `0` stubs with `// TODO` until those Tauri commands exist.

---

## 4. Titlebar: Add App Name to Left

**File:** `Titlebar.tsx`, `App.css`

With the sidebar wordmark removed, add the app name back to the titlebar left side.

```
┌──────────────────────────────────────────────────────┬──────────┐
│ 🤖 RustyAgent   │   Board                            │ — □ ×   │
└────────────────────────────────────────────────────────────────┘
 ↑ left (not draggable)   ↑ center (draggable)          ↑ controls
```

- Left side: `Bot` icon (14px, accent) + `"RustyAgent"` text (`--text-sm`, weight 600)
- Left side is **not** part of the drag region — wrap in `onMouseDown={e => e.stopPropagation()}`
- Clicking the app name navigates to `/agents` (home)
- Center title (current page name) remains centered via `position: absolute; left: 50%`

---

## Acceptance Criteria
- [ ] Activity bar renders at 48px width with icon-only nav items
- [ ] Tooltips appear on hover after 200ms showing label and keyboard shortcut
- [ ] Settings item is pinned to the bottom of the activity bar
- [ ] Active item has 2px accent left border and `bg-accent-subtle` fill
- [ ] Clicking the active activity bar item toggles the side panel open/closed
- [ ] Side panel is resizable by dragging its right edge; respects 160px min and 480px max
- [ ] Side panel width and open/closed state persist in `localStorage`
- [ ] Dragging disables the CSS transition for performance; transition re-enables after drop
- [ ] Snap-to-close: dragging within 20px of 0 snaps the panel closed
- [ ] Per-page side panel content is injected via `SidePanelContext`; all pages show at minimum a stub
- [ ] Status bar renders at 28px with HiTL pending count wired to live data
- [ ] Status bar background changes to `--warning-subtle` when HiTL count > 0
- [ ] Clicking status bar HiTL item navigates to `/board`
- [ ] Theme toggle in status bar switches `data-theme` on `<html>` and persists to `localStorage`
- [ ] Titlebar left side shows app icon + "RustyAgent" wordmark; clicking navigates to `/agents`
- [ ] Titlebar wordmark area is excluded from the drag region
- [ ] Sidebar badges removed (replaced by status bar)
- [ ] Keyboard shortcuts `Ctrl+1–6` still work correctly with the new activity bar
