# RUSTYAGE-19: Story board UX: kanban layout, story cards, filters & detail panel

- Story ID: ecbeb476-1545-408e-9d51-afcdfdaae9d6
- Story Type: Story
- Status: done
- Priority: High
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: ux, design, design-system, phase-1, frontend, board, kanban
- Created At: 04/09/2026 20:56:39

## Description

Define the UX pattern for the Story Board page — the primary workspace where users manage and assign work. This page serves two distinct audiences: humans who create and triage stories, and the app itself showing agent-generated story statuses in real time.

## User Problem
The Board is the central hub of the app. Without a thoughtful layout, users won't be able to quickly see what agents are working on, what's waiting, or what needs their attention. Both kanban and list views need to feel native and coherent.

## Page Header Pattern (applies to all pages)
```
┌─────────────────────────────────────────────────┐
│ Board                            [+ New Story]   │  ← page header
│ ──────────────────────────────────────────────── │
│ [All ▾] [Priority ▾] [Assignee ▾]  [Kanban|List]│  ← filter/view toolbar
└─────────────────────────────────────────────────┘
```

This header pattern is shared across Agents, Runs, and MCP Servers pages. Each page gets:
- Page title (left, `text-xl`, font-weight 600)
- Primary CTA button (right, "New X")
- Filter bar beneath (page-specific filters, left-aligned)
- View switcher (Kanban / List) for Board only

## Kanban View

### Column Layout
```
┌──────────┬──────────┬──────────┬──────────┬──────────┬──────────┐
│ Backlog  │  Ready   │In Progress│ Blocked  │  Review  │   Done   │
│  (3)     │  (5)     │  (2)     │  (1)     │  (0)     │  (12)    │
├──────────┼──────────┼──────────┼──────────┼──────────┼──────────┤
│ [card]   │ [card]   │ [card]   │ [card]   │          │ [card]   │
│ [card]   │ [card]   │ [card]   │          │ Empty ↑  │ [card]   │
│ [card]   │ [card]   │          │          │          │ ...      │
│ ...      │ ...      │          │          │          │ [+12]    │
├──────────┴──────────┴──────────┴──────────┴──────────┴──────────┤
│ Human ★  [card: "Agent needs your input on story pricing…"]      │
└──────────────────────────────────────────────────────────────────┘
```

### Human Stories Section
- Fixed lane at the bottom of the board, **always visible**
- Highlighted with --warning amber border
- Shows all unresolved Human-type stories
- Clicking opens the RunPanel directly to the human input event

### Story Card (Kanban)
```
┌─────────────────────────────┐
│ [●high] Research pricing  ⋮ │  ← priority dot + title + more menu
│ Assigned: GPT-4o Agent      │  ← assigned agent or "Unassigned"
│ #3  ·  task  ·  1h ago      │  ← story key, type, updated time
└─────────────────────────────┘

Height: fixed 88px
Width: fills column (columns fixed width ~180px, horizontal scroll if window narrow)
Hover: border-strong + grab cursor (drag enabled)
Drag: ghost card follows cursor; drop zone columns highlight on hover
```

### Priority Dot Colors
```
● critical  → --error red
● high      → --warning amber
● medium    → --info blue
● low       → --text-muted
```

### Drag and Drop
- Native HTML5 drag or `@dnd-kit` library
- Drop target column flashes with bg-accent-subtle when card is hovering over it
- Dropping updates story status immediately (optimistic update); reverts on error
- Undo: toast appears "Story moved to In Progress [Undo]" with 5s undo window

## List View
```
┌─────┬────────────────────────────┬───────────┬──────────┬──────┬──────────────┐
│ ▢   │ Title                      │ Assignee  │ Priority │ Type │ Updated      │
├─────┼────────────────────────────┼───────────┼──────────┼──────┼──────────────┤
│ ▢   │ Research competitor...     │ GPT-4 Ag  │ ● High   │ task │ 2h ago       │
│ ▢   │ Write blog post            │ Human     │ ● Med    │ human│ yesterday    │
│ ▢   │ Analyze sales data         │ Research  │ ● Crit   │ task │ just now     │
└─────┴────────────────────────────┴───────────┴──────────┴──────┴──────────────┘
- Row click → opens right detail panel (same as Kanban card click)
- Checkbox → multi-select for bulk actions (delete, reassign)
- Bulk action bar appears at bottom when items selected
```

## Story Detail Panel
Opens on right (480px wide, same as form panels) for both views.

```
┌──────────────────────────────────────────────┐
│ #3  Research competitor pricing          [×] │  ← header
│ [● In Progress]  [● High]  [task]            │  ← status + priority + type badges
├──────────────────────────────────────────────┤
│ Assigned to: [GPT-4o Agent ▾]               │  ← inline edit
│ Labels: [phase-1] [research]                 │
│ [+ Edit]                       [▶ Run Now]  │
├──────────────────────────────────────────────┤
│ Description                                  │  ← markdown rendered
│ Research competitor pricing across...        │
├──────────────────────────────────────────────┤
│ Latest Run                                   │
│ [● Done]  12 min  ·  1,204 tokens  ·  $0.02  │  ← run summary
│ [View full run →]                            │
└──────────────────────────────────────────────┘
```

## Filter Bar
- **All / My Stories / Unassigned** — quick pills (not dropdowns) for common filters
- **Priority** — dropdown multi-select
- **Assignee** — dropdown with agent profile avatars/initials
- **Type** — task | human | pipeline
- **Label** — tag multi-select
- Active filter count badge on each dropdown
- "Clear all filters" link appears when any filter is active

## Acceptance Criteria
- [ ] Board renders Kanban and List views; view toggle persists between sessions
- [ ] Kanban has 6 columns: Backlog, Ready, In Progress, Blocked, Review, Done
- [ ] Story cards show title, priority dot, assignee, type, and recency
- [ ] Human stories lane always visible at bottom of kanban; amber bordered
- [ ] Drag and drop moves cards between columns; optimistic update + undo toast
- [ ] Story detail panel opens on card/row click (480px right panel)
- [ ] Inline assignment in detail panel updates without requiring form open
- [ ] Filter bar with quick pills and dropdown filters for priority, assignee, type, label
- [ ] "Clear all" link when filters are active
- [ ] Bulk selection with action bar in list view
- [ ] Page header pattern (title + CTA + filters) reusable across all pages
