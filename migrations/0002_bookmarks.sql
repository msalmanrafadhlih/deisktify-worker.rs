-- Bookmark bawaan di jendela Browser. Bookmark tambahan buatan pengunjung
-- tetap hanya di sesi (tidak disimpan di sini).
CREATE TABLE bookmarks (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    label      TEXT NOT NULL CHECK (length(label) BETWEEN 1 AND 80),
    url        TEXT NOT NULL CHECK (
        length(url) BETWEEN 1 AND 2048
        AND instr(url, ' ') = 0
        AND (url LIKE 'http://%' OR url LIKE 'https://%')
    ),
    sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX bookmarks_sort ON bookmarks (sort_order, id);
