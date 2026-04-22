-- Pipeline step run tracking for multi-agent collaboration (RUSTYAGE-10)
CREATE TABLE IF NOT EXISTS pipeline_step_runs (
    id                 TEXT PRIMARY KEY NOT NULL,
    pipeline_run_id    TEXT NOT NULL REFERENCES story_runs(id) ON DELETE CASCADE,
    step_index         INTEGER NOT NULL,
    story_id           TEXT NOT NULL,
    agent_profile_id   TEXT NOT NULL,
    -- set once the step's run is started
    run_id             TEXT REFERENCES story_runs(id),
    -- 'pending' | 'running' | 'done' | 'failed'
    status             TEXT NOT NULL DEFAULT 'pending',
    -- last assistant turn output truncated to 8 KB for sequential handoff
    output             TEXT,
    created_at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
