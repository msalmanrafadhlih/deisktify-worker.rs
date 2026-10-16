-- Stasiun di aplikasi Radio. `is_active = 0` menyembunyikan stasiun tanpa
-- menghapusnya (URL stream radio sering mati).
CREATE TABLE radio_stations (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    label         TEXT NOT NULL UNIQUE CHECK (length(label) BETWEEN 1 AND 80),
    slogan        TEXT NOT NULL DEFAULT '' CHECK (length(slogan) <= 160),
    stream_url    TEXT NOT NULL CHECK (
        length(stream_url) BETWEEN 1 AND 2048
        AND instr(stream_url, ' ') = 0
        AND (stream_url LIKE 'http://%' OR stream_url LIKE 'https://%')
    ),
    official_link TEXT CHECK (
        official_link IS NULL
        OR (length(official_link) <= 2048
            AND instr(official_link, ' ') = 0
            AND (official_link LIKE 'http://%' OR official_link LIKE 'https://%'))
    ),
    image_url     TEXT CHECK (
        image_url IS NULL
        OR (length(image_url) <= 2048
            AND instr(image_url, ' ') = 0
            AND (image_url LIKE 'http://%' OR image_url LIKE 'https://%'))
    ),
    sort_order    INTEGER NOT NULL DEFAULT 0,
    is_active     INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1))
);

CREATE INDEX radio_stations_listing ON radio_stations (is_active, sort_order, id);
