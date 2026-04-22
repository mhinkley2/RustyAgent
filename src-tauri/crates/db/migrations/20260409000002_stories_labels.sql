-- Add JSON labels column to stories table.
-- Empty JSON array is the default so existing rows are valid immediately.
ALTER TABLE stories ADD COLUMN labels TEXT NOT NULL DEFAULT '[]';
