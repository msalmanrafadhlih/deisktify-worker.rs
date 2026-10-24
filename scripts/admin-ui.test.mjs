// Tes dashboard admin (web/admin): logika murni, klien API dengan fetch palsu, dan
// pemeriksaan statis (CSP, tidak ada innerHTML/eval/penyimpanan token, pustaka
// bawaan tidak berubah, path sama dengan kontrak Rust).
//
//   node --test scripts/admin-ui.test.mjs        (Node >= 22.7)

import { test } from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const web = join(root, "web");
const admin = join(web, "admin");
const read = (...p) => readFileSync(join(...p), "utf8");
const load = (name) => import(pathToFileURL(join(admin, name)).href);

const logic = await load("logic.js");
const { createApi, ApiError, PATHS } = await load("api.js");

// ------------------------------------------------------------------ logika

test("retry-after is only a plain number of seconds", () => {
  assert.equal(logic.parseRetryAfter("42"), 42);
  assert.equal(logic.parseRetryAfter(" 7 "), 7);
  for (const bad of [null, undefined, "", "abc", "-1", "1.5", "1e3", "Wed, 21 Oct 2026 07:28:00 GMT", "1234567"]) {
    assert.equal(logic.parseRetryAfter(bad), null, String(bad));
  }
});

test("waiting times read naturally and never lie about being zero", () => {
  assert.equal(logic.formatWait(1), "1 detik");
  assert.equal(logic.formatWait(59), "59 detik");
  assert.equal(logic.formatWait(60), "1 menit");
  assert.equal(logic.formatWait(61), "2 menit");
  assert.equal(logic.formatWait(800), "14 menit");
  for (const bad of [0, -5, NaN, null, undefined]) assert.equal(logic.formatWait(bad), "", String(bad));
});

test("errors become Indonesian messages by status and never echo server text", () => {
  const text = (status, extra = {}) => logic.describeError({ status, message: "SECRET-FROM-SERVER", ...extra });
  assert.match(text(0), /Tidak bisa menghubungi server/);
  assert.match(text(401), /Token salah/);
  assert.match(text(429, { retryAfter: 800 }), /14 menit/);
  assert.match(text(429, { retryAfter: null }), /nanti/);
  assert.match(text(502), /GitHub/);
  assert.match(text(503), /database/);
  assert.match(text(500), /kesalahan/);
  for (const s of [0, 400, 401, 429, 500, 502, 503]) assert.doesNotMatch(text(s), /SECRET/);
  assert.match(logic.describeError(null), /Tidak bisa menghubungi server/);
  assert.match(logic.describeError(new Error("boom")), /Tidak bisa menghubungi server/);
});

test("time ago is measured against the server's clock", () => {
  const now = "2026-10-22T12:00:00Z";
  assert.equal(logic.timeAgo(null, now), "belum pernah");
  assert.equal(logic.timeAgo("2026-10-22T11:59:30Z", now), "baru saja");
  assert.equal(logic.timeAgo("2026-10-22T12:00:30Z", now), "baru saja", "a little in the future");
  assert.equal(logic.timeAgo("2026-10-22T11:55:00Z", now), "5 menit lalu");
  assert.equal(logic.timeAgo("2026-10-22T06:00:00Z", now), "6 jam lalu");
  assert.equal(logic.timeAgo("2026-10-19T12:00:00Z", now), "3 hari lalu");
  assert.equal(logic.timeAgo("nonsense", now), "tidak diketahui");
  assert.equal(logic.timeAgo("2026-10-22T11:00:00Z", "nonsense"), "tidak diketahui");
});

test("local time formatting keeps the original text when it cannot be read", () => {
  assert.equal(logic.formatLocal("nonsense"), "nonsense");
  const jakarta = logic.formatLocal("2026-10-22T00:30:00Z", "Asia/Jakarta");
  assert.match(jakarta, /22/);
  assert.match(jakarta, /2026/);
  assert.match(jakarta, /07[.:]30/, "UTC+7");
});

test("idle detection uses timestamps, so a sleeping tab still locks", () => {
  assert.equal(logic.isIdle(0, logic.IDLE_LIMIT_MS - 1), false);
  assert.equal(logic.isIdle(0, logic.IDLE_LIMIT_MS), true);
  assert.equal(logic.isIdle(1000, 1000 + 5 * 60 * 1000, 5 * 60 * 1000), true);
  assert.equal(logic.IDLE_LIMIT_MS, 10 * 60 * 1000);
});

test("labels cover every table the server reports and fall back to the raw name", () => {
  const sql = read(root, "crates/api/src/panel.rs");
  const block = /pub const TABLES: \[&str; \d+\] = \[([^\]]+)\]/.exec(sql)[1];
  const names = [...block.matchAll(/"([a-z_]+)"/g)].map((m) => m[1]);
  assert.ok(names.length >= 12);
  for (const n of names) assert.notEqual(logic.tableLabel(n), n, `${n} has no label`);
  assert.equal(logic.tableLabel("unknown_table"), "unknown_table");
  assert.equal(logic.actionLabel("login"), "Masuk");
  assert.equal(logic.actionLabel("something.new"), "something.new");
  assert.equal(logic.targetLabel("message:12"), "Entri #12");
  assert.equal(logic.targetLabel("message:x"), "Entri #x");
  assert.equal(logic.targetLabel("repo:flexinix"), "Repo flexinix");
  assert.equal(logic.targetLabel("stack:Rust"), "Stack Rust");
  assert.equal(logic.targetLabel("github:moch"), "Akun moch");
  assert.equal(logic.targetLabel("table:x"), "table:x");
  assert.equal(logic.targetLabel(null), "");
  assert.equal(logic.whoLabel("abc", "abc"), "Perangkat ini");
  assert.equal(logic.whoLabel("abc", "xyz"), "Lainnya (abc)");
});

const goodOverview = () => ({
  now: "2026-10-22T12:00:00Z",
  me: "01234567",
  tables: [{ name: "embiences", rows: 3 }],
  guestbook: { total: 5, hidden: 1, reported: 2 },
  journal: { fetched_at: "2026-10-22T06:00:00Z", source: "github", snapshots: 4, github_configured: true },
  migrations: { applied: 10, latest: "0010_admin" },
});

