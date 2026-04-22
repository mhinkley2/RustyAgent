-- Human-in-the-loop: extend stories for human-type questions and add
-- approval_requests table for gated tool execution.

-- Add columns for human-type stories that link back to the paused run.
-- SQLite does not support adding multiple columns in one ALTER TABLE,
-- so we use separate statements.
ALTER TABLE stories ADD COLUMN parent_run_id TEXT REFERENCES story_runs(id) ON DELETE SET NULL;
ALTER TABLE stories ADD COLUMN human_question TEXT;    -- agent's question to the user
ALTER TABLE stories ADD COLUMN human_response TEXT;    -- user's reply

-- Approval requests: created by the runtime before each tool call when
-- requires_approval=true.  The frontend reads these, the user decides,
-- and the runtime resumes based on the decision.
CREATE TABLE IF NOT EXISTS approval_requests (
    id          TEXT PRIMARY KEY NOT NULL,
    run_id      TEXT NOT NULL REFERENCES story_runs(id) ON DELETE CASCADE,
    tool_name   TEXT NOT NULL,
    tool_input  TEXT NOT NULL DEFAULT '{}',   -- JSON
    status      TEXT NOT NULL DEFAULT 'pending', -- 'pending' | 'approved' | 'rejected'
    rejection_reason TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    decided_at  TEXT
);

CREATE INDEX IF NOT EXISTS idx_approval_requests_run
    ON approval_requests(run_id, status);

CREATE INDEX IF NOT EXISTS idx_stories_human
    ON stories(story_type, status);
