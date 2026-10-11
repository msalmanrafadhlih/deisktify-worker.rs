# Migrasi database

File `.sql` berurutan, mis. `0001_generations.sql`. Skema pertama datang di
tahap D2 (lihat README repo frontend, bagian "Fullstack dan database").

Aturan:

- Nama: `NNNN_deskripsi.sql`, nomor naik terus. Jangan mengubah file yang sudah
  pernah dijalankan di produksi; buat migrasi baru.
- Tanpa `BEGIN`/`COMMIT` di dalam file (runner membungkusnya dalam transaksi).
- Dialek SQLite (sama untuk `turso dev` lokal dan Turso produksi).
- `sqlx` tidak dipakai: tidak compile ke wasm32 Workers dan tidak bicara ke Turso.
  Runner migrasi ditulis di D2 di atas crate `turso-http`.
