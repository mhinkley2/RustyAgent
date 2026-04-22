-- Per-agent-profile permission settings.
-- Controls which tools, file paths, shell commands, and network hosts
-- the agent is allowed to use, and whether writes require human approval.

CREATE TABLE IF NOT EXISTS agent_permissions (
    profile_id TEXT PRIMARY KEY NOT NULL REFERENCES agent_profiles(id) ON DELETE CASCADE,

    -- Tool name allow-list: JSON array of strings.
    -- Empty array = ALL built-in tools allowed (open policy).
    allowed_tools TEXT NOT NULL DEFAULT '[]',

    -- File-system read allow-list: JSON array of absolute directory prefixes.
    -- Empty array = no file-read restrictions.
    allow_file_read_paths TEXT NOT NULL DEFAULT '[]',

    -- File-system write allow-list: JSON array of absolute directory prefixes.
    -- Empty array = no file-write restrictions.
    allow_file_write_paths TEXT NOT NULL DEFAULT '[]',

    -- Shell command allow-list: JSON array of command name prefixes.
    -- Empty array = shell execution not permitted.
    allow_shell_commands TEXT NOT NULL DEFAULT '[]',

    -- Network hostname allow-list: JSON array of hostnames / CIDR prefixes.
    -- Empty array = no network restrictions (all outbound allowed).
    allow_network_hosts TEXT NOT NULL DEFAULT '[]',

    -- When true, every file-write tool call requires human approval before
    -- the runtime executes it.
    require_approval_on_write INTEGER NOT NULL DEFAULT 0,

    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_agent_permissions_profile
    ON agent_permissions(profile_id);
