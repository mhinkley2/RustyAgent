-- Custom shell command tools that can be bound to agent profiles.
CREATE TABLE IF NOT EXISTS custom_tools (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    command     TEXT NOT NULL,
    working_dir TEXT NOT NULL DEFAULT '.',
    timeout_secs INTEGER NOT NULL DEFAULT 30,
    workspace_id TEXT REFERENCES workspaces(id) ON DELETE CASCADE,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Junction table: which custom tools an agent profile can call.
CREATE TABLE IF NOT EXISTS agent_custom_tool_bindings (
    agent_profile_id TEXT NOT NULL REFERENCES agent_profiles(id) ON DELETE CASCADE,
    custom_tool_id   TEXT NOT NULL REFERENCES custom_tools(id) ON DELETE CASCADE,
    PRIMARY KEY (agent_profile_id, custom_tool_id)
);
