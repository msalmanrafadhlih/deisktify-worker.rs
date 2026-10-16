# tquilla-api: panduan project

- **Server tidak boleh percaya klien.** Validasi panjang/format input di server, normalisasi ulang URL di server, rate limit per IP (`CF-Connecting-IP`), batas ukuran body. Konstanta batas ada di `api-types::limits`, tetapi server memeriksa ulang.
- **Jangan simpan avatar/lampiran sebagai base64 di database.** Simpan di R2, DB hanya berisi URL (D4).
- **Tanpa kredensial di repo.** Rahasia lewat `wrangler secret` / `.dev.vars` (di-.gitignore). `Debug` untuk config/klien tidak boleh mencetak token (ada test-nya).
- **Pesan error ke klien umum saja.** Detail masuk `worker::console_error!`, bukan ke respons.
- **Semua kode di `crates/api` harus compile ke wasm32**: tanpa tokio, tanpa thread, tanpa crate native (`sqlx`, `libsql`, `rusqlite`, ...). Akses DB lewat `turso-http`.
- **Logika murni di luar `fetch`** supaya bisa dites native (`config`, `cors`, `health::ping` dengan transport palsu, router lewat `tower::oneshot`). Handler yang menyentuh `fetch` JS tidak bisa jalan di host.
- **`api-types` hanya boleh bergantung pada `serde`** (dipakai frontend di wasm32). Menambah endpoint = tambah konstanta di `paths` dan di `paths::ALL`. Validator URL bersama (`api_types::validate`) dipakai server dan frontend; jangan menduplikasinya. Endpoint konten: SQL di `crates/api/src/queries/*.sql` (juga diuji di `scripts/migrations.test.mjs`), logika generik atas `Transport` di `content.rs`, handler tipis di `routes.rs`.
- **Fixture protokol** (`crates/turso-http/tests/fixtures/req_*.json`) dihasilkan encoder referensi lewat `scripts/gen_golden.mjs`; jangan diedit tangan. `resp_*.json` ditulis tangan dan divalidasi decoder referensi.
- Versi Rust dikunci di `rust-toolchain.toml`; saat menaikkannya, ubah juga `dtolnay/rust-toolchain@<versi>` di `.github/workflows/ci.yml` dan samakan dengan repo frontend.
- **Skema = file di `migrations/`** (lihat `migrations/README.md`). Jangan mengubah migrasi yang sudah dijalankan (checksum dicek runner); buat nomor baru. Tanpa `BEGIN`/`COMMIT` di dalam file. Setiap tabel baru dapat `CHECK` untuk URL (`http(s)://`, avatar/lampiran `https://` saja) dan panjang teks. `seeds/*.sql` dibangkitkan oleh `scripts/gen_seed.mjs`, jangan diedit tangan. Query selalu pakai `ORDER BY` eksplisit dan parameter (`?`), tidak pernah menyisipkan input ke string SQL.
- **`crates/migrate` native saja**, tidak boleh jadi dependensi `crates/api` (ureq dan TLS-nya tidak ada gunanya di Worker). Runner (`turso-http::migrate`) tetap murni dan dites dengan transport palsu.
- **Perintah yang harus hijau sebelum commit:**
  ```sh
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --locked -- -D warnings
  cargo test --workspace --locked
  cargo clippy -p api --target wasm32-unknown-unknown --locked -- -D warnings
  node --test scripts/migrations.test.mjs
  ```
- Kerjakan tahap demi tahap sesuai README repo frontend (D1 → D8) dan centang di sana.
