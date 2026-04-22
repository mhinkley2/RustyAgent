# RUSTYAGE-20: Board page: fix bugs and UX issues found in audit

- Story ID: ffb134a8-31e6-4814-bf09-9397c62a8e06
- Story Type: Story
- Status: done
- Priority: High
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: frontend, bug, ux, board, phase-1
- Created At: 04/10/2026 13:57:42

## Description

Fix 10 bugs and UX issues identified in a code audit of the Board page. All items are in the existing Board components — no new features, no new pages.

---

## Bug Fixes

### 1. CSS class mismatch on human lane label
**File:** `KanbanView.tsx` and `board.css`

In `KanbanView.tsx` the element uses `className="kb__human-label"` but `board.css` defines `.kb__human-lane-label`. The amber warning color for the "Needs your input" label never applies.

**Fix:** Change the class in `KanbanView.tsx` from `kb__human-label` to `kb__human-lane-label`.

---

### 2. Dragging cards have no visual feedback
**File:** `KanbanView.tsx`, `StoryCard` in `KanbanView.tsx`, `board.css`

`draggingId` is tracked in state but never applied to anything — it is only used in a lint-suppression comment `{draggingId && null}`. The card being dragged looks identical to every other card.

**Fix:**
- Pass `isDragging` boolean prop to `StoryCard` (true when `story.id === draggingId`)
- Add `.story-card--dragging` class when `isDragging` is true
- In `board.css`, style `.story-card--dragging` with `opacity: 0.4` and `border-style: dashed`

---

### 3. "Respond" banner button does nothing
**File:** `BoardPage.tsx`

The "Respond" button in the `hitl-banner` has an empty click handler. If a user dismisses the `HumanInputDialog` via "Dismiss", they cannot reopen it by clicking "Respond" because `dismissedHumanIds` still contains the request ID.

**Fix:**
- On "Respond" button click: remove the active human request ID from `dismissedHumanIds`, which will cause `activeHumanRequest` to become non-null and reopen the dialog.

---

### 4. Bulk delete is a stub — silently does nothing
**File:** `ListView.tsx`

The `BulkActionBar` `onDelete` callback clears selection but has a `// TODO` comment and never calls any delete function. Users see what appears to be a successful delete but nothing is deleted.

**Fix:**
- Wire the bulk delete to `deleteStory` (already available in `BoardPage` via `useStories`).
- Pass an `onDelete` handler down through `ListView` → `BulkActionBar` that calls `deleteStory` for each selected ID.
- Show a `ConfirmDialog` before deleting: "Delete X stories? This cannot be undone."
- Clear selection and show a success toast after deletion.
- `ListView` will need a new `onDeleteStories?: (ids: string[]) => void` prop.

---

### 5. "My Stories" filter shows all assigned stories, not current user's
**File:** `BoardPage.tsx`, `FilterBar.tsx`

The "mine" quick filter logic is:
```ts
if (filters.quick === "mine" && !s.assignee) return false;
```
This returns stories with *any* assignee, not the current user's stories. Since there is no "current user" concept yet, the pill's behavior is misleading.

**Fix:** Until a current user concept exists, relabel the pill from "My Stories" to "Assigned" in `FilterBar.tsx`. Update `aria-label` and the button text. This makes the actual behavior match the label.

---

### 6. Markdown in story description renders as raw text
**File:** `StoryDetailPanel.tsx`

The description field accepts markdown (the form labels it "Description (markdown)") but the detail panel renders it in a plain `<p>` tag. Backticks, asterisks, and newlines all appear literally.

**Fix:**
- Add a lightweight markdown renderer. Use `react-markdown` (already a common dependency) or a simple home-grown renderer for the subset needed: bold, italic, inline code, fenced code blocks, bullet lists, newlines.
- Replace the `<p className="sdp__description">{story.description}</p>` with a `<Markdown>` component.
- Add prose styles for the rendered markdown in `board.css` under `.sdp__description`.

---

### 7. No active filter indicator on the filter bar
**File:** `FilterBar.tsx`, `board.css`

`activeCount()` is computed but its result is never rendered anywhere. If a user applies filters and navigates away then back, there is nothing to remind them filters are active.

**Fix:**
- Show an active count badge next to the "Priority", "Type", or "Label" group labels when their respective filter arrays are non-empty (e.g., "Priority (2)").
- Show a "Clear all" button at the end of the filter bar when `activeCount > 0`. The CSS class `.filter-bar__clear` already exists — just render the button conditionally.

---

### 8. View toggle labels say "Board" and "List" — should be "Kanban" and "List"
**File:** `BoardPage.tsx`

The `view-toggle` buttons read "Board" and "List". The page is already titled "Board", so the toggle option labeled "Board" is confusing. The actual view type is Kanban.

**Fix:** Change the button label and `aria-label` from "Board" / "Kanban view" to "Kanban" / "Kanban view".

---

### 9. Approval gate dialog is hidden when a human input request is also active
**File:** `BoardPage.tsx`

The approval gate renders only when there is no active human request:
```tsx
{activeApprovalRequest && !activeHumanRequest && (
```
If both conditions exist simultaneously, the approval gate is silently hidden with no indication to the user.

**Fix:**
- When an approval request exists but is blocked by a human input request, show a warning inside the `HumanInputDialog`: e.g., "1 tool approval is also waiting." as a small inline note above the action buttons.
- After the human input is submitted or dismissed, the approval gate dialog will surface naturally.

---

### 10. Kanban empty column message "Drop here" is not a real empty state
**File:** `KanbanView.tsx`, `board.css`

Empty columns render `<div className="kb-col__empty">Drop here</div>`. This communicates drag-and-drop affordance but not meaning — a user new to the board sees "Drop here" in every column with no context.

**Fix:**
- Change the empty message to be column-specific and contextual:
  - Backlog: "No backlog stories"
  - Ready: "Nothing queued yet"
  - In Progress: "No active runs"
  - Blocked: "Nothing blocked"
  - Review: "Nothing in review"
  - Done: "No completed stories"
- Update `KanbanColumn` to accept an `emptyMessage` prop, and pass the column-specific string from `KanbanView`.
- Style: keep existing `.kb-col__empty` style, the text just becomes more meaningful.

---

## Acceptance Criteria
- [ ] Human lane label renders in amber (`.kb__human-lane-label` class applied correctly)
- [ ] Dragged card shows 40% opacity and dashed border; all other cards remain full opacity
- [ ] Clicking "Respond" banner button reopens the HumanInputDialog after it was dismissed
- [ ] Bulk delete shows confirmation dialog, calls delete for all selected IDs, shows success toast
- [ ] Quick filter pill labeled "Assigned" (not "My Stories"); filter logic unchanged
- [ ] Story descriptions render markdown formatting in the detail panel (bold, italic, code, lists)
- [ ] Active filter count shown next to group label when filters are applied
- [ ] "Clear all" button appears in filter bar when any filter is active
- [ ] View toggle buttons read "Kanban" and "List"
- [ ] When both human input and approval requests are pending simultaneously, a note appears in the HumanInputDialog indicating an approval is also waiting
- [ ] Each kanban column has a contextually meaningful empty state message
