# tquilla-api

API untuk situs [tquilla.is-a.dev](https://tquilla.is-a.dev): **Axum (Rust) di Cloudflare Workers** (`workers-rs`) dengan database **Turso (libSQL)**. Frontend-nya ada di repo terpisah (Dioxus, GitHub Pages) dan hanya memakai REST + JSON.

Rencana dan keputusan lengkap (D0–D8) ada di README repo frontend, bagian "Fullstack dan database". Repo ini baru sampai **D1 (fondasi)**: satu endpoint `GET /api/v1/health`, belum ada data.

## Isi repo

| Path | Isi |
| --- | --- |
| `crates/api-types` | Kontrak API: struct serde, path, konstanta batas. **Dipakai bersama frontend** (hanya `serde`, ikut dikompilasi ke wasm32) |
| `crates/turso-http` | Klien API HTTP Turso (`/v2/pipeline`): `execute` dan `transaction`. Tanpa runtime tertentu; transport bisa diganti |
| `crates/api` | Worker: router Axum, CORS, error JSON, `GET /api/v1/health`. Berisi `wrangler.toml` |
| `migrations/` | Migrasi `.sql` (mulai D2) |
| `scripts/gen_golden.mjs` | Membangkitkan/memvalidasi fixture protokol dengan klien referensi resmi Turso |
| `.github/workflows/ci.yml` | fmt, clippy, test (native), clippy + build wasm32, cek fixture |

## Endpoint

| Method | Path | Respons |
| --- | --- | --- |
| GET | `/api/v1/health` | `200 {"status":"ok","version":"0.1.0","db":true}`; `503` dengan `"status":"degraded","db":false` bila database tidak terjangkau atau belum dikonfigurasi. Tidak pernah memuat pesan error internal |

Semua error berbentuk `{"error":{"code":"not_found","message":"Not found."}}`. Semua respons membawa `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Cache-Control: no-store`.

CORS: hanya origin di `ALLOWED_ORIGINS` (skema + host + port, tanpa path/wildcard; entri tidak valid dibuang). Kosong = tidak ada origin yang diizinkan. Tanpa cookie/credentials.

## Pengembangan lokal

Prasyarat: Rust 1.97.0 (`rust-toolchain.toml`) dengan target `wasm32-unknown-unknown`, `worker-build` (`cargo install worker-build --version 0.8.7 --locked`), `wrangler`, dan [Turso CLI](https://docs.turso.tech/cli/installation). Dengan Nix: `nix develop` (atau `direnv`) menyediakan semuanya kecuali `worker-build`.

```sh
# Terminal 1: database SQLite lokal yang bicara protokol Turso (port 8081, karena 8080 dipakai dx serve)
turso dev --db-file local.db --port 8081

# Terminal 2: Worker
cp crates/api/.dev.vars.example crates/api/.dev.vars   # lalu sesuaikan
cd crates/api && npx wrangler dev                       # http://127.0.0.1:8787

curl -i http://127.0.0.1:8787/api/v1/health
```

Pemeriksaan yang sama dengan CI:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo clippy -p api --target wasm32-unknown-unknown --locked -- -D warnings
```

## Deploy manual pertama (sebelum ada workflow deploy di D8)

```sh
turso auth login
turso db create tquilla
turso db show tquilla --url              # libsql://tquilla-<org>.turso.io
turso db tokens create tquilla           # token; simpan, hanya tampil sekali

cd crates/api
npx wrangler login
npx wrangler secret put TURSO_AUTH_TOKEN
npx wrangler secret put TURSO_DATABASE_URL
npx wrangler deploy

curl -i https://tquilla-api.<subdomain-anda>.workers.dev/api/v1/health
```

Jangan commit token. `.dev.vars` dan `.env` sudah di-`.gitignore`.

## Dipakai oleh frontend

Setelah repo ini dipush dan diberi tag `v0.1.0`, repo frontend memakai tipenya lewat git dependency:

```toml
# packages/components/Cargo.toml (repo frontend)
api-types = { git = "https://github.com/msalmanrafadhlih/tquilla-api", tag = "v0.1.0" }
```

Perubahan yang tidak kompatibel di `api-types` = tag baru; frontend memilih kapan ikut naik.

## Yang sudah dan belum terverifikasi

Sudah (dijalankan di sandbox dengan rustc 1.97.0): `cargo fmt`, `cargo clippy --workspace --all-targets -D warnings` di target host, 41 test (kontrak `api-types`, klien `turso-http`, serta router/CORS/health `api` lewat `tower::oneshot`). Request klien Turso dicocokkan dengan keluaran encoder referensi resmi `@libsql/hrana-client` 0.10.0, dan fixture respons diterima decoder referensinya.

**Belum:**
- Kompilasi ke `wasm32-unknown-unknown` dan `worker-build` (sandbox tidak punya target itu; job `wasm` di CI yang pertama kali membuktikannya).
- `wrangler dev` / `wrangler deploy`, serta tempat `worker-build` mencari crate di dalam workspace (jika gagal, pindahkan `wrangler.toml` ke root dan sesuaikan).
- Perilaku terhadap server Turso/`turso dev` sungguhan. Bentuk JSON sudah cocok dengan klien referensi, tetapi tidak ada server libSQL di sandbox. Request `batch` dipakai persis seperti `@libsql/client`; dokumentasi publik Turso hanya menyebut `execute` dan `close`.
- Batas plan gratis Cloudflare (ukuran wasm, waktu CPU per request): cek angkanya di dokumentasi Cloudflare terhadap ukuran hasil build.
- Dev shell Nix (`flake.nix`, `devenv.nix`): hanya sintaksnya yang diperiksa.

## Lisensi

MIT, lihat `LICENSE`.
