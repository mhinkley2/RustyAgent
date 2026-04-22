# RUSTYAGE-28: Git Diffs: Show file changes per run in Run Detail Panel

- Story ID: 61d2a981-2d80-4c1b-aa5c-125225d4b15e
- Story Type: Story
- Status: done
- Priority: High
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: diffs, observability, git, run-panel
- Created At: 04/11/2026 00:17:35

## Description

## Problem
After an agent runs, users have no way to see which files were changed and what the changes were. The event log shows tool calls but not the actual before/after content of modified files.

## Goal
Surface file-level diffs for every run directly in the Run Detail Panel, so users can review exactly what the agent changed, file by file.

## User Story
As a user, I want to see a diff of every file the agent touched during a run so that I can review, understand, and verify the changes it made.

## Where the User Sees This
- **Story Detail Panel → Run History list → click any run → "Changes" tab**
- The Run Detail Panel gains two tabs: "Events" (existing event log) and "Changes" (new file diff view)
- The Changes tab lists every file the agent wrote during that run; clicking a file shows a before/after diff
- If no files were changed, the tab shows an empty state: "No file changes recorded for this run"

## Acceptance Criteria
- [ ] Before a run starts, the backend records the git HEAD SHA (or `null` if the workspace is not a git repo) on the `story_runs` row
- [ ] After a run completes, the backend runs `git diff <before_sha>` in the workspace root and stores the unified diff output on the run record (or in a related `run_diffs` table)
- [ ] The Run Detail Panel has an "Events" tab and a "Changes" tab
- [ ] The Changes tab lists all files with changes (file path + stats: lines added/removed)
- [ ] Clicking a file shows a side-by-side or inline diff view using a diff viewer component (react-diff-viewer-continued or Monaco diff editor)
- [ ] If the workspace has no git repo, the Changes tab shows an informational message: "Git not detected — file diffs unavailable"
- [ ] Diffs are stored per-run and are pruned alongside run_events when the retention cap is applied

## Technical Notes
- Backend: add `before_sha TEXT` and `diff_output TEXT` (or separate `run_diffs` table) to support the git diff flow
- Use `git diff <sha>` subprocess in the workspace crate; capture stdout as unified diff text
- Parse unified diff on the frontend to split by file for navigation
- Library recommendation: `react-diff-viewer-continued` (lightweight, no Monaco dependency) or reuse Monaco if already bundled
- If `before_sha` is null (non-git workspace), skip diff capture entirely — no error
