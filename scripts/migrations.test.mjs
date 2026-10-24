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
    "admin_attempts", "admin_audit", "bookmarks", "chat_messages", "chat_rate_limits", "embience_preset_items", "embience_presets",
    "embiences", "generations", "journal_pinned_repos", "journal_profile", "journal_snapshots",
    "journal_stack", "message_reports", "radio_checks", "radio_stations",
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

test("content seed applies once: deletions and edits made later are never undone", () => {
  const content = sqlFiles("seeds").find((f) => f.name === "0001_content.sql").sql;
  const db = freshDb();
  db.exec(content);
  assert.equal(count(db, "bookmarks"), 3);
  assert.equal(count(db, "_seeded"), 1);
  db.exec("DELETE FROM bookmarks WHERE id = 1; DELETE FROM radio_stations; DELETE FROM embience_preset_items; DELETE FROM embience_presets WHERE id = 2");
  db.exec("UPDATE generations SET label = 'Edited' WHERE number = 1");
  db.exec(content);
  assert.equal(count(db, "bookmarks"), 2, "deleted bookmark stays deleted");
  assert.equal(count(db, "radio_stations"), 0, "an emptied table stays empty");
  assert.equal(count(db, "embience_preset_items"), 0);
  assert.equal(count(db, "embience_presets"), 2);
  assert.equal(db.prepare("SELECT label FROM generations WHERE number = 1").get().label, "Edited");
  assert.equal(count(db, "_seeded"), 1);
});

test("content seed on a database that already has content (the production case) only adds the marker", () => {
  const content = sqlFiles("seeds").find((f) => f.name === "0001_content.sql").sql;
  const db = freshDb();
  db.exec("INSERT INTO bookmarks (id, label, url, sort_order) VALUES (1, 'Mine', 'https://mine.example', 0)");
  db.exec(content);
  assert.equal(db.prepare("SELECT label FROM bookmarks WHERE id = 1").get().label, "Mine", "INSERT OR IGNORE keeps existing rows");
  assert.equal(count(db, "bookmarks"), 3);
  assert.equal(count(db, "_seeded"), 1);
});

test("content seed is separate from the fake samples", () => {
  const files = sqlFiles("seeds").map((s) => s.name);
  assert.deepEqual(files, ["0001_content.sql", "0002_samples.sql"]);
  const content = readFileSync(join(root, "seeds/0001_content.sql"), "utf8");
  assert.doesNotMatch(content, /chat_messages|journal_snapshots/);
});

// Satu stasiun (id 1) lalu satu baris radio_checks dengan nilai yang bisa diganti.
function radioCheck({ id = 1, at = "'2026-01-01T00:00:00Z'", since = "NULL", result = "'http'", failures = 1, status = "NULL", url = "'https://a.b/s'" }, withStation = true) {
  const station = withStation ? "INSERT INTO radio_stations (id, label, stream_url) VALUES (1, 's', 'https://a.b/s'); " : "";
  return `${station}INSERT INTO radio_checks (station_id, checked_url, last_checked_at, failing_since, consecutive_failures, last_result, last_status) VALUES (${id}, ${url}, ${at}, ${since}, ${failures}, ${result}, ${status})`;
}

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
  ["second journal profile row", "INSERT INTO journal_profile (id, github_username) VALUES (2, 'other')"],
  ["journal username with a space", "UPDATE journal_profile SET github_username = 'a b'"],
  ["journal username with a quote", "UPDATE journal_profile SET github_username = 'a\"b'"],
  ["journal username too long", `UPDATE journal_profile SET github_username = '${"a".repeat(40)}'`],
  ["empty journal username", "UPDATE journal_profile SET github_username = ''"],
  ["pinned repo with a quote", "INSERT INTO journal_pinned_repos (repo_name) VALUES ('x\") { y }')"],
  ["pinned repo with a slash", "INSERT INTO journal_pinned_repos (repo_name) VALUES ('a/b')"],
  ["pinned repo with a space", "INSERT INTO journal_pinned_repos (repo_name) VALUES ('a b')"],
  ["empty pinned repo", "INSERT INTO journal_pinned_repos (repo_name) VALUES ('')"],
  ["pinned repo too long", `INSERT INTO journal_pinned_repos (repo_name) VALUES ('${"a".repeat(101)}')`],
  ["duplicate pinned repo", "INSERT INTO journal_pinned_repos (repo_name) VALUES ('flexinix')"],
  ["empty pinned topic", "INSERT INTO journal_pinned_repos (repo_name, topic) VALUES ('new-one', '')"],
  ["pinned topic too long", `INSERT INTO journal_pinned_repos (repo_name, topic) VALUES ('new-one', '${"t".repeat(61)}')`],
  ["empty stack name", "INSERT INTO journal_stack (name) VALUES ('')"],
  ["duplicate stack name", "INSERT INTO journal_stack (name) VALUES ('Rust')"],
  ["preset item with unknown ids", "INSERT INTO embience_preset_items (preset_id, embience_id) VALUES (99, 99)"],
  ["empty attempt key", "INSERT INTO admin_attempts (key, window) VALUES ('', 1)"],
  ["negative attempt window", "INSERT INTO admin_attempts (key, window) VALUES ('k', -1)"],
  ["zero attempt hits", "INSERT INTO admin_attempts (key, window, hits) VALUES ('k', 1, 0)"],
  ["audit with a malformed timestamp", "INSERT INTO admin_audit (at, action, who) VALUES ('now', 'login', 'w')"],
  ["audit with an empty action", "INSERT INTO admin_audit (at, action, who) VALUES ('2026-01-01T00:00:00Z', '', 'w')"],
  ["audit with an empty target", "INSERT INTO admin_audit (at, action, target, who) VALUES ('2026-01-01T00:00:00Z', 'a', '', 'w')"],
  ["audit with a too long target", `INSERT INTO admin_audit (at, action, target, who) VALUES ('2026-01-01T00:00:00Z', 'a', '${"t".repeat(121)}', 'w')`],
  ["audit without a who", "INSERT INTO admin_audit (at, action, who) VALUES ('2026-01-01T00:00:00Z', 'a', '')"],
  ["radio check for an unknown station", radioCheck({ id: 99 }, false)],
  ["radio check with a malformed timestamp", radioCheck({ at: "'yesterday'" })],
  ["radio check with a malformed failing_since", radioCheck({ since: "'6 hours ago'" })],
  ["radio check with an unknown result", radioCheck({ result: "'weird'" })],
  ["radio check with a negative streak", radioCheck({ failures: -1 })],
  ["radio check with a runaway streak", radioCheck({ failures: 1001 })],
  ["radio check with a status below 100", radioCheck({ status: 99 })],
  ["radio check with a status above 599", radioCheck({ status: 600 })],
  ["radio check with a javascript: URL", radioCheck({ url: "'javascript:alert(1)'" })],
  ["radio check with a data: URL", radioCheck({ url: "'data:audio/mp3;base64,AAAA'" })],
  ["radio check URL with a space", radioCheck({ url: "'https://a.b/c d'" })],
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

