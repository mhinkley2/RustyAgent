# RUSTYAGE-27: Agent History: Optional per-story run event logging

- Story ID: adab27a1-e0ca-4977-855f-e0f5242a63fd
- Story Type: Story
- Status: done
- Priority: High
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: history, observability, db-migration
- Created At: 04/11/2026 00:17:16

## Description

## Problem
When an agent runs, there is currently no user-facing control over whether the full event log (messages, tool calls, thoughts, errors) is persisted. For noisy pipeline sub-tasks this can flood the `run_events` table with low-value data, while for manually-created stories the history is essential for understanding what the agent did.

## Goal
Give users visibility into agent activity on a per-story basis while keeping storage under control.

## User Story
As a user, I want to optionally enable full run event history on a story so that I can review exactly what the agent did without filling the database with noise from automated pipeline sub-tasks.

## Acceptance Criteria
- [ ] Stories have a `track_history` boolean field (default `true` for user-created stories, `false` for pipeline sub-task stories)
- [ ] When `track_history` is `false`, `message`, `tool_call`, `tool_result`, and `thought` events are NOT written to `run_events`; `error` and `approval_request` events are ALWAYS written regardless of the flag
- [ ] The story create/edit form (`StoryForm.tsx`) exposes the `track_history` toggle
- [ ] The Run Detail Panel shows all persisted events for that run in chronological order
- [ ] A global setting ("Event retention: keep last N runs per story") auto-prunes old `run_events` rows; default N = 10
- [ ] DB migration adds `track_history INTEGER NOT NULL DEFAULT 1` to the `stories` table and the global settings seed value

## Technical Notes
- Migration: add `track_history` column to `stories` table
- Runtime: the agent runner (runtime crate) checks `story.track_history` before inserting non-critical events
- Always persist: `error`, `approval_request`, `approval_response` event types
- Pruning: a helper called after each run completion deletes oldest run_events rows beyond the retention cap
- Frontend: toggle in `StoryForm`, existing `RunDetailPanel` already renders events — no new component needed
