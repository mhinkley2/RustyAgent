# Feature: Automations (Local)

- Note ID: ad3fa203-f635-4a02-8e47-b99803521207
- Project ID: 792eb04c-6091-419f-bfc2-dc573bef45d2
- Story ID: None
- Parent ID: None
- Order: 4
- Favorited: False
- Created At: 04/06/2026 23:45:58
- Updated At: 04/06/2026 23:45:58

---

# Automations (Local)

## Summary
Schedule agent runs or trigger them based on local events (file changes, cron schedules, etc.) — without requiring cloud infrastructure.

## Notes
<!-- Add your requirements, ideas, and decisions here -->

## Open Questions
<!-- What do you need to figure out? -->

## References
- Cursor 3.0: Automations triggered by schedules or events from Slack, Linear, GitHub, PagerDuty, webhooks
- Local scope: cron scheduling via tokio-cron-scheduler; file watching via notify crate
