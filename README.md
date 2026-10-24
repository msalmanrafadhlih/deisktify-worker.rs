# tquilla-api

API untuk situs [tquilla.is-a.dev](https://tquilla.is-a.dev): **Axum (Rust) di Cloudflare Workers** (`workers-rs`) dengan database **Turso (libSQL)**. Frontend-nya ada di repo terpisah (Dioxus, GitHub Pages) dan hanya memakai REST + JSON.

Rencana dan keputusan lengkap (D0–D8) ada di README repo frontend, bagian "Fullstack dan database". Repo ini sampai **D4 (Guestbook)**: `GET /api/v1/health`, lima endpoint konten read-only, serta `GET/POST /api/v1/messages` (buku tamu; isinya Markdown yang dirender klien) dengan validasi, pembatas kecepatan, filter kata terlarang, laporan pembaca, dan moderasi admin; ditambah skema database, seed, dan runner migrasi. **Tidak ada unggah gambar**: R2 meminta kartu pembayaran, jadi avatar dibuat klien dari username dan lampiran dihapus dari kontrak (v0.4.0). Sejak **D5** ada GitHub Journal (cron), sejak **D6a** ada dashboard admin di `/admin/` (lihat bagian "Dashboard admin"), dan sejak **D7a** ada pemeriksa stasiun radio (cron kedua; lihat bagian "Pemeriksa stasiun radio"), dengan panelnya di dashboard sejak **D7b**.

## Isi repo

| Path | Isi |
| --- | --- |
| `crates/api-types` | Kontrak API: struct serde, path, konstanta batas. **Dipakai bersama frontend** (hanya `serde`, ikut dikompilasi ke wasm32) |
| `crates/turso-http` | Klien API HTTP Turso (`/v2/pipeline`): `execute` dan `transaction`, plus runner migrasi (`migrate`). Tanpa runtime tertentu; transport bisa diganti |
| `crates/api` | Worker: router Axum, CORS, error JSON, `health`, endpoint konten (`content.rs`), Guestbook (`chat.rs`), otorisasi admin (`admin.rs`), dashboard admin (`panel.rs`: pembatas percobaan gagal, riwayat, ringkasan), pemeriksa stasiun radio (`radio_check.rs` logika murni, `radio_probe.rs` hanya `fetch`), `routes.rs`, `queries/*.sql`. Berisi `wrangler.toml` |
| `web/` | **Dashboard admin** (D6a): JavaScript biasa tanpa build step (Preact + htm yang disalin ke repo), dilayani Worker yang sama di `/admin/`. `_headers` berisi CSP. Lihat bagian "Dashboard admin" |
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
| GET | `/api/v1/journal` | Snapshot terbaru jurnal GitHub (halaman `/profile`): `{"fetched_at":"…Z","source":"github"\|"seed","data":{hero,pinned,chronicle}}`. **Data, bukan tampilan**: tidak ada HTML dan tidak ada kalimat (lihat `api_types::journal`). Payload diperiksa lagi sebelum dikirim. `503` bila belum pernah ada snapshot. `Cache-Control: public, max-age=300, stale-while-revalidate=600` |
| POST | `/api/v1/admin/journal/refresh` | Admin saja: ambil dari GitHub sekarang tanpa menunggu cron → `200 {"fetched_at":"…Z"}`. Paling cepat sekali per menit (`429` + `Retry-After`); `503` bila `GITHUB_TOKEN` belum diisi; `502` bila GitHub gagal; `401` tanpa/salah token |
| GET | `/api/v1/admin/radio` | Admin saja (D7b): `{now, next_run_at, rules:{fail_threshold,min_fail_hours,batch,min_manual_secs}, items:[{id,label,is_active,checkable,link_changed,last_result,last_status,last_checked_at,last_ok_at,failing_since,consecutive_failures,disabled_by_check}]}` untuk semua stasiun (aktif maupun tidak). **Tanpa URL stream** (memuat token). `no-store` |
| POST | `/api/v1/admin/radio/check` | Admin saja (D7b): periksa stasiun aktif sekarang, tanpa menunggu cron → `200 {checked,ok,failed,skipped,inconclusive,disabled:[id]}` (tanpa URL). Paling cepat sekali per menit sejak pemeriksaan terakhir, cron atau manual (`429` + `Retry-After`); dicatat di riwayat sebagai `radio.check` |
| POST | `/api/v1/admin/login` | Admin saja: memeriksa token, mencatat "login" di riwayat, menjawab ringkasan (sama dengan `overview`). `401` token salah, `429` + `Retry-After` setelah 10 percobaan gagal dalam 15 menit |
| GET | `/api/v1/admin/overview` | Admin saja: `{now, me, tables:[{name,rows}], guestbook:{total,hidden,reported}, journal:{fetched_at,source,snapshots,github_configured}, migrations:{applied,latest}}`. `me` = delapan karakter pertama hash IP pemanggil. Tidak pernah memuat token atau nilai secret |
| GET | `/api/v1/admin/audit?before=<id>&limit=<n>` | Admin saja: `{"items":[{id,at,action,target,who}]}`, terbaru dulu (default 50, maks 100, `before` = halaman lebih lama). Hanya aksi yang berhasil, tanpa isi data |
| GET | `/api/v1/admin/journal` | Admin saja: `{now, github_username, repos:[{id,repo_name,topic}], stack:[{id,name}], snapshots:[{id,fetched_at,source}], caps:{max_pinned,max_stack}}`. Repo dan stack menurut urutan tampil; snapshot 20 terbaru, tanpa payload |
| PUT | `/api/v1/admin/journal/profile` | Admin saja: `{"github_username"}` (huruf, angka, `-`, maks 39) → `200 {"github_username"}` |
| POST | `/api/v1/admin/journal/repos` | Admin saja: `{"repo_name","topic"?}` → `201 {"id"}`, ditambahkan di urutan terakhir. `400` input salah atau daftar penuh (maks 12), `409` nama sudah ada |
| PUT / DELETE | `/api/v1/admin/journal/repos/{id}` | Admin saja: ubah (`{"repo_name","topic"?}`) atau hapus → `204`; `404` bila tidak ada, `409` nama kembar |
| PUT | `/api/v1/admin/journal/repos/order` | Admin saja: `{"ids":[...]}` harus memuat semua id yang ada tepat sekali, kalau tidak `400` dan tidak ada yang berubah → `204` |
| POST | `/api/v1/admin/journal/stack` | Admin saja: `{"name"}` (maks 40) → `201 {"id"}`; `400` penuh (maks 12) atau salah, `409` kembar |
| PUT / DELETE | `/api/v1/admin/journal/stack/{id}` | Admin saja: ubah (`{"name"}`) atau hapus → `204`; `404`, `409` |
| PUT | `/api/v1/admin/journal/stack/order` | Admin saja: seperti urutan repo |
| GET | `/api/v1/admin/messages?filter=reported\|hidden\|all&before=<id>&limit=<n>` | Admin saja: `{"items":[{id,username,url?,message,created_at,hidden_at?,reports,last_reported_at?}]}`, id terbesar dulu (default 30, maks 100). `reported` = masih tampil dan punya laporan. Teks lengkap, mentah |
| POST | `/api/v1/admin/messages/{id}/restore` | Admin saja: menampilkan lagi entri yang disembunyikan → `204`; `404` bila tidak ada atau sudah tampil |
| DELETE | `/api/v1/admin/messages/{id}/reports` | Admin saja: menolak (menghapus) semua laporan sebuah entri; entrinya tetap → `204`; `404` bila tidak ada laporan |
| GET | `/api/v1/admin/tables` | Admin saja: `{"items":[{name,label,editable,key,key_label,key_given,paged,max_rows,rows,columns:[{name,label,kind,nullable,min?,max?}]}]}`: daftar putih tabel yang boleh dikelola dan bentuk kolomnya (UI membangun formulir dari sini). Tabel di luar daftar tidak ada: `404` |
| GET / POST | `/api/v1/admin/tables/{table}/rows` | Admin saja. `GET` → `{"items":[…]}` (tabel kecil: semua baris, maks 500; tabel besar: `?before=<id>&limit=<n>`). `POST` (JSON sesuai kolom) → `201 {"id"}`; `400` isian salah atau tabel penuh, `409` nilai kembar, tabel hanya-baca → `400` |
| PUT / DELETE | `/api/v1/admin/tables/{table}/rows/{id}` | Admin saja: ubah (semua kolom, kunci tidak ikut) atau hapus → `204`; `404` bila baris tidak ada; `409` nilai kembar. Hapus induk ikut menghapus anaknya dalam satu transaksi |
| GET | `/api/v1/admin/tables/{table}/export?format=json\|csv` | Admin saja: seluruh isi tabel (maks 5.000 baris) sebagai unduhan. Sel CSV yang diawali `=`, `+`, `-`, `@` diberi awalan `'` (anti formula spreadsheet). Kolom `who` dan hash pelapor tidak diekspor |
| GET | `/api/v1/admin/presets/items` | Admin saja: `{"items":[{preset_id,embience_id,position}]}` isi semua preset Embience |
| PUT | `/api/v1/admin/presets/{id}/items` | Admin saja: `{"ids":[embience_id,…]}` (maks 20, tanpa kembar, id harus ada) menggantikan isi preset dalam satu transaksi → `204`; `404` bila preset tidak ada |
| POST | `/api/v1/admin/messages/{id}/purge` | Admin saja: hapus **permanen** entri Guestbook beserta laporannya. Hanya untuk entri yang sudah disembunyikan, dan body harus `{"confirm":"purge:<id>"}` persis (diperiksa server) → `204`; `400` salah konfirmasi atau entri masih tampil, `404` bila tidak ada |

Endpoint konten: baris dengan URL tidak aman dibuang di server (URL opsional radio yang buruk dikosongkan), klien memeriksa ulang dengan `api_types::validate`. Respons sukses memakai `Cache-Control: public, max-age=60, stale-while-revalidate=300`; error tetap `no-store`. Database tidak terjangkau → `503` kode `unavailable`; data yang bentuknya salah → `500`. Method selain `GET` → `405`.

**Guestbook (D4).** Server tidak percaya klien: username (1–32 karakter, spasi dirapikan), pesan (1–2000 karakter, maks 40 baris; tab menjadi 4 spasi), dan tautan (`http(s)`, maks 300 karakter, skema selalu dinormalisasi ulang) dirapikan dan divalidasi lagi di `api_types::chat`; karakter kontrol dan karakter tak terlihat/pembalik arah (zero-width, U+202E) ditolak, kecuali ZWJ (U+200D) di pesan supaya emoji gabungan tetap utuh. Kata terlarang dicocokkan tanpa peduli huruf besar/kecil dan tanda pemisah (`b.a-d` cocok dengan `bad`). Pesan error umum dan **tidak pernah memantulkan input**. POST wajib `Content-Type: application/json` (memaksa preflight CORS) dan, bila ada header `Origin`, harus ada di `ALLOWED_ORIGINS`.

**Markdown.** Server menyimpan teks persis seperti dikirim. Yang merender adalah klien (parser → pohon → komponen Dioxus), dan klien tidak pernah membuat string HTML: tag HTML di entri (mis. `<img src="https://…">`) hanya dikenali dari daftar putih dan dibangun ulang sebagai komponen, jadi tidak ada jalur XSS lewat entri. Konsekuensinya: server tidak bisa menjamin isi Markdown "aman" untuk klien lain (mis. aplikasi pihak ketiga); klien lain wajib memperlakukannya sebagai teks tak tepercaya. Avatar dibuat klien dari username; tidak ada unggahan atau base64 di database. Gambar di dalam entri hanya berupa alamat `https://` ke server lain: server ini tidak menyimpan, memeriksa, atau memproksinya.

Pembatas kecepatan: **5 entri per menit dan 40 per jam per pengirim**, disimpan di tabel `chat_rate_limits` (migrasi `0007`). Kunci pengirim = SHA-256(`RATE_LIMIT_SALT` + IP dari `CF-Connecting-IP`), jadi alamat IP asli tidak pernah ditulis ke database. Batas menit diperiksa dulu; kalau penuh, hitungan jam tidak ikut naik. Baris lama dibersihkan di transaksi yang sama. Dipilih tabel di database (bukan binding Rate Limiting Cloudflare) karena binding itu hanya konsisten per lokasi dan tidak dirancang untuk kunci berupa IP. **Tidak ada batas global**: banyak IP berbeda tetap bisa memenuhi buku tamu; itulah gunanya moderasi di bawah.

**Moderasi (migrasi `0008`).** Pembaca melaporkan lewat `POST /messages/{id}/report`; tabel `message_reports` menyimpan satu baris per (entri, hash pelapor), jadi satu orang tidak bisa menggelembungkan hitungan. Tidak ada penyembunyian otomatis (laporan massal tidak boleh bisa membungkam orang lain). Admin melihat daftar dan menyembunyikan entri lewat `curl` (dashboard `/admin/` akan memakainya di D6b); CORS sengaja **tidak** mengizinkan header `Authorization` maupun `DELETE`, jadi browser situs mana pun tidak bisa memanggil endpoint admin. Dashboard memanggilnya dari origin yang sama (Worker ini), jadi CORS tidak perlu dilonggarkan. Token dibandingkan dengan perbandingan waktu-konstan (lewat digest SHA-256) dan endpoint admin **gagal tertutup**: token tidak diset atau kurang dari 32 karakter → semua permintaan admin `401`.

```sh
export API=https://<worker>.workers.dev
export ADMIN_TOKEN=...                                  # sama dengan secret Worker; jangan di-commit

curl -s -H "Authorization: Bearer $ADMIN_TOKEN" "$API/api/v1/admin/reports"
curl -s -X DELETE -H "Authorization: Bearer $ADMIN_TOKEN" "$API/api/v1/admin/messages/42" -w '%{http_code}\n'   # 204
curl -s -X POST -H "Authorization: Bearer $ADMIN_TOKEN" "$API/api/v1/admin/journal/refresh"                     # {"fetched_at":"…"}
```

**Dashboard admin (D6a migrasi `0010`, D6b dan D6c tanpa migrasi).** Tujuannya: tidak perlu `curl` lagi. D6a: login, Ringkasan, Riwayat. D6b: tab Journal dan Guestbook. D6c: tab Data (editor umum, Embience, radio, ekspor) dan penghapusan permanen di Guestbook.

- **Di mana.** `web/admin/` (HTML, CSS, JavaScript biasa; tanpa npm, tanpa build) dilayani Worker yang sama lewat Workers Static Assets (`[assets]` di `wrangler.toml`). Alamatnya `https://<worker>.workers.dev/admin/`. Karena satu origin dengan API, CORS **tidak dilonggarkan**: situs publik (tquilla.is-a.dev) tidak membawa kode admin sama sekali dan tidak bisa memanggil endpoint admin. Pustaka UI (htm + Preact, 13 KB) disalin ke `web/admin/vendor/` dengan SHA-256 yang dites; tidak ada skrip dari host lain.
- **Alamatnya bukan rahasia** (repo ini publik). Yang melindungi adalah `ADMIN_TOKEN` yang diperiksa server, bukan path yang disembunyikan.
- **Masuk.** Ketik token di kolom password (password manager bisa mengisinya). Token hanya disimpan di memori halaman: tidak di URL, `localStorage`, `sessionStorage`, atau cookie, dan hilang saat halaman ditutup. Dashboard terkunci sendiri setelah 10 menit tanpa aktivitas, juga ada tombol Kunci. Token salah atau sesi berakhir → kembali ke layar masuk.
- **Pembatas percobaan gagal** (`panel::check_access`, tabel `admin_attempts`): 10 percobaan gagal per jendela 15 menit per IP (hash) → `429` + `Retry-After`, **juga untuk token yang benar** selama terkunci. Hanya kegagalan yang menulis ke database. Admin mati (token kosong atau < 32 karakter) → `401` tanpa menyentuh database. Token benar tetapi database mati → `503`; token salah dan database mati → tetap `401`.
- **Riwayat** (`admin_audit`): mencatat login, refresh journal, dan penyembunyian entri, dengan hash IP delapan karakter. Tidak berisi isi data. Paling baru 500 baris. Gagal mencatat tidak membatalkan aksinya (hanya tercatat di log).
- **CSP ketat** (`web/_headers`): `default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; ...`, tidak ada inline, tidak boleh di-frame, `noindex`, `no-store`. Semua data dari server ditampilkan sebagai teks (tidak ada `innerHTML`; dites).
- **Tab Journal (D6b).** Status data GitHub dan tombol *Refresh sekarang* (jika terlalu cepat, tombol menghitung mundur dari `Retry-After`); ganti akun GitHub; repo sematan dan stack: tambah, ubah di tempat, hapus (dua klik), geser naik/turun; daftar snapshot. Semua isian diperiksa ulang server (aturan sama dengan CHECK di migrasi `0009` dan `journal::load_config`); batas 12 repo dan 12 stack ditegakkan di SQL (`INSERT ... WHERE count < batas`), nama kembar → `409`. Urutan disimpan sebagai `position` 1..n dalam satu transaksi, dan hanya diterima kalau `ids` persis sama dengan id yang ada. **Perubahan baru terlihat di situs setelah refresh berikutnya** (tombol atau cron), karena data tampil berasal dari snapshot.
- **Tab Guestbook (D6b, D6c).** Saringan *Dilaporkan* (bawaan), *Disembunyikan*, *Semua*; teks entri lengkap sebagai **teks biasa** (tidak dirender Markdown, tidak ada tautan yang bisa diklik); *Sembunyikan* (dua klik), *Tampilkan lagi*, *Tolak laporan* (menghapus laporan, entri tetap); *Muat yang lebih lama*. **Hapus permanen (D6c)** hanya muncul pada entri yang sudah disembunyikan: harus mengetik `#<id>` entri itu (tombol baru aktif bila cocok) dan server memeriksa ulang `purge:<id>`. Entri dan laporannya hilang dari database, tidak bisa dikembalikan.
- **Tab Data (D6c).** Satu editor untuk lima tabel konten yang boleh diubah (`generations`, `bookmarks`, `radio_stations`, `embiences`, `embience_presets`), dibangun dari **daftar putih** di `crates/api/src/tables.rs`: nama tabel dan kolom hanya berasal dari daftar itu (tidak pernah dari klien), semua nilai di-*bind* sebagai parameter, dan **tidak ada SQL mentah** di mana pun. Tambah baris lewat formulir yang tipe tiap kolomnya diperiksa dua kali (klien untuk pesan cepat, server sebagai penentu: teks dan panjang, bilangan bulat dengan batas, `http(s)://` untuk URL, rute internal atau URL untuk tautan, kotak centang, kolom opsional kosong → `NULL`); ubah di tempat; hapus dengan dua klik. Batas jumlah baris per tabel ditegakkan di SQL (`INSERT ... SELECT ... WHERE count < batas`), nilai kembar → pesan "Nilai itu sudah ada." (`409`). Empat tabel lain (`chat_messages`, `admin_audit`, `journal_pinned_repos`, `journal_stack`) hanya bisa **dilihat dan diekspor**; entri Guestbook dimoderasi di tab Guestbook dan repo/stack Journal diubah di tab Journal.
- **Tab Radio (D7b).** Ringkasan (aktif, sehat, sedang gagal, dimatikan otomatis, jadwal cron berikutnya), daftar semua stasiun dengan keadaannya dalam kalimat (Sehat, Gagal 2× sejak..., Dimatikan otomatis, Tidak diperiksa, Tautan baru, Belum meyakinkan), dan tombol *Cek sekarang* yang menghitung mundur dari jam server. Aturan (ambang, jeda, ukuran putaran) dibaca dari respons, tidak disalin ke JavaScript. URL stream tidak pernah dikirim ke halaman: tautan baru ditempel lewat tab Data, lalu stasiun diaktifkan lagi di sana. "Cek sekarang" memakai putaran yang sama dengan cron (kegagalan tetap harus memenuhi ambang dan jeda 6 jam), dan memakai maksimal 41 dari 50 subrequest plan gratis (dijaga `const` assertion).
- **Embience dan radio (D6c).** Stasiun radio dan suara latar diedit lewat editor yang sama. Isi preset (suara mana, urutannya) punya editor sendiri: naik/turun, tambah, buang, lalu *Simpan isi* (diganti dalam satu transaksi, maks 20, tanpa kembar).
- **Ekspor (D6c).** *Ekspor JSON* / *Ekspor CSV* per tabel, diunduh dari browser lewat `Blob` (CSP tidak dilonggarkan; **belum dicoba di browser sungguhan**). Sebelum mengubah banyak data, ekspor dulu sebagai cadangan kasar. Cadangan database penuh = D8.
- **Seed hanya berlaku sekali (D6c).** Karena CI menjalankan `migrate seed --yes` di setiap tag `v*`, konten awal dulu akan hidup lagi setelah dihapus lewat dashboard. Sekarang `seeds/0001_content.sql` memasang penanda di tabel `_seeded` dan melewati seluruh isinya bila penanda sudah ada. Akibatnya, **mengubah `seeds/source/*.json` tidak lagi mengubah database yang sudah terisi**: kelola konten lewat dashboard. Database baru tetap terisi lengkap.
- **Riwayat** kini juga mencatat: `message.restore`, `message.dismiss`, `journal.profile`, `journal.repo.{add,edit,delete,order}`, `journal.stack.{add,edit,delete,order}`, serta (D6c) `table.{insert,update,delete}` (target `<tabel>:<id>`), `table.export` (target = nama tabel), `preset.items`, dan `message.purge` (target = nama repo/stack, `message:<id>`, dst.).
- **Kolom baru di tabel tidak lewat dashboard.** Perubahan skema hanya lewat berkas di `migrations/`.

Konfigurasi baru (tidak ada di repo):

| Nama | Jenis | Isi |
| --- | --- | --- |
| `RATE_LIMIT_SALT` | secret | string acak panjang: `openssl rand -hex 32`, lalu `npx wrangler secret put RATE_LIMIT_SALT`. Kosong tetap jalan, tetapi hash IP lebih mudah ditebak. Mengganti nilainya mereset hitungan semua orang |
| `ADMIN_TOKEN` | secret | token admin, minimal 32 karakter: `openssl rand -hex 32`, lalu `npx wrangler secret put ADMIN_TOKEN`. Tidak diset = endpoint admin mati (`401`). Mengganti nilainya langsung mencabut token lama |
| `GITHUB_TOKEN` | secret | token GitHub untuk cron jurnal. Cukup scope `read:user` (data publik); tambahkan `repo` hanya bila kontribusi privat mau ikut dihitung. Buat token baru (fine-grained atau classic), `npx wrangler secret put GITHUB_TOKEN`, lalu cabut token Worker lama. Tidak diset = cron dan refresh manual gagal (tercatat di log), halaman tetap menampilkan snapshot terakhir |
| `BANNED_WORDS` | secret atau var | kata dipisah koma atau baris baru (maks 500 kata). Sengaja tidak di repo publik. Kosong = tidak ada filter |

**GitHub Journal (D5, migrasi `0009`).** Pengganti Worker JavaScript lama (`github-journal-stats`). Cron Trigger `0 */6 * * *` (`[triggers]` di `crates/api/wrangler.toml`) menjalankan query GraphQL yang sama dengan Worker lama, menyimpan hasilnya ke `journal_snapshots` (`source = 'github'`, 28 terbaru dipertahankan, seed tidak pernah dipangkas), dan `GET /journal` hanya membaca snapshot terakhir. Kalau GitHub gagal, snapshot lama tetap dipakai dan galatnya masuk log Worker.

- **Pengaturan ada di tabel, bukan kode**, supaya D6 (dashboard) bisa mengubahnya: `journal_profile` (satu baris: akun GitHub), `journal_pinned_repos` (`position`, `repo_name`, `topic`) dan `journal_stack` (`position`, `name`). Nilai awalnya (isi `CONFIG` Worker lama) ditulis di migrasi `0009`, **bukan** di `seeds/`: seed berjalan ulang tiap rilis dan akan menghidupkan lagi baris yang sudah kamu hapus.
- **Nilai dari database tidak dipercaya.** Nama repo dikirim ke GitHub sebagai *variabel* GraphQL (`$repo0`, `$repo1`, ...), tidak pernah disisipkan ke teks query; akun dan nama repo diperiksa ulang saat dibaca (huruf, angka, `-`, `.`, `_`) dan baris yang buruk dibuang. `CHECK` di tabel adalah pagar terakhir.
- **Tanpa HTML.** Worker lama membuat `monthly_focus_html` dengan menempelkan nama repo mentah ke string HTML yang lalu ditampilkan klien lewat `dangerous_inner_html`. Sekarang payload hanya berisi data (`monthly_focus: {repo, language}`); klien menyusun kalimatnya. `Data::validate` dijalankan sebelum menyimpan **dan** sebelum mengirim.
- **Perbedaan kecil dari Worker lama** (disengaja): commit per repositori hanya diminta untuk bulan ini (dulu 12 bulan lalu disaring; lebih ringan untuk batas CPU 10 ms), periode "tahun lalu" berakhir sedetik sebelum periode sekarang (dulu kehilangan hari terakhirnya), hari tersibuk tidak menyertakan hari tanpa kontribusi, dan hari produktif tanpa aktivitas jatuh ke hari ini (dulu selalu Minggu).
- **Refresh manual** lewat tombol di tab Journal (atau `POST /admin/journal/refresh`; CORS tetap tidak mengizinkan `Authorization`). Berguna tepat setelah deploy pertama, supaya halaman tidak kosong sampai cron pertama berjalan.
- Batas plan gratis (lihat [dokumentasi Cloudflare](https://developers.cloudflare.com/workers/platform/limits)): **CPU 10 ms per eksekusi, termasuk cron**, dan 5 cron per akun. Menunggu jaringan tidak dihitung; parse JSON dan transformasi dihitung. Parse memakai struct yang hanya memuat kolom yang dipakai. Angka sebenarnya belum terukur (lihat bawah).

**Pemeriksa stasiun radio (D7a, migrasi `0011`).** URL stream beberapa stasiun memuat token (`rj-tok`) yang bisa kedaluwarsa. Cron Trigger `17 * * * *` (tiap jam, menit ke-17 UTC) mengetuk stasiun aktif, mencatat hasilnya di `radio_checks`, dan menandai `is_active = 0` bila kegagalannya meyakinkan. Pemeriksa **hanya menandai**: tautan baru tetap ditempel admin lewat tab Data. Cron ini dan cron jurnal adalah dua ekspresi berbeda di `[triggers]`; `scheduled` membedakannya lewat `ScheduledEvent::cron()`, jadi masing-masing eksekusi punya anggaran subrequest dan CPU sendiri.

- **Hanya baca header.** `GET` dengan `redirect: manual`; begitu header diterima, `AbortController` memutus koneksi (stream tidak pernah selesai, badannya tidak diunduh). Batas tunggu 8 detik per permintaan, enam stasiun serentak.
- **URL yang diperiksa dibatasi** (`radio_check::probeable`): `https`, nama host biasa (bukan IP, `localhost`, `.local`, `.internal`), tanpa userinfo, port 443. URL lain dicatat `skipped` dan **tidak pernah** dimatikan. Termasuk di sini stasiun dengan port non-standar (Jak FM dan Gen98 memakai `:1936`): `fetch` di Workers produksi dilaporkan mengabaikan port ([cloudflare-docs#4299](https://github.com/cloudflare/cloudflare-docs/issues/4299); statusnya hari ini belum terverifikasi), jadi hasilnya akan gagal palsu.
- **Redirect diikuti satu per satu**, maksimal 2 lompatan; tiap tujuan harus lolos `probeable` lagi (tidak ada penurunan ke `http`, tidak ada tujuan IP atau port lain). Tujuan yang tidak boleh diketuk dicatat `skipped`; redirect tanpa `Location` atau berantai terlalu panjang dihitung gagal.
- **Penilaian status:** 2xx sehat; 429 tidak meyakinkan (tidak dihitung); status lain, kegagalan jaringan, dan waktu habis dihitung gagal. Isi badan tidak dilihat, jadi respons `200` berisi halaman HTML tidak terdeteksi.
- **Satu kegagalan tidak mematikan stasiun.** Dibutuhkan 3 kegagalan berturut-turut **dan** sedikitnya 6 jam sejak kegagalan pertama (`FAIL_THRESHOLD`, `MIN_FAIL_SPAN_SECS`). Satu hasil sehat menghapus hitungan.
- **Pengaman gangguan di sisi Worker.** Kalau dalam satu putaran semua stasiun yang dinilai gagal (minimal 3 yang dinilai), putaran itu dicatat `inconclusive` dan tidak dihitung: kemungkinan besar IP Cloudflare diblokir atau jaringan bermasalah. Pemblokiran yang hanya menimpa sebagian stasiun tidak terdeteksi (stasiun itu akan dimatikan setelah ambangnya terlampaui; aktifkan lagi dari tab Data).
- **Tautan diganti = riwayat mulai dari nol** (`checked_url` dibandingkan dengan `stream_url`). Pemeriksa mematikan stasiun hanya jika `stream_url` masih sama dengan yang diperiksa. Stasiun yang diaktifkan lagi admin setelah dimatikan pemeriksa (`disabled_at` terisi) juga mulai dari nol, supaya satu kegagalan berikutnya tidak langsung mematikannya lagi. **Tidak ada pengaktifan otomatis**, dan stasiun nonaktif tidak diperiksa lagi.
- **Giliran:** tiap putaran memeriksa maksimal 12 stasiun aktif, yang belum pernah atau paling lama belum dicek lebih dulu. Hasil ditulis dalam satu transaksi (`radio_checks` di-upsert, stasiun yang pantas dimatikan di-`UPDATE`, baris yatim dibersihkan).
- **Batas plan gratis** ([dokumentasi Cloudflare](https://developers.cloudflare.com/workers/platform/limits)): 50 subrequest per eksekusi, *termasuk* panggilan ke Turso dan tiap lompatan redirect. Anggaran terburuk: 1 baca + 12 x (1 + 2 redirect) + 1 tulis = 38; dijaga oleh `const` assertion di `radio_check.rs`. CPU 10 ms (menunggu jaringan tidak dihitung). Maksimal 5 cron per akun, dan ekspresi yang berbeda dihitung sendiri-sendiri: cek jumlah cron Worker lain di akunmu sebelum deploy (repo ini sekarang memakai 2).
- **Log tanpa URL** (URL memuat token): hanya hitungan dan id, mis. `radio: checked=5 ok=3 failed=0 skipped=2 inconclusive=0 disabled=[]`. Hasilnya terlihat di tab Radio (D7b), atau lewat `npx wrangler tail` (baris di atas) atau `turso db shell tquilla "SELECT station_id, last_result, last_status, consecutive_failures, last_checked_at FROM radio_checks ORDER BY station_id"` (sengaja tanpa kolom URL).

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
# dashboard admin: http://127.0.0.1:8787/admin/  (ADMIN_TOKEN dari .dev.vars)
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
node --test scripts/admin-ui.test.mjs       # dashboard admin (Node >= 22.7)
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
- **Langkah:** `migrate status` → `migrate up --yes` → `migrate seed --yes` (tanpa `--samples`) → `migrate status` (harus "up to date"). Aman diulang: `up` hanya menerapkan migrasi baru, seed hanya berlaku sekali (penanda `_seeded`).
- **Secret yang dibutuhkan** (Settings > Secrets and variables > Actions): `TURSO_DATABASE_URL` dan `TURSO_AUTH_TOKEN`. Nilainya harus **persis sama** dengan secret Worker; kalau beda, Worker membaca database lain daripada yang dimigrasi dan semua endpoint konten menjawab 500 `no such table`.
- Job memakai `environment: production`; di Settings > Environments kamu bisa menambah "Required reviewers" untuk persetujuan manual. Log Actions repo publik itu publik, jadi nama host Turso disamarkan dengan `::add-mask::`.
- **Aturan migrasi:** job ini jalan *sebelum* Worker baru di-deploy, jadi Worker lama masih berjalan di skema baru. Migrasi harus kompatibel mundur (tambah tabel/kolom, jangan hapus atau ganti nama dulu; hapus di rilis berikutnya).
- Seed konten berlaku **sekali** (penanda `_seeded`, lihat bagian Dashboard admin): mengubah `seeds/source/*.json` **tidak** memperbarui database yang sudah terisi. Mengubah konten yang sudah tayang = lewat dashboard (tab Data), atau migrasi baru bila bentuknya berubah. Aman diulang tiap tag: seed kedua tidak melakukan apa-apa.
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

Sudah (dijalankan di sandbox dengan rustc 1.97.0): `cargo fmt`, `cargo clippy --workspace --all-targets --locked -D warnings` di target host, 302 test Rust (kontrak dan validator `api-types`, klien dan runner migrasi `turso-http`, CLI `migrate`, serta router/CORS/health/konten/guestbook/moderasi `api` lewat `tower::oneshot` dan transport palsu) dan 172 test node (122 untuk SQL dan 50 untuk dashboard; termasuk semua query `.sql`, termasuk `before`, entri tersembunyi, laporan, dan migrasi `0008` di atas data yang sudah ada, terhadap SQLite sungguhan). Migrasi dan seed diterapkan di SQLite 3.45 sungguhan (Python `sqlite3` dan `node:sqlite`): skema valid, seed idempotent, CHECK menolak URL `javascript:`, avatar base64, JSON rusak, dsb. CLI `migrate` dijalankan end-to-end lewat server Hrana tiruan (skrip sementara di atas SQLite, bukan `sqld`): `up`, `seed`, jalan ulang, checksum berubah ditolak, migrasi gagal ter-rollback. Request klien Turso dicocokkan dengan keluaran encoder referensi resmi `@libsql/hrana-client` 0.10.0, dan fixture respons diterima decoder referensinya.

**Belum:**
- Kompilasi ke `wasm32-unknown-unknown` dan `worker-build` (sandbox tidak punya target itu; job `wasm` di CI yang pertama kali membuktikannya).
- `wrangler dev` / `wrangler deploy`, serta tempat `worker-build` mencari crate di dalam workspace (jika gagal, pindahkan `wrangler.toml` ke root dan sesuaikan).
- Migrasi dan seed sudah dicoba Moch di `turso dev` sungguhan (2026-10-16: `up`, ulangan, seed idempotent). **Belum** dicoba terhadap Turso cloud, dan kelima endpoint konten (D3) belum dijalankan terhadap `sqld`/Turso: querynya baru diuji di SQLite biasa. Server tiruan di sandbox hanya menafsirkan protokol seperti klien referensi.
- Batas plan gratis Cloudflare (ukuran wasm, waktu CPU per request): cek angkanya di dokumentasi Cloudflare terhadap ukuran hasil build.
- Guestbook dan moderasi (D4) belum dijalankan terhadap `sqld`/Turso cloud dan Worker sungguhan: logikanya diuji dengan transport palsu dan, end-to-end, lewat server Hrana tiruan di atas SQLite (5 pesan diterima, ke-6 ditolak `429`, pengirim lain tidak terkena, menit berikutnya longgar lagi; ini hasil D4a, belum diulang untuk laporan/admin). Endpoint laporan dan admin baru diuji lewat `tower::oneshot` dan SQLite, belum dengan `ADMIN_TOKEN` sungguhan di Worker. Header `CF-Connecting-IP` hanya ada di Cloudflare; lokal semua pengirim dihitung satu (`local`).
- Jurnal (D5): logika (query, transformasi, simpan, baca, refresh, batas refresh manual) diuji dengan fixture GraphQL buatan sendiri dan transport palsu, dan SQL-nya di SQLite sungguhan (`scripts/migrations.test.mjs`). **Belum**: panggilan nyata ke `api.github.com` (tidak ada token di sandbox, jadi bentuk respons GitHub yang sebenarnya belum dicocokkan), cron yang berjalan di Cloudflare, dan waktu CPU per eksekusi (cek di dashboard Cloudflare setelah cron pertama; kalau melewati 10 ms, eksekusi gagal dan snapshot lama dipakai terus).
- Dashboard admin (D6a, D6b, D6c): backend (gerbang token, pembatas, riwayat, ringkasan) diuji lewat `tower::oneshot`, transport palsu, dan SQLite sungguhan; UI diuji logikanya, klien API (fetch palsu), dan aturan statis (CSP, tanpa `innerHTML`/`eval`, hash pustaka). Satu kali di sandbox, halaman juga dijalankan di jsdom dengan server palsu berkeadaan (D6a: masuk, token salah, terkunci, Ringkasan, Riwayat + halaman berikutnya, kunci manual dan otomatis; D6b: refresh dengan hitung mundur, ganti akun, tambah repo salah/kembar/penuh, geser urutan, ubah di tempat, hapus dengan konfirmasi, baris yang sudah hilang di server → 404 lalu daftar dimuat ulang, stack, moderasi Guestbook: sembunyikan, tampilkan lagi, tolak laporan, `<img onerror>` tampil sebagai teks, 401 mengunci dashboard); D6c: pilih tabel, tambah dengan isian salah lalu benar (tanpa permintaan saat salah), tabel penuh, ubah, hapus dengan konfirmasi, baris yang sudah hilang, kotak centang dan kolom opsional, kunci yang dipilih admin dan nilai kembar → pesan, editor isi preset, ekspor (unduhan `Blob` dipalsukan), tabel hanya-baca dengan halaman berikutnya, hapus permanen dengan ketik konfirmasi); skrip itu tidak ada di repo (butuh jsdom). **Belum**: browser sungguhan (termasuk apakah unduhan `Blob` jalan di bawah CSP `default-src 'none'`); ekspor tabel besar terhadap batas CPU Worker 10 ms; `wrangler dev`/`deploy` dengan `[assets]` untuk Worker Rust (dokumentasi Cloudflare hanya memberi contoh JavaScript dan Python); apakah pola `/*` di `web/_headers` benar-benar memasang CSP pada `/admin/` (cek header respons di DevTools setelah deploy); `/admin` tanpa garis miring dialihkan ke `/admin/`; serta pembatas dengan `CF-Connecting-IP` dan `ADMIN_TOKEN` sungguhan.
- Pemeriksa radio (D7a): logika (URL yang boleh diperiksa, redirect, penilaian status, ambang dan jeda, pengaman "semua gagal", tautan diganti, diaktifkan lagi) diuji dengan probe dan transport palsu (29 test), dan SQL-nya (CHECK, upsert, urutan giliran, `UPDATE` bersyarat URL, cascade hapus stasiun) di SQLite sungguhan (`scripts/migrations.test.mjs`). **Belum**: `radio_probe.rs` (satu-satunya yang menyentuh `fetch`) tidak bisa jalan di sandbox; apakah `redirect: manual` mengembalikan `Location`, apakah `AbortController` benar-benar memutus stream tanpa mengunduhnya, dan apakah koneksi yang di-abort tetap dihitung terhadap batas enam koneksi serentak, baru terbukti setelah cron pertama jalan; apakah stasiun nyata (Revma, Radiojar) menjawab `GET` tanpa `Range` dari IP Cloudflare dengan 2xx atau malah memblokirnya (lihat pengaman di atas); angka waktu CPU per eksekusi; dan apakah port non-standar masih diabaikan `fetch` di produksi.
- Panel Radio (D7b): endpoint (gerbang token, pembatas "Cek sekarang", status tanpa URL) diuji lewat `tower::oneshot`, transport palsu, dan SQLite sungguhan; logika tampilan (kalimat tiap keadaan, hitung mundur, pemeriksaan bentuk respons) dan klien API lewat `scripts/admin-ui.test.mjs`. Satu kali di sandbox, komponen `Radio` juga dirender di jsdom dengan API palsu (muat, tombol Cek sekarang, daftar dimuat ulang, label berisi `<b>` tampil sebagai teks); skrip itu tidak ada di repo. **Belum**: browser sungguhan, dan "Cek sekarang" terhadap Worker sungguhan (lama permintaan dengan stasiun yang lambat: terburuk sekitar 48 detik, 2 kelompok x 3 lompatan x batas tunggu 8 detik; hanya terjadi bila banyak stasiun macet bersamaan).
- Dev shell Nix (`flake.nix`, `devenv.nix`): hanya sintaksnya yang diperiksa.

## Lisensi

MIT, lihat `LICENSE`.
