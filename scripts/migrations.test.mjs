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
    "bookmarks", "chat_messages", "chat_rate_limits", "embience_preset_items", "embience_presets",
    "embiences", "generations", "journal_snapshots", "message_reports", "radio_stations",
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

const sqlOf = (name) => readFileSync(join(root, "crates/api/src/queries", `${name}.sql`), "utf8");
const insertMsg = (db, username, message, at = "2026-10-17T00:00:00Z", url = null) =>
  db.prepare(sqlOf("messages_insert")).get(username, url, message, at);

test("chat queries: insert returns the stored row; created_at must be a UTC timestamp", () => {
  const db = freshDb();
  const row = insertMsg(db, "moch", "hello", "2026-10-17T05:30:00Z", "https://a.dev");
  assert.deepEqual({ ...row }, {
    id: 1, username: "moch", url: "https://a.dev",
    message: "hello", created_at: "2026-10-17T05:30:00Z",
  });
  assert.equal(insertMsg(db, "x", "y").id, 2);
  assert.throws(() => insertMsg(db, "x", "y", "17-10-2026"), /CHECK/);
  assert.throws(() => insertMsg(db, "x", "y", "2026-10-17T00:00:00Z", "javascript:alert(1)"), /CHECK/);
});

test("chat queries: recent returns the last N oldest-first; after returns only newer", () => {
  const db = freshDb();
  for (let i = 1; i <= 7; i++) insertMsg(db, "u", `m${i}`);
  const recent = db.prepare(sqlOf("messages_recent")).all(3);
  assert.deepEqual(recent.map((r) => r.message), ["m5", "m6", "m7"]);
  const after = db.prepare(sqlOf("messages_after")).all(4, 100);
  assert.deepEqual(after.map((r) => r.id), [5, 6, 7]);
  assert.deepEqual(db.prepare(sqlOf("messages_after")).all(4, 2).map((r) => r.id), [5, 6]);
  assert.deepEqual(db.prepare(sqlOf("messages_after")).all(7, 100), []);
  assert.deepEqual(db.prepare(sqlOf("messages_recent")).all(100).length, 7);
});

test("chat queries: before returns the page just older than the cursor, oldest-first", () => {
  const db = freshDb();
  for (let i = 1; i <= 9; i++) insertMsg(db, "u", `m${i}`);
  const before = (id, n) => db.prepare(sqlOf("messages_before")).all(id, n).map((r) => r.id);
  assert.deepEqual(before(10, 3), [7, 8, 9]);
  assert.deepEqual(before(7, 3), [4, 5, 6]);
  assert.deepEqual(before(4, 3), [1, 2, 3]);
  assert.deepEqual(before(1, 3), [], "nothing older than the first entry");
  assert.deepEqual(before(3, 10), [1, 2]);
});

test("moderation: hidden entries disappear from every public read but stay in the table", () => {
  const db = freshDb();
  for (let i = 1; i <= 6; i++) insertMsg(db, "u", `m${i}`);
  const hide = (id, at = "2026-10-18T00:00:00Z") => db.prepare(sqlOf("message_delete")).get(id, at);
  assert.deepEqual({ ...hide(3) }, { id: 3 });
  assert.equal(hide(3), undefined, "hiding twice finds nothing to hide");
  assert.equal(hide(99), undefined, "unknown id");
  assert.deepEqual(db.prepare(sqlOf("messages_recent")).all(10).map((r) => r.id), [1, 2, 4, 5, 6]);
  assert.deepEqual(db.prepare(sqlOf("messages_after")).all(1, 10).map((r) => r.id), [2, 4, 5, 6]);
  assert.deepEqual(db.prepare(sqlOf("messages_before")).all(6, 10).map((r) => r.id), [1, 2, 4, 5]);
  // "recent" fills its page from the visible entries, not the raw id range.
  assert.deepEqual(db.prepare(sqlOf("messages_recent")).all(3).map((r) => r.id), [4, 5, 6]);
  assert.equal(count(db, "chat_messages"), 6);
  assert.throws(() => hide(1, "yesterday"), /CHECK/);
});

