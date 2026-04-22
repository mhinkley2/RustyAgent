# RUSTYAGE-26: Workspace-Scoped Data Isolation

- Story ID: 4e0ab801-3c78-4dba-87e9-74532cad2a0a
- Story Type: Story
- Status: done
- Priority: High
- Assigned To: Unassigned
- Epic: None
- Estimated Points: None
- Due Date: None
- Labels: workspace, data-isolation, core
- Created At: 04/10/2026 21:27:29

## Description

## Overview

All core application data — board, chat history, agents, file explorer state, settings, and run history — should be scoped to the currently active workspace. Opening a different workspace loads its own isolated data, so projects never bleed into one another.

## User Stories

**As a developer**, I want my board (stories/tasks) to be tied to the active workspace so that I can manage separate project backlogs without them mixing.

**As a developer**, I want my chat history to be tied to the active workspace so that conversations from one project are not visible when working on another.

**As a developer**, I want my configured agents to be scoped to the active workspace so that I can use different agent configurations for different projects.

**As a developer**, I want my file explorer state (open files, tabs, tree expansions) to be persisted per workspace so that switching workspaces restores the correct file context.

**As a developer**, I want my run history to be scoped to the active workspace so that I can review past runs without noise from other projects.

**As a developer**, I want settings to have both global defaults and per-workspace overrides so that I can share common preferences while customizing per project.

## Acceptance Criteria

- [ ] Opening or switching a workspace loads board, chat, agents, runs, and file state specific to that workspace
- [ ] Data created in workspace A is not visible when workspace B is active
- [ ] Closing and reopening a workspace restores all scoped data correctly
- [ ] A workspace selector (e.g., in the titlebar or sidebar) makes the active workspace clear
- [ ] Global settings remain available as defaults; workspace settings can override them
- [ ] New workspaces start with an empty board, empty chat, and no agents (clean slate)

## Out of Scope (for now)
- Sharing agents or board templates across workspaces
- Real-time sync of workspaces across machines
