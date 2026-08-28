-- The conversation runtime used to finish runs with status 'completed', while
-- the pipeline engine and the frontend's RunStatus union both use 'done'. That
-- left every successfully finished run rendering a blank status badge and
-- invisible to the "Done" filter. Normalize the historical rows.
UPDATE story_runs SET status = 'done' WHERE status = 'completed';
