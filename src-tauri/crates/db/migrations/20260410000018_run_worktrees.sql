-- Per-run git worktree isolation.
--
-- A run no longer writes into the user's checkout: it gets a linked git
-- worktree of its own, on its own branch, and these columns are how the UI and
-- the startup sweep find it again afterwards.
--
-- worktree_path    Absolute path of the isolated checkout. Kept after cleanup
--                  as part of the run's record; isolation_status says whether
--                  it still exists.
-- branch_name      Branch the worktree had checked out ('rustyagent/run-<id>').
-- after_sha        Commit made on that branch when the run finished, or NULL
--                  when the run changed nothing.
-- isolation_status 'isolated' | 'not_a_git_repo' | 'unavailable' | 'no_workspace'
--                  and, after the user decides, 'accepted' | 'reverted'.
--                  NULL for runs that predate this migration.
-- isolation_note   Operator-facing explanation — why a run was not isolated,
--                  or what was surprising about the one that was.
ALTER TABLE story_runs ADD COLUMN worktree_path TEXT;
ALTER TABLE story_runs ADD COLUMN branch_name TEXT;
ALTER TABLE story_runs ADD COLUMN after_sha TEXT;
ALTER TABLE story_runs ADD COLUMN isolation_status TEXT;
ALTER TABLE story_runs ADD COLUMN isolation_note TEXT;
