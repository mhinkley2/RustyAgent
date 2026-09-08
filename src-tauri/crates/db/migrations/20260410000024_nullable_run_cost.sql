-- Let a run say it does not know what it cost.
--
-- `estimated_cost_usd` was `REAL NOT NULL DEFAULT 0.0`, so the column had no
-- way to spell "unknown". `finish_run` already computes `None` for a model the
-- price table does not know — every DeepSeek and OpenRouter run — and its
-- comment says COALESCE "leaves the column at its default rather than
-- asserting $0.00". But the default *is* 0.0, so withholding a price and
-- quoting zero produced the same stored value. The code believed it was
-- declining to answer while the schema answered "free" on its behalf.
--
-- Zero is a real answer for Ollama, which runs locally and genuinely costs
-- nothing, and for a run that consumed no tokens at all. Those must stay
-- distinguishable from a run whose price nobody can quote, so this is three
-- states in one column: a number, 0.0, and NULL.
--
-- ADD / UPDATE / DROP / RENAME rather than the twelve-step table rebuild:
-- every statement here is an additive ALTER, no index or foreign key
-- references this column, and SQLite has supported DROP COLUMN since 3.35
-- (this build bundles 3.46). A rebuild of `story_runs` would mean restating a
-- schema that twenty-three migrations have shaped, which is a much larger way
-- to be wrong.

ALTER TABLE story_runs ADD COLUMN cost_usd REAL;

-- Backfilling honestly, given rows that cannot say which zero they meant:
--
--   * a recorded non-zero cost is real, whoever wrote it — keep it;
--   * zero against tokens that were actually spent is the unknown case this
--     migration exists for — NULL. The frontend already guessed exactly this
--     (`formatEstimatedCost` rendered an em dash for it), so history keeps the
--     reading it has been shown with, now as data rather than as a heuristic;
--   * zero against zero tokens genuinely cost nothing — keep 0.0. "Zero
--     tokens" counts the cache columns too, matching `Usage::is_zero`: a run
--     that only ever read a cached prefix still spent something, and calling
--     that free would be the same mistake one column over.
--
-- This misfiles one case: an Ollama run that spent tokens and truly cost
-- nothing becomes "unknown" instead of "free". That is the safe direction —
-- history is being reinterpreted without the provider on the row to check
-- against, and claiming a run was free is a stronger statement than admitting
-- we cannot say. Runs from here on record the provider's own answer.
UPDATE story_runs
   SET cost_usd = CASE
         WHEN estimated_cost_usd != 0.0 THEN estimated_cost_usd
         WHEN input_tokens = 0 AND output_tokens = 0
              AND COALESCE(cache_read_input_tokens, 0) = 0
              AND COALESCE(cache_creation_input_tokens, 0) = 0 THEN 0.0
         ELSE NULL
       END;

ALTER TABLE story_runs DROP COLUMN estimated_cost_usd;
ALTER TABLE story_runs RENAME COLUMN cost_usd TO estimated_cost_usd;
