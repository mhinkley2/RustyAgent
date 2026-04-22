# RUSTYAGE-18: Run panel UX: live streaming output, tool call display & approval gates

- Story ID: 28ede63e-079f-42c7-9020-ebee7c36a3ff
- Story Type: Story
- Status: done
- Priority: High
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: ux, design, design-system, phase-1, frontend, run-panel, streaming
- Created At: 04/09/2026 20:55:55

## Description

Design the Run Panel — the most technically complex UI surface in the app. It displays live streaming agent output, tool calls, and run events in real time. Users need to observe what the agent is doing, understand tool actions, and respond to approval gates without losing context.

## User Problem
Watching an agent work is the core observability experience of the app. Without a well-designed streaming panel, tool calls look cryptic, errors are buried, and users can't tell if the agent is making progress or stuck.

## Run Panel Layout

The RunPanel is a split-view that appears when a story is actively running (or viewing a completed run).

```
┌──────────────────────────────────────────────────────────┐
│ ← Story #3: Research competitor pricing    [● Running]   │  ← panel header
│                                    [▶ Tokens: 2,341 in · │
│                                       1,204 out · $0.04] │
├──────────────────────────────────────────────────────────┤
│                                                          │
│  [assistant]  I'll start by searching for...            │  ← streaming text
│                                                          │
│  [tool_call]  ┌──────────────────────────────────────┐  │
│               │ web_search                             │  │
│               │ {"query": "competitor pricing 2026"}  │  │  ← collapsed tool
│               └──────────────────────────────────────┘  │
│                                                          │
│  [tool_result] ┌─────────────────────────────────────┐  │
│               │ Found 3 results:                       │  │
│               │ 1. Acme Corp: $49/mo...               │  │  ← result (auto-truncated)
│               │ [Show full result ▾]                  │  │
│               └─────────────────────────────────────┘   │
│                                                          │
│  [assistant]  Based on the search results...  ▌         │  ← live cursor
│                                                          │
│  ─────────────── Iteration 2 / 20 ───────────────────   │  ← iteration divider
│                                                          │
├──────────────────────────────────────────────────────────┤
│  [Stop Run]                   [↓ Auto-scroll: ON]        │  ← footer bar
└──────────────────────────────────────────────────────────┘
```

## Event Rendering Rules

### Message Events (assistant / user / tool)
- `[assistant]` label in --accent color
- `[user]` label in --text-secondary
- `[tool]` label monospaced in --warning color
- Line-by-line streaming: tokens append character by character with a blinking cursor

### Tool Call Events
```
Collapsed (default):
  [⚙ tool_call] web_search  ▾

Expanded (click to open):
  [⚙ tool_call] web_search
  ┌───────────────────────────────────────┐
  │ {                                     │
  │   "query": "competitor pricing 2026"  │
  │ }                                     │
  └───────────────────────────────────────┘

Rules:
  - JSON highlighted with syntax colors
  - Mono font (JetBrains Mono)
  - "Copy" button appears on hover
  - Max height 200px; overflow scrolls inside the block
```

### Tool Result Events
```
- Shown directly below the matching tool call
- Auto-truncated at 300 chars with [Show full result ▾] toggle
- Error results: red border, error icon, full text always shown
- Copy button on hover
```

### Error Events
```
[✗ error]  ┌─────────────────────────────────────────┐
           │ RateLimitError: Too many requests.       │
           │ Retry in 30 seconds.                     │
           └─────────────────────────────────────────┘
Red border, --error color label
```

### Approval Gate Events (Human-in-the-Loop)
```
[⚠ approval_request]
┌─────────────────────────────────────────────────────┐
│ Approve tool call?                                   │
│                                                      │
│ Tool: delete_file                                    │
│ Input: { "path": "/var/data/report.csv" }            │
│                                                      │
│  [Reject]                           [Approve]        │
└─────────────────────────────────────────────────────┘
Amber border; both buttons always visible
Buttons disabled after response is submitted
```

### Human Input Request Events
```
[👤 human_input]
┌─────────────────────────────────────────────────────┐
│ The agent needs your input:                          │
│                                                      │
│ "Which pricing tier should I focus on? Basic or Pro?"│
│                                                      │
│ ┌─────────────────────────────────────────────────┐ │
│ │ Type your response...                            │ │
│ └─────────────────────────────────────────────────┘ │
│                                               [Send] │
└─────────────────────────────────────────────────────┘
Blue border, textarea auto-focused when appears
```

## Auto-Scroll Behavior
- Auto-scroll to bottom while streaming is active
- **Pause auto-scroll** if user manually scrolls up
- Footer shows "[↑ New content below ▼]" pill when paused + new content arrives  
- Clicking the pill or reaching the bottom resumes auto-scroll

## Token Counter
- Live updating during run
- Compact format: "2,341 in · 1,204 out · $0.04"
- Tooltip on hover: full breakdown (input tokens, output tokens, price per 1M tokens, model)

## Completed Run View
Same panel, but all events static. Header shows duration + final status badge.
Export button: "[↓ Export as .jsonl]"

## Acceptance Criteria
- [ ] RunPanel renders all event types: message, tool_call, tool_result, error, approval_request, human_input
- [ ] Streaming tokens append in real time from Tauri events
- [ ] Tool call events collapsed by default; expand/collapse on click
- [ ] Tool result events truncated at 300 chars with expand toggle
- [ ] Approval gate UI shows both Approve/Reject; disables after response
- [ ] Human input textarea auto-focuses when input request appears
- [ ] Auto-scroll active during streaming; pauses on manual scroll up
- [ ] "Jump to bottom" pill appears when paused and new content arrives
- [ ] Token counter updates live and shows breakdown on hover
- [ ] Completed run shows export button; exports all events as .jsonl
- [ ] JSON in tool calls rendered with syntax highlighting and copy button
- [ ] Error events visually distinct (red border) from successful events
