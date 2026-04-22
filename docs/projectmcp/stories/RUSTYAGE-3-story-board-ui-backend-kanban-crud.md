# RUSTYAGE-3: Story board UI & backend (kanban + CRUD)

- Story ID: 62c0f25a-9faa-4234-b5d7-b0deddea2414
- Story Type: Story
- Status: done
- Priority: Critical
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: phase-1, frontend, backend, stories
- Created At: 04/09/2026 20:03:40

## Description

Build the story board UI and backend. Users can create and manage stories. Supports kanban and list views.

**Acceptance Criteria:**
- [ ] Kanban board view with columns: Backlog, Ready, In Progress, Blocked, Review, Done — drag to move
- [ ] List view as compact alternative
- [ ] Create story form: title, description (markdown), type, priority, assignee (agent or human), labels, requires_approval toggle
- [ ] Story detail panel/modal: full fields, subtask checklist, linked stories
- [ ] Agents can be assigned from the agent profiles list
- [ ] Tauri commands: get_stories, get_story, create_story, update_story, delete_story
- [ ] Stories persisted to stories table
- [ ] Filter bar: by status, type, priority, assignee, label
