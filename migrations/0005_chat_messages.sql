-- Live Chat. Batas panjang di sini hanya pagar terakhir (longgar); batas
-- sebenarnya diberlakukan API (D4) dan boleh lebih ketat.
-- Avatar dan lampiran hanya URL `https://` (nanti objek di R2). Data base64 /
-- `data:` URI sengaja ditolak oleh CHECK di bawah.
CREATE TABLE chat_messages (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    username       TEXT NOT NULL CHECK (length(username) BETWEEN 1 AND 64),
    url            TEXT CHECK (
        url IS NULL
        OR (length(url) <= 2048
            AND instr(url, ' ') = 0
            AND (url LIKE 'http://%' OR url LIKE 'https://%'))
    ),
    avatar_url     TEXT CHECK (
        avatar_url IS NULL
        OR (length(avatar_url) <= 2048
            AND instr(avatar_url, ' ') = 0
            AND avatar_url LIKE 'https://%')
    ),
    message        TEXT NOT NULL CHECK (length(message) BETWEEN 1 AND 4000),
    attachment_url TEXT CHECK (
        attachment_url IS NULL
        OR (length(attachment_url) <= 2048
            AND instr(attachment_url, ' ') = 0
            AND attachment_url LIKE 'https://%')
    ),
    -- UTC, `YYYY-MM-DDTHH:MM:SSZ`; urutan teks sama dengan urutan waktu.
    created_at     TEXT NOT NULL CHECK (
        created_at GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
    )
);

CREATE INDEX chat_messages_recent ON chat_messages (created_at, id);
