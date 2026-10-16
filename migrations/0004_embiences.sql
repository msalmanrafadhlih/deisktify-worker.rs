-- Suara latar (Embience), preset, dan isi preset.
-- Hanya URL yang disimpan; berkas audio dan ikon tidak ada di database.
CREATE TABLE embiences (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL UNIQUE CHECK (length(name) BETWEEN 1 AND 40),
    icon_url   TEXT NOT NULL CHECK (
        length(icon_url) BETWEEN 1 AND 2048
        AND instr(icon_url, ' ') = 0
        AND (icon_url LIKE 'http://%' OR icon_url LIKE 'https://%')
    ),
    stream_url TEXT NOT NULL CHECK (
        length(stream_url) BETWEEN 1 AND 2048
        AND instr(stream_url, ' ') = 0
        AND (stream_url LIKE 'http://%' OR stream_url LIKE 'https://%')
    ),
    sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE embience_presets (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    label      TEXT NOT NULL UNIQUE CHECK (length(label) BETWEEN 1 AND 40),
    sort_order INTEGER NOT NULL DEFAULT 0
);

-- Foreign key hanya ditegakkan bila koneksi menyalakan `PRAGMA foreign_keys`;
-- API tetap harus memeriksa id sebelum menulis (D4+).
CREATE TABLE embience_preset_items (
    preset_id   INTEGER NOT NULL REFERENCES embience_presets (id) ON DELETE CASCADE,
    embience_id INTEGER NOT NULL REFERENCES embiences (id) ON DELETE CASCADE,
    position    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (preset_id, embience_id)
) WITHOUT ROWID;

CREATE INDEX embiences_sort ON embiences (sort_order, id);
CREATE INDEX embience_presets_sort ON embience_presets (sort_order, id);
