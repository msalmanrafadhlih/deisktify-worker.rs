#!/usr/bin/env node
// Menghasilkan SQL seed dari berkas JSON dummy. Hasilnya di-commit di `seeds/`.
//
//   node scripts/gen_seed.mjs [folder-sumber=seeds/source] [folder-keluaran=seeds]
//
// Sumbernya adalah salinan berkas JSON dummy dari repo frontend (`packages/components/data/`
// pada saat D2), disimpan di `seeds/source/` supaya seed bisa dibangkitkan ulang walau
// berkas aslinya sudah dihapus dari frontend (D3).
//
// Keluaran:
//   0001_content.sql  generations, bookmarks, radio, embience + preset
//                     (konten situs; aman untuk produksi)
//   0002_samples.sql  contoh chat dan contoh snapshot jurnal
//                     (hanya untuk dev/demo; dijalankan dengan `--samples`)
//
// Semua INSERT memakai `INSERT OR IGNORE` dengan id eksplisit. Seed konten juga hanya
// berlaku sekali (penanda `_seeded`), supaya baris yang dihapus lewat dashboard tidak
// dikembalikan; contoh (0002) boleh diulang karena hanya untuk lokal.
// Berkas dibangkitkan; jangan diedit tangan, jalankan ulang skrip ini.

import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { join, resolve } from "node:path";

const [dataDirArg = "seeds/source", outDirArg = "seeds"] = process.argv.slice(2);
const dataDir = resolve(dataDirArg);
const outDir = resolve(outDirArg);

const load = (name) => JSON.parse(readFileSync(join(dataDir, name), "utf8"));

function fail(message) {
  console.error(`gen_seed: ${message}`);
  process.exit(1);
}

// ---------------------------------------------------------------- helper SQL

function str(value, what) {
  if (typeof value !== "string") fail(`${what}: expected a string, got ${JSON.stringify(value)}`);
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/.test(value)) fail(`${what}: control character`);
  return `'${value.replaceAll("'", "''")}'`;
}

function opt(value, what) {
  return value === null || value === undefined ? "NULL" : str(value, what);
}

function int(value, what) {
  if (!Number.isSafeInteger(value)) fail(`${what}: expected an integer, got ${JSON.stringify(value)}`);
  return String(value);
}

function httpUrl(value, what, { httpsOnly = false } = {}) {
  str(value, what);
  const ok = httpsOnly ? /^https:\/\//i.test(value) : /^https?:\/\//i.test(value);
  if (!ok || /\s/.test(value)) fail(`${what}: not a valid ${httpsOnly ? "https" : "http(s)"} URL: ${value}`);
  return value;
}

function insert(table, columns, rows) {
  if (rows.length === 0) return `-- (no rows for ${table})\n`;
  const values = rows.map((r) => `    (${r.join(", ")})`).join(",\n");
  return `INSERT OR IGNORE INTO ${table} (${columns.join(", ")}) VALUES\n${values};\n`;
}

// Seed konten hanya berlaku SEKALI. Sejak D6c konten situs dikelola lewat dashboard,
// dan seed dijalankan ulang tiap rilis; tanpa penanda, baris yang dihapus admin akan
// hidup lagi. Penanda `_seeded` ditulis di akhir transaksi yang sama dengan isinya.
const ONCE = "content";

function insertOnce(table, columns, rows) {
  if (rows.length === 0) return `-- (no rows for ${table})\n`;
  const values = rows.map((r) => `    (${r.join(", ")})`).join(",\n");
  const cols = columns.map((_, i) => `column${i + 1}`).join(", ");
  return (
    `INSERT OR IGNORE INTO ${table} (${columns.join(", ")})\n` +
    `SELECT ${cols} FROM (VALUES\n${values})\n` +
    `WHERE NOT EXISTS (SELECT 1 FROM _seeded WHERE name = '${ONCE}');\n`
  );
}

