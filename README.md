# tquilla-api

API untuk situs [tquilla.is-a.dev](https://tquilla.is-a.dev): **Axum (Rust) di Cloudflare Workers** (`workers-rs`) dengan database **Turso (libSQL)**. Frontend-nya ada di repo terpisah (Dioxus, GitHub Pages) dan hanya memakai REST + JSON.

Rencana dan keputusan lengkap (D0–D8) ada di README repo frontend, bagian "Fullstack dan database". Repo ini sampai **D4 (Guestbook)**: `GET /api/v1/health`, lima endpoint konten read-only, serta `GET/POST /api/v1/messages` (buku tamu; isinya Markdown yang dirender klien) dengan validasi, pembatas kecepatan, filter kata terlarang, laporan pembaca, dan moderasi admin; ditambah skema database, seed, dan runner migrasi. **Tidak ada unggah gambar**: R2 meminta kartu pembayaran, jadi avatar dibuat klien dari username dan lampiran dihapus dari kontrak (v0.4.0).

## Isi repo

| Path | Isi |
| --- | --- |
| `crates/api-types` | Kontrak API: struct serde, path, konstanta batas. **Dipakai bersama frontend** (hanya `serde`, ikut dikompilasi ke wasm32) |
| `crates/turso-http` | Klien API HTTP Turso (`/v2/pipeline`): `execute` dan `transaction`, plus runner migrasi (`migrate`). Tanpa runtime tertentu; transport bisa diganti |
| `crates/api` | Worker: router Axum, CORS, error JSON, `health`, endpoint konten (`content.rs`), Guestbook (`chat.rs`), otorisasi admin (`admin.rs`), `routes.rs`, `queries/*.sql`. Berisi `wrangler.toml` |
| `crates/migrate` | CLI native `migrate status/up/seed` di atas `turso-http` (bukan bagian dari Worker) |
| `migrations/` | Migrasi `.sql` berurutan (skema); lihat `migrations/README.md` |
| `seeds/` | SQL seed hasil `scripts/gen_seed.mjs` (konten situs, dan contoh untuk dev) |
| `scripts/gen_golden.mjs` | Membangkitkan/memvalidasi fixture protokol dengan klien referensi resmi Turso |
| `scripts/gen_seed.mjs` | Membangkitkan `seeds/*.sql` dari `seeds/source/*.json` |
| `scripts/migrations.test.mjs` | Menerapkan migrasi + seed di SQLite sungguhan (`node:sqlite`) dan menguji constraint |
| `.github/workflows/ci.yml` | fmt, clippy, test (native), clippy + build wasm32, cek fixture, uji migrasi di SQLite |

## Endpoint

| Method | Path | Respons |
| --- | --- | --- |
| GET | `/api/v1/health` | `200 {"status":"ok","version":"0.1.0","db":true}`; `503` dengan `"status":"degraded","db":false` bila database tidak terjangkau atau belum dikonfigurasi. Tidak pernah memuat pesan error internal |
| GET | `/api/v1/generations` | `{"items":[{number,label,link,kernel,built_on}]}`, terbaru dulu. `link` = rute internal (`/deisktify`) atau `http(s)://` |
| GET | `/api/v1/bookmarks` | `{"items":[{id,label,url}]}` menurut `sort_order` |
| GET | `/api/v1/radio` | `{"items":[{id,label,slogan,stream_url,official_link?,image_url?}]}`, hanya stasiun aktif |
| GET | `/api/v1/embiences` | `{"items":[{id,name,icon_url,stream_url}]}` |
| GET | `/api/v1/presets` | `{"items":[{id,label,embience_ids:[...]}]}`; id yang tidak ada dibuang, duplikat dihapus |
| GET | `/api/v1/messages?after=<id>&before=<id>&limit=<n>` | `{"items":[{id,username,url?,message,created_at}]}`, **terlama dulu dalam satu halaman**. Tanpa kursor: `limit` entri terbaru (default 50, maks 100). `after`: hanya yang lebih baru dari id itu (tombol Refresh). `before`: halaman tepat di bawah id itu ("Load older"). `after` dan `before` tidak boleh bersama; parameter lain atau bukan angka → `400`. Entri yang disembunyikan moderator tidak pernah muncul. `no-store` |
| POST | `/api/v1/messages` | Body `{"username","url"?,"message"}` (JSON, maks 16 KiB) → `201` dengan entri yang tersimpan. `message` adalah Markdown, disimpan **apa adanya** (server tidak merender dan tidak membuat HTML). `400` input ditolak atau bukan `application/json`, `403` Origin tidak diizinkan, `413` body terlalu besar, `429` terlalu sering (+ `Retry-After`) |
| POST | `/api/v1/messages/{id}/report` | Body diabaikan → `204` baik entri ada maupun tidak (tidak membocorkan id). Satu laporan per pengirim (hash IP) per entri; maks 20 laporan per jam per pengirim → `429`. Tidak ada auto-hide |
| GET | `/api/v1/admin/reports` | Admin saja (`Authorization: Bearer <ADMIN_TOKEN>`): `{"items":[{id,username,url?,message,created_at,reports,last_reported_at}]}`, paling banyak dilaporkan dulu (maks 50). `401` tanpa/salah token |
| DELETE | `/api/v1/admin/messages/{id}` | Admin saja: menyembunyikan entri (soft delete, kolom `deleted_at`) → `204`; `404` bila tidak ada atau sudah tersembunyi; `401` tanpa/salah token |

Endpoint konten: baris dengan URL tidak aman dibuang di server (URL opsional radio yang buruk dikosongkan), klien memeriksa ulang dengan `api_types::validate`. Respons sukses memakai `Cache-Control: public, max-age=60, stale-while-revalidate=300`; error tetap `no-store`. Database tidak terjangkau → `503` kode `unavailable`; data yang bentuknya salah → `500`. Method selain `GET` → `405`.

**Guestbook (D4).** Server tidak percaya klien: username (1–32 karakter, spasi dirapikan), pesan (1–2000 karakter, maks 40 baris; tab menjadi 4 spasi), dan tautan (`http(s)`, maks 300 karakter, skema selalu dinormalisasi ulang) dirapikan dan divalidasi lagi di `api_types::chat`; karakter kontrol dan karakter tak terlihat/pembalik arah (zero-width, U+202E) ditolak, kecuali ZWJ (U+200D) di pesan supaya emoji gabungan tetap utuh. Kata terlarang dicocokkan tanpa peduli huruf besar/kecil dan tanda pemisah (`b.a-d` cocok dengan `bad`). Pesan error umum dan **tidak pernah memantulkan input**. POST wajib `Content-Type: application/json` (memaksa preflight CORS) dan, bila ada header `Origin`, harus ada di `ALLOWED_ORIGINS`.

**Markdown.** Server menyimpan teks persis seperti dikirim. Yang merender adalah klien (parser → pohon → komponen Dioxus), dan klien tidak pernah membuat string HTML: tag HTML di entri (mis. `<img src="https://…">`) hanya dikenali dari daftar putih dan dibangun ulang sebagai komponen, jadi tidak ada jalur XSS lewat entri. Konsekuensinya: server tidak bisa menjamin isi Markdown "aman" untuk klien lain (mis. aplikasi pihak ketiga); klien lain wajib memperlakukannya sebagai teks tak tepercaya. Avatar dibuat klien dari username; tidak ada unggahan atau base64 di database. Gambar di dalam entri hanya berupa alamat `https://` ke server lain: server ini tidak menyimpan, memeriksa, atau memproksinya.

Pembatas kecepatan: **5 entri per menit dan 40 per jam per pengirim**, disimpan di tabel `chat_rate_limits` (migrasi `0007`). Kunci pengirim = SHA-256(`RATE_LIMIT_SALT` + IP dari `CF-Connecting-IP`), jadi alamat IP asli tidak pernah ditulis ke database. Batas menit diperiksa dulu; kalau penuh, hitungan jam tidak ikut naik. Baris lama dibersihkan di transaksi yang sama. Dipilih tabel di database (bukan binding Rate Limiting Cloudflare) karena binding itu hanya konsisten per lokasi dan tidak dirancang untuk kunci berupa IP. **Tidak ada batas global**: banyak IP berbeda tetap bisa memenuhi buku tamu; itulah gunanya moderasi di bawah.

**Moderasi (migrasi `0008`).** Pembaca melaporkan lewat `POST /messages/{id}/report`; tabel `message_reports` menyimpan satu baris per (entri, hash pelapor), jadi satu orang tidak bisa menggelembungkan hitungan. Tidak ada penyembunyian otomatis (laporan massal tidak boleh bisa membungkam orang lain). Admin melihat daftar dan menyembunyikan entri lewat `curl`; CORS sengaja **tidak** mengizinkan header `Authorization` maupun `DELETE`, jadi browser situs mana pun tidak bisa memanggil endpoint admin. Token dibandingkan dengan perbandingan waktu-konstan (lewat digest SHA-256) dan endpoint admin **gagal tertutup**: token tidak diset atau kurang dari 32 karakter → semua permintaan admin `401`.

```sh
export API=https://<worker>.workers.dev
export ADMIN_TOKEN=...                                  # sama dengan secret Worker; jangan di-commit

curl -s -H "Authorization: Bearer $ADMIN_TOKEN" "$API/api/v1/admin/reports"
curl -s -X DELETE -H "Authorization: Bearer $ADMIN_TOKEN" "$API/api/v1/admin/messages/42" -w '%{http_code}\n'   # 204
```

Konfigurasi baru (tidak ada di repo):

| Nama | Jenis | Isi |
| --- | --- | --- |
| `RATE_LIMIT_SALT` | secret | string acak panjang: `openssl rand -hex 32`, lalu `npx wrangler secret put RATE_LIMIT_SALT`. Kosong tetap jalan, tetapi hash IP lebih mudah ditebak. Mengganti nilainya mereset hitungan semua orang |
| `ADMIN_TOKEN` | secret | token admin, minimal 32 karakter: `openssl rand -hex 32`, lalu `npx wrangler secret put ADMIN_TOKEN`. Tidak diset = endpoint admin mati (`401`). Mengganti nilainya langsung mencabut token lama |
| `BANNED_WORDS` | secret atau var | kata dipisah koma atau baris baru (maks 500 kata). Sengaja tidak di repo publik. Kosong = tidak ada filter |

Respons `GET /messages` tidak boleh di-cache (`no-store`): entri baru dan entri yang baru disembunyikan harus langsung terlihat saat Refresh.

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

Skema dan data awal (terminal 3, database `turso dev` harus sudah jalan):

```sh
export TURSO_DATABASE_URL=http://127.0.0.1:8081
cargo run -p migrate -- up               # buat tabel
cargo run -p migrate -- seed --samples   # konten situs + contoh chat/jurnal (lokal saja)
```

Detail dan aturan migrasi: `migrations/README.md`.

Pemeriksaan yang sama dengan CI:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo clippy -p api --target wasm32-unknown-unknown --locked -- -D warnings
node --test scripts/migrations.test.mjs     # Node >= 22.13
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

### Migrasi otomatis di CI

Sejak D3, migrasi dan seed konten ke Turso production dijalankan otomatis oleh job `migrate` di `.github/workflows/ci.yml`:

- **Pemicu:** push tag `v*` (setelah fmt, clippy, test, wasm32, fixture, dan uji SQL hijau), atau tombol **Run workflow** di tab Actions (hanya dari `main`). Pull request tidak pernah menyentuh database.
- **Langkah:** `migrate status` → `migrate up --yes` → `migrate seed --yes` (tanpa `--samples`) → `migrate status` (harus "up to date"). Aman diulang: `up` hanya menerapkan migrasi baru, seed memakai `INSERT OR IGNORE`.
- **Secret yang dibutuhkan** (Settings > Secrets and variables > Actions): `TURSO_DATABASE_URL` dan `TURSO_AUTH_TOKEN`. Nilainya harus **persis sama** dengan secret Worker; kalau beda, Worker membaca database lain daripada yang dimigrasi dan semua endpoint konten menjawab 500 `no such table`.
- Job memakai `environment: production`; di Settings > Environments kamu bisa menambah "Required reviewers" untuk persetujuan manual. Log Actions repo publik itu publik, jadi nama host Turso disamarkan dengan `::add-mask::`.
- **Aturan migrasi:** job ini jalan *sebelum* Worker baru di-deploy, jadi Worker lama masih berjalan di skema baru. Migrasi harus kompatibel mundur (tambah tabel/kolom, jangan hapus atau ganti nama dulu; hapus di rilis berikutnya).
- Seed memakai `INSERT OR IGNORE`: mengubah isi `seeds/source/*.json` **tidak** memperbarui baris yang sudah ada. Mengubah konten yang sudah tayang butuh migrasi baru atau (D6) endpoint admin.
- Migrasi `0007_chat_rate_limits` dan `0008_chat_moderation` ikut diterapkan job ini; log akhir harus "up to date (8 applied)". `0008` hanya menambah kolom `deleted_at` dan tabel `message_reports`, jadi Worker lama (v0.3.0) tetap jalan di skema baru. Nama tabel `chat_messages` dan path `/messages` sengaja tidak diganti jadi "guestbook" karena alasan yang sama.
- Deploy Worker tetap terpisah (`npx wrangler deploy` dari `crates/api`, atau D8).

### Migrasi manual (debug, atau database lokal)

Dari mesin sendiri (token hanya lewat environment, tidak dicetak):

```sh
export TURSO_DATABASE_URL=libsql://tquilla-<org>.turso.io
export TURSO_AUTH_TOKEN=...                       # dari `turso db tokens create`
cargo run -p migrate -- status
cargo run -p migrate -- up   --yes
cargo run -p migrate -- seed --yes                # tanpa --samples: itu data palsu
```

Jangan commit token. `.dev.vars` dan `.env` sudah di-`.gitignore`.

## Dipakai oleh frontend

Setelah repo ini dipush dan diberi tag (kini `v0.4.0`, yang membawa Guestbook dan moderasi), repo frontend memakai tipenya lewat git dependency:

```toml
# packages/components/Cargo.toml (repo frontend)
api-types = { git = "https://github.com/msalmanrafadhlih/deisktify-worker.rs", tag = "v0.4.0" }
```

Perubahan yang tidak kompatibel di `api-types` = tag baru; frontend memilih kapan ikut naik.

## Yang sudah dan belum terverifikasi

Sudah (dijalankan di sandbox dengan rustc 1.97.0): `cargo fmt`, `cargo clippy --workspace --all-targets --locked -D warnings` di target host, 140 test Rust (kontrak dan validator `api-types`, klien dan runner migrasi `turso-http`, CLI `migrate`, serta router/CORS/health/konten/guestbook/moderasi `api` lewat `tower::oneshot` dan transport palsu) dan 31 test node (termasuk semua query `.sql`, termasuk `before`, entri tersembunyi, laporan, dan migrasi `0008` di atas data yang sudah ada, terhadap SQLite sungguhan). Migrasi dan seed diterapkan di SQLite 3.45 sungguhan (Python `sqlite3` dan `node:sqlite`): skema valid, seed idempotent, CHECK menolak URL `javascript:`, avatar base64, JSON rusak, dsb. CLI `migrate` dijalankan end-to-end lewat server Hrana tiruan (skrip sementara di atas SQLite, bukan `sqld`): `up`, `seed`, jalan ulang, checksum berubah ditolak, migrasi gagal ter-rollback. Request klien Turso dicocokkan dengan keluaran encoder referensi resmi `@libsql/hrana-client` 0.10.0, dan fixture respons diterima decoder referensinya.

**Belum:**
- Kompilasi ke `wasm32-unknown-unknown` dan `worker-build` (sandbox tidak punya target itu; job `wasm` di CI yang pertama kali membuktikannya).
- `wrangler dev` / `wrangler deploy`, serta tempat `worker-build` mencari crate di dalam workspace (jika gagal, pindahkan `wrangler.toml` ke root dan sesuaikan).
- Migrasi dan seed sudah dicoba Moch di `turso dev` sungguhan (2026-10-16: `up`, ulangan, seed idempotent). **Belum** dicoba terhadap Turso cloud, dan kelima endpoint konten (D3) belum dijalankan terhadap `sqld`/Turso: querynya baru diuji di SQLite biasa. Server tiruan di sandbox hanya menafsirkan protokol seperti klien referensi.
- Batas plan gratis Cloudflare (ukuran wasm, waktu CPU per request): cek angkanya di dokumentasi Cloudflare terhadap ukuran hasil build.
- Guestbook dan moderasi (D4) belum dijalankan terhadap `sqld`/Turso cloud dan Worker sungguhan: logikanya diuji dengan transport palsu dan, end-to-end, lewat server Hrana tiruan di atas SQLite (5 pesan diterima, ke-6 ditolak `429`, pengirim lain tidak terkena, menit berikutnya longgar lagi; ini hasil D4a, belum diulang untuk laporan/admin). Endpoint laporan dan admin baru diuji lewat `tower::oneshot` dan SQLite, belum dengan `ADMIN_TOKEN` sungguhan di Worker. Header `CF-Connecting-IP` hanya ada di Cloudflare; lokal semua pengirim dihitung satu (`local`).
- Dev shell Nix (`flake.nix`, `devenv.nix`): hanya sintaksnya yang diperiksa.

## Lisensi

MIT, lihat `LICENSE`.
