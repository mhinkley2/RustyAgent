-- Semantic memory: fastembed-powered run summaries stored as JSON float arrays.
-- Searched at run start to inject relevant past context into the agent's system prompt.
CREATE TABLE IF NOT EXISTS memory_semantic (
    id                 TEXT PRIMARY KEY NOT NULL,
    agent_profile_id   TEXT NOT NULL REFERENCES agent_profiles(id) ON DELETE CASCADE,
    content            TEXT NOT NULL,          -- original text that was embedded
    embedding          TEXT NOT NULL,          -- JSON float array (384-dim AllMiniLML6V2)
    created_at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS memory_semantic_profile_idx
    ON memory_semantic(agent_profile_id);
