-- Dashboard admin (D6a): pembatas percobaan masuk yang gagal dan riwayat aksi.
--
-- Keduanya hanya berisi hash IP (SHA-256 dengan RATE_LIMIT_SALT, sama seperti
-- pembatas Guestbook), tidak pernah IP asli dan tidak pernah isi data.

-- Percobaan masuk yang GAGAL, per kunci (hash IP) per jendela 15 menit. Setelah
-- 10 kegagalan dalam satu jendela, kunci itu ditolak sampai jendela berganti,
-- bahkan jika token berikutnya benar. Baris lama dibuang saat ada kegagalan baru.
CREATE TABLE admin_attempts (
    key    TEXT    NOT NULL CHECK (length(key) BETWEEN 1 AND 64),
    window INTEGER NOT NULL CHECK (window >= 0),
    hits   INTEGER NOT NULL DEFAULT 1 CHECK (hits >= 1),
    PRIMARY KEY (key, window)
) WITHOUT ROWID;

CREATE INDEX admin_attempts_window ON admin_attempts (window);

-- Riwayat aksi admin yang BERHASIL (masuk, refresh jurnal, sembunyikan entri,
-- dan seterusnya). Hanya jenis aksi dan sasarannya, bukan isi baris. Paling
-- baru 500 baris yang disimpan (dipangkas saat mencatat).
CREATE TABLE admin_audit (
    id     INTEGER PRIMARY KEY AUTOINCREMENT,
    at     TEXT NOT NULL CHECK (
        at GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
    ),
    action TEXT NOT NULL CHECK (length(action) BETWEEN 1 AND 40),
    target TEXT CHECK (target IS NULL OR length(target) BETWEEN 1 AND 120),
    who    TEXT NOT NULL CHECK (length(who) BETWEEN 1 AND 64)
);
