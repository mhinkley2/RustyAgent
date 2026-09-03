-- Timestamps were written two ways. Every DEFAULT in the schema is
-- strftime('%Y-%m-%dT%H:%M:%fZ','now') -- RFC 3339, which is what the app
-- parses -- but application code wrote CURRENT_TIMESTAMP, whose
-- 'YYYY-MM-DD HH:MM:SS' output has a space for the T and no offset and does
-- not parse. A row that took its default was readable and a row written
-- explicitly was not, so finished runs showed no duration.
--
-- The writers now use the schema's spelling (db::timestamps::NOW_ISO8601).
-- This normalises the rows already written the other way, so history reads
-- like anything written from here on rather than staying permanently blank.
--
-- Both forms are UTC, so this is a pure reformat with no timezone risk. The
-- guard is the length: a CURRENT_TIMESTAMP value is exactly 19 characters and
-- contains a space at index 10, which no RFC 3339 value does. Rows already in
-- the right format do not match and are left untouched, making this
-- idempotent.
UPDATE story_runs
   SET started_at = replace(started_at, ' ', 'T') || '.000Z'
 WHERE started_at IS NOT NULL
   AND length(started_at) = 19
   AND substr(started_at, 11, 1) = ' ';

UPDATE story_runs
   SET finished_at = replace(finished_at, ' ', 'T') || '.000Z'
 WHERE finished_at IS NOT NULL
   AND length(finished_at) = 19
   AND substr(finished_at, 11, 1) = ' ';

UPDATE stories
   SET updated_at = replace(updated_at, ' ', 'T') || '.000Z'
 WHERE updated_at IS NOT NULL
   AND length(updated_at) = 19
   AND substr(updated_at, 11, 1) = ' ';

UPDATE stories
   SET created_at = replace(created_at, ' ', 'T') || '.000Z'
 WHERE created_at IS NOT NULL
   AND length(created_at) = 19
   AND substr(created_at, 11, 1) = ' ';
