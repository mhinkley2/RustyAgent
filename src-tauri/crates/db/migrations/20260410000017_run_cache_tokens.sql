-- Cache-read and cache-write tokens are billed at different rates from plain
-- input, so a single input_tokens column cannot represent what a run cost.
-- Splitting them out is also what makes the prompt-cache saving visible in the
-- run detail view rather than silently folded into the input count.
--
-- story_runs.input_tokens keeps its existing meaning of *uncached* input, so
-- the total context read by a run is the sum of these three columns.
ALTER TABLE story_runs ADD COLUMN cache_read_input_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE story_runs ADD COLUMN cache_creation_input_tokens INTEGER NOT NULL DEFAULT 0;