test("the overview is checked defensively and renamed for the screen", () => {
  const o = logic.normalizeOverview(goodOverview());
  assert.deepEqual(o.tables, [{ name: "embiences", rows: 3 }]);
  assert.equal(o.journal.fetchedAt, "2026-10-22T06:00:00Z");
  assert.equal(o.journal.githubConfigured, true);
  assert.equal(o.migrations.latest, "0010_admin");

  const empty = goodOverview();
  empty.journal = { fetched_at: null, source: null, snapshots: 0, github_configured: false };
  empty.migrations.latest = null;
  const e = logic.normalizeOverview(empty);
  assert.equal(e.journal.fetchedAt, null);
  assert.equal(e.journal.githubConfigured, false);
  assert.equal(e.migrations.latest, null);

  const broken = [
    null, [], "x", {},
    { ...goodOverview(), tables: "no" },
    { ...goodOverview(), tables: [{ name: "x", rows: -1 }] },
    { ...goodOverview(), tables: [{ name: 1, rows: 1 }] },
    { ...goodOverview(), guestbook: { total: "5", hidden: 1, reported: 2 } },
    { ...goodOverview(), guestbook: null },
    { ...goodOverview(), me: null },
    { ...goodOverview(), now: undefined },
  ];
  for (const b of broken) assert.equal(logic.normalizeOverview(b), null, JSON.stringify(b));
});

test("audit pages are checked row by row", () => {
  const row = { id: 3, at: "2026-10-22T00:00:00Z", action: "login", target: null, who: "01234567" };
  assert.deepEqual(logic.normalizeAudit({ items: [row] }), [row]);
  assert.deepEqual(logic.normalizeAudit({ items: [] }), []);
  for (const bad of [null, {}, { items: "x" }, { items: [{ ...row, id: "3" }] }, { items: [{ ...row, who: 1 }] }, { items: [null] }]) {
    assert.equal(logic.normalizeAudit(bad), null, JSON.stringify(bad));
  }
});

// --------------------------------------------------------------- klien API

function fakeFetch(handler) {
  const calls = [];
  const fn = async (url, init) => {
    calls.push({ url, init });
    return handler(url, init);
  };
  fn.calls = calls;
  return fn;
}

const reply = (status, body, headers = {}) => ({
  ok: status >= 200 && status < 300,
  status,
  headers: { get: (n) => headers[n.toLowerCase()] ?? null },
  json: async () => body,
});

test("login posts the token as a bearer header, never in the URL, and keeps it only on success", async () => {
  const f = fakeFetch(() => reply(200, goodOverview()));
  const api = createApi(f);
  assert.equal(api.hasToken(), false);
  const out = await api.login("  s3cret-token  ");
  assert.equal(out.me, "01234567");
  assert.equal(api.hasToken(), true);
  const { url, init } = f.calls[0];
  assert.equal(url, "/api/v1/admin/login");
  assert.equal(init.method, "POST");
  assert.equal(init.headers.authorization, "Bearer s3cret-token", "trimmed");
  assert.equal(init.cache, "no-store");
  assert.equal(init.credentials, "omit");
  assert.equal(init.referrerPolicy, "no-referrer");
  assert.ok(!url.includes("s3cret"));
  assert.equal(init.body, undefined);

  await api.overview();
  await api.audit(null, 30);
  await api.audit(12, 30);
  assert.equal(f.calls[1].url, "/api/v1/admin/overview");
  assert.equal(f.calls[2].url, "/api/v1/admin/audit?limit=30");
  assert.equal(f.calls[3].url, "/api/v1/admin/audit?limit=30&before=12");
  for (const c of f.calls) assert.ok(!c.url.includes("s3cret"));

  api.lock();
  assert.equal(api.hasToken(), false);
  await assert.rejects(() => api.overview(), (e) => e instanceof ApiError && e.status === 401);
  assert.equal(f.calls.length, 4, "a locked client makes no request");
});

test("a rejected login forgets the token and reports the status and wait time", async () => {
  for (const [status, headers, wait] of [[401, {}, null], [429, { "retry-after": "800" }, 800], [503, {}, null]]) {
    const api = createApi(fakeFetch(() => reply(status, { error: { message: "SECRET" } }, headers)));
    await assert.rejects(
      () => api.login("wrong"),
      (e) => e instanceof ApiError && e.status === status && e.retryAfter === wait && !String(e.message).includes("SECRET"),
    );
    assert.equal(api.hasToken(), false);
  }
});

test("network failures and broken bodies become ApiErrors with no leaked detail", async () => {
  let api = createApi(fakeFetch(() => { throw new TypeError("Failed to fetch https://secret.host"); }));
  await assert.rejects(() => api.login("t"), (e) => e instanceof ApiError && e.status === 0 && !e.message.includes("secret"));
  api = createApi(fakeFetch(() => ({ ok: true, status: 200, headers: { get: () => null }, json: async () => { throw new SyntaxError("x"); } })));
  await assert.rejects(() => api.login("t"), (e) => e instanceof ApiError && e.status === 500);
  assert.equal(api.hasToken(), false);
});

test("an empty token never reaches the network", async () => {
  const f = fakeFetch(() => reply(200, {}));
  const api = createApi(f);
  await assert.rejects(() => api.login("   "), (e) => e.status === 401);
  assert.equal(f.calls.length, 0);
});

// ------------------------------------------------------------------ statis

const jsFiles = ["app.js", "api.js", "logic.js", "parts.js", "journal.js", "guestbook.js", "data.js", "radio.js"];

