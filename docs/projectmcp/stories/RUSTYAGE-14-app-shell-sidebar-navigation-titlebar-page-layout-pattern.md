# RUSTYAGE-14: App shell: sidebar navigation, titlebar & page layout pattern

- Story ID: 8aaf356e-028e-4ed5-9f29-71bcac083262
- Story Type: Story
- Status: done
- Priority: Critical
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: ux, design, design-system, phase-1, frontend, navigation
- Created At: 04/09/2026 20:53:12

## Description

Define the persistent app shell — the outer container that wraps every page. Users need a clear, fast way to navigate between the main sections of the app without losing their place.

## User Problem
Without a defined shell, each page would need to invent its own chrome. Navigation would be inconsistent, and users would struggle to orient themselves or switch contexts quickly.

## Layout Pattern

```
┌─────────────────────────────────────────────────────────┐
│  [Titlebar: drag region, window controls on right]       │
├──────┬──────────────────────────────────────────────────┤
│      │                                                  │
│  S   │                                                  │
│  I   │          <Page Content Area>                     │
│  D   │                                                  │
│  E   │                                                  │
│  B   │                                                  │
│  A   │                                                  │
│  R   │                                                  │
│      │                                                  │
├──────┴──────────────────────────────────────────────────┤
│  [Status bar: optional — agent running indicator]        │
└─────────────────────────────────────────────────────────┘
```

## Sidebar
- **Width**: 220px fixed, no collapse (desktop app — always visible)
- **Sections**:
  1. **App logo / name** at top (12px padding all sides)
  2. **Primary nav** (main pages):
     - Agents
     - Board (stories)
     - Runs
     - MCP Servers
     - Settings
  3. **Bottom area**: version number, any global status
- **Nav item states**:
  - Default: text-secondary, no background
  - Hover: bg-subtle, text-primary
  - Active: accent left border (2px), accent text, bg-accent-subtle
- **Icons**: 16px, alongside label (icon + label always — no mystery meat)
- **Attention badge**: red dot on "Board" when Human stories are waiting for input; number badge when > 0 pending approvals

## Custom Titlebar
Tauri apps have a custom titlebar by default. Requirements:
- Drag region covers full width (no clicks in drag zone)
- Window controls (minimize, maximize, close) — native or custom styled to match theme
- Center: current page title (updates on navigation)
- Right of title: breadcrumb if inside a detail view (e.g., "Runs / Run #42")

## Page Content Area
- Full remaining width and height
- Internal scroll per page — never the whole app scrolls
- Max content width: none (full width) — this is a desktop dashboard, not a blog
- Consistent internal padding: 24px horizontal, 20px vertical on the page container

## Routing
- Client-side routing via React Router (or similar)
- Each nav item maps to a top-level route: `/agents`, `/board`, `/runs`, `/mcp`, `/settings`
- Deep links: `/agents/:id`, `/runs/:runId`, etc.
- Browser history not needed (Tauri window); use hash or memory router

## Keyboard Navigation
- `Ctrl+1` through `Ctrl+5` jump to each nav section
- `Escape` closes any open panel/modal
- Tab order follows visual order in sidebar then main content

## Acceptance Criteria
- [ ] App shell renders with sidebar and content area at all times
- [ ] Custom titlebar shows drag region and current page title
- [ ] All 5 primary nav items present with icons and labels
- [ ] Active nav item visually distinct from others
- [ ] "Board" nav item shows badge count for pending human stories
- [ ] Page content area scrolls independently (sidebar is fixed)
- [ ] Keyboard shortcuts `Ctrl+1–5` navigate between sections
- [ ] `Escape` dismisses open modals and panels
- [ ] Shell is responsive to window resize (min window size: 900×600px)
