# RUSTYAGE-16: Form patterns: inputs, validation, slide-out panels & confirmation dialogs

- Story ID: ef4776ae-2008-46a3-8b05-92afaa7da2ea
- Story Type: Story
- Status: done
- Priority: High
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: ux, design, design-system, phase-1, frontend, forms
- Created At: 04/09/2026 20:54:38

## Description

Define consistent patterns for all form interactions in the app. Forms appear when creating/editing agent profiles, stories, and MCP servers — all with significantly different field types and complexity levels.

## User Problem
Forms are the primary way users configure the system. Inconsistent form patterns cause confusion, validation errors feel arbitrary, and complex forms (like agent profiles) become exhausting to fill out without good progressive disclosure.

## Form Layout Pattern

### Single-column, label-above
```
┌──────────────────────────────────────────┐
│ Agent Name                 * required    │
│ ┌────────────────────────────────────┐   │
│ │ My Research Agent                  │   │
│ └────────────────────────────────────┘   │
│ Used as the agent's display name         │  ← helper text
│                                          │
│ System Prompt                            │
│ ┌────────────────────────────────────┐   │
│ │ You are a helpful research agent   │   │  ← textarea
│ │ ...                                │   │
│ └────────────────────────────────────┘   │
│                                          │
│ [Cancel]                      [Save]     │  ← footer sticky
└──────────────────────────────────────────┘
```

- Labels always **above** the field, never placeholder-only
- Required fields: subtle asterisk (not red, as red implies error)
- Helper text: below input, text-muted, text-xs
- Never use multi-column layouts on forms inside panels/modals

### Form delivery
- Agent Profile form: **Slide-out panel** (right side, 480px wide) — keeps context visible behind it
- Create/Edit Story: **Modal dialog** — centered, 560px max-width
- MCP Server config: **Slide-out panel** (same as agent profile)
- Settings: **Full page** with sections

## Input Components

### Text Input
```
State mapping:
  Default   → border (--border), bg-subtle
  Focus     → border-strong (accent), ring 2px accent/20%
  Error     → border-error red, error message below
  Disabled  → opacity-50, cursor not-allowed
```

### Textarea (System Prompt, Description)
- Min height: 120px
- Resizable vertically (not horizontally)
- Mono font option for system prompt field (toggle)
- Character counter for fields with limits

### Select / Dropdown
- Custom styled to match theme (not browser-native)
- Provider dropdown → triggers model list refresh: show spinner in model dropdown while loading
- Clear affordance for optional selects (× button on right)

### Toggle / Switch
- Used for: persistent memory, requires_approval, auto_restart
- Right-aligned label: "Persistent Memory [toggle]"
- Tap/click entire row activates toggle

### Number Input with constraints
- Token limit fields: number input with min/max, increment buttons on hover
- Show "unlimited" option as a checkbox beside the input

### Cron Expression Input
- Conditionally shown when run_mode = 'scheduled'
- Plain text input + "Preview" button that shows next 3 scheduled dates
- Placeholder: `0 9 * * 1-5` (weekdays at 9 AM)

### Key-Value Input (MCP env_vars)
```
┌─────────────────┬─────────────────┬───┐
│ Key             │ Value           │ × │
├─────────────────┼─────────────────┼───┤
│ API_URL         │ http://...      │ × │
└─────────────────┴─────────────────┴───┘
[+ Add variable]
```

## Validation

### When to validate
- On blur (when leaving a field) — not on every keystroke
- On submit — full pass; scroll to first error
- Never disable the submit button before submission (user doesn't know why it's disabled)

### Error messages
Pattern: "What happened + how to fix it"
```
✅ "Model is required. Select a model for this provider."
❌ "Invalid selection"
```
- Error text appears below the field, in --error color, with an X icon
- Inline, never in a toast or alert banner

### Confirmation Dialogs (Destructive Actions)
Delete patterns:
```
┌──────────────────────────────────┐
│  Delete "My Research Agent"?     │
│                                  │
│  This will also delete all runs  │
│  and memory for this agent.      │
│  This cannot be undone.          │
│                                  │
│  [Cancel]        [Delete Agent]  │  ← destructive button is red
└──────────────────────────────────┘
```
- Never auto-focus the destructive action
- Describe the exact consequence ("also deletes X runs")

## Slide-out Panel Pattern
```
┌─────────────────────────────────────────────────────────┐
│ main content (dimmed)         │ Create Agent         ×  │
│                               ├─────────────────────────┤
│                               │ [form content]          │
│                               │                         │
│                               │                         │
│                               ├─────────────────────────┤
│                               │ [Cancel]    [Save Agent]│
└───────────────────────────────┴─────────────────────────┘
Width: 480px, full height, slides in from right
Overlay: semi-transparent scrim on left (bg-base/60)
Close: × button, Escape key, clicking scrim
Footer: sticky at bottom with Cancel + primary action
```

## Acceptance Criteria
- [ ] All form inputs use label-above pattern with visible labels (not placeholder-as-label)
- [ ] Input states (default, focus, error, disabled) are visually distinct
- [ ] Validation fires on blur and on submit (not on every keystroke)
- [ ] Error messages include recovery hint, not just "Invalid"
- [ ] Conditional fields (cron expression) show/hide based on related field value
- [ ] Slide-out panel pattern implemented and used for Agent and MCP Server forms
- [ ] Confirmation dialog for all destructive actions; destructive button never auto-focused
- [ ] Key-value input supports add/remove rows for env_vars and args fields
- [ ] Toggle/switch components render correctly with accessible label association
- [ ] Submit button enabled from the start; validation on submit scrolls to first error
