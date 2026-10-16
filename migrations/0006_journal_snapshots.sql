-- Cache data GitHub Journal (pengganti cache di Worker lama, D5).
-- `source = 'seed'` menandai contoh dari berkas JSON supaya tidak tertukar
-- dengan data asli (`'github'`).
CREATE TABLE journal_snapshots (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    payload    TEXT NOT NULL CHECK (json_valid(payload) AND length(payload) <= 1048576),
    source     TEXT NOT NULL DEFAULT 'github' CHECK (source IN ('github', 'seed')),
    fetched_at TEXT NOT NULL CHECK (
        fetched_at GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
    )
);

CREATE INDEX journal_snapshots_latest ON journal_snapshots (fetched_at, id);
