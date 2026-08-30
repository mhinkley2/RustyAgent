-- Settle on one story-status vocabulary.
--
-- `stories.status` had five spellings of what a story may be, spread across
-- the board's columns, three agent-tool schemas and two doc comments, and no
-- two of them agreed. There is no CHECK constraint, so nothing caught it.
--
-- The canonical set is `db::story_status::STORY_STATUSES`: backlog, ready,
-- in_progress, blocked, review, done. It is what the board draws, what
-- `create_story` already accepted, and what `db::story_status` writes when a
-- run moves a card on its own.
--
-- `failed` is the odd one out. Two write paths accepted it and no column
-- rendered it, so a card set to `failed` left the board entirely and could be
-- found only through search or the list view. Those rows move to `blocked`,
-- which is where `RunOutcome::story_status` already sends a story whose run
-- failed — so the two agree afterwards rather than merely coexisting — and
-- which the board does draw.
--
-- Nothing is lost that the row itself did not already say: a story that
-- reached `failed` did so because something stopped, and `blocked` is the
-- column for work that has stopped and needs a person.
UPDATE stories SET status = 'blocked' WHERE status = 'failed';

-- Anything else outside the vocabulary — a value written by a client that
-- predates this, or by hand — is also parked in `blocked` rather than left
-- invisible. `blocked` is the honest destination for a card nobody can see:
-- it says a human needs to look, which is true.
UPDATE stories
   SET status = 'blocked'
 WHERE status NOT IN ('backlog', 'ready', 'in_progress', 'blocked', 'review', 'done');