// ---- Journal (D5): pengaturan, snapshot, pemangkasan

test("journal settings start with the old Worker's account, repositories and stack", () => {
  const db = freshDb();
  assert.equal(db.prepare("SELECT github_username AS u FROM journal_profile").get().u, "msalmanrafadhlih");
  assert.deepEqual(
    query(db, "journal_pinned").map((r) => r.repo_name),
    ["flexinix", "racooonfig", "template.nix", "fastfetch.md", "tquilla.is-a.bot", "pocket-ai.gemini"],
  );
  assert.deepEqual(query(db, "journal_stack").map((r) => r.name), ["NixOS", "Rust", "Helix", "Figma", "Canva"]);
  assert.deepEqual(Object.keys(query(db, "journal_pinned")[0]), ["repo_name", "topic"]);
});

test("journal settings are not part of the seeds (a seed rerun would resurrect deleted rows)", () => {
  const db = freshDb();
  db.exec("DELETE FROM journal_pinned_repos WHERE repo_name = 'flexinix'");
  db.exec("DELETE FROM journal_stack WHERE name = 'Canva'");
  for (let round = 0; round < 2; round++) for (const s of sqlFiles("seeds")) db.exec(s.sql);
  assert.equal(count(db, "journal_pinned_repos"), 5);
  assert.equal(count(db, "journal_stack"), 4);
});

test("journal settings are ordered by position, then id; edits are plain row changes", () => {
  const db = freshDb();
  db.exec("UPDATE journal_pinned_repos SET position = 0 WHERE repo_name = 'pocket-ai.gemini'");
  db.exec("INSERT INTO journal_pinned_repos (position, repo_name, topic) VALUES (0, 'zz-new', NULL)");
  const names = query(db, "journal_pinned").map((r) => r.repo_name);
  assert.deepEqual(names.slice(0, 2), ["pocket-ai.gemini", "zz-new"]);
  assert.equal(query(db, "journal_pinned").find((r) => r.repo_name === "zz-new").topic, null);
  db.exec("UPDATE journal_profile SET github_username = 'someone-else'");
  assert.equal(query(db, "journal_profile")[0].github_username, "someone-else");
});

const insertSnapshot = (db, at, source = "github", payload = "{}") =>
  db.prepare(sqlOf("journal_insert").replace("'github'", "?3")).run(payload, at, source);

test("journal queries: latest snapshot wins by fetched_at, then id", () => {
  const db = freshDb();
  assert.deepEqual(query(db, "journal_latest"), []);
  insertSnapshot(db, "2026-10-20T06:00:00Z", "github", '{"n":1}');
  insertSnapshot(db, "2026-10-20T12:00:00Z", "github", '{"n":2}');
  insertSnapshot(db, "2026-10-20T12:00:00Z", "github", '{"n":3}');
  insertSnapshot(db, "2026-10-19T00:00:00Z", "github", '{"n":4}');
  const [row] = query(db, "journal_latest");
  assert.deepEqual({ ...row }, { payload: '{"n":3}', source: "github", fetched_at: "2026-10-20T12:00:00Z" });
  assert.deepEqual(query(db, "journal_latest_time").map((r) => ({ ...r })), [{ fetched_at: "2026-10-20T12:00:00Z" }]);
});

test("journal insert only ever writes source 'github' and rejects bad timestamps", () => {
  const db = freshDb();
  db.prepare(sqlOf("journal_insert")).run("{}", "2026-10-20T12:00:00Z");
  assert.equal(db.prepare("SELECT source FROM journal_snapshots").get().source, "github");
  assert.throws(() => db.prepare(sqlOf("journal_insert")).run("{}", "yesterday"), /constraint/i);
  assert.throws(() => db.prepare(sqlOf("journal_insert")).run("{nope", "2026-10-20T12:00:00Z"), /constraint/i);
});

test("journal prune keeps the newest N github snapshots and never touches seed rows", () => {
  const db = freshDb();
  insertSnapshot(db, "1970-01-01T00:00:00Z", "seed");
  for (let i = 0; i < 40; i++) {
    const hh = String(i % 24).padStart(2, "0");
    const dd = String(1 + Math.floor(i / 24)).padStart(2, "0");
    insertSnapshot(db, `2026-10-${dd}T${hh}:00:00Z`);
  }
  db.prepare(sqlOf("journal_prune")).run(28);
  assert.equal(db.prepare("SELECT count(*) AS n FROM journal_snapshots WHERE source = 'github'").get().n, 28);
  assert.equal(db.prepare("SELECT count(*) AS n FROM journal_snapshots WHERE source = 'seed'").get().n, 1);
  const oldest = db.prepare("SELECT min(fetched_at) AS t FROM journal_snapshots WHERE source = 'github'").get().t;
  assert.equal(oldest, "2026-10-01T12:00:00Z");
  // Menjalankannya lagi tidak mengubah apa pun.
  db.prepare(sqlOf("journal_prune")).run(28);
  assert.equal(count(db, "journal_snapshots"), 29);
});