const HEADER = (title) =>
  `-- ${title}\n` +
  `-- DIBANGKITKAN oleh scripts/gen_seed.mjs dari berkas JSON frontend. Jangan diedit tangan.\n` +
  `-- Aman dijalankan berulang kali (INSERT OR IGNORE dengan id eksplisit).\n\n`;

function uniqueBy(items, key, what) {
  const seen = new Set();
  for (const item of items) {
    const k = key(item);
    if (seen.has(k)) fail(`${what}: duplicate ${JSON.stringify(k)}`);
    seen.add(k);
  }
}

// ------------------------------------------------------------------- konten

const generations = load("generations.json");
uniqueBy(generations, (g) => g.number, "generations.number");
const generationRows = generations.map((g, i) => {
  const w = `generations[${i}]`;
  const link = str(g.link, `${w}.link`) && g.link;
  if (!(link.startsWith("/") || /^https?:\/\//i.test(link)) || /\s/.test(link)) {
    fail(`${w}.link: expected a route or http(s) URL: ${link}`);
  }
  if (!/^\d{4}-\d{2}-\d{2}$/.test(g.date)) fail(`${w}.date: expected YYYY-MM-DD: ${g.date}`);
  return [int(g.number, `${w}.number`), str(g.label, `${w}.label`), str(link, `${w}.link`),
    str(g.kernel, `${w}.kernel`), str(g.date, `${w}.date`)];
}).sort((a, b) => Number(a[0]) - Number(b[0]));

const bookmarks = load("browser.json");
const bookmarkRows = bookmarks.map((b, i) => [
  int(i + 1, "id"), str(b.placeholder, `bookmarks[${i}].placeholder`),
  str(httpUrl(b.url, `bookmarks[${i}].url`), "url"), int(i, "sort_order"),
]);

const radio = load("radio.json");
uniqueBy(radio, (r) => r.label, "radio.label");
const radioRows = radio.map((r, i) => {
  const w = `radio[${i}]`;
  return [
    int(i + 1, "id"), str(r.label, `${w}.label`), str(r.slogan ?? "", `${w}.slogan`),
    str(httpUrl(r.url_stream, `${w}.url_stream`), "stream_url"),
    r.official_link == null ? "NULL" : str(httpUrl(r.official_link, `${w}.official_link`), "official_link"),
    r.image == null ? "NULL" : str(httpUrl(r.image, `${w}.image`), "image_url"),
    int(i, "sort_order"), "1",
  ];
});

const embience = load("embience.json");
uniqueBy(embience.embiences, (e) => e.embiences_id, "embiences_id");
uniqueBy(embience.embiences, (e) => e.embiences_name, "embiences_name");
const idByName = new Map(embience.embiences.map((e) => [e.embiences_name, e.embiences_id]));
const embienceRows = embience.embiences.map((e, i) => {
  const w = `embiences[${i}]`;
  return [
    int(e.embiences_id, `${w}.embiences_id`), str(e.embiences_name, `${w}.embiences_name`),
    str(httpUrl(e.embiences_icon, `${w}.embiences_icon`), "icon_url"),
    str(httpUrl(e.embiences_stream, `${w}.embiences_stream`), "stream_url"), int(i, "sort_order"),
  ];
});

// `preset_id` di JSON sumber dobel (dua preset ber-id 2). Id dibuat dari urutan.
const jsonPresetIds = embience.presets.map((p) => p.preset_id);
if (new Set(jsonPresetIds).size !== jsonPresetIds.length) {
  console.warn(`gen_seed: warning: duplicate preset_id in embience.json (${jsonPresetIds}); ids assigned by position`);
}
uniqueBy(embience.presets, (p) => p.preset_label, "preset_label");
const presetRows = [];
const itemRows = [];
embience.presets.forEach((p, i) => {
  const presetId = i + 1;
  presetRows.push([int(presetId, "id"), str(p.preset_label, `presets[${i}].preset_label`), int(i, "sort_order")]);
  uniqueBy(p.embience_list, (n) => n, `preset ${p.preset_label}`);
  p.embience_list.forEach((name, position) => {
    if (!idByName.has(name)) fail(`preset ${JSON.stringify(p.preset_label)} refers to unknown embience ${JSON.stringify(name)}`);
    itemRows.push([int(presetId, "preset_id"), int(idByName.get(name), "embience_id"), int(position, "position")]);
  });
});

const content =
  HEADER("Seed konten situs: generations, bookmarks, radio, embience.").replace(
    "-- Aman dijalankan berulang kali (INSERT OR IGNORE dengan id eksplisit).",
    "-- HANYA BERLAKU SEKALI: setelah penanda `_seeded` ada, menjalankannya lagi tidak mengubah apa pun,\n-- jadi baris yang dihapus atau diubah lewat dashboard tidak dikembalikan.",
  ) +
  "CREATE TABLE IF NOT EXISTS _seeded (\n" +
  "    name TEXT PRIMARY KEY,\n" +
  "    at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))\n" +
  ");\n\n" +
  "-- generations\n" +
  insertOnce("generations", ["number", "label", "link", "kernel", "built_on"], generationRows) +
  "\n-- bookmarks\n" +
  insertOnce("bookmarks", ["id", "label", "url", "sort_order"], bookmarkRows) +
  "\n-- radio_stations\n" +
  insertOnce("radio_stations", ["id", "label", "slogan", "stream_url", "official_link", "image_url", "sort_order", "is_active"], radioRows) +
  "\n-- embiences\n" +
  insertOnce("embiences", ["id", "name", "icon_url", "stream_url", "sort_order"], embienceRows) +
  "\n-- embience_presets\n" +
  insertOnce("embience_presets", ["id", "label", "sort_order"], presetRows) +
  "\n-- embience_preset_items\n" +
  insertOnce("embience_preset_items", ["preset_id", "embience_id", "position"], itemRows) +
  `\nINSERT OR IGNORE INTO _seeded (name) VALUES ('${ONCE}');\n`;

// ------------------------------------------------------------------ contoh

const chat = load("chat_sample.json");
uniqueBy(chat, (m) => m.message_id, "chat.message_id");
const chatRows = chat.map((m, i) => {
  const w = `chat[${i}]`;
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/.test(m.timestamp)) fail(`${w}.timestamp: expected UTC YYYY-MM-DDTHH:MM:SSZ: ${m.timestamp}`);
  return [
    int(m.message_id, `${w}.message_id`), str(m.username, `${w}.username`),
    m.url == null ? "NULL" : str(httpUrl(m.url, `${w}.url`), "url"),
    m.avatar == null ? "NULL" : str(httpUrl(m.avatar, `${w}.avatar`, { httpsOnly: true }), "avatar_url"),
    str(m.message, `${w}.message`),
    m.attachment == null ? "NULL" : str(httpUrl(m.attachment, `${w}.attachment`, { httpsOnly: true }), "attachment_url"),
    str(m.timestamp, `${w}.timestamp`),
  ];
});

const journal = JSON.stringify(load("journal.sample.json"));
// Waktu tetap (bukan jam sekarang) supaya keluaran skrip deterministik; snapshot
// asli apa pun akan lebih baru dan menggantikannya.
const SEED_TIME = "1970-01-01T00:00:00Z";

const samples =
  HEADER("Data contoh (dev/demo): pesan chat dan snapshot jurnal. Jangan dijalankan di produksi.") +
  "-- chat_messages\n" +
  insert("chat_messages", ["id", "username", "url", "avatar_url", "message", "attachment_url", "created_at"], chatRows) +
  "\n-- journal_snapshots (source = 'seed')\n" +
  insert("journal_snapshots", ["id", "payload", "source", "fetched_at"],
    [["1", str(journal, "journal"), "'seed'", str(SEED_TIME, "fetched_at")]]);

mkdirSync(outDir, { recursive: true });
writeFileSync(join(outDir, "0001_content.sql"), content);
writeFileSync(join(outDir, "0002_samples.sql"), samples);
console.log(`gen_seed: wrote ${join(outDir, "0001_content.sql")} and ${join(outDir, "0002_samples.sql")}`);