test("the page has no inline script or style and loads one module from its own folder", () => {
  const html = read(admin, "index.html");
  assert.match(html, /<script type="module" src="app\.js"><\/script>/);
  assert.equal([...html.matchAll(/<script\b/g)].length, 1);
  assert.doesNotMatch(html, /<style\b/i);
  assert.doesNotMatch(html, /\sstyle\s*=/i);
  assert.doesNotMatch(html, /\son[a-z]+\s*=/i, "no inline event handlers");
  assert.doesNotMatch(html, /(src|href)="(https?:)?\/\//, "no other hosts");
  assert.match(html, /name="robots" content="noindex, nofollow"/);
  assert.match(html, /name="referrer" content="no-referrer"/);
});

test("the headers file sets a strict CSP and no unsafe sources", () => {
  const headers = read(web, "_headers");
  const csp = /Content-Security-Policy:\s*(.+)/.exec(headers)[1];
  const directive = (name) => new RegExp(`(?:^|;\\s*)${name}\\s+([^;]+)`).exec(csp)?.[1].trim();
  assert.equal(directive("default-src"), "'none'");
  assert.equal(directive("script-src"), "'self'");
  assert.equal(directive("style-src"), "'self'");
  assert.equal(directive("connect-src"), "'self'");
  assert.equal(directive("base-uri"), "'none'");
  assert.equal(directive("frame-ancestors"), "'none'");
  assert.equal(directive("form-action"), "'none'");
  assert.doesNotMatch(csp, /unsafe-|https?:|\*|data:/);
  for (const h of ["X-Content-Type-Options: nosniff", "Referrer-Policy: no-referrer", "X-Robots-Tag: noindex", "Cache-Control: no-store"]) {
    assert.ok(headers.includes(h), h);
  }
  assert.ok(headers.split("\n").length < 100, "well under the 100 rule limit");
});

test("our scripts never touch markup injection, eval, storage or the URL for the token", () => {
  for (const name of jsFiles) {
    const src = read(admin, name).replace(/\/\/.*$/gm, "");
    for (const bad of ["innerHTML", "outerHTML", "insertAdjacentHTML", "dangerouslySetInnerHTML", "document.write", "eval(", "new Function", "localStorage", "sessionStorage", "indexedDB", "document.cookie", "location.hash", "location.search", "history.pushState"]) {
      assert.ok(!src.includes(bad), `${name} uses ${bad}`);
    }
  }
});

test("every script parses and every import points at a file that exists", () => {
  for (const name of jsFiles) {
    execFileSync(process.execPath, ["--check", join(admin, name)], { stdio: "pipe" });
    for (const m of read(admin, name).matchAll(/from\s+"(\.[^"]+)"/g)) {
      assert.ok(existsSync(resolve(admin, m[1])), `${name} imports ${m[1]}`);
    }
  }
});

test("the vendored library is exactly the audited file and says so", () => {
  const file = join(admin, "vendor", "standalone.module.js");
  const sha = createHash("sha256").update(readFileSync(file)).digest("hex");
  assert.equal(sha, "72284e8e9079c87817145df1110f74e8a2aa040b2fc384922e18dfcb46fc1fd7");
  assert.ok(read(admin, "vendor", "README.md").includes(sha));
  assert.doesNotMatch(readFileSync(file, "utf8"), /eval\(|new Function/);
  for (const l of ["LICENSE.htm", "LICENSE.preact"]) assert.ok(existsSync(join(admin, "vendor", l)), l);
});

test("client paths match the Rust contract", () => {
  const rust = read(root, "crates/api-types/src/lib.rs");
  const path = (name) => new RegExp(`pub const ${name}: &str = "([^"]+)"`).exec(rust)[1];
  assert.equal(PATHS.login, path("ADMIN_LOGIN"));
  assert.equal(PATHS.overview, path("ADMIN_OVERVIEW"));
  assert.equal(PATHS.audit, path("ADMIN_AUDIT"));
  const pairs = {
    journal: "ADMIN_JOURNAL",
    journalRefresh: "ADMIN_JOURNAL_REFRESH",
    journalProfile: "ADMIN_JOURNAL_PROFILE",
    journalRepos: "ADMIN_JOURNAL_REPOS",
    journalReposOrder: "ADMIN_JOURNAL_REPOS_ORDER",
    journalRepo: "ADMIN_JOURNAL_REPO",
    journalStack: "ADMIN_JOURNAL_STACK",
    journalStackOrder: "ADMIN_JOURNAL_STACK_ORDER",
    journalStackItem: "ADMIN_JOURNAL_STACK_ITEM",
    messages: "ADMIN_MESSAGES",
    message: "ADMIN_MESSAGE",
    messageRestore: "ADMIN_MESSAGE_RESTORE",
    messageReports: "ADMIN_MESSAGE_REPORTS",
    tables: "ADMIN_TABLES",
    tableRows: "ADMIN_TABLE_ROWS",
    tableRow: "ADMIN_TABLE_ROW",
    tableExport: "ADMIN_TABLE_EXPORT",
    presetItems: "ADMIN_PRESET_ITEMS",
    presetItemsSet: "ADMIN_PRESET_ITEMS_SET",
    messagePurge: "ADMIN_MESSAGE_PURGE",
    radio: "ADMIN_RADIO",
    radioCheck: "ADMIN_RADIO_CHECK",
  };
  for (const [key, rustName] of Object.entries(pairs)) assert.equal(PATHS[key], path(rustName), key);
  assert.deepEqual(Object.keys(PATHS).sort(), [...Object.keys(pairs), "login", "overview", "audit"].sort(), "every client path is checked");
});

test("wrangler serves the web folder, which holds the admin page", () => {
  const toml = read(root, "crates/api/wrangler.toml");
  const dir = /^\[assets\][^\[]*?^directory\s*=\s*"([^"]+)"/ms.exec(toml)?.[1];
  assert.ok(dir, "[assets] directory is set");
  const resolved = resolve(root, "crates/api", dir);
  assert.equal(resolved, web);
  assert.ok(existsSync(join(resolved, "admin", "index.html")));
  assert.ok(existsSync(join(resolved, "_headers")));
  assert.doesNotMatch(toml, /run_worker_first/, "assets are matched first; only unmatched paths reach the Worker");
});

// -------------------------------------------------------------- D6b: logika

