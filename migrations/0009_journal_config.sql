-- Pengaturan GitHub Journal (D5): akun, repositori sematan, dan daftar stack.
-- Dulu konstanta `CONFIG` di Worker lama; sekarang baris database supaya bisa
-- diubah dari dashboard admin (D6) tanpa deploy ulang.
--
-- Nilai awal ditulis DI SINI (migrasi hanya jalan sekali), bukan di `seeds/`:
-- seed dijalankan ulang tiap rilis dan akan menghidupkan lagi baris yang sudah
-- dihapus lewat dashboard.
--
-- Nilai ini dipakai membentuk query GraphQL ke GitHub. Server tetap
-- memeriksanya lagi saat membaca (`journal::load_config`); CHECK di sini adalah
-- pagar terakhir kalau seseorang mengubah tabel langsung.

-- Tepat satu baris (id selalu 1).
CREATE TABLE journal_profile (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    github_username TEXT NOT NULL CHECK (
        length(github_username) BETWEEN 1 AND 39
        AND github_username NOT GLOB '*[^A-Za-z0-9-]*'
    )
);

-- Repositori yang disematkan di halaman, urut menurut `position` lalu `id`.
CREATE TABLE journal_pinned_repos (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    position  INTEGER NOT NULL DEFAULT 0,
    repo_name TEXT NOT NULL UNIQUE CHECK (
        length(repo_name) BETWEEN 1 AND 100
        AND repo_name NOT GLOB '*[^A-Za-z0-9._-]*'
    ),
    topic     TEXT CHECK (topic IS NULL OR length(topic) BETWEEN 1 AND 60)
);

-- Teknologi pilihan yang ditampilkan di bagian "Core Stack".
CREATE TABLE journal_stack (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    position INTEGER NOT NULL DEFAULT 0,
    name     TEXT NOT NULL UNIQUE CHECK (length(name) BETWEEN 1 AND 40)
);

INSERT INTO journal_profile (id, github_username) VALUES (1, 'msalmanrafadhlih');

INSERT INTO journal_pinned_repos (position, repo_name, topic) VALUES
    (1, 'flexinix', 'Nixos Configuration Flakes'),
    (2, 'racooonfig', 'Linux Dotfiles'),
    (3, 'template.nix', 'Nix Development Templates'),
    (4, 'fastfetch.md', 'termux config'),
    (5, 'tquilla.is-a.bot', 'discord bot'),
    (6, 'pocket-ai.gemini', 'project exam');

INSERT INTO journal_stack (position, name) VALUES
    (1, 'NixOS'),
    (2, 'Rust'),
    (3, 'Helix'),
    (4, 'Figma'),
    (5, 'Canva');
