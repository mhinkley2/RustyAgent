-- Add git diff support to story_runs.
-- before_sha: the git HEAD commit SHA captured before the run started.
-- diff_output: the unified diff text from `git diff <before_sha>` captured after the run finished.
ALTER TABLE story_runs ADD COLUMN before_sha TEXT;
ALTER TABLE story_runs ADD COLUMN diff_output TEXT;
