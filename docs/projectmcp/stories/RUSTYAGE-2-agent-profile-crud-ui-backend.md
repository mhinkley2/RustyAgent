# RUSTYAGE-2: Agent profile CRUD (UI + backend)

- Story ID: 64f83fcc-f0ec-420c-bf59-27ac77911880
- Story Type: Story
- Status: done
- Priority: Critical
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: phase-1, frontend, backend, agents
- Created At: 04/09/2026 20:03:33

## Description

Build the Agent Profile management UI and Tauri backend commands. Users can create, edit, and delete agent profiles with all configuration fields.

**Acceptance Criteria:**
- [ ] List view showing all agent profiles with name, provider, model, run mode
- [ ] Create / edit form with all fields: name, description, system prompt (textarea), provider dropdown, model (dynamic based on provider), context strategy, persistent memory toggle, token limits, run mode, cron expression (shown when scheduled)
- [ ] Delete with confirmation dialog
- [ ] Tauri commands: get_profiles, get_profile, create_profile, update_profile, delete_profile
- [ ] Profile data persisted to agent_profiles table
- [ ] System prompt field supports multi-line markdown editing
