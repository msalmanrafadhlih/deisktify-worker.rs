// Menjalankan migrasi dan seed di SQLite sungguhan (node:sqlite, dalam memori)
// untuk memastikan skema valid, seed idempotent, dan CHECK menolak input buruk.
// libSQL adalah turunan SQLite, jadi dialeknya sama.
//
//   node --test scripts/migrations.test.mjs        (Node >= 22.13)

import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

const sqlFiles = (dir) =>
  readdirSync(join(root, dir))
    .filter((f) => f.endsWith(".sql"))
    .sort()
    .map((f) => ({ name: f, sql: readFileSync(join(root, dir, f), "utf8") }));

function freshDb() {
  const db = new DatabaseSync(":memory:");
  db.exec("PRAGMA foreign_keys = ON");
  for (const m of sqlFiles("migrations")) db.exec(m.sql);
  return db;
}

const count = (db, table) => db.prepare(`SELECT count(*) AS n FROM ${table}`).get().n;

test("migration file names follow NNNN_description and are unique", () => {
  const names = sqlFiles("migrations").map((m) => m.name);
  assert.ok(names.length >= 6);
  for (const n of names) assert.match(n, /^\d{4}_[a-z0-9_]+\.sql$/, n);
  const numbers = names.map((n) => n.slice(0, 4));
  assert.equal(new Set(numbers).size, numbers.length, "duplicate migration number");
  assert.deepEqual(numbers, [...numbers].sort());
});

test("migrations contain no transaction control (the runner wraps them)", () => {
  for (const m of sqlFiles("migrations")) {
    const code = m.sql.replace(/--.*$/gm, "");
    assert.doesNotMatch(code, /^\s*(BEGIN|COMMIT|ROLLBACK|SAVEPOINT|RELEASE)\b/im, m.name);
  }
});

test("all migrations apply to an empty database", () => {
  const db = freshDb();
  const tables = db
    .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
    .all()
    .map((r) => r.name);
  assert.deepEqual(tables, [
    "bookmarks", "chat_messages", "embience_preset_items", "embience_presets",
    "embiences", "generations", "journal_snapshots", "radio_stations",
  ]);
});

test("seeds load, are idempotent, and keep references intact", () => {
  const db = freshDb();
  for (let round = 0; round < 2; round++) {
    for (const s of sqlFiles("seeds")) db.exec(s.sql);
    assert.equal(count(db, "generations"), 5, `round ${round}`);
    assert.equal(count(db, "bookmarks"), 3);
    assert.equal(count(db, "radio_stations"), 5);
    assert.equal(count(db, "embiences"), 12);
    assert.equal(count(db, "embience_presets"), 3);
    assert.equal(count(db, "embience_preset_items"), 7);
    assert.equal(count(db, "chat_messages"), 6);
    assert.equal(count(db, "journal_snapshots"), 1);
  }
  assert.deepEqual(db.prepare("PRAGMA foreign_key_check").all(), []);
  const seeded = db.prepare("SELECT source, payload FROM journal_snapshots").get();
  assert.equal(seeded.source, "seed");
  assert.ok(JSON.parse(seeded.payload).hero);
  const busy = db
    .prepare(
      `SELECT group_concat(name, '|') AS names FROM (
         SELECT e.name AS name FROM embience_preset_items i
         JOIN embience_presets p ON p.id = i.preset_id JOIN embiences e ON e.id = i.embience_id
         WHERE p.label = 'Busy' ORDER BY i.position)`,
    )
    .get();
  assert.equal(busy.names, "City|Train|Coffee Shop");
});

test("content seed is separate from the fake samples", () => {
  const files = sqlFiles("seeds").map((s) => s.name);
  assert.deepEqual(files, ["0001_content.sql", "0002_samples.sql"]);
  const content = readFileSync(join(root, "seeds/0001_content.sql"), "utf8");
  assert.doesNotMatch(content, /chat_messages|journal_snapshots/);
});

const rejected = [
  ["javascript: bookmark URL", "INSERT INTO bookmarks (label, url) VALUES ('x', 'javascript:alert(1)')"],
  ["protocol-relative route", "INSERT INTO generations VALUES (9, 'x', '//evil.example', '1', '2026-01-01')"],
  ["backslash route", "INSERT INTO generations VALUES (9, 'x', '/\\evil.example', '1', '2026-01-01')"],
  ["URL with a space", "INSERT INTO bookmarks (label, url) VALUES ('x', 'https://a.b/c d')"],
  ["radio is_active = 2", "INSERT INTO radio_stations (label, stream_url, is_active) VALUES ('z', 'https://a.b', 2)"],
  ["empty username", "INSERT INTO chat_messages (username, message, created_at) VALUES ('', 'm', '2026-01-01T00:00:00Z')"],
  ["base64 avatar", "INSERT INTO chat_messages (username, message, created_at, avatar_url) VALUES ('u', 'm', '2026-01-01T00:00:00Z', 'data:image/png;base64,AAAA')"],
  ["http attachment", "INSERT INTO chat_messages (username, message, created_at, attachment_url) VALUES ('u', 'm', '2026-01-01T00:00:00Z', 'http://a.b/x.png')"],
  ["malformed timestamp", "INSERT INTO chat_messages (username, message, created_at) VALUES ('u', 'm', 'yesterday')"],
  ["invalid JSON snapshot", "INSERT INTO journal_snapshots (payload, fetched_at) VALUES ('{nope', '2026-01-01T00:00:00Z')"],
  ["unknown snapshot source", "INSERT INTO journal_snapshots (payload, source, fetched_at) VALUES ('{}', 'x', '2026-01-01T00:00:00Z')"],
  ["preset item with unknown ids", "INSERT INTO embience_preset_items (preset_id, embience_id) VALUES (99, 99)"],
];

