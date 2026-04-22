-- Workspaces: locally opened project folders
CREATE TABLE IF NOT EXISTS workspaces (
    id            TEXT PRIMARY KEY NOT NULL,
    path          TEXT NOT NULL UNIQUE,       -- absolute fs path
    name          TEXT NOT NULL,              -- display name (last path component)
    last_opened_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
