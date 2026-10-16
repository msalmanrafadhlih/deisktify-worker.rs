-- Menu boot di halaman utama ("Generation 5 ..."). Data konten, sedikit baris.
-- `link` adalah rute internal ("/deisktify"; `//host` dan `/\` ditolak karena
-- bisa menjadi alamat luar) atau URL http(s) penuh.
CREATE TABLE generations (
    number   INTEGER PRIMARY KEY CHECK (number > 0),
    label    TEXT NOT NULL CHECK (length(label) BETWEEN 1 AND 80),
    link     TEXT NOT NULL CHECK (
        length(link) BETWEEN 1 AND 2048
        AND instr(link, ' ') = 0
        AND ((link LIKE '/%' AND link NOT LIKE '//%' AND link NOT LIKE '/\%')
         OR link LIKE 'http://%' OR link LIKE 'https://%')
    ),
    kernel   TEXT NOT NULL CHECK (length(kernel) BETWEEN 1 AND 32),
    built_on TEXT NOT NULL CHECK (built_on GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]')
);
