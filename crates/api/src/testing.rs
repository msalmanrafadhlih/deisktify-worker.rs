//! Peralatan tes bersama (hanya `cfg(test)`): transport palsu yang menjawab
//! dari antrian dan mencatat permintaannya, serta pembangun balasan Hrana.
//!
//! `chat.rs` dan `journal.rs` masih punya salinan sendiri dari `Script`; modul
//! baru memakai yang ini.

use std::cell::RefCell;
use std::future::Future;

use serde_json::{json, Value as Json};
use turso_http::{Client, HttpResponse, Transport};

/// Menjawab setiap POST dengan balasan berikutnya dari antrian.
pub struct Script {
    replies: RefCell<Vec<Result<HttpResponse, String>>>,
    seen: RefCell<Vec<Json>>,
}

impl Script {
    pub fn new(replies: Vec<Result<HttpResponse, String>>) -> Self {
        let mut replies = replies;
        replies.reverse();
        Self {
            replies: RefCell::new(replies),
            seen: RefCell::new(Vec::new()),
        }
    }

    /// Jumlah permintaan HTTP yang sudah dikirim ke database.
    pub fn requests(&self) -> usize {
        self.seen.borrow().len()
    }

    /// Seluruh badan permintaan ke-`i` sebagai teks (untuk `contains`).
    pub fn body(&self, i: usize) -> String {
        self.seen.borrow()[i].to_string()
    }

    /// Argumen terikat pada statement ke-`stmt` dari permintaan ke-`i` yang
    /// berupa `execute` (stmt 0) atau `batch` (langkah ke-`stmt`, BEGIN = 0).
    pub fn args(&self, i: usize, stmt: usize) -> Vec<Json> {
        let seen = self.seen.borrow();
        let req = &seen[i]["requests"][0];
        let node = if req["type"] == "batch" {
            &req["batch"]["steps"][stmt]["stmt"]
        } else {
            &req["stmt"]
        };
        node["args"].as_array().cloned().unwrap_or_default()
    }
}

impl Transport for &Script {
    fn post(
        &self,
        _: &str,
        _: Option<&str>,
        body: String,
    ) -> impl Future<Output = Result<HttpResponse, String>> {
        self.seen
            .borrow_mut()
            .push(serde_json::from_str(&body).unwrap());
        let reply = self
            .replies
            .borrow_mut()
            .pop()
            .expect("more requests than scripted replies");
        async move { reply }
    }
}

pub fn client(s: &Script) -> Client<&Script> {
    Client::new("http://127.0.0.1:1", None, s).unwrap()
}

fn cell(v: &Json) -> Json {
    match v {
        Json::Null => json!({"type": "null"}),
        Json::Number(n) => json!({"type": "integer", "value": n.to_string()}),
        Json::String(s) => json!({"type": "text", "value": s}),
        other => panic!("unsupported {other}"),
    }
}

/// Satu hasil statement Hrana (`cols` + `rows`).
pub fn result(cols: &[&str], rows: &[Vec<Json>]) -> Json {
    json!({
        "cols": cols.iter().map(|c| json!({"name": c})).collect::<Vec<_>>(),
        "rows": rows.iter().map(|r| r.iter().map(cell).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "affected_row_count": 0
    })
}

pub fn ok(body: Json) -> Result<HttpResponse, String> {
    Ok(HttpResponse {
        status: 200,
        body: body.to_string(),
    })
}

/// Balasan untuk `client.execute`.
pub fn execute_reply(res: Json) -> Result<HttpResponse, String> {
    ok(json!({"results": [
        {"type": "ok", "response": {"type": "execute", "result": res}},
        {"type": "ok", "response": {"type": "close"}}
    ]}))
}

/// Balasan untuk `client.transaction` yang berhasil; `stmts` adalah hasil tiap
/// statement (BEGIN/COMMIT ditambahkan di sini).
pub fn tx_reply(stmts: Vec<Json>) -> Result<HttpResponse, String> {
    let n = stmts.len();
    let mut steps = vec![result(&[], &[])];
    steps.extend(stmts);
    steps.push(json!(null));
    steps.push(json!(null));
    ok(json!({"results": [
        {"type": "ok", "response": {"type": "batch", "result": {
            "step_results": steps,
            "step_errors": vec![json!(null); n + 3]
        }}},
        {"type": "ok", "response": {"type": "close"}}
    ]}))
}
