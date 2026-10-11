// Menghasilkan dan memvalidasi fixture protokol Turso memakai implementasi
// REFERENSI resmi (`@libsql/hrana-client`), supaya klien Rust (`turso-http`)
// diuji terhadap bentuk JSON yang sebenarnya, bukan terhadap tebakan kita.
//
//   mkdir /tmp/hrana && cd /tmp/hrana && npm init -y >/dev/null \
//     && npm install --ignore-scripts @libsql/hrana-client
//   HRANA_CLIENT_DIR=/tmp/hrana/node_modules/@libsql/hrana-client \
//     node <repo>/scripts/gen_golden.mjs
//
// - `req_*.json`  : DITULIS ulang tiap dijalankan, keluaran encoder referensi.
// - `resp_*.json` : ditulis tangan; skrip ini memastikan decoder referensi
//                   menerimanya (artinya bentuknya valid menurut klien resmi).
import { readFileSync, writeFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = process.env.HRANA_CLIENT_DIR;
if (!root) {
  console.error("Set HRANA_CLIENT_DIR ke folder paket @libsql/hrana-client");
  process.exit(2);
}
const lib = (p) => import(pathToFileURL(join(root, "lib-esm", p)).href);
const { writeJsonObject } = await lib("encoding/json/encode.js");
const { PipelineReqBody } = await lib("http/json_encode.js");
const { PipelineRespBody } = await lib("http/json_decode.js");

const fixtures = join(dirname(fileURLToPath(import.meta.url)), "..", "crates", "turso-http", "tests", "fixtures");
const emit = (name, msg) => {
  const json = writeJsonObject(msg, PipelineReqBody);
  writeFileSync(join(fixtures, name), JSON.stringify(JSON.parse(json), null, 2) + "\n");
  console.log("wrote", name);
};

const stmt = (sql, args = [], namedArgs = [], wantRows = true) => ({
  sql, sqlId: undefined, args, namedArgs, wantRows,
});

// 1. execute dengan semua tipe nilai + parameter bernama
emit("req_execute.json", {
  baton: undefined,
  requests: [
    {
      type: "execute",
      stmt: stmt(
        "SELECT ?, ?, ?, ?, ?, :name",
        [1234567890123n, 2.5, 'héllo "dunia" \\ \n', null, new Uint8Array([0, 1, 255])],
        [{ name: "name", value: "Turso" }],
      ),
    },
    { type: "close" },
  ],
});

// 2. transaksi dua statement, urutan langkah sama dengan executeHranaBatch
//    di @libsql/client (BEGIN IMMEDIATE, ok(prev), COMMIT, ROLLBACK)
const ok = (step) => ({ type: "ok", step });
emit("req_transaction.json", {
  baton: undefined,
  requests: [
    {
      type: "batch",
      batch: {
        steps: [
          { condition: undefined, stmt: stmt("BEGIN IMMEDIATE", [], [], false) },
          { condition: ok(0), stmt: stmt("INSERT INTO t(a) VALUES (?)", [1n]) },
          { condition: ok(1), stmt: stmt("INSERT INTO t(a) VALUES (?)", [2n]) },
          { condition: ok(2), stmt: stmt("COMMIT", [], [], false) },
          { condition: { type: "not", cond: ok(3) }, stmt: stmt("ROLLBACK", [], [], false) },
        ],
      },
    },
    { type: "close" },
  ],
});

// 3. validasi fixture respons dengan decoder referensi
for (const f of readdirSync(fixtures).filter((n) => n.startsWith("resp_")).sort()) {
  const body = JSON.parse(readFileSync(join(fixtures, f), "utf8"));
  const decoded = PipelineRespBody(body);
  console.log("decoder referensi menerima", f, `(${decoded.results.length} hasil)`);
}
