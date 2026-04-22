# RUSTYAGE-30: Kanban Drag & Drop: Within-column story reordering with @dnd-kit

- Story ID: 1935d694-b4a2-47ea-a006-daed35c9ca2c
- Story Type: Story
- Status: done
- Priority: Medium
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: kanban, ux, drag-and-drop, db-migration
- Created At: 04/11/2026 00:18:11

## Description

## Problem
The Kanban board supports dragging cards between columns (status changes) but cards cannot be reordered within a column. Users have no way to express priority order visually on the board.

## Goal
Enable full drag-and-drop with within-column reordering so users can arrange stories in the order they want them worked on.

## User Story
As a user, I want to drag and drop stories both between columns and within a column so that I can visually prioritize and organize my backlog.

## Acceptance Criteria
- [ ] Stories can be dragged within a column to change their display order
- [ ] Stories can be dragged between columns (existing behavior preserved), and the card drops at the position where it was released
- [ ] Order is persisted to the database — reloading the page preserves the user's arrangement
- [ ] Drag interactions work with keyboard (arrow keys + space/enter) for accessibility
- [ ] A drop indicator (ghost line or placeholder card) shows where the card will land before release
- [ ] Touch/pointer events work (for future mobile consideration)
- [ ] The list view (`ListView.tsx`) respects the same `sort_order` for consistent ordering

## Technical Notes
- DB migration: add `sort_order INTEGER NOT NULL DEFAULT 0` to `stories` table
- On reorder, issue a batch update to the affected rows (only rows whose `sort_order` changed); avoid updating all rows
- Use `@dnd-kit/core` + `@dnd-kit/sortable` — wrap each `StoryCard` with `useSortable`, each `KanbanColumn`'s card list with `SortableContext`, and the board with `DndContext`
- Replace existing native `draggable`/`onDragStart`/`onDrop` attributes on `StoryCard` with dnd-kit equivalents
- Cross-column move: use `rectIntersection` or `closestCorners` collision detection strategy
- New Tauri command: `batch_update_story_order(updates: Vec<{id, sort_order}>)` for efficient bulk updates
- Animate drag with `CSS.transform` via dnd-kit's `transform` style helper (no layout shift)
