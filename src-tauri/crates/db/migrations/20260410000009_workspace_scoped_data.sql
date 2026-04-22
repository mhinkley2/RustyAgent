-- Scope stories and agent profiles to their originating workspace.
-- Existing rows get workspace_id = NULL, which means "no workspace / legacy global".
-- The application treats NULL workspace_id as belonging to the "no workspace open" context,
-- so legacy data is still accessible when no workspace is active.

ALTER TABLE stories ADD COLUMN workspace_id TEXT REFERENCES workspaces(id) ON DELETE CASCADE;
ALTER TABLE agent_profiles ADD COLUMN workspace_id TEXT REFERENCES workspaces(id) ON DELETE SET NULL;
