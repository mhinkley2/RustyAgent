# Feature: Design Mode (UI Annotation)

- Note ID: 68c36aa6-c1ae-43b8-8e83-3ee6f78d4129
- Project ID: 792eb04c-6091-419f-bfc2-dc573bef45d2
- Story ID: None
- Parent ID: None
- Order: 5
- Favorited: False
- Created At: 04/06/2026 23:45:58
- Updated At: 04/06/2026 23:45:58

---

# Design Mode (UI Annotation)

## Summary
An overlay mode where users can annotate UI elements in the integrated browser and send them directly to an agent as context — enabling precise visual feedback.

## Notes
<!-- Add your requirements, ideas, and decisions here -->

## Open Questions
<!-- What do you need to figure out? -->

## References
- Cursor 3.0: Design Mode lets you annotate and target UI elements directly in the browser via Shift+drag, then add to chat
- Implementation: Inject JS into WebView panel to create selection overlay; route selections back via Tauri IPC
