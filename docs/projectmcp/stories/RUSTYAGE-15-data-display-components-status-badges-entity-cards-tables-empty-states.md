# RUSTYAGE-15: Data display components: status badges, entity cards, tables & empty states

- Story ID: 89e44353-2729-4b22-931f-5ec22cbc49bc
- Story Type: Story
- Status: done
- Priority: High
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: ux, design, design-system, phase-1, frontend, components
- Created At: 04/09/2026 20:53:59

## Description

Define the reusable component patterns for displaying data: status badges, entity cards, tables/lists, and empty states. These components appear on every page of the app (Agents, Board, Runs, MCP Servers).

## User Problem
Without defined patterns, each page invents its own way to show status, cards, and lists. The result is visual inconsistency and wasted implementation time on problems that have already been solved elsewhere.

## Components

### Status Badge
Used everywhere: agent profile status, story status, run status, MCP server status.

```
Visual pattern: small pill with icon + label

Variants (mapped to design tokens):
  ● Running      → --success (green)   icon: pulse/spinner
  ● Done         → --success (muted)   icon: checkmark
  ● Scheduled    → --warning (amber)   icon: clock
  ● Idle         → --text-muted        icon: pause
  ● Failed       → --error (red)       icon: X
  ● Blocked      → --error (red)       icon: lock
  ● Backlog      → --text-muted        icon: circle-dashed
  ● In Progress  → --info (blue)       icon: spinner
  ● Ready        → --info (blue)       icon: arrow-right

Sizes: sm (badge inside table row) | md (default, standalone)
```

Key rule: **Always pair color with an icon and label** — never rely on color alone.

### Entity Card
Used on Agents page and potentially MCP Servers page. A card represents one entity.

```
┌─────────────────────────────────────────┐
│ [Icon] Agent Name              [●Running]│  ← header row
│ gpt-4o · anthropic · manual             │  ← subtitle (model · provider · mode)
├─────────────────────────────────────────┤
│ Description text truncated to 2 lines   │  ← body (optional)
├─────────────────────────────────────────┤
│ 3 stories  │  2.4k tokens  │  $0.03     │  ← stat row
│            [Edit]  [▶ Run]              │  ← actions (on hover)
└─────────────────────────────────────────┘

- bg-surface, rounded-lg, border
- Hover: border-strong, slight bg elevation
- Actions appear on hover, not always visible (keeps default state clean)
- Click card → opens detail panel (not a new page)
```

### Data Table / List Row
Used on Runs page, MCP Servers page, and list views of agents and stories.

```
Row anatomy:
  [Checkbox] [Title]  [Secondary info]  [Status badge]  [Timestamp]  [Actions ...]

Rules:
  - Alternating row bg — use bg-subtle on even rows for scannability
  - Row height: 40px for compact views, 52px when description is shown
  - Actions: appear on row hover as icon buttons (no visible buttons in resting state)
  - Clicking row title → opens detail panel; clicking action icons → discrete action
  - Sticky column headers
  - Sortable columns indicated by sort arrow icon

Pagination vs. infinite scroll:
  - Use pagination for Runs page (runs can be numerous)
  - Use full list for Agents and MCP Servers (generally < 50 items)
```

### Empty State
Every list or board column needs an empty state.

```
Layout (centered in the empty space):
  [Illustration or large icon — 48px]
  [Heading: "No agents yet"]
  [Body: "Create your first agent to start running tasks."]
  [CTA button: "Create Agent" (primary)]

Rules:
  - Never show a blank white box — always explain why it's empty
  - First-time empty state can include a short tip
  - Filter-caused empty state: "No agents match your filters" + "Clear filters" link
  - Illustration: simple, single-color line icon in --text-muted color
```

### Loading State
For async data fetches.

```
- Skeleton screen pattern: placeholder bars matching the shape of expected content
  - Card skeleton: 3 lines of varying width (60%, 40%, 80%)
  - Table skeleton: 5 rows, each with 3 placeholder bars
- Never use a full-page spinner — skeleton is always preferred
- Show skeleton immediately on mount while fetch is in flight
```

## Acceptance Criteria
- [ ] StatusBadge component renders all variants with correct color + icon + label
- [ ] StatusBadge never uses color alone (always paired with icon and text)
- [ ] EntityCard renders header, subtitle, stat row, and hover actions
- [ ] EntityCard hover state elevates border color
- [ ] Table rows have hover reveal for action buttons
- [ ] All list and table pages have an empty state with heading, body, and CTA
- [ ] Empty state distinguishes first-time-empty from filter-caused-empty
- [ ] Skeleton loaders replace spinners for all async fetches
