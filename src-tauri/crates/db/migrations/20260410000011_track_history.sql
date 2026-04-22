-- Add per-story run event history flag.
-- Defaults to 1 (enabled) so all existing stories keep full history.
ALTER TABLE stories ADD COLUMN track_history INTEGER NOT NULL DEFAULT 1;
