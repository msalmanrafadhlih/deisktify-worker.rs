-- Pembatas kecepatan kirim pesan Live Chat (D4). Satu baris = satu kunci di
-- satu jendela waktu; `hits` dinaikkan atomik oleh API (`rate_hit.sql`).
--   scope 'm' : jendela per menit  (window = detik_unix / 60)
--   scope 'h' : jendela per jam    (window = detik_unix / 3600)
-- `key` adalah hash IP pengirim (SHA-256 + garam rahasia, dipotong), bukan IP
-- asli. Baris lama dihapus oleh API setiap ada pesan baru.
CREATE TABLE chat_rate_limits (
    scope  TEXT    NOT NULL CHECK (scope IN ('m', 'h')),
    key    TEXT    NOT NULL CHECK (length(key) BETWEEN 1 AND 64),
    window INTEGER NOT NULL CHECK (window >= 0),
    hits   INTEGER NOT NULL DEFAULT 1 CHECK (hits >= 1),
    PRIMARY KEY (scope, key, window)
) WITHOUT ROWID;

CREATE INDEX chat_rate_limits_window ON chat_rate_limits (window);
