# RUSTYAGE-17: Feedback & action components: buttons, toasts, alerts & tooltips

- Story ID: 33c410dd-4e43-4ebd-870c-0630f596e338
- Story Type: Story
- Status: done
- Priority: High
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: ux, design, design-system, phase-1, frontend, components
- Created At: 04/09/2026 20:55:13

## Description

Define the patterns for feedback, system response, and attention — toast notifications, alert banners, action buttons, and the global button system. These elements tell users what just happened and what they can do next.

## User Problem
Without consistent patterns for feedback and action, users won't know when operations succeed or fail, critical actions will look the same as safe ones, and the app will feel unresponsive or untrustworthy.

## Button System

### Variants
```
Primary    — filled accent color. One per view/modal. "Save Agent", "Run Story"
Secondary  — border only, transparent bg. "Cancel", "Edit"
Ghost      — no border, no bg. Icon-only or subtle text actions in compact UI
Destructive — red variant of Primary. "Delete Agent", "Reject Tool Call"
Link       — underlined text, accent color. Inline links in body copy
```

### Sizes
```
sm   — 28px height, 12px padding. Used inside table rows, badges
md   — 36px height, 16px padding. Default for panels and toolbars
lg   — 44px height, 20px padding. Hero CTAs only
```

### Icon Buttons
- Square aspect ratio version of sm/md
- Tooltip on hover (always — no mystery buttons)
- Focus ring visible for keyboard nav
- Example: [▶] Play, [⏹] Stop, [✏] Edit, [🗑] Delete, [⋮] More

### Button Loading State
When async actions are in flight:
```
[▶ Running...]  ← spinner replaces icon, text changes to "Running…", disabled
```
- Never just disable without visual indication of why

### Button Placement
- Primary right-aligned in forms and panels (Footer: `[Cancel]` left, `[Save]` right)
- Page-level CTAs: top-right of the page header area (`[+ New Agent]`)
- Destructive actions: separated visually or at bottom of group

## Toast Notifications

Used for: results of async operations — saves, runs started, errors, API key saved.

```
Position: top-right, stacked, 320px max-width
Duration: 4s auto-dismiss (error = no auto-dismiss, requires manual close)
Animation: slide in from right, fade out

Variants:
  ✓ Success  — green left border: "Agent saved successfully"
  ✗ Error    — red left border: "Failed to save. Check your API key."
  ⚑ Warning  — amber left border: "Run stopped by user"
  ℹ Info     — blue left border: "Run started for Story #3"

Structure:
  ┌─────────────────────────────────┬───┐
  │ ✓ Agent saved                   │ × │
  │   "My Research Agent" updated.  │   │
  └─────────────────────────────────┴───┘

Rules:
  - Toasts for async operations only (not for successful navigation)
  - Errors in toasts must include an action when possible: [View Details] [Retry]
  - Max 3 toasts visible at once; queue additional
  - Errors stay until dismissed; successes auto-dismiss
```

## Inline Alert Banner
Used for page-level states, not individual field errors.

```
Variants: info | warning | error
Example use: "API key not configured for Anthropic. [Add in Settings →]"

Layout:
  [Icon] Message text          [Action link]   [×]

Position: below page header, full-width, top of content area
Rules:
  - Dismissible unless critical
  - Disappears when the condition is resolved (reactive)
  - Maximum one banner per page at a time
```

## Tooltip Pattern
- Delay: 200ms on hover
- Max width: 240px
- Appears above by default, flips as needed
- Used for: icon-only buttons, truncated text, token counts with full breakdown
- Never put interactive content in a tooltip (use a popover instead)

## Popover Pattern
- Triggered by click (not hover)
- Used for: "More" menus on cards, filter panels, cron schedule preview
- Focused by default when opened
- Closes on outside click or Escape

## Keyboard & Focus Management
- Focus rings are always visible (never `outline: none` without a custom replacement)
- Focus ring style: 2px offset, accent color, rounded to match element
- Modal opens → focus first interactive element inside
- Modal closes → focus returns to the triggering element
- Toast announced by `aria-live="polite"` region

## Acceptance Criteria
- [ ] All 5 button variants implemented with sm/md/lg sizes
- [ ] Icon buttons always have a tooltip
- [ ] Button loading state (spinner + text change) for all async actions
- [ ] Toast component renders success/error/warning/info variants
- [ ] Error toasts do not auto-dismiss; success toasts dismiss after 4s
- [ ] Toast stack limited to 3 visible; extras queued
- [ ] Inline alert banner renders and is dismissible
- [ ] Alerts resolve reactively when underlying condition clears
- [ ] Tooltips appear after 200ms delay and never on click targets
- [ ] All focus rings visible; focus returns to trigger element after modal close
- [ ] `aria-live` region announces toasts to screen readers
