-- Per-workspace settings overrides.
-- Global settings live in settings.json (AppSettings struct).
-- This table holds JSON overrides for a specific workspace; fields present
-- here take precedence over the global defaults when that workspace is active.
CREATE TABLE IF NOT EXISTS workspace_settings (
    workspace_id TEXT PRIMARY KEY NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    settings_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
