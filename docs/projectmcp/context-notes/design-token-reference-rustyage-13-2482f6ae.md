# Design Token Reference (RUSTYAGE-13)

- Note ID: 2482f6ae-528e-483a-8542-42cd0c7d6024
- Project ID: 792eb04c-6091-419f-bfc2-dc573bef45d2
- Story ID: None
- Parent ID: None
- Order: 12
- Favorited: False
- Created At: 04/09/2026 21:35:52
- Updated At: 04/09/2026 21:35:52

---

# RustyAgent Design Token Reference

Implemented in `src/styles/tokens.css`. All components must use these tokens — no hardcoded colors, sizes, or spacing.

---

## Color Tokens

Use via CSS `var(--token)` or Tailwind utilities (`bg-bg-base`, `text-accent`, etc.).

### Backgrounds
| Token | Value | Usage |
|---|---|---|
| `--bg-base` | `#0d1117` | Outermost canvas |
| `--bg-surface` | `#161b22` | Cards, panels, sidebar |
| `--bg-elevated` | `#1c2128` | Dropdowns, modals, tooltips |
| `--bg-subtle` | `#21262d` | Hover states, striped rows, input bg |

### Borders
| Token | Value | Usage |
|---|---|---|
| `--border` | `#30363d` | Default separator |
| `--border-strong` | `#484f58` | Focused / active selection |

### Text
| Token | Value | Contrast on bg-base | Usage |
|---|---|---|---|
| `--text-primary` | `#e6edf3` | 12.8:1 ✓ AAA | Primary content |
| `--text-secondary` | `#7d8590` | 4.7:1 ✓ AA | Metadata, labels, helper text |
| `--text-muted` | `#484f58` | 2.3:1 | Disabled, placeholder |
| `--text-on-accent` | `#ffffff` | — | Text on colored backgrounds |

### Accent (Brand)
| Token | Value | Usage |
|---|---|---|
| `--accent` | `#4493f8` | Links, selected nav, primary buttons |
| `--accent-subtle` | `#1f2d45` | Tint background for accent areas |

### Semantic Status
| Token | Value | Usage |
|---|---|---|
| `--success` | `#3fb950` | Done / running healthy |
| `--warning` | `#d29922` | Scheduled / awaiting / pending |
| `--error` | `#f85149` | Failed / rejected |
| `--info` | `#58a6ff` | Neutral status / information |

Subtle variants (`--success-subtle`, `--warning-subtle`, etc.) are available for badge backgrounds.

---

## Typography

Font stacks defined as Tailwind `--font-sans` and `--font-mono`.

| Token | Font |
|---|---|
| `--font-sans` | Inter → system-ui → sans-serif |
| `--font-mono` | JetBrains Mono → Fira Code → monospace |

Fonts loaded via Google Fonts in `index.html`.

### Size Scale
| Tailwind class | px | Usage |
|---|---|---|
| `text-xs` | 11px | Timestamps, metadata, badge labels |
| `text-sm` | 13px | Body copy, table rows, helper text |
| `text-base` | 15px | Primary readable content (default) |
| `text-lg` | 17px | Section headings, panel titles |
| `text-xl` | 20px | Page headings |
| `text-2xl` | 24px | Modal titles, hero numbers |

### Font Weights
- 400 body text
- 500 labels and nav items
- 600 headings
- 700 urgent badges only

### Line Height
- `leading-[1.4]` — dense UI (default body)
- `leading-[1.6]` — long-form content (system prompts, descriptions) — use `.prose` class

---

## Spacing Scale (4px base)

Tailwind v4 defaults match exactly — no overrides needed.

| Tailwind | px | 
|---|---|
| `p-1` / `gap-1` | 4px |
| `p-2` / `gap-2` | 8px |
| `p-3` / `gap-3` | 12px |
| `p-4` / `gap-4` | 16px |
| `p-5` / `gap-5` | 20px |
| `p-6` / `gap-6` | 24px |
| `p-8` / `gap-8` | 32px |
| `p-10` / `gap-10` | 40px |
| `p-12` / `gap-12` | 48px |

**Rule**: Use only these named spacing values. No arbitrary values like `p-[13px]`.

---

## Border Radius

| Tailwind class | px | Usage |
|---|---|---|
| `rounded-sm` | 4px | Badges, tags, small chips |
| `rounded` | 6px | Inputs, buttons, card content |
| `rounded-lg` | 8px | Cards, panels, modals |

---

## Shadows

Minimal shadows — use border + bg elevation instead of drop shadows.

| Token | Usage |
|---|---|
| `shadow-sm` | Subtle element separation |
| `shadow-modal` | Modals only |

---

## Dark Mode / Theme Switching

`data-theme="dark"` is set on `<html>` by default (dark-first).  
Switch to light mode by setting `data-theme="light"` — light mode values are defined as a placeholder for future implementation.

---

## Accessibility Rules

1. Never use color alone to convey meaning — always pair with an icon or label
2. Focus rings are always visible (`outline: 2px solid var(--accent)`) — never `outline: none`
3. `prefers-reduced-motion` is respected globally — all animations/transitions disabled
4. Minimum contrast: 4.5:1 for body text (AA), 3:1 for UI elements

---

## File Location

`src/styles/tokens.css` — single source of truth. Both `@theme` (Tailwind utilities) and `var()` aliases are defined here.