for (const [label, sql] of rejected) {
  test(`rejects: ${label}`, () => {
    const db = freshDb();
    assert.throws(() => db.exec(sql), /constraint/i);
  });
}

test("accepts a normal chat message and an internal route", () => {
  const db = freshDb();
  db.exec("INSERT INTO chat_messages (username, message, created_at, avatar_url) VALUES ('u', 'hi', '2026-01-01T00:00:00Z', 'https://cdn.example/a.png')");
  db.exec("INSERT INTO generations VALUES (9, 'x', '/deisktify', '1', '2026-01-01')");
  assert.equal(count(db, "chat_messages"), 1);
});

// ---- query endpoint API (crates/api/src/queries/*.sql) di database yang sudah di-seed

const query = (db, name) =>
  db.prepare(readFileSync(join(root, "crates/api/src/queries", `${name}.sql`), "utf8")).all();

function seededDb() {
  const db = freshDb();
  for (const s of sqlFiles("seeds")) db.exec(s.sql);
  return db;
}

test("API queries: generations come newest first", () => {
  const rows = query(seededDb(), "generations");
  assert.deepEqual(rows.map((r) => r.number), [5, 4, 3, 2, 1]);
  assert.equal(rows[0].link, "/deisktify");
  assert.deepEqual(Object.keys(rows[0]), ["number", "label", "link", "kernel", "built_on"]);
});

test("API queries: bookmarks and embiences follow sort_order", () => {
  const db = seededDb();
  assert.deepEqual(query(db, "bookmarks").map((r) => r.label), ["Github", "windah basudara", "Motion IME"]);
  const names = query(db, "embiences").map((r) => r.name);
  assert.equal(names.length, 12);
  assert.equal(names[0], "Rain");
  assert.equal(names[11], "Coffee Shop");
});

test("API queries: radio hides inactive stations", () => {
  const db = seededDb();
  assert.equal(query(db, "radio").length, 5);
  db.exec("UPDATE radio_stations SET is_active = 0 WHERE label = 'Jak FM'");
  const labels = query(db, "radio").map((r) => r.label);
  assert.equal(labels.length, 4);
  assert.ok(!labels.includes("Jak FM"));
  assert.deepEqual(Object.keys(query(db, "radio")[0]), [
    "id", "label", "slogan", "stream_url", "official_link", "image_url",
  ]);
});

test("API queries: presets keep item order, empty presets, and drop dangling items", () => {
  const db = seededDb();
  const group = (rows) => {
    const out = [];
    for (const r of rows) {
      if (out.at(-1)?.id !== r.preset_id) out.push({ id: r.preset_id, label: r.label, ids: [] });
      if (r.embience_id !== null) out.at(-1).ids.push(r.embience_id);
    }
    return out;
  };
  const idOf = (name) => db.prepare("SELECT id FROM embiences WHERE name = ?").get(name).id;

  let presets = group(query(db, "presets"));
  assert.deepEqual(presets.map((p) => p.label), ["Learning", "Chill", "Busy"]);
  assert.deepEqual(presets[2].ids, [idOf("City"), idOf("Train"), idOf("Coffee Shop")]);

  // preset tanpa isi tetap muncul
  db.exec("INSERT INTO embience_presets (label, sort_order) VALUES ('Empty', 9)");
  presets = group(query(db, "presets"));
  assert.deepEqual(presets.at(-1), { id: presets.at(-1).id, label: "Empty", ids: [] });

  // item yang menunjuk embience yang sudah hilang tidak ikut (FK sengaja dimatikan)
  db.exec("PRAGMA foreign_keys = OFF");
  db.exec("DELETE FROM embiences WHERE name = 'Train'");
  presets = group(query(db, "presets"));
  assert.ok(!presets[2].ids.includes(6));
  assert.equal(presets[2].ids.length, 2);
});

test("API queries are bounded", () => {
  for (const name of ["generations", "bookmarks", "radio", "embiences", "presets"]) {
    const sql = readFileSync(join(root, "crates/api/src/queries", `${name}.sql`), "utf8");
    assert.match(sql, /\bLIMIT\s+\d+\s*$/im, name);
    assert.match(sql, /\bORDER BY\b/i, name);
  }
});
