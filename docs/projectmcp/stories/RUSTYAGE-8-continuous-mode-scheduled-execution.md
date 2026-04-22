# RUSTYAGE-8: Continuous mode & scheduled execution

- Story ID: 116dfbc4-54ed-4223-87f5-43336687471a
- Story Type: Story
- Status: done
- Priority: High
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: phase-2, backend, scheduling
- Created At: 04/09/2026 20:04:29

## Description

Add continuous and scheduled execution modes to agent profiles. Agents in continuous mode poll for assigned stories; scheduled agents run on a cron expression.

**Acceptance Criteria:**
- [ ] Continuous mode: when enabled, agent polls the story queue every N seconds (configurable) and picks up the next Ready story assigned to it
- [ ] Only one story runs at a time per agent in continuous mode
- [ ] Scheduled mode: cron expression on profile triggers a run at the specified time
- [ ] Scheduler uses the `cron` Rust crate; runs in a background Tokio task
- [ ] Agent status indicator on Agents page: Idle / Running / Scheduled (next run time)
- [ ] Start/stop continuous mode from the UI without editing the profile
- [ ] Scheduler persists across app restarts (re-registered on startup)