test("the sample journal snapshot has the current payload shape (no markup fields)", () => {
  const db = seededDb();
  const payload = JSON.parse(db.prepare("SELECT payload FROM journal_snapshots").get().payload);
  assert.deepEqual(Object.keys(payload).sort(), ["chronicle", "hero", "pinned"]);
  const c = payload.chronicle;
  assert.equal(c.monthly_focus_html, undefined);
  assert.equal(c.growth_percentage, undefined);
  assert.equal(typeof c.growth_percent, "number");
  assert.ok(Array.isArray(c.timeline) && c.timeline.every((d) => /^\d{4}-\d{2}-\d{2}$/.test(d.date)));
  assert.ok(c.languages.every((l) => /^#[0-9a-f]{6}$/.test(l.color)));
});

// ---- Dashboard admin (D6a): pembatas percobaan gagal, riwayat, ringkasan

test("admin attempts: failures count per key and window; cleanup keeps the current and previous window", () => {
  const db = freshDb();
  const hit = (key, window) => db.prepare(sqlOf("admin_attempts_hit")).get(key, window).hits;
  const read = (key, window) => db.prepare(sqlOf("admin_attempts_count")).get(key, window)?.hits ?? 0;
  assert.equal(read("ip1", 10), 0, "no row before the first failure");
  assert.equal(hit("ip1", 10), 1);
  assert.equal(hit("ip1", 10), 2);
  assert.equal(hit("ip2", 10), 1, "other key");
  assert.equal(hit("ip1", 11), 1, "next window starts again");
  assert.equal(read("ip1", 10), 2);
  assert.equal(read("ip1", 11), 1);
  db.prepare(sqlOf("admin_attempts_hit")).get("ip1", 12);
  db.prepare(sqlOf("admin_attempts_cleanup")).run(11);
  const left = db
    .prepare("SELECT key, window FROM admin_attempts ORDER BY key, window")
    .all()
    .map((r) => ({ ...r }));
  assert.deepEqual(left, [
    { key: "ip1", window: 11 },
    { key: "ip1", window: 12 },
  ]);
});

test("admin audit: newest first with a cursor, and pruning keeps only the newest N", () => {
  const db = freshDb();
  const insert = db.prepare(sqlOf("admin_audit_insert"));
  for (let i = 0; i < 7; i++) {
    const id = insert.get(`2026-10-22T00:00:0${i}Z`, i === 0 ? "login" : "message.hide", i === 0 ? null : `message:${i}`, "abcdef0123456789").id;
    assert.equal(id, i + 1);
  }
  const list = (before, limit) =>
    db.prepare(sqlOf("admin_audit_list")).all(before, limit).map((r) => ({ ...r }));
  assert.deepEqual(list(null, 3).map((r) => r.id), [7, 6, 5]);
  assert.deepEqual(list(5, 10).map((r) => r.id), [4, 3, 2, 1]);
  assert.equal(list(null, 1)[0].target, "message:6");
  assert.equal(list(null, 10).at(-1).target, null);
  assert.deepEqual(Object.keys(list(null, 1)[0]), ["id", "at", "action", "target", "who"]);

  db.prepare(sqlOf("admin_audit_prune")).run(3);
  assert.deepEqual(list(null, 10).map((r) => r.id), [7, 6, 5], "keeps the newest 3");
  db.prepare(sqlOf("admin_audit_prune")).run(500);
  assert.equal(count(db, "admin_audit"), 3, "a large keep value removes nothing");
});

test("admin overview: counts every listed table, the guestbook and the journal, in one row", () => {
  const db = seededDb();
  // `_migrations` is created by the runner (crates/turso-http/src/migrate.rs), not by a migration file.
  db.exec(
    "CREATE TABLE IF NOT EXISTS _migrations (name TEXT PRIMARY KEY, checksum TEXT NOT NULL, applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')))",
  );
  for (const m of sqlFiles("migrations")) {
    db.prepare("INSERT INTO _migrations (name, checksum) VALUES (?, 'x')").run(m.name.replace(/\.sql$/, ""));
  }
  db.exec(`
    INSERT INTO chat_messages (username, message, created_at) VALUES
      ('zz-a', 'one', '2026-01-01T00:00:00Z'), ('zz-b', 'two', '2026-01-01T00:00:01Z'), ('zz-c', 'three', '2026-01-01T00:00:02Z');
    UPDATE chat_messages SET deleted_at = '2026-01-02T00:00:00Z' WHERE username = 'zz-c';
  `);
  const idOf = (u) => db.prepare("SELECT id FROM chat_messages WHERE username = ?").get(u).id;
  const report = db.prepare("INSERT INTO message_reports (message_id, reporter, created_at) VALUES (?, ?, '2026-01-03T00:00:00Z')");
  report.run(idOf("zz-a"), "r1");
  report.run(idOf("zz-a"), "r2");
  report.run(idOf("zz-b"), "r1");
  report.run(idOf("zz-c"), "r1");
  const rows = db.prepare(sqlOf("admin_overview")).all();
  assert.equal(rows.length, 1);
  const row = { ...rows[0] };
  for (const t of [
    "generations", "bookmarks", "radio_stations", "embiences", "embience_presets", "embience_preset_items",
    "chat_messages", "message_reports", "journal_snapshots", "journal_pinned_repos", "journal_stack", "admin_audit",
  ]) {
    assert.equal(row[t], count(db, t), t);
  }
  assert.equal(row.messages_hidden, 1);
  assert.equal(row.messages_reported, 2, "distinct visible entries with reports; the hidden one does not count");
  assert.equal(row.migrations_applied, sqlFiles("migrations").length);
  assert.equal(row.migrations_latest, sqlFiles("migrations").at(-1).name.replace(/\.sql$/, ""));
  assert.equal(row.journal_fetched_at, db.prepare(sqlOf("journal_latest_time")).get().fetched_at);
  assert.ok(["github", "seed"].includes(row.journal_source));
});

test("admin overview: an empty database reports zeros and nulls, not errors", () => {
  const db = freshDb();
  db.exec("CREATE TABLE _migrations (name TEXT PRIMARY KEY, checksum TEXT NOT NULL)");
  const row = { ...db.prepare(sqlOf("admin_overview")).get() };
  assert.equal(row.chat_messages, 0);
  assert.equal(row.messages_reported, 0);
  assert.equal(row.migrations_applied, 0);
  assert.equal(row.migrations_latest, null);
  assert.equal(row.journal_fetched_at, null);
  assert.equal(row.journal_source, null);
});

test("admin queries use bound parameters and an explicit order where they list rows", () => {
  const list = readFileSync(join(root, "crates/api/src/queries/admin_audit_list.sql"), "utf8");
  assert.match(list, /\bORDER BY id DESC\b/);
  assert.match(list, /\bLIMIT\s+\?2\s*$/m);
  for (const name of ["admin_attempts_count", "admin_attempts_hit", "admin_attempts_cleanup", "admin_audit_insert", "admin_audit_prune", "admin_audit_list"]) {
    const sql = readFileSync(join(root, "crates/api/src/queries", `${name}.sql`), "utf8");
    assert.match(sql, /\?\d/, `${name} must take its values as parameters`);
  }
});

// ---- D6b: editor Journal dan moderasi Guestbook

const rows = (db, name, ...args) => db.prepare(sqlOf(name)).all(...args).map((r) => ({ ...r }));
const names = (db, table) => db.prepare(`SELECT name FROM ${table} ORDER BY position, id`).all().map((r) => r.name);

test("journal editor: the account is upserted and the CHECK still guards the alphabet", () => {
  const db = seededDb();
  assert.equal(rows(db, "admin_journal_profile_set", "moch")[0].github_username, "moch");
  assert.equal(count(db, "journal_profile"), 1);
  assert.throws(() => db.prepare(sqlOf("admin_journal_profile_set")).run("bad name"));
  db.exec("DELETE FROM journal_profile");
  assert.equal(rows(db, "admin_journal_profile_set", "again")[0].github_username, "again", "also works when the row is missing");
  assert.equal(count(db, "journal_profile"), 1);
});

test("journal editor: new repos go last, the cap is enforced in SQL, duplicates are refused", () => {
  const db = seededDb();
  const insert = (name, topic = null, cap = 12) => rows(db, "admin_journal_repo_insert", name, topic, cap);
  const before = count(db, "journal_pinned_repos");
  const [a] = insert("new-one", "Topic");
  assert.ok(a.id);
  assert.equal(rows(db, "admin_journal_repos").at(-1).repo_name, "new-one");
  assert.equal(rows(db, "admin_journal_repos").at(-1).topic, "Topic");
  assert.throws(() => insert("new-one"), /UNIQUE constraint failed/);
  assert.equal(count(db, "journal_pinned_repos"), before + 1);
  // Penuh: tidak ada baris yang dimasukkan (dan bukan galat).
  while (count(db, "journal_pinned_repos") < 12) insert(`fill-${count(db, "journal_pinned_repos")}`);
  assert.deepEqual(insert("one-too-many"), []);
  assert.equal(count(db, "journal_pinned_repos"), 12);
  // Daftar kosong: posisi pertama = 1.
  db.exec("DELETE FROM journal_pinned_repos");
  insert("first");
  assert.equal(db.prepare("SELECT position FROM journal_pinned_repos").get().position, 1);
});

test("journal editor: edit and delete report whether a row was touched", () => {
  const db = seededDb();
  const first = rows(db, "admin_journal_repos")[0];
  assert.equal(rows(db, "admin_journal_repo_update", first.id, "renamed", null)[0].repo_name, "renamed");
  assert.equal(rows(db, "admin_journal_repos")[0].topic, null, "topic can be cleared");
  assert.deepEqual(rows(db, "admin_journal_repo_update", 99999, "x", null), []);
  const second = rows(db, "admin_journal_repos")[1];
  assert.throws(() => rows(db, "admin_journal_repo_update", second.id, "renamed", null), /UNIQUE constraint failed/);
  assert.equal(rows(db, "admin_journal_repo_delete", first.id)[0].repo_name, "renamed");
  assert.deepEqual(rows(db, "admin_journal_repo_delete", first.id), []);
  assert.throws(() => rows(db, "admin_journal_repo_update", second.id, "bad name", null), /CHECK constraint failed/);
});

test("journal editor: stack add, edit, delete, cap and duplicates behave the same way", () => {
  const db = seededDb();
  const ins = (n, cap = 12) => rows(db, "admin_journal_stack_insert", n, cap);
  assert.ok(ins("Zig")[0].id);
  assert.equal(names(db, "journal_stack").at(-1), "Zig");
  assert.throws(() => ins("Zig"), /UNIQUE constraint failed/);
  while (count(db, "journal_stack") < 12) ins(`s${count(db, "journal_stack")}`);
  assert.deepEqual(ins("extra"), []);
  const id = rows(db, "admin_journal_stack")[0].id;
  assert.equal(rows(db, "admin_journal_stack_update", id, "Changed")[0].name, "Changed");
  assert.deepEqual(rows(db, "admin_journal_stack_update", 99999, "x"), []);
  assert.equal(rows(db, "admin_journal_stack_delete", id)[0].name, "Changed");
  assert.deepEqual(rows(db, "admin_journal_stack_delete", id), []);
});

test("journal editor: positions written one by one reorder the list (list queries sort by position, id)", () => {
  const db = seededDb();
  const ids = rows(db, "admin_journal_repo_ids").map((r) => r.id);
  const wanted = [...ids].reverse();
  wanted.forEach((id, i) => db.prepare(sqlOf("admin_journal_repo_position")).run(i + 1, id));
  assert.deepEqual(rows(db, "admin_journal_repo_ids").map((r) => r.id), wanted);
  assert.deepEqual(rows(db, "admin_journal_repos").map((r) => r.id), wanted);

  const sids = rows(db, "admin_journal_stack_ids").map((r) => r.id);
  const swapped = [sids[1], sids[0], ...sids.slice(2)];
  swapped.forEach((id, i) => db.prepare(sqlOf("admin_journal_stack_position")).run(i + 1, id));
  assert.deepEqual(rows(db, "admin_journal_stack_ids").map((r) => r.id), swapped);
});

test("journal editor: snapshots list newest first and respects the limit", () => {
  const db = freshDb();
  db.exec("DELETE FROM journal_snapshots");
  for (const [at, src] of [
    ["2026-01-01T00:00:00Z", "seed"],
    ["2026-01-03T00:00:00Z", "github"],
    ["2026-01-02T00:00:00Z", "github"],
  ]) {
    insertSnapshot(db, at, src);
  }
  const list = rows(db, "admin_journal_snapshots", 2);
  assert.deepEqual(list.map((r) => r.fetched_at), ["2026-01-03T00:00:00Z", "2026-01-02T00:00:00Z"]);
  assert.deepEqual(Object.keys(list[0]).sort(), ["fetched_at", "id", "source"], "the payload is never listed");
});

function moderationDb() {
  const db = freshDb();
  db.exec("DELETE FROM chat_messages");
  const add = (user, msg, at, url = null) => insertMsg(db, user, msg, at, url);
  for (let i = 1; i <= 6; i++) add(`u${i}`, `message ${i}`, `2026-02-0${i}T00:00:00Z`);
  const id = (u) => db.prepare("SELECT id FROM chat_messages WHERE username = ?").get(u).id;
  const report = db.prepare("INSERT INTO message_reports (message_id, reporter, created_at) VALUES (?, ?, ?)");
  report.run(id("u2"), "r1", "2026-03-01T00:00:00Z");
  report.run(id("u2"), "r2", "2026-03-02T00:00:00Z");
  report.run(id("u4"), "r1", "2026-03-03T00:00:00Z");
  report.run(id("u5"), "r1", "2026-03-04T00:00:00Z");
  db.prepare("UPDATE chat_messages SET deleted_at = '2026-03-05T00:00:00Z' WHERE id = ?").run(id("u5"));
  db.prepare("UPDATE chat_messages SET deleted_at = '2026-03-06T00:00:00Z' WHERE id = ?").run(id("u6"));
  return { db, id };
}

test("moderation list: filters, counts, newest first, and cursor paging", () => {
  const { db, id } = moderationDb();
  const list = (before, filter, limit = 50) => rows(db, "admin_messages_list", before, filter, limit);
  assert.deepEqual(list(null, "reported").map((r) => r.id), [id("u4"), id("u2")], "visible entries with reports only");
  assert.deepEqual(list(null, "hidden").map((r) => r.id), [id("u6"), id("u5")]);
  assert.equal(list(null, "all").length, 6);
  assert.deepEqual(list(null, "anything-else").length, 6, "an unknown filter behaves like all (the API only sends the three)");
  const u2 = list(null, "reported").find((r) => r.id === id("u2"));
  assert.equal(u2.reports, 2);
  assert.equal(u2.last_reported_at, "2026-03-02T00:00:00Z");
  const plain = list(null, "all").find((r) => r.id === id("u1"));
  assert.equal(plain.reports, 0);
  assert.equal(plain.last_reported_at, null);
  assert.equal(plain.deleted_at, null);
  assert.equal(list(null, "all").find((r) => r.id === id("u5")).deleted_at, "2026-03-05T00:00:00Z");
  // Halaman demi halaman tanpa duplikat.
  const seen = [];
  let cursor = null;
  for (;;) {
    const page = list(cursor, "all", 4);
    if (!page.length) break;
    seen.push(...page.map((r) => r.id));
    cursor = page.at(-1).id;
  }
  assert.equal(seen.length, 6);
  assert.equal(new Set(seen).size, 6);
  assert.deepEqual(seen, [...seen].sort((a, b) => b - a));
});

test("moderation: restore only touches hidden entries, dismiss only removes reports", () => {
  const { db, id } = moderationDb();
  assert.equal(rows(db, "admin_message_restore", id("u5")).length, 1);
  assert.deepEqual(rows(db, "admin_message_restore", id("u5")), [], "already visible");
  assert.deepEqual(rows(db, "admin_message_restore", 99999), []);
  assert.equal(db.prepare("SELECT deleted_at FROM chat_messages WHERE id = ?").get(id("u5")).deleted_at, null);
  // u5 punya satu laporan lama; setelah dipulihkan ia kembali muncul di "reported".
  assert.ok(rows(db, "admin_messages_list", null, "reported", 50).some((r) => r.id === id("u5")));

  assert.equal(rows(db, "admin_reports_dismiss", id("u2")).length, 2, "one row per removed report");
  assert.deepEqual(rows(db, "admin_reports_dismiss", id("u2")), []);
  assert.equal(count(db, "chat_messages"), 6, "dismissing never removes the entry");
  assert.ok(!rows(db, "admin_messages_list", null, "reported", 50).some((r) => r.id === id("u2")));
});

test("D6b queries use bound parameters", () => {
  for (const name of [
    "admin_journal_profile_set", "admin_journal_repo_insert", "admin_journal_repo_update", "admin_journal_repo_delete",
    "admin_journal_repo_position", "admin_journal_stack_insert", "admin_journal_stack_update", "admin_journal_stack_delete",
    "admin_journal_stack_position", "admin_journal_snapshots", "admin_messages_list", "admin_message_restore", "admin_reports_dismiss",
  ]) {
    assert.match(sqlOf(name), /\?\d/, `${name} must take its values as parameters`);
  }
  for (const name of ["admin_journal_snapshots", "admin_messages_list", "admin_journal_repos", "admin_journal_stack"]) {
    assert.match(sqlOf(name), /ORDER BY/, `${name} needs an explicit order`);
  }
});

// ---- D6c: editor data umum (SQL yang dibangun Rust, lewat berkas emas)

const golden = JSON.parse(readFileSync(join(root, "crates/api/tests-data/tables_sql.json"), "utf8"));
const editable = Object.entries(golden.tables).filter(([, t]) => t.editable);

let sampleN = 0;
function sampleValue(col, variant = 0) {
  const n = ++sampleN;
  switch (col.kind) {
    case "text": return col.min === 0 && variant === 0 ? "" : `v${n}`;
    case "url": return col.nullable && variant === 0 ? null : `https://e.example/${n}`;
    case "route": return `/p${n}`;
    case "int": return 7;
    case "bool": return 1;
    case "date": return "2026-01-02";
    default: throw new Error(`no sample for ${col.kind}`);
  }
}
const sampleRow = (t, variant = 0) => t.columns.map((c) => sampleValue(c, variant));
const insertArgs = (t, key, values, cap) => [...(t.key_given ? [key] : []), ...values, cap];
const run = (db, sql, ...args) => db.prepare(sql).all(...args).map((r) => ({ ...r }));

test("golden file: every table is on the allowlist and has the full set of statements", () => {
  assert.deepEqual(
    Object.keys(golden.tables).sort(),
    ["admin_audit", "bookmarks", "chat_messages", "embience_presets", "embiences", "generations", "journal_pinned_repos", "journal_stack", "radio_stations"],
  );
  assert.ok(!Object.keys(golden.tables).some((n) => /^_|admin_attempts|message_reports|journal_snapshots|journal_profile/.test(n) && n !== "admin_audit"));
  for (const [name, t] of Object.entries(golden.tables)) {
    for (const sql of [t.sql.list, t.sql.exists, t.sql.insert, t.sql.update, ...t.sql.delete]) {
      assert.ok(!/[;']/.test(sql), `${name}: no literals or second statements: ${sql}`);
    }
  }
});

test("golden counts: one row with every table, matching the real counts", () => {
  const db = seededDb();
  const rows = run(db, golden.counts);
  assert.equal(rows.length, 1);
  for (const name of Object.keys(golden.tables)) assert.equal(rows[0][name], count(db, name), name);
  assert.equal(rows[0].generations, 5);
});

for (const [name, t] of editable) {
  test(`table editor SQL (${name}): add, list, change, delete against real SQLite`, () => {
    const db = seededDb();
    const cap = t.max_rows;
    const before = count(db, name);
    const key = t.key_given ? 900 : undefined;
    const values = sampleRow(t, 1);
    const added = run(db, t.sql.insert, ...insertArgs(t, key, values, cap));
    assert.equal(added.length, 1);
    const id = added[0][t.key];
    if (t.key_given) assert.equal(id, 900);
    assert.equal(count(db, name), before + 1);

    const listed = run(db, t.sql.list, null, 500);
    assert.equal(listed.length, before + 1);
    assert.deepEqual(Object.keys(listed[0]), [t.key, ...t.columns.map((c) => c.name)], "key first, then columns in registry order");
    const mine = listed.find((r) => r[t.key] === id);
    t.columns.forEach((c, i) => assert.equal(mine[c.name], values[i], `${name}.${c.name}`));

    assert.equal(run(db, t.sql.exists, id).length, 1);
    assert.equal(run(db, t.sql.exists, 123456).length, 0);

    const next = sampleRow(t, 1);
    const changed = run(db, t.sql.update, id, ...next);
    assert.equal(changed.length, 1);
    assert.equal(run(db, t.sql.update, 123456, ...next).length, 0, "missing row");
    const after = run(db, t.sql.list, null, 500).find((r) => r[t.key] === id);
    t.columns.forEach((c, i) => assert.equal(after[c.name], next[i]));
    assert.equal(count(db, name), before + 1, "update adds no rows");

    // Hapus: pernyataan terakhir menentukan apakah baris ada.
    let last;
    for (const sql of t.sql.delete) last = run(db, sql, id);
    assert.equal(last.length, 1);
    assert.equal(count(db, name), before);
    assert.equal(run(db, t.sql.delete.at(-1), id).length, 0);
  });

  test(`table editor SQL (${name}): the row cap is enforced inside the INSERT`, () => {
    const db = seededDb();
    const now = count(db, name);
    const key = t.key_given ? 901 : undefined;
    // Batas sama dengan jumlah sekarang: tidak ada baris yang masuk, dan itu bukan galat.
    assert.deepEqual(run(db, t.sql.insert, ...insertArgs(t, key, sampleRow(t, 1), now)), []);
    assert.equal(count(db, name), now);
    assert.equal(run(db, t.sql.insert, ...insertArgs(t, key, sampleRow(t, 1), now + 1)).length, 1);
    assert.deepEqual(run(db, t.sql.insert, ...insertArgs(t, t.key_given ? 902 : undefined, sampleRow(t, 1), now + 1)), []);
  });

  test(`table editor SQL (${name}): the database CHECKs still refuse what the API would refuse`, () => {
    const db = seededDb();
    const key = t.key_given ? 903 : undefined;
    for (const [i, c] of t.columns.entries()) {
      const bad = (() => {
        switch (c.kind) {
          case "text": return c.min > 0 ? "" : "x".repeat(c.max + 1);
          case "url": return "javascript:alert(1)";
          case "route": return "//evil.example";
          case "bool": return 2;
          case "date": return "not-a-date";
          default: return null;
        }
      })();
      if (bad === null) continue;
      const values = sampleRow(t, 1);
      values[i] = bad;
      assert.throws(() => run(db, t.sql.insert, ...insertArgs(t, key, values, 99)), /CHECK constraint failed|NOT NULL/, `${name}.${c.name}`);
    }
  });
}

test("table editor SQL: unique labels and names are refused with a UNIQUE error", () => {
  const db = seededDb();
  for (const [name, col] of [["radio_stations", "label"], ["embiences", "name"], ["embience_presets", "label"]]) {
    const t = golden.tables[name];
    const existing = db.prepare(`SELECT ${col} AS v FROM ${name} LIMIT 1`).get().v;
    const values = sampleRow(t, 1);
    values[t.columns.findIndex((c) => c.name === col)] = existing;
    assert.throws(() => run(db, t.sql.insert, ...insertArgs(t, undefined, values, 99)), /UNIQUE constraint failed/, name);
  }
  const g = golden.tables.generations;
  assert.throws(() => run(db, g.sql.insert, ...insertArgs(g, 1, sampleRow(g, 1), 99)), /UNIQUE constraint failed: generations.number/);
});

test("table editor SQL: deleting an embience or a preset also removes its preset items", () => {
  const db = seededDb();
  const items = () => count(db, "embience_preset_items");
  const start = items();
  const emb = golden.tables.embiences.sql.delete;
  assert.equal(emb.length, 2);
  const used = db.prepare("SELECT embience_id AS id, count(*) AS n FROM embience_preset_items GROUP BY embience_id ORDER BY n DESC").get();
  for (const sql of emb) run(db, sql, used.id);
  assert.equal(items(), start - used.n);
  assert.equal(db.prepare("SELECT count(*) AS n FROM embience_preset_items WHERE embience_id = ?").get(used.id).n, 0);
  const pre = golden.tables.embience_presets.sql.delete;
  const preset = db.prepare("SELECT preset_id AS id, count(*) AS n FROM embience_preset_items GROUP BY preset_id").get();
  const mid = items();
  for (const sql of pre) run(db, sql, preset.id);
  assert.equal(items(), mid - preset.n);
  assert.deepEqual(db.prepare("PRAGMA foreign_key_check").all(), []);
});

test("table editor SQL: paged tables page by key descending without gaps or repeats", () => {
  const db = freshDb();
  db.exec("DELETE FROM chat_messages");
  for (let i = 1; i <= 7; i++) insertMsg(db, `u${i}`, `m${i}`, `2026-01-0${i}T00:00:00Z`);
  const t = golden.tables.chat_messages;
  assert.equal(t.paged, true);
  const seen = [];
  let cursor = null;
  for (;;) {
    const page = run(db, t.sql.list, cursor, 3);
    if (!page.length) break;
    seen.push(...page.map((r) => r.id));
    cursor = page.at(-1).id;
  }
  assert.deepEqual(seen, [7, 6, 5, 4, 3, 2, 1]);
  assert.ok(!Object.keys(run(db, t.sql.list, null, 1)[0]).some((k) => /avatar|attachment/.test(k)), "legacy columns are not exposed");
  const audit = golden.tables.admin_audit;
  assert.ok(!audit.columns.some((c) => c.name === "who"), "who is not exported");
});

test("table editor SQL: small tables list in display order", () => {
  const db = seededDb();
  const order = (name) => run(db, golden.tables[name].sql.list, null, 500).map((r) => r[golden.tables[name].key]);
  assert.deepEqual(order("generations"), [5, 4, 3, 2, 1], "newest generation first");
  assert.deepEqual(order("bookmarks"), run(db, "SELECT id FROM bookmarks ORDER BY sort_order, id").map((r) => r.id));
  assert.deepEqual(order("embiences"), run(db, "SELECT id FROM embiences ORDER BY sort_order, id").map((r) => r.id));
});

test("preset items: replaced in the given order, listed by preset and position", () => {
  const db = seededDb();
  const list = () => run(db, sqlOf("admin_preset_items"));
  assert.equal(list().length, 7);
  assert.deepEqual(run(db, sqlOf("admin_preset_exists"), 2).length, 1);
  assert.deepEqual(run(db, sqlOf("admin_preset_exists"), 99), []);
  assert.equal(run(db, sqlOf("admin_embience_ids")).length, 12);
  run(db, sqlOf("admin_preset_items_delete"), 3);
  [[9, 0], [12, 1], [6, 2]].forEach(([e, p]) => run(db, sqlOf("admin_preset_item_insert"), 3, e, p));
  assert.deepEqual(list().filter((r) => r.preset_id === 3).map((r) => r.embience_id), [9, 12, 6]);
  assert.equal(list().filter((r) => r.preset_id === 1).length, 2, "other presets are untouched");
  assert.throws(() => run(db, sqlOf("admin_preset_item_insert"), 3, 9, 5), /UNIQUE constraint failed/, "an embience appears once per preset");
  assert.deepEqual(list().map((r) => r.preset_id), [...list().map((r) => r.preset_id)].sort((a, b) => a - b));
});

test("purge: only a hidden entry is removed, together with its reports", () => {
  const db = freshDb();
  db.exec("DELETE FROM chat_messages");
  insertMsg(db, "a", "visible", "2026-01-01T00:00:00Z");
  insertMsg(db, "b", "hidden", "2026-01-02T00:00:00Z");
  const ids = Object.fromEntries(["a", "b"].map((u) => [u, db.prepare("SELECT id FROM chat_messages WHERE username = ?").get(u).id]));
  const id = (u) => ids[u];
  db.prepare("UPDATE chat_messages SET deleted_at = '2026-01-03T00:00:00Z' WHERE id = ?").run(id("b"));
  const report = db.prepare("INSERT INTO message_reports (message_id, reporter, created_at) VALUES (?, ?, '2026-01-03T00:00:00Z')");
  report.run(id("a"), "r1");
  report.run(id("b"), "r1");
  report.run(id("b"), "r2");
  // Dalam satu transaksi, urutan seperti di Rust: entri dulu, lalu laporan yatim.
  const purge = (n) => [run(db, sqlOf("admin_message_purge"), n), run(db, sqlOf("admin_message_purge_reports"), n)];
  assert.deepEqual(purge(id("a"))[0], [], "a visible entry cannot be purged");
  assert.equal(count(db, "chat_messages"), 2);
  assert.equal(count(db, "message_reports"), 3, "its reports are kept too");
  assert.equal(purge(id("b"))[0].length, 1);
  assert.equal(count(db, "chat_messages"), 1);
  assert.equal(db.prepare("SELECT count(*) AS n FROM message_reports WHERE message_id = ?").get(id("b")).n, 0);
  assert.equal(count(db, "message_reports"), 1, "only the purged entry's reports are gone");
  assert.deepEqual(purge(999)[0], []);
});

// ---- D7a: pemeriksa stasiun radio (crates/api/src/queries/radio_check_*.sql)

const stationRows = (db) => db.prepare("SELECT id, is_active FROM radio_stations ORDER BY id").all().map((r) => ({ ...r }));
const addStation = (db, id, url, active = 1) =>
  db.prepare("INSERT INTO radio_stations (id, label, stream_url, is_active) VALUES (?, ?, ?, ?)").run(id, `s${id}`, url, active);
const record = (db, id, at, { url = `https://s${id}.example/live`, failures = 0, result = "ok" } = {}) =>
  db.prepare(sqlOf("radio_check_upsert")).run(id, url, at, null, null, failures, result, null, null);

test("radio checks: a valid row is accepted and the upsert updates it in place", () => {
  const db = freshDb();
  db.exec(radioCheck({ status: 403 }));
  assert.equal(count(db, "radio_checks"), 1);
  record(db, 1, "2026-02-02T00:00:00Z", { url: "https://a.b/s", failures: 2, result: "timeout" });
  assert.equal(count(db, "radio_checks"), 1, "same station: updated, not duplicated");
  const row = { ...db.prepare("SELECT * FROM radio_checks").get() };
  assert.equal(row.last_checked_at, "2026-02-02T00:00:00Z");
  assert.equal(row.consecutive_failures, 2);
  assert.equal(row.last_result, "timeout");
  assert.equal(row.last_status, null, "the upsert writes every column it is given");
});

test("radio checks: the due query is active stations only, never-checked first, then oldest first", () => {
  const db = freshDb();
  for (let id = 1; id <= 5; id++) addStation(db, id, `https://s${id}.example/live`, id === 4 ? 0 : 1);
  record(db, 1, "2026-03-01T10:00:00Z");
  record(db, 2, "2026-03-01T08:00:00Z");
  record(db, 3, "2026-03-01T09:00:00Z");
  const due = (limit) => rows(db, "radio_check_due", limit).map((r) => r.id);
  assert.deepEqual(due(10), [5, 2, 3, 1], "never-checked 5 first; inactive 4 never");
  assert.deepEqual(due(2), [5, 2], "LIMIT is a parameter");
  record(db, 5, "2026-03-01T11:00:00Z");
  assert.deepEqual(due(10), [2, 3, 1, 5], "once checked it goes to the back of the line");
});

test("radio checks: the due query carries the stored history next to the current URL", () => {
  const db = freshDb();
  addStation(db, 1, "https://new.example/live");
  db.prepare(sqlOf("radio_check_upsert")).run(
    1, "https://old.example/live", "2026-03-01T00:00:00Z", "2026-02-28T00:00:00Z", "2026-02-28T12:00:00Z", 2, "http", 404, null,
  );
  const [row] = rows(db, "radio_check_due", 12);
  assert.deepEqual(row, {
    id: 1, stream_url: "https://new.example/live", checked_url: "https://old.example/live",
    failures: 2, failing_since: "2026-02-28T12:00:00Z", last_ok_at: "2026-02-28T00:00:00Z", disabled_at: null,
  });
  addStation(db, 2, "https://x.example/live");
  const fresh = rows(db, "radio_check_due", 12).find((r) => r.id === 2);
  assert.equal(fresh.checked_url, null);
  assert.equal(fresh.failures, null);
});

test("radio checks: disabling only touches an active station whose URL is still the one that was checked", () => {
  const db = freshDb();
  addStation(db, 1, "https://a.example/live?rj-tok=NEW");
  addStation(db, 2, "https://b.example/live");
  const disable = (id, url) => db.prepare(sqlOf("radio_check_disable")).run(id, url).changes;
  assert.equal(disable(1, "https://a.example/live?rj-tok=OLD"), 0, "the admin pasted a new link meanwhile");
  assert.deepEqual(stationRows(db), [{ id: 1, is_active: 1 }, { id: 2, is_active: 1 }]);
  assert.equal(disable(2, "https://b.example/live"), 1);
  assert.deepEqual(stationRows(db), [{ id: 1, is_active: 1 }, { id: 2, is_active: 0 }]);
  assert.equal(disable(2, "https://b.example/live"), 0, "already inactive");
  // Stasiun nonaktif tidak diperiksa lagi (kebijakan D7a: tidak ada pengaktifan otomatis).
  assert.deepEqual(rows(db, "radio_check_due", 12).map((r) => r.id), [1]);
});

test("radio checks: deleting a station removes its check row (the dashboard editor must still work)", () => {
  const db = freshDb();
  addStation(db, 1, "https://a.example/live");
  addStation(db, 2, "https://b.example/live");
  record(db, 1, "2026-03-01T00:00:00Z");
  record(db, 2, "2026-03-01T00:00:00Z");
  db.prepare("DELETE FROM radio_stations WHERE id = 1").run();
  assert.deepEqual(db.prepare("SELECT station_id FROM radio_checks").all().map((r) => r.station_id), [2]);
});

test("radio checks: cleanup removes rows whose station is gone even when foreign keys are off", () => {
  const db = freshDb();
  addStation(db, 1, "https://a.example/live");
  addStation(db, 2, "https://b.example/live");
  record(db, 1, "2026-03-01T00:00:00Z");
  record(db, 2, "2026-03-01T00:00:00Z");
  db.exec("PRAGMA foreign_keys = OFF");
  db.prepare("DELETE FROM radio_stations WHERE id = 1").run();
  assert.equal(count(db, "radio_checks"), 2, "orphan left behind without the cascade");
  db.prepare(sqlOf("radio_check_cleanup")).run();
  assert.deepEqual(db.prepare("SELECT station_id FROM radio_checks").all().map((r) => r.station_id), [2]);
});

test("radio checks: a seeded database has no check rows and its stations are all due", () => {
  const db = seededDb();
  assert.equal(count(db, "radio_checks"), 0);
  assert.equal(rows(db, "radio_check_due", 12).length, count(db, "radio_stations"));
  // Seed berlaku sekali dan tidak menyentuh radio_checks.
  assert.equal(readFileSync(join(root, "seeds/0001_content.sql"), "utf8").includes("radio_checks"), false);
});

test("radio check queries take every value as a bound parameter", () => {
  for (const name of ["radio_check_due", "radio_check_upsert", "radio_check_disable"]) {
    assert.match(sqlOf(name), /\?\d/, `${name} must take its values as parameters`);
  }
  assert.match(sqlOf("radio_check_due"), /\bORDER BY\b/);
});

// ---- D7b: status radio untuk dashboard (admin_radio_list.sql, admin_radio_last.sql)

test("radio status lists every station, active or not, in player order, with its check history", () => {
  const db = freshDb();
  addStation(db, 1, "https://a.example/live");
  addStation(db, 2, "https://b.example/live", 0);
  addStation(db, 3, "https://c.example/live");
  db.prepare("UPDATE radio_stations SET sort_order = ? WHERE id = ?").run(5, 1);
  db.prepare("UPDATE radio_stations SET sort_order = ? WHERE id = ?").run(1, 3);
  record(db, 3, "2026-03-01T00:00:00Z", { failures: 2, result: "http" });
  const list = rows(db, "admin_radio_list");
  assert.deepEqual(list.map((r) => [r.id, r.is_active]), [[2, 0], [3, 1], [1, 1]], "by sort_order then id");
  const checked = list.find((r) => r.id === 3);
  assert.equal(checked.failures, 2);
  assert.equal(checked.last_result, "http");
  assert.equal(list.find((r) => r.id === 1).last_result, null, "never checked: nulls, not missing rows");
});

test("radio status: the last check time is the newest of all, or null on an empty table", () => {
  const db = freshDb();
  assert.equal(rows(db, "admin_radio_last").length, 1);
  assert.equal(rows(db, "admin_radio_last")[0].at, null);
  addStation(db, 1, "https://a.example/live");
  addStation(db, 2, "https://b.example/live");
  record(db, 1, "2026-03-01T10:00:00Z");
  record(db, 2, "2026-03-01T11:30:00Z");
  assert.equal(rows(db, "admin_radio_last")[0].at, "2026-03-01T11:30:00Z");
});

test("radio status queries order explicitly and cap the list", () => {
  assert.match(sqlOf("admin_radio_list"), /ORDER BY s\.sort_order, s\.id\s+LIMIT 200/);
});
