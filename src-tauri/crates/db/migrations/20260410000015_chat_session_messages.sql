-- Persist chat transcript messages so sessions can be resumed across restarts.
CREATE TABLE IF NOT EXISTS chat_session_messages (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES stories(id) ON DELETE CASCADE,
    role TEXT NOT NULL, -- 'user' | 'assistant'
    content TEXT NOT NULL,
    agent_profile_id TEXT REFERENCES agent_profiles(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_chat_session_messages_session_created
    ON chat_session_messages(session_id, created_at);
