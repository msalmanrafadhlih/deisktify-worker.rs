-- Moderasi Live Chat (D4b-1).
--
-- Kompatibel mundur: migrasi ini jalan SEBELUM Worker baru dideploy, jadi Worker
-- lama (yang tidak tahu `deleted_at`) harus tetap bekerja. Kolom baru boleh NULL
-- dan query lama tidak menyebutnya, jadi aman.
--
-- Penghapusan = penanda `deleted_at` (soft delete), bukan DELETE: baris tetap ada
-- untuk ditinjau admin, dan laporan yang menunjuknya tidak menggantung. Semua
-- query publik wajib menyaring `deleted_at IS NULL`.
ALTER TABLE chat_messages ADD COLUMN deleted_at TEXT CHECK (
    deleted_at IS NULL
    OR deleted_at GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
);

-- Laporan pembaca. Satu pelapor (hash IP + garam, bukan IP asli) hanya dihitung
-- sekali per pesan; melapor dua kali tidak menambah hitungan.
-- Tanpa FOREIGN KEY: `PRAGMA foreign_keys` tidak dijamin menyala per koneksi
-- (lihat README migrasi); API memeriksa pesannya sebelum menulis.
CREATE TABLE message_reports (
    message_id INTEGER NOT NULL CHECK (message_id >= 1),
    reporter   TEXT    NOT NULL CHECK (length(reporter) BETWEEN 1 AND 64),
    created_at TEXT    NOT NULL CHECK (
        created_at GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
    ),
    PRIMARY KEY (message_id, reporter)
) WITHOUT ROWID;
