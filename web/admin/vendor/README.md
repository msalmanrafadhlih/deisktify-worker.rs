# Pustaka bawaan (vendored)

`standalone.module.js` adalah **`htm@3.1.1`**, berkas `preact/standalone.module.js`
(htm + Preact 10 + hooks dalam satu modul, 13 KB). Disalin apa adanya dari paket npm
`htm@3.1.1`, tanpa diubah dan tanpa build step.

- SHA-256: `72284e8e9079c87817145df1110f74e8a2aa040b2fc384922e18dfcb46fc1fd7`
  (dicek oleh `scripts/admin-ui.test.mjs`; kalau berkasnya berubah, tes gagal).
- Lisensi: htm Apache-2.0 (`LICENSE.htm`), Preact MIT (`LICENSE.preact`).
- Kenapa disalin ke repo dan bukan dimuat dari CDN: halaman ini memegang token admin
  di memori, jadi tidak ada skrip dari host lain yang boleh berjalan di dalamnya
  (CSP `script-src 'self'`, lihat `../../_headers`).

Memperbarui: unduh paket baru (`npm pack htm@<versi>`), salin berkas yang sama, perbarui
versi dan SHA-256 di sini dan di tes, lalu coba dashboard secara manual.
