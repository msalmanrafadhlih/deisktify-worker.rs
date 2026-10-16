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
