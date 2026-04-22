# RUSTYAGE-13: Design token foundation: color, typography, spacing & dark mode

- Story ID: 5218fe11-e3fc-4ddc-959e-8ee700e939b1
- Story Type: Story
- Status: Done
- Priority: Critical
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: ux, design, design-system, phase-1, frontend
- Created At: 04/09/2026 20:52:47

## Description

Establish the visual language for the entire app. RustyAgent is a power-user desktop tool — the design language should feel like a premium dev tool (think VS Code, Linear, Raycast): dark-first, dense, calm, and precise.

## User Problem
There is currently no consistent visual language. Each page and future component risks looking and feeling disconnected without a shared foundation to build on.

## Token System

### Color Palette (dark-mode first)
Define semantic color roles — not raw hex values. Every component uses tokens, never hardcoded colors.

```
Background layers:
  --bg-base      Dark canvas (the outermost layer)
  --bg-surface   Cards, panels, sidebars
  --bg-elevated  Dropdowns, modals, tooltips
  --bg-subtle    Hover states, striped rows, input fields

Border:
  --border       Default separator
  --border-strong  Focused element or active selection

Text:
  --text-primary    Primary content
  --text-secondary  Metadata, labels, helper text
  --text-muted      Disabled / placeholder
  --text-on-accent  Text over colored backgrounds

Accent (brand):
  --accent        Primary action color (links, selected nav, primary button)
  --accent-subtle Background tint for accent-related areas

Semantic:
  --success       Done / running healthy
  --warning       Scheduled / awaiting / pending
  --error         Failed / rejected
  --info          Neutral status / information

```

### Typography
- **Font**: Inter (already referenced in codebase) — no change needed
- **Scale** (rem-based, 8px root):
  - `text-xs`  — 11px: timestamps, metadata, labels inside badges
  - `text-sm`  — 13px: body copy, table rows, helper text
  - `text-base` — 15px: primary readable content
  - `text-lg`  — 17px: section headings, panel titles
  - `text-xl`  — 20px: page headings
  - `text-2xl` — 24px: modal titles or hero numbers (token counts)
- **Weights**: 400 body, 500 label/nav, 600 heading, 700 only for urgent badges
- **Line height**: 1.4 for dense UI, 1.6 for long-form content (system prompts, descriptions)
- **Mono font**: `JetBrains Mono` or `Fira Code` for tool inputs/outputs, token counts, JSON

### Spacing Scale (4px base)
```
1 = 4px
2 = 8px
3 = 12px
4 = 16px
5 = 20px
6 = 24px
8 = 32px
10 = 40px
12 = 48px
```
All padding, margin, and gap values must use this scale. No magic numbers.

### Border Radius
- `rounded-sm` — 4px: badges, tags, small chips
- `rounded` — 6px: inputs, buttons, cards inner content
- `rounded-lg` — 8px: cards, panels, modals

### Shadow
Minimal shadows — this is a dark app. Use border + subtle bg elevation instead of drop shadows. Reserve shadows for modals only.

## Accessibility Requirements
- Text contrast ratio minimum 4.5:1 (AA) for all body text on dark backgrounds
- Interactive element contrast 3:1 for borders/icons
- All semantic color tokens must meet AA even for color-blind variants (don't rely on red/green alone — pair with icon or label)
- `prefers-reduced-motion` media query supported from the start

## Acceptance Criteria
- [ ] All color tokens defined in a single CSS variables file (`:root` and `[data-theme=dark]`)
- [ ] Typography scale documented and applied via Tailwind config or CSS vars
- [ ] Spacing scale enforced through Tailwind config (no arbitrary values in components)
- [ ] Dark mode enabled by default; light mode as future option (add `data-theme` attribute on `<html>`)
- [ ] Color contrast passes WCAG AA for all text/background combinations
- [ ] Tokens reference doc created as a context note in ProjectMCP for team reference
