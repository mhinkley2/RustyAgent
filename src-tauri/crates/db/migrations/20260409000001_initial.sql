-- Agent profiles: the configuration for each AI agent
CREATE TABLE IF NOT EXISTS agent_profiles (
    id TEXT PRIMARY KEY NOT NULL,           -- UUID
    name TEXT NOT NULL,
    description TEXT,
    system_prompt TEXT NOT NULL DEFAULT '',
    provider TEXT NOT NULL,                 -- 'anthropic' | 'openrouter' | 'ollama'
    model TEXT NOT NULL,
    context_strategy TEXT NOT NULL DEFAULT 'recent', -- 'recent' | 'summary' | 'full'
    persistent_memory INTEGER NOT NULL DEFAULT 0,   -- bool
    max_input_tokens INTEGER,
    max_output_tokens INTEGER,
    run_mode TEXT NOT NULL DEFAULT 'manual',        -- 'manual' | 'continuous' | 'scheduled'
    cron_expression TEXT,
    continuous_poll_interval_secs INTEGER NOT NULL DEFAULT 30,
    max_iterations INTEGER NOT NULL DEFAULT 20,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Bindings between agent profiles and MCP servers (tool groups)
CREATE TABLE IF NOT EXISTS agent_tool_bindings (
    id TEXT PRIMARY KEY NOT NULL,           -- UUID
    agent_profile_id TEXT NOT NULL REFERENCES agent_profiles(id) ON DELETE CASCADE,
    mcp_server_id TEXT NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    allowed_tools TEXT,                     -- JSON array of allowed tool names; NULL means all
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Stories: units of work assigned to agents or humans
CREATE TABLE IF NOT EXISTS stories (
    id TEXT PRIMARY KEY NOT NULL,           -- UUID
    title TEXT NOT NULL,
    description TEXT,
    story_type TEXT NOT NULL DEFAULT 'task', -- 'task' | 'human' | 'pipeline'
    status TEXT NOT NULL DEFAULT 'ready',    -- see db::story_status::STORY_STATUSES
    priority TEXT NOT NULL DEFAULT 'medium', -- 'low' | 'medium' | 'high' | 'critical'
    assigned_agent_id TEXT REFERENCES agent_profiles(id) ON DELETE SET NULL,
    requires_approval INTEGER NOT NULL DEFAULT 0,  -- bool
    pipeline_config TEXT,                   -- JSON pipeline definition (null for regular stories)
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Story runs: each execution of a story by an agent
CREATE TABLE IF NOT EXISTS story_runs (
    id TEXT PRIMARY KEY NOT NULL,           -- UUID
    story_id TEXT NOT NULL REFERENCES stories(id) ON DELETE CASCADE,
    agent_profile_id TEXT NOT NULL REFERENCES agent_profiles(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'running', -- 'running' | 'done' | 'failed' | 'cancelled'
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    estimated_cost_usd REAL NOT NULL DEFAULT 0.0,
    iteration_count INTEGER NOT NULL DEFAULT 0,
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    finished_at TEXT
);

-- Run events: append-only log of everything that happened in a run
CREATE TABLE IF NOT EXISTS run_events (
    id TEXT PRIMARY KEY NOT NULL,           -- UUID
    run_id TEXT NOT NULL REFERENCES story_runs(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,               -- 'message' | 'tool_call' | 'tool_result' | 'thought' | 'error' | 'approval_request' | 'approval_response'
    role TEXT,                              -- 'user' | 'assistant' | 'tool' (for message events)
    content TEXT,                           -- raw text or JSON
    tool_name TEXT,                         -- for tool_call / tool_result events
    tool_input TEXT,                        -- JSON
    tool_output TEXT,                       -- JSON
    is_error INTEGER NOT NULL DEFAULT 0,    -- bool
    sequence_num INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Agent memory: episodic key-value store scoped by agent and scope
CREATE TABLE IF NOT EXISTS agent_memory (
    id TEXT PRIMARY KEY NOT NULL,           -- UUID
    agent_profile_id TEXT NOT NULL REFERENCES agent_profiles(id) ON DELETE CASCADE,
    scope TEXT NOT NULL DEFAULT 'persistent', -- 'session' | 'persistent' | 'shared_scratchpad'
    pipeline_run_id TEXT,                   -- non-null for shared_scratchpad scope
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(agent_profile_id, scope, pipeline_run_id, key)
);

-- MCP servers: external tool servers managed by the app
CREATE TABLE IF NOT EXISTS mcp_servers (
    id TEXT PRIMARY KEY NOT NULL,           -- UUID
    name TEXT NOT NULL UNIQUE,
    command TEXT NOT NULL,
    args TEXT NOT NULL DEFAULT '[]',        -- JSON array of strings
    env_vars TEXT NOT NULL DEFAULT '{}',    -- JSON object (non-secret env vars)
    auto_restart INTEGER NOT NULL DEFAULT 1, -- bool
    max_restart_attempts INTEGER NOT NULL DEFAULT 3,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Indexes for common query patterns
CREATE INDEX IF NOT EXISTS idx_stories_status ON stories(status);
CREATE INDEX IF NOT EXISTS idx_stories_assigned_agent ON stories(assigned_agent_id);
CREATE INDEX IF NOT EXISTS idx_story_runs_story_id ON story_runs(story_id);
CREATE INDEX IF NOT EXISTS idx_story_runs_status ON story_runs(status);
CREATE INDEX IF NOT EXISTS idx_run_events_run_id ON run_events(run_id);
CREATE INDEX IF NOT EXISTS idx_run_events_sequence ON run_events(run_id, sequence_num);
CREATE INDEX IF NOT EXISTS idx_agent_memory_lookup ON agent_memory(agent_profile_id, scope, key);
CREATE INDEX IF NOT EXISTS idx_agent_tool_bindings_agent ON agent_tool_bindings(agent_profile_id);