test("moderation: one report per reporter per entry; hidden or unknown entries take none", () => {
  const db = freshDb();
  for (let i = 1; i <= 3; i++) insertMsg(db, "u", `m${i}`);
  const report = (id, who, at) => db.prepare(sqlOf("report_insert")).get(id, who, at);
  assert.deepEqual({ ...report(2, "a", "2026-10-18T01:00:00Z") }, { message_id: 2 });
  assert.equal(report(2, "a", "2026-10-18T02:00:00Z"), undefined, "same reporter twice");
  assert.ok(report(2, "b", "2026-10-18T03:00:00Z"));
  assert.ok(report(3, "a", "2026-10-18T04:00:00Z"));
  assert.equal(report(99, "a", "2026-10-18T05:00:00Z"), undefined, "no such entry");
  db.prepare(sqlOf("message_delete")).get(1, "2026-10-18T00:00:00Z");
  assert.equal(report(1, "c", "2026-10-18T06:00:00Z"), undefined, "hidden entry");
  assert.equal(count(db, "message_reports"), 3);
  // INSERT OR IGNORE juga melewati baris yang melanggar CHECK: tidak error, tidak tersimpan.
  assert.equal(report(2, "", "2026-10-18T07:00:00Z"), undefined);
  assert.equal(report(2, "x", "later"), undefined);
  assert.equal(count(db, "message_reports"), 3);

  const list = db.prepare(sqlOf("reports_list")).all(50).map((r) => ({ ...r }));
  assert.deepEqual(list.map((r) => [r.id, r.reports]), [[2, 2], [3, 1]]);
  assert.deepEqual(Object.keys(list[0]).sort(), [
    "created_at", "id", "last_reported_at", "message", "reports", "url", "username",
  ]);
  assert.equal(list[0].last_reported_at, "2026-10-18T03:00:00Z");
  assert.equal(db.prepare(sqlOf("reports_list")).all(1).length, 1);
  // Entri yang disembunyikan keluar dari antrean laporan.
  db.prepare(sqlOf("message_delete")).get(2, "2026-10-18T08:00:00Z");
  assert.deepEqual(db.prepare(sqlOf("reports_list")).all(50).map((r) => r.id), [3]);
});

test("moderation: the report counter only counts that reporter inside the window", () => {
  const db = freshDb();
  for (let i = 1; i <= 4; i++) insertMsg(db, "u", `m${i}`);
  const report = (id, who, at) => db.prepare(sqlOf("report_insert")).get(id, who, at);
  report(1, "a", "2026-10-18T00:30:00Z");
  report(2, "a", "2026-10-18T01:10:00Z");
  report(3, "a", "2026-10-18T01:20:00Z");
  report(1, "b", "2026-10-18T01:30:00Z");
  const n = (who, since) => db.prepare(sqlOf("report_count")).get(who, since).n;
  assert.equal(n("a", "2026-10-18T01:00:00Z"), 2);
  assert.equal(n("a", "2026-10-18T00:00:00Z"), 3);
  assert.equal(n("a", "2026-10-18T02:00:00Z"), 0);
  assert.equal(n("b", "2026-10-18T00:00:00Z"), 1);
  assert.equal(n("nobody", "2026-10-18T00:00:00Z"), 0);
});

test("migration 0008 applies on top of a database that already holds entries", () => {
  const db = new DatabaseSync(":memory:");
  const all = sqlFiles("migrations");
  for (const m of all.filter((m) => m.name < "0008")) db.exec(m.sql);
  db.exec("INSERT INTO chat_messages (username, message, created_at) VALUES ('u', 'old', '2026-01-01T00:00:00Z')");
  // Worker lama (tanpa deleted_at) tetap jalan setelah migrasi ini.
  db.exec(all.find((m) => m.name.startsWith("0008")).sql);
  assert.equal(db.prepare("SELECT message FROM chat_messages WHERE id = 1").get().message, "old");
  assert.equal(db.prepare("SELECT deleted_at FROM chat_messages WHERE id = 1").get().deleted_at, null);
  db.exec("INSERT INTO chat_messages (username, message, created_at) VALUES ('u', 'from the old worker', '2026-01-02T00:00:00Z')");
  assert.equal(count(db, "chat_messages"), 2);
});

test("rate limit queries: hits count up per key and window; cleanup only removes old windows", () => {
  const db = freshDb();
  const hit = (scope, key, window) => db.prepare(sqlOf("rate_hit")).get(scope, key, window).hits;
  assert.equal(hit("m", "ip1", 100), 1);
  assert.equal(hit("m", "ip1", 100), 2);
  assert.equal(hit("m", "ip1", 100), 3);
  assert.equal(hit("m", "ip2", 100), 1, "other key");
  assert.equal(hit("m", "ip1", 101), 1, "next window");
  assert.equal(hit("h", "ip1", 100), 1, "other scope");
  assert.throws(() => hit("x", "ip1", 1), /CHECK/);
  db.prepare(sqlOf("rate_cleanup")).run(101, 100);
  const left = db
    .prepare("SELECT scope, key, window FROM chat_rate_limits ORDER BY scope, key, window")
    .all()
    .map((r) => ({ ...r }));
  assert.deepEqual(left, [
    { scope: "h", key: "ip1", window: 100 },
    { scope: "m", key: "ip1", window: 101 },
  ]);
});

test("API queries are bounded", () => {
  for (const name of ["generations", "bookmarks", "radio", "embiences", "presets", "messages_after", "reports_list"]) {
    const sql = readFileSync(join(root, "crates/api/src/queries", `${name}.sql`), "utf8");
    assert.match(sql, /\bLIMIT\s+(\d+|\?\d+)\s*$/im, name);
    assert.match(sql, /\bORDER BY\b/i, name);
  }
});
