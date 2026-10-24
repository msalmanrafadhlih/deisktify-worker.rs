-- Pemeriksa stasiun radio (D7a): hasil cek terakhir tiap stasiun aktif.
--
-- Hanya tabel baru (tidak mengubah tabel yang ada), jadi Worker lama tetap
-- jalan selama job `migrate` berjalan sebelum deploy. Pemeriksa membaca header
-- stream saja lalu memutus koneksi; URL-nya tidak pernah disalin ke sini
-- kecuali `checked_url`, yaitu URL yang diperiksa terakhir (untuk mengenali
-- bahwa tautan sudah diganti admin, lalu mulai dari nol).
--
-- Satu kegagalan tidak mematikan stasiun: `consecutive_failures` harus
-- mencapai ambang DAN `failing_since` cukup lama (lihat radio_check.rs).
-- `disabled_at` terisi bila pemeriksa sendiri yang menonaktifkan stasiun;
-- kalau admin mengaktifkan lagi, baris itu dianggap baru.
CREATE TABLE radio_checks (
    station_id           INTEGER PRIMARY KEY
                         REFERENCES radio_stations (id) ON DELETE CASCADE,
    checked_url          TEXT NOT NULL CHECK (
        length(checked_url) BETWEEN 1 AND 2048
        AND instr(checked_url, ' ') = 0
        AND (checked_url LIKE 'http://%' OR checked_url LIKE 'https://%')
    ),
    last_checked_at      TEXT NOT NULL CHECK (
        last_checked_at GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
    ),
    last_ok_at           TEXT CHECK (
        last_ok_at IS NULL
        OR last_ok_at GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
    ),
    failing_since        TEXT CHECK (
        failing_since IS NULL
        OR failing_since GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
    ),
    consecutive_failures INTEGER NOT NULL DEFAULT 0 CHECK (consecutive_failures BETWEEN 0 AND 1000),
    last_result          TEXT NOT NULL CHECK (
        last_result IN ('ok', 'http', 'network', 'timeout', 'redirect', 'skipped', 'inconclusive')
    ),
    last_status          INTEGER CHECK (last_status IS NULL OR last_status BETWEEN 100 AND 599),
    disabled_at          TEXT CHECK (
        disabled_at IS NULL
        OR disabled_at GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
    )
);

-- Urutan "paling lama belum dicek" untuk giliran pemeriksaan.
CREATE INDEX radio_checks_rotation ON radio_checks (last_checked_at);