test("every audit action the server writes has a readable label", () => {
  const src = read(root, "crates/api/src/routes.rs");
  const actions = [...src.matchAll(/audit\(&admin, "([a-z.]+)"/g)].map((m) => m[1]);
  assert.ok(actions.length >= 12, `found ${actions.length}`);
  for (const a of actions) assert.notEqual(logic.actionLabel(a), a, `${a} has no label`);
  assert.ok(actions.every((a) => a.length <= 40), "fits the audit column");
});

test("new error statuses have their own Indonesian message", () => {
  for (const s of [404, 409, 413]) assert.doesNotMatch(logic.describeError({ status: s }), /kesalahan di server/, String(s));
  assert.match(logic.describeError({ status: 409 }), /sudah ada/);
  assert.match(logic.describeError({ status: 404 }), /tidak ditemukan/);
});

test("form checks mirror the server rules and give a reason", () => {
  assert.equal(logic.validateUsername("moch-1"), "");
  for (const bad of ["", "  ", "a b", "a_b", "x".repeat(40), "é"]) assert.notEqual(logic.validateUsername(bad), "", JSON.stringify(bad));
  assert.equal(logic.validateRepo("flex.inix_1-a", "Nix flakes"), "");
  assert.equal(logic.validateRepo("a", ""), "");
  for (const bad of ["", " ", "a b", "a/b", "é", "x".repeat(101)]) assert.notEqual(logic.validateRepo(bad, ""), "", JSON.stringify(bad));
  assert.notEqual(logic.validateRepo("a", "t".repeat(61)), "");
  assert.equal(logic.validateRepo("a", "t".repeat(60)), "");
  assert.equal(logic.validateStack("Rust  lang"), "");
  for (const bad of ["", " \n ", "n".repeat(41)]) assert.notEqual(logic.validateStack(bad), "", JSON.stringify(bad));
  assert.equal(logic.squash("  a \n b\t c "), "a b c");
});

test("moving an item swaps neighbours and refuses to leave the list", () => {
  assert.deepEqual(logic.moveId([1, 2, 3], 1, -1), [2, 1, 3]);
  assert.deepEqual(logic.moveId([1, 2, 3], 1, 1), [1, 3, 2]);
  assert.equal(logic.moveId([1, 2, 3], 0, -1), null);
  assert.equal(logic.moveId([1, 2, 3], 2, 1), null);
  assert.equal(logic.moveId([1, 2, 3], 5, 1), null);
  assert.equal(logic.moveId([], 0, 1), null);
  const original = [1, 2, 3];
  logic.moveId(original, 0, 1);
  assert.deepEqual(original, [1, 2, 3], "the input is not mutated");
});

const goodJournal = () => ({
  now: "2026-10-22T12:00:00Z",
  github_username: "moch",
  repos: [{ id: 2, repo_name: "flexinix", topic: "Nix" }, { id: 3, repo_name: "dots", topic: null }],
  stack: [{ id: 1, name: "Rust" }],
  snapshots: [{ id: 9, fetched_at: "2026-10-22T06:00:00Z", source: "github" }],
  caps: { max_pinned: 12, max_stack: 12 },
});

test("journal settings are checked and renamed for the screen", () => {
  const j = logic.normalizeJournal(goodJournal());
  assert.equal(j.username, "moch");
  assert.deepEqual(j.repos[1], { id: 3, name: "dots", topic: null });
  assert.equal(j.snapshots[0].fetchedAt, "2026-10-22T06:00:00Z");
  assert.deepEqual(j.caps, { maxPinned: 12, maxStack: 12 });
  const broken = [
    null, [], {}, { ...goodJournal(), caps: null }, { ...goodJournal(), repos: "x" },
    { ...goodJournal(), repos: [{ id: 0, repo_name: "a", topic: null }] },
    { ...goodJournal(), repos: [{ id: 1, repo_name: 5, topic: null }] },
    { ...goodJournal(), stack: [{ id: 1 }] },
    { ...goodJournal(), snapshots: [{ id: 1, fetched_at: "x" }] },
    { ...goodJournal(), now: 5 },
    { ...goodJournal(), github_username: null },
    { ...goodJournal(), caps: { max_pinned: -1, max_stack: 1 } },
  ];
  for (const b of broken) assert.equal(logic.normalizeJournal(b), null, JSON.stringify(b));
});

test("guestbook entries are checked row by row and keep their text untouched", () => {
  const row = (extra = {}) => ({
    id: 5, username: "ani", url: null, message: "<img src=x onerror=alert(1)> **hi**", created_at: "2026-01-01T00:00:00Z",
    hidden_at: null, reports: 2, last_reported_at: "2026-01-02T00:00:00Z", ...extra,
  });
  const out = logic.normalizeMessages({ items: [row(), row({ id: 6, hidden_at: "2026-02-01T00:00:00Z", url: "https://a.example", reports: 0, last_reported_at: null })] });
  assert.equal(out[0].message, "<img src=x onerror=alert(1)> **hi**", "never sanitised into something else; it is rendered as text");
  assert.equal(out[1].hiddenAt, "2026-02-01T00:00:00Z");
  assert.equal(out[1].url, "https://a.example");
  assert.deepEqual(logic.normalizeMessages({ items: [] }), []);
  for (const bad of [null, {}, { items: "x" }, { items: [row({ id: -1 })] }, { items: [row({ message: 1 })] }, { items: [row({ reports: "2" })] }, { items: [null] }]) {
    assert.equal(logic.normalizeMessages(bad), null, JSON.stringify(bad));
  }
  assert.deepEqual(logic.MESSAGE_FILTERS.map((f) => f.id), ["reported", "hidden", "all"]);
});

// ----------------------------------------------------------- D6b: klien API

async function signedIn(handler) {
  const f = fakeFetch(handler);
  const api = createApi(f);
  await api.login("tok-123");
  f.calls.length = 0;
  return { api, f };
}

test("journal calls use the right method, path, JSON body and no-content handling", async () => {
  const { api, f } = await signedIn((url, init) => (init.method === "POST" && url.endsWith("/repos") ? reply(201, { id: 7 }) : init.method === "GET" ? reply(200, goodJournal()) : reply(204, null)));
  await api.journal();
  assert.equal(await api.setUsername("moch"), null);
  assert.deepEqual((await api.addRepo("flexinix", "Nix")), { id: 7 });
  await api.addRepo("plain", "");
  await api.editRepo(2, "flexinix", "Topic");
  await api.deleteRepo(2);
  await api.orderRepos([3, 2]);
  await api.addStack("Zig");
  await api.editStack(1, "Rust");
  await api.deleteStack(1);
  await api.orderStack([1]);
  await api.journalRefresh();
  const seen = f.calls.map((c) => `${c.init.method} ${c.url}`);
  assert.deepEqual(seen, [
    "GET /api/v1/admin/journal",
    "PUT /api/v1/admin/journal/profile",
    "POST /api/v1/admin/journal/repos",
    "POST /api/v1/admin/journal/repos",
    "PUT /api/v1/admin/journal/repos/2",
    "DELETE /api/v1/admin/journal/repos/2",
    "PUT /api/v1/admin/journal/repos/order",
    "POST /api/v1/admin/journal/stack",
    "PUT /api/v1/admin/journal/stack/1",
    "DELETE /api/v1/admin/journal/stack/1",
    "PUT /api/v1/admin/journal/stack/order",
    "POST /api/v1/admin/journal/refresh",
  ]);
  const body = (i) => JSON.parse(f.calls[i].init.body);
  assert.deepEqual(body(1), { github_username: "moch" });
  assert.deepEqual(body(2), { repo_name: "flexinix", topic: "Nix" });
  assert.deepEqual(body(3), { repo_name: "plain", topic: null }, "an empty topic is sent as null");
  assert.deepEqual(body(6), { ids: [3, 2] });
  for (const i of [1, 2, 4, 6, 7, 8, 10]) assert.equal(f.calls[i].init.headers["content-type"], "application/json");
  for (const i of [0, 5, 9, 11]) assert.equal(f.calls[i].init.body, undefined, "no body on " + seen[i]);
  for (const c of f.calls) {
    assert.equal(c.init.headers.authorization, "Bearer tok-123");
    assert.ok(!c.url.includes("tok-123"));
  }
});

test("guestbook calls carry the filter and cursor in the query and the id in the path", async () => {
  const { api, f } = await signedIn((url, init) => (init.method === "GET" ? reply(200, { items: [] }) : reply(204, null)));
  await api.messages("reported", null, 30);
  await api.messages("all", 41, 30);
  await api.hideMessage(5);
  await api.restoreMessage(5);
  await api.dismissReports(5);
  assert.deepEqual(f.calls.map((c) => `${c.init.method} ${c.url}`), [
    "GET /api/v1/admin/messages?filter=reported&limit=30",
    "GET /api/v1/admin/messages?filter=all&limit=30&before=41",
    "DELETE /api/v1/admin/messages/5",
    "POST /api/v1/admin/messages/5/restore",
    "DELETE /api/v1/admin/messages/5/reports",
  ]);
});

test("an id that is not a positive integer never reaches the network", async () => {
  const { api, f } = await signedIn(() => reply(204, null));
  for (const bad of [0, -1, 1.5, NaN, "5", "../admin", null, undefined, 2 ** 60]) {
    await assert.rejects(() => api.deleteRepo(bad), (e) => e instanceof ApiError && e.status === 400, String(bad));
    await assert.rejects(() => api.hideMessage(bad), (e) => e.status === 400);
  }
  assert.equal(f.calls.length, 0);
});

test("a 429 from refresh keeps its wait time and a 409 is reported as such", async () => {
  const { api } = await signedIn((url) => (url.endsWith("login") ? reply(200, goodOverview()) : url.endsWith("refresh") ? reply(429, {}, { "retry-after": "41" }) : reply(409, {})));
  await assert.rejects(() => api.journalRefresh(), (e) => e.status === 429 && e.retryAfter === 41);
  await assert.rejects(() => api.addStack("Rust"), (e) => e.status === 409);
});

test("the new screens only render server text as text", () => {
  for (const name of ["journal.js", "guestbook.js", "parts.js", "data.js", "radio.js"]) {
    const src = read(admin, name);
    assert.doesNotMatch(src, /href=|src=\$\{|target=/, `${name} must not build links or images from data`);
  }
});

// ----------------------------------------------------------- D6c: logika

const radioTable = () => ({
  name: "radio_stations", label: "Stasiun radio", editable: true, key: "id", keyLabel: "ID", keyGiven: false,
  paged: false, maxRows: 60, rows: 2,
  columns: [
    { name: "label", label: "Nama", kind: "text", nullable: false, min: 1, max: 80 },
    { name: "slogan", label: "Slogan", kind: "text", nullable: false, min: 0, max: 160 },
    { name: "stream_url", label: "Alamat stream", kind: "url", nullable: false, min: null, max: null },
    { name: "official_link", label: "Situs resmi", kind: "url", nullable: true, min: null, max: null },
    { name: "sort_order", label: "Urutan", kind: "int", nullable: false, min: -1000000, max: 1000000 },
    { name: "is_active", label: "Aktif", kind: "bool", nullable: false, min: null, max: null },
  ],
});
const wireTable = () => ({
  name: "radio_stations", label: "Stasiun radio", editable: true, key: "id", key_label: "ID", key_given: false, paged: false, max_rows: 60, rows: 2,
  columns: radioTable().columns.map((c) => ({ name: c.name, label: c.label, kind: c.kind, nullable: c.nullable, ...(c.min === null ? {} : { min: c.min, max: c.max }) })),
});

test("table list from the server is checked and renamed", () => {
  const out = logic.normalizeTables({ items: [wireTable()] });
  assert.deepEqual(out[0], radioTable());
  const broken = [
    null, {}, { items: [null] }, { items: [{ ...wireTable(), name: "Bad Name" }] }, { items: [{ ...wireTable(), editable: "yes" }] },
    { items: [{ ...wireTable(), rows: -1 }] }, { items: [{ ...wireTable(), columns: [{ name: "x", label: "x", kind: "weird", nullable: false }] }] },
    { items: [{ ...wireTable(), columns: "x" }] }, { items: [{ ...wireTable(), max_rows: "60" }] },
  ];
  for (const b of broken) assert.equal(logic.normalizeTables(b), null, JSON.stringify(b));
});

test("rows are checked against their table's column kinds", () => {
  const t = radioTable();
  const row = (extra = {}) => ({ id: 1, label: "Delta", slogan: "", stream_url: "https://a.example", official_link: null, sort_order: 2, is_active: true, ...extra });
  assert.deepEqual(logic.normalizeRows({ items: [row()] }, t), [row()]);
  for (const bad of [{ id: 0 }, { label: 5 }, { official_link: 5 }, { stream_url: null }, { sort_order: "2" }, { sort_order: 1.5 }, { is_active: 1 }]) {
    assert.equal(logic.normalizeRows({ items: [row(bad)] }, t), null, JSON.stringify(bad));
  }
  assert.equal(logic.normalizeRows({ items: "x" }, t), null);
  assert.deepEqual(logic.normalizeRows({ items: [] }, t), []);
});

test("cells read naturally", () => {
  const [label, , , official, , active] = radioTable().columns;
  assert.equal(logic.displayValue(label, "Delta"), "Delta");
  assert.equal(logic.displayValue(official, null), "—");
  assert.equal(logic.displayValue(active, true), "Ya");
  assert.equal(logic.displayValue(active, false), "Tidak");
  assert.match(logic.displayValue({ kind: "timestamp" }, "2026-10-22T00:30:00Z"), /2026/);
  assert.equal(logic.displayValue({ kind: "int" }, 0), "0", "zero is shown, not a dash");
});

test("field checks follow the server's rules and say what is wrong", () => {
  const [label, slogan, stream, official, order] = radioTable().columns;
  assert.equal(logic.validateField(label, "  Delta  FM "), "");
  assert.match(logic.validateField(label, "  "), /wajib/);
  assert.match(logic.validateField(label, "x".repeat(81)), /paling banyak 80/);
  assert.equal(logic.validateField(slogan, ""), "", "min 0 allows empty");
  assert.equal(logic.validateField(official, ""), "", "nullable url may be empty");
  assert.match(logic.validateField(stream, ""), /wajib/);
  for (const bad of ["javascript:alert(1)", "ftp://a.example", "//a.example", "https://", "https://a b", "a.example", "/relative"]) {
    assert.notEqual(logic.validateField(stream, bad), "", bad);
  }
  assert.equal(logic.validateField(stream, "https://a.example/x?y=1"), "");
  assert.equal(logic.validateField(stream, "http://a.example"), "");
  const route = { name: "link", label: "Tautan", kind: "route", nullable: false };
  for (const ok of ["/profile", "/", "https://a.example"]) assert.equal(logic.validateField(route, ok), "", ok);
  for (const bad of ["//evil.example", "/\\evil", "javascript:x", "", "/a b"]) assert.notEqual(logic.validateField(route, bad), "", bad);
  assert.equal(logic.validateField(order, "-5"), "");
  for (const bad of ["", "1.5", "abc", "1e3", "1000001", " "]) assert.notEqual(logic.validateField(order, bad), "", bad);
  const date = { name: "d", label: "Tanggal", kind: "date", nullable: false };
  assert.equal(logic.validateField(date, "2026-02-28"), "");
  for (const bad of ["2026-13-01", "2026-1-1", "01/02/2026", "", "2026-10-32"]) assert.notEqual(logic.validateField(date, bad), "", bad);
});

test("drafts become request bodies with the right types and nulls", () => {
  const t = radioTable();
  assert.deepEqual(logic.emptyDraft(t), { label: "", slogan: "", stream_url: "", official_link: "", sort_order: "0", is_active: true });
  const draft = { label: "  Delta   FM ", slogan: "", stream_url: " https://a.example ", official_link: "  ", sort_order: " 3 ", is_active: false };
  assert.equal(logic.validateDraft(t, draft, true), "");
  assert.deepEqual(logic.buildBody(t, draft, true), { label: "Delta FM", slogan: "", stream_url: "https://a.example", official_link: null, sort_order: 3, is_active: false });
  assert.match(logic.validateDraft(t, { ...draft, label: "" }, true), /Nama/);
  const row = { id: 4, label: "Jak", slogan: "s", stream_url: "https://j.example", official_link: null, sort_order: -2, is_active: true };
  assert.deepEqual(logic.draftFromRow(t, row), { label: "Jak", slogan: "s", stream_url: "https://j.example", official_link: "", sort_order: "-2", is_active: true });
  assert.ok(!("id" in logic.buildBody(t, logic.draftFromRow(t, row), false)), "the key is never in the body");
});

test("a table whose key is chosen by the admin asks for it and sends it as a number", () => {
  const t = { ...radioTable(), key: "number", keyLabel: "Nomor", keyGiven: true, columns: [radioTable().columns[0]] };
  const draft = { number: " 6 ", label: "Baru" };
  assert.equal(logic.validateDraft(t, draft, true), "");
  assert.deepEqual(logic.buildBody(t, draft, true), { number: 6, label: "Baru" });
  assert.deepEqual(logic.buildBody(t, draft, false), { label: "Baru" }, "never sent on update");
  for (const bad of ["", "0", "-1", "1.5", "abc", "1234567890"]) assert.match(logic.validateDraft(t, { ...draft, number: bad }, true), /Nomor/, bad);
  assert.equal(logic.validateDraft(t, { number: "", label: "x" }, false), "");
});

test("preset contents are ordered by position and checked", () => {
  const raw = { items: [{ preset_id: 1, embience_id: 12, position: 1 }, { preset_id: 1, embience_id: 6, position: 0 }, { preset_id: 2, embience_id: 7, position: 0 }] };
  const items = logic.normalizePresetItems(raw);
  assert.deepEqual(logic.presetIds(items, 1), [6, 12]);
  assert.deepEqual(logic.presetIds(items, 9), []);
  for (const bad of [null, { items: [{ preset_id: 0, embience_id: 1, position: 0 }] }, { items: [{ preset_id: 1, embience_id: 1 }] }, { items: "x" }]) {
    assert.equal(logic.normalizePresetItems(bad), null);
  }
  assert.equal(logic.downloadName("radio_stations", "csv"), "radio_stations.csv");
  assert.equal(logic.downloadName("radio_stations", "json"), "radio_stations.json");
  assert.equal(logic.downloadName("radio_stations", "../x"), "radio_stations.json");
});

test("history labels cover the generic table actions and targets", () => {
  assert.equal(logic.targetLabel("bookmarks:5"), "Bookmark #5");
  assert.equal(logic.targetLabel("embience_presets:2"), "Preset suara latar #2");
  assert.equal(logic.targetLabel("radio_stations"), "Stasiun radio");
  assert.equal(logic.targetLabel("unknown_table:5"), "unknown_table:5");
  assert.equal(logic.actionLabel("table.insert"), "Tambah data");
  assert.equal(logic.actionLabel("message.purge"), "Hapus permanen entri Guestbook");
});

// ------------------------------------------------------------ D6c: klien API

test("data calls use the table and id in the path and never anything else", async () => {
  const { api, f } = await signedIn((url, init) => (init.method === "GET" ? reply(200, url.includes("export") ? "x" : { items: [] }) : reply(init.method === "POST" && url.endsWith("/rows") ? 201 : 204, { id: 3 })));
  await api.tables();
  await api.tableRows("bookmarks", null, 50);
  await api.tableRows("chat_messages", 12, 50);
  await api.insertRow("bookmarks", { label: "a" });
  await api.updateRow("bookmarks", 3, { label: "b" });
  await api.deleteRow("bookmarks", 3);
  await api.presetItems();
  await api.setPresetItems(2, [3, 1]);
  await api.purgeMessage(9);
  assert.deepEqual(f.calls.map((c) => `${c.init.method} ${c.url}`), [
    "GET /api/v1/admin/tables",
    "GET /api/v1/admin/tables/bookmarks/rows?limit=50",
    "GET /api/v1/admin/tables/chat_messages/rows?limit=50&before=12",
    "POST /api/v1/admin/tables/bookmarks/rows",
    "PUT /api/v1/admin/tables/bookmarks/rows/3",
    "DELETE /api/v1/admin/tables/bookmarks/rows/3",
    "GET /api/v1/admin/presets/items",
    "PUT /api/v1/admin/presets/2/items",
    "POST /api/v1/admin/messages/9/purge",
  ]);
  assert.deepEqual(JSON.parse(f.calls[7].init.body), { ids: [3, 1] });
  assert.deepEqual(JSON.parse(f.calls[8].init.body), { confirm: "purge:9" });
});

test("table names that are not plain lowercase never reach the network", async () => {
  const { api, f } = await signedIn(() => reply(200, { items: [] }));
  for (const bad of ["", "Bookmarks", "../admin", "a/b", "a b", "x".repeat(41), null, undefined, 5, "bookmarks?x=1"]) {
    await assert.rejects(() => api.tableRows(bad, null, 5), (e) => e instanceof ApiError && e.status === 400, String(bad));
    await assert.rejects(() => api.exportTable(bad, "csv"), (e) => e.status === 400);
  }
  assert.equal(f.calls.length, 0);
});

test("export reads the file as text and only ever asks for json or csv", async () => {
  const text = "id,label\r\n1,a\r\n";
  const { api, f } = await signedIn((url) => url.endsWith("login")
    ? reply(200, goodOverview())
    : { ok: true, status: 200, headers: { get: () => null }, text: async () => text, json: async () => { throw new Error("no json"); } });
  assert.equal(await api.exportTable("bookmarks", "csv"), text);
  await api.exportTable("bookmarks", "json");
  await api.exportTable("bookmarks", "xml; DROP");
  assert.deepEqual(f.calls.map((c) => c.url), [
    "/api/v1/admin/tables/bookmarks/export?format=csv",
    "/api/v1/admin/tables/bookmarks/export?format=json",
    "/api/v1/admin/tables/bookmarks/export?format=json",
  ]);
});

test("the data tab keeps every table decision on the server", () => {
  const src = read(admin, "data.js");
  assert.doesNotMatch(src, /"(generations|bookmarks|radio_stations|admin_audit|journal_[a-z_]+)"/, "no table list in the page");
  assert.equal([...src.matchAll(/"chat_messages"/g)].length, 1, "one hint pointing to the Guestbook tab");
  assert.match(src, /embience_presets/, "only the preset editor is special-cased");
  assert.equal([...src.matchAll(/"embience[a-z_]*"/g)].length, 2, "just the preset table name and the embience list the preset editor needs");
});

// ------------------------------------------------------------ D7b: Radio

const wireStation = (over = {}) => ({
  id: 1, label: "Delta FM", is_active: true, checkable: true, link_changed: false,
  last_result: "ok", last_status: 200, last_checked_at: "2026-10-24T11:17:00Z", last_ok_at: "2026-10-24T11:17:00Z",
  failing_since: null, consecutive_failures: 0, disabled_by_check: false, ...over,
});
const wireRadio = (items = [wireStation()]) => ({
  now: "2026-10-24T12:00:00Z",
  next_run_at: "2026-10-24T12:17:00Z",
  rules: { fail_threshold: 3, min_fail_hours: 6, batch: 12, min_manual_secs: 60 },
  items,
});
const rules = { failThreshold: 3, minFailHours: 6, batch: 12, minManualSecs: 60 };
const NOW = "2026-10-24T12:00:00Z";
const station = (over, r = rules) => {
  const radio = logic.normalizeRadio(wireRadio([wireStation(over)]));
  return logic.describeStation(radio.items[0], r, NOW);
};

test("radio status is checked row by row and renamed for the screen", () => {
  const radio = logic.normalizeRadio(wireRadio([wireStation(), wireStation({ id: 2, label: "Off", is_active: false, last_result: null, last_checked_at: null })]));
  assert.equal(radio.nextRunAt, "2026-10-24T12:17:00Z");
  assert.deepEqual(radio.rules, rules);
  assert.equal(radio.items.length, 2);
  assert.deepEqual(radio.items[0], {
    id: 1, label: "Delta FM", isActive: true, checkable: true, linkChanged: false, result: "ok", status: 200,
    checkedAt: "2026-10-24T11:17:00Z", okAt: "2026-10-24T11:17:00Z", failingSince: null, failures: 0, disabledByCheck: false,
  });
  assert.equal(radio.items[1].result, null);
  for (const bad of [
    null, [], {}, { ...wireRadio(), items: "x" }, { ...wireRadio(), rules: null }, { ...wireRadio(), now: 5 },
    { ...wireRadio(), rules: { ...wireRadio().rules, batch: -1 } },
    wireRadio([wireStation({ id: 0 })]),
    wireRadio([wireStation({ label: 7 })]),
    wireRadio([wireStation({ last_result: "weird" })]),
    wireRadio([wireStation({ consecutive_failures: "3" })]),
  ]) assert.equal(logic.normalizeRadio(bad), null, JSON.stringify(bad)?.slice(0, 60));
});

test("every station state is explained in Indonesian with a tone", () => {
  assert.deepEqual([station({}).tone, station({}).label], ["ok", "Sehat"]);
  assert.match(station({}).detail, /Dicek 43 menit lalu/);

  const failing = station({ last_result: "http", last_status: 403, consecutive_failures: 2, failing_since: "2026-10-24T03:17:00Z" });
  assert.equal(failing.tone, "warn");
  assert.equal(failing.label, "Gagal 2×");
  assert.match(failing.detail, /menjawab 403/);
  assert.match(failing.detail, /3 kegagalan berturut-turut dan minimal 6 jam/);
  assert.match(station({ last_result: "timeout", consecutive_failures: 1 }).detail, /terlalu lama/);
  assert.match(station({ last_result: "network", consecutive_failures: 1 }).detail, /tidak bisa tersambung/);
  assert.match(station({ last_result: "redirect", last_status: 302, consecutive_failures: 1 }).detail, /pengalihan/);
  assert.match(station({ last_result: "http", last_status: null, consecutive_failures: 1 }).detail, /menolak/);

  const auto = station({ is_active: false, disabled_by_check: true, consecutive_failures: 3 });
  assert.deepEqual([auto.tone, auto.label], ["bad", "Dimatikan otomatis"]);
  assert.match(auto.detail, /tab Data/);
  const manual = station({ is_active: false });
  assert.deepEqual([manual.tone, manual.label], ["muted", "Nonaktif"]);

  assert.equal(station({ link_changed: true }).label, "Tautan baru");
  assert.equal(station({ checkable: false }).label, "Tidak diperiksa");
  assert.match(station({ checkable: false }).detail, /tidak akan dimatikan/);
  assert.equal(station({ last_result: null, last_checked_at: null }).label, "Belum diperiksa");
  assert.equal(station({ last_result: "skipped", last_status: 302 }).label, "Tidak diperiksa");
  const unsure = station({ last_result: "inconclusive", consecutive_failures: 2 });
  assert.deepEqual([unsure.tone, unsure.label], ["warn", "Belum meyakinkan"]);
  assert.match(unsure.detail, /Sebelumnya gagal 2 kali/);
  assert.equal(station({ last_result: "inconclusive" }).tone, "muted");
  // Penjelasan "dimatikan otomatis" tidak berlaku untuk stasiun yang masih aktif tetapi tak bisa diperiksa.
  assert.doesNotMatch(station({ checkable: false }).detail, /Dimatikan otomatis setelah/);
});

test("the manual check button waits out the server's minimum gap, by the server's clock", () => {
  const radio = (checkedAt, now = NOW) => ({ ...logic.normalizeRadio(wireRadio([wireStation({ last_checked_at: checkedAt })])), now });
  assert.equal(logic.manualWait(radio("2026-10-24T11:59:30Z")), 30);
  assert.equal(logic.manualWait(radio("2026-10-24T11:59:00Z")), 0);
  assert.equal(logic.manualWait(radio("2026-10-24T11:17:00Z")), 0);
  assert.equal(logic.manualWait(radio(null)), 0, "never checked");
  assert.equal(logic.manualWait(radio("2026-10-24T18:00:00Z")), 60, "a clock that ran backwards never locks it for longer than the gap");
  assert.equal(logic.manualWait({ ...radio("2026-10-24T11:59:30Z"), now: "nonsense" }), 0);
});

test("the manual check result is checked and summarised with station names", () => {
  const wire = { checked: 5, ok: 2, failed: 1, skipped: 2, inconclusive: 0, disabled: [3] };
  const report = logic.normalizeRadioReport(wire);
  assert.deepEqual(report, { ...wire });
  const line = logic.describeRadioReport(report, (id) => (id === 3 ? "Delta FM" : "?"));
  assert.match(line, /5 stasiun diperiksa, 2 sehat, 1 gagal, 2 dilewati\./);
  assert.match(line, /Dimatikan otomatis: Delta FM\./);
  assert.doesNotMatch(logic.describeRadioReport(logic.normalizeRadioReport({ ...wire, failed: 0, skipped: 0, disabled: [] }), String), /Dimatikan|gagal|dilewati/);
  assert.match(logic.describeRadioReport(logic.normalizeRadioReport({ checked: 0, ok: 0, failed: 0, skipped: 0, inconclusive: 0, disabled: [] }), String), /Tidak ada stasiun aktif/);
  for (const bad of [null, {}, { ...wire, checked: -1 }, { ...wire, disabled: [0] }, { ...wire, disabled: "x" }, { ...wire, ok: "2" }]) {
    assert.equal(logic.normalizeRadioReport(bad), null, JSON.stringify(bad));
  }
});

test("radio calls use GET for the status and POST for the check, with no body and no query", async () => {
  const { api, f } = await signedIn((url, init) => (init.method === "GET" ? reply(200, wireRadio()) : reply(200, { checked: 0, ok: 0, failed: 0, skipped: 0, inconclusive: 0, disabled: [] })));
  assert.ok(logic.normalizeRadio(await api.radio()));
  assert.ok(logic.normalizeRadioReport(await api.radioCheck()));
  assert.deepEqual(f.calls.slice(-2).map((c) => `${c.init.method} ${c.url}`), ["GET /api/v1/admin/radio", "POST /api/v1/admin/radio/check"]);
  assert.equal(f.calls.at(-1).init.body, undefined);
});

test("a 429 from the manual check keeps its wait time", async () => {
  const { api } = await signedIn((url) => (url.endsWith("/radio/check") ? reply(429, { error: {} }, { "retry-after": "42" }) : reply(200, goodOverview())));
  await assert.rejects(api.radioCheck(), (e) => e.status === 429 && e.retryAfter === 42);
});

test("the radio screen never asks for or shows stream URLs", () => {
  for (const name of ["radio.js", "logic.js"]) {
    const src = read(admin, name);
    assert.doesNotMatch(src, /stream_url|checked_url|streamUrl|checkedUrl/, name);
  }
  assert.match(read(admin, "app.js"), /id: "radio", label: "Radio"/);
});
