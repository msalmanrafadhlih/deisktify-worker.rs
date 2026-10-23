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

const jsFiles = ["app.js", "api.js", "logic.js", "parts.js", "journal.js", "guestbook.js"];

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
  for (const name of ["journal.js", "guestbook.js", "parts.js"]) {
    const src = read(admin, name);
    assert.doesNotMatch(src, /href=|src=\$\{|target=/, `${name} must not build links or images from data`);
  }
});
