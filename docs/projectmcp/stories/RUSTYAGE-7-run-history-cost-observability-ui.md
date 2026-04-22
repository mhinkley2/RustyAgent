# RUSTYAGE-7: Run history & cost observability UI

- Story ID: 70862816-151c-4da5-a8b9-46c0c1797782
- Story Type: Story
- Status: done
- Priority: High
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: phase-1, frontend, observability
- Created At: 04/09/2026 20:04:16

## Description

Build the run history and observability UI. Users can review past runs, see the full event log, understand cost, and export runs as `.jsonl`.

**Acceptance Criteria:**
- [ ] Runs page: list of all story runs with story title, agent, status, duration, token cost
- [ ] Run detail view: full append-only event log (messages, tool calls with inputs/outputs, errors, approval events)
- [ ] Token cost breakdown: input tokens, output tokens, estimated USD cost per run
- [ ] Tool call entries expandable to show full inputs and outputs
- [ ] Filter runs by: agent, story, status, date range
- [ ] Story detail panel shows the most recent completed run inline
- [ ] Per-agent daily token spend visible on agent profile card
- [ ] Export run as `.jsonl` file (one JSON event object per line) for portability and external tooling
