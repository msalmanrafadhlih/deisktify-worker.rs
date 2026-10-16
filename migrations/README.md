# Migrasi database

File `.sql` berurutan: `NNNN_deskripsi.sql` (4 digit, huruf kecil, `_` sebagai
pemisah), mis. `0001_generations.sql`. Tabel pertama datang di tahap D2.

| Tabel | Migrasi |
| --- | --- |
| `generations` | `0001_generations.sql` |
| `bookmarks` | `0002_bookmarks.sql` |
| `radio_stations` | `0003_radio_stations.sql` |
| `embiences`, `embience_presets`, `embience_preset_items` | `0004_embiences.sql` |
| `chat_messages` | `0005_chat_messages.sql` |
| `journal_snapshots` | `0006_journal_snapshots.sql` |

## Aturan

- Jangan mengubah file yang sudah pernah dijalankan (di mana pun). Runner menyimpan
  checksum tiap migrasi di tabel `_migrations` dan menolak berjalan kalau isinya
  berubah. Buat migrasi baru dengan nomor lebih besar.
- Tanpa `BEGIN`/`COMMIT`/`ROLLBACK`/`SAVEPOINT` di dalam file (runner membungkus
  satu file dalam satu transaksi, jadi migrasi yang gagal tidak meninggalkan
  setengah skema). `BEGIN ... END` di dalam `CREATE TRIGGER` boleh.
- Dialek SQLite (sama untuk `turso dev` lokal dan Turso produksi). Tanpa `STRICT`
  dan tanpa fitur khusus versi baru, supaya aman di server libSQL mana pun.
- URL disimpan dengan `CHECK` skema `http(s)://` (avatar dan lampiran chat hanya
  `https://`). Data base64 / `data:` sengaja ditolak. CHECK adalah pagar terakhir;
  validasi sebenarnya ada di API.
- `PRAGMA foreign_keys` harus menyala per koneksi agar `REFERENCES` ditegakkan.
  Jangan mengandalkannya: periksa id di API sebelum menulis.
- Urutan baris tidak dijamin oleh tabel; selalu `ORDER BY` (mis. `number DESC`,
  `sort_order`, `position`) di query.
- `sqlx` tidak dipakai: tidak compile ke wasm32 Workers dan tidak bicara ke Turso.
  Runner ada di `crates/turso-http/src/migrate.rs`, CLI-nya di `crates/migrate`.

## Menjalankan

```sh
turso dev --db-file local.db --port 8081            # terminal lain
export TURSO_DATABASE_URL=http://127.0.0.1:8081     # TURSO_AUTH_TOKEN hanya untuk Turso cloud

cargo run -p migrate -- status                      # lihat applied / pending (tidak mengubah apa pun)
cargo run -p migrate -- up                          # terapkan migrasi baru
cargo run -p migrate -- seed                        # isi konten situs (aman diulang)
cargo run -p migrate -- seed --samples              # + contoh chat dan jurnal (hanya lokal)
```

Production tidak dimigrasi dari mesin sendiri: job `migrate` di `.github/workflows/ci.yml` menjalankannya
saat tag `v*` di-push (lihat README utama, "Migrasi otomatis di CI"). Perintah di atas untuk lokal dan debug.
Kalau endpoint menjawab 500 `no such table`, jalankan `status` dengan `TURSO_DATABASE_URL` yang **sama**
dengan yang dipakai Worker: baris `pending` berarti database itu belum dimigrasi.

Untuk database selain lokal, `up` dan `seed` butuh `--yes`. `--samples` ditolak di
database non-lokal. Token tidak pernah dicetak.

## Seed

`seeds/` dibangkitkan dari `seeds/source/*.json` (salinan JSON dummy yang dulu ada di repo frontend; sejak D3 frontend tidak lagi menyimpannya, jadi di sinilah sumbernya). Hasilnya di-commit:

```sh
node scripts/gen_seed.mjs   # seeds/source -> seeds
```

- `0001_content.sql`: generations, bookmarks, radio, embience dan preset (konten situs).
- `0002_samples.sql`: pesan chat contoh dan satu snapshot jurnal (`source = 'seed'`).
  Hanya untuk dev/demo.

Semua `INSERT OR IGNORE` dengan id eksplisit: dijalankan ulang tidak menggandakan baris
dan tidak menimpa data yang sudah diubah. Skrip menolak URL tidak valid dan nama
embience yang tidak dikenal. `preset_id` di JSON sumber kembar (dua preset ber-id 2),
jadi id preset dibuat dari urutan.
