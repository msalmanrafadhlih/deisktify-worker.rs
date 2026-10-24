// Pemeriksaan statis .github/workflows/ci.yml (D8a): urutan job, rahasia, versi dipin.
// Tanpa pustaka YAML (tidak ada npm di CI); yang diperiksa teksnya.
//
//   node --test scripts/workflow.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const ci = readFileSync(join(root, ".github/workflows/ci.yml"), "utf8");

/** Isi satu job (dari `  nama:` sampai job berikutnya atau akhir berkas). */
function job(name) {
  const start = ci.search(new RegExp(`^  ${name}:\\n`, "m"));
  assert.notEqual(start, -1, `job ${name} exists`);
  const rest = ci.slice(start + 1);
  const next = rest.search(/^  [a-z][a-z-]*:\n/m);
  return next === -1 ? rest : rest.slice(0, next);
}

const code = (text) => text.replace(/^\s*#.*$/gm, "");

test("deploy runs only after migrate, only from a version tag, in the production environment", () => {
  const deploy = job("deploy");
  assert.match(deploy, /^    needs: \[migrate\]$/m);
  const cond = /^    if: (.+)$/m.exec(deploy)?.[1] ?? "";
  assert.match(cond, /github\.event_name == 'push'/);
  assert.match(cond, /startsWith\(github\.ref, 'refs\/tags\/v'\)/);
  assert.doesNotMatch(cond, /workflow_dispatch|pull_request|\|\|/, "no manual or PR path to production");
  assert.match(deploy, /^    environment: production$/m);
  assert.match(deploy, /^    permissions:\n      contents: read$/m);
  assert.match(deploy, /cancel-in-progress: false/, "a second deploy waits instead of cancelling the first");
});

test("migrate still gates on every check, and deploy therefore does too", () => {
  assert.match(job("migrate"), /^    needs: \[native, wasm, reference-fixtures, sql\]$/m);
});

test("the pull request path never reaches production", () => {
  assert.doesNotMatch(code(ci), /pull_request_target/);
  for (const name of ["migrate", "deploy"]) {
    assert.doesNotMatch(/^    if: (.+)$/m.exec(job(name))[1], /pull_request/, name);
  }
});

test("only the two Cloudflare secrets reach the deploy job; Worker secrets stay in Cloudflare", () => {
  const used = [...code(job("deploy")).matchAll(/secrets\.([A-Z0-9_]+)/g)].map((m) => m[1]);
  assert.deepEqual([...new Set(used)].sort(), ["CLOUDFLARE_ACCOUNT_ID", "CLOUDFLARE_API_TOKEN"]);
  assert.doesNotMatch(code(job("deploy")), /ADMIN_TOKEN|TURSO_|RATE_LIMIT_SALT|BANNED_WORDS|GITHUB_TOKEN/);
  // Token Cloudflare tidak boleh ada di job lain (hak sekecil mungkin).
  for (const name of ["native", "wasm", "reference-fixtures", "sql", "migrate"]) {
    assert.doesNotMatch(job(name), /CLOUDFLARE_/, name);
  }
});

test("secret values are never echoed", () => {
  const deploy = code(job("deploy"));
  assert.doesNotMatch(deploy, /echo[^\n]*\$\{?CLOUDFLARE_(API_TOKEN|ACCOUNT_ID)/);
  assert.doesNotMatch(deploy, /\bset -x\b|--verbose|\bdebug\b/i);
});

test("wrangler and worker-build are pinned to exact versions", () => {
  const wrangler = /WRANGLER_VERSION: "(\d+\.\d+\.\d+)"/.exec(ci)?.[1];
  assert.ok(wrangler, "WRANGLER_VERSION is an exact x.y.z");
  assert.match(ci, /npx --yes "wrangler@\$\{WRANGLER_VERSION\}" deploy/);
  assert.doesNotMatch(code(ci), /wrangler@(latest|\^|~)|npx[^\n]* wrangler(?!@)/);

  const builds = [...ci.matchAll(/cargo install worker-build --version (\S+) --locked/g)].map((m) => m[1]);
  assert.equal(builds.length, 2, "wasm job and deploy job");
  assert.equal(new Set(builds).size, 1, "same worker-build everywhere");
  const toml = readFileSync(join(root, "crates/api/wrangler.toml"), "utf8");
  assert.ok(toml.includes(`cargo install worker-build --version ${builds[0]} --locked`), "wrangler.toml names the same version");
});

test("the Rust toolchain in CI matches rust-toolchain.toml", () => {
  const pinned = /channel = "([^"]+)"/.exec(readFileSync(join(root, "rust-toolchain.toml"), "utf8"))[1];
  const used = [...ci.matchAll(/dtolnay\/rust-toolchain@(\S+)/g)].map((m) => m[1]);
  assert.ok(used.length >= 4);
  assert.deepEqual([...new Set(used)], [pinned]);
});

test("only first-party and well known actions are used (the deploy token must not meet other code)", () => {
  const actions = [...code(ci).matchAll(/uses: ([^\s@]+)@(\S+)/g)].map((m) => m[1]);
  const allowed = new Set(["actions/checkout", "actions/setup-node", "dtolnay/rust-toolchain", "Swatinem/rust-cache"]);
  for (const a of actions) assert.ok(allowed.has(a), `${a} is not on the allowlist`);
  assert.doesNotMatch(ci, /cloudflare\/wrangler-action/);
});

test("the deploy step runs from the folder with wrangler.toml and the smoke check only reads public URLs", () => {
  const deploy = job("deploy");
  assert.match(deploy, /working-directory: crates\/api\n\s+run: npx --yes/);
  const smoke = deploy.slice(deploy.indexOf("Cek Worker yang baru tayang"));
  const urls = [...code(smoke).matchAll(/^\s*curl .*"\$API_BASE_URL([^"]*)"\s*$/gm)].map((m) => m[1]).sort();
  assert.deepEqual(urls, ["/admin/", "/api/v1/health"]);
  assert.doesNotMatch(code(smoke), /authorization|ADMIN_TOKEN|bearer|-X (POST|PUT|DELETE)|--data/i, "read-only, no credentials");
});
