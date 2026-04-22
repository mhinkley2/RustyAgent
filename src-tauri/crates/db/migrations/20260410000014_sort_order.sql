-- Add sort_order to stories for kanban / list-view ordering.
ALTER TABLE stories ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;

-- Initialise from rowid so existing rows get a deterministic, unique order.
UPDATE stories SET sort_order = rowid - 1;
