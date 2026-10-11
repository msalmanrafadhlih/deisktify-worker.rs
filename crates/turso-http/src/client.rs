//! Klien: membangun request pipeline, mengirim lewat [`Transport`], dan
//! menerjemahkan respons ke [`Error`]/[`StmtResult`].

use std::fmt;
use std::future::Future;

use crate::proto::{
    Batch, BatchCond, BatchStep, PipelineRequest, PipelineResponse, Stmt, StmtResult,
    StreamRequest, StreamResponse, StreamResult,
};

/// Respons HTTP mentah dari transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

/// Cara mengirim `POST` JSON. Diimplementasikan oleh `fetch` di Worker dan
/// oleh transport palsu di test. Tidak mensyaratkan `Send` (future di wasm32
/// memang tidak `Send`).
pub trait Transport {
    /// `bearer`, jika ada, dikirim sebagai `Authorization: Bearer ...`.
    /// `Err` berisi deskripsi kegagalan jaringan (tanpa rahasia).
    fn post(
        &self,
        url: &str,
        bearer: Option<&str>,
        body: String,
    ) -> impl Future<Output = Result<HttpResponse, String>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// URL atau pengaturan klien tidak valid.
    Config(String),
    /// Jaringan gagal sebelum ada respons HTTP.
    Transport(String),
    /// Server menjawab non-2xx.
    Http { status: u16, message: String },
    /// Respons tidak sesuai protokol.
    Protocol(String),
    /// Server menolak statement (SQL salah, constraint, dsb.).
    Sql {
        message: String,
        code: Option<String>,
    },
    /// Statement ke-`index` (mulai 0) dalam transaksi gagal; transaksi di-rollback.
    Step {
        index: usize,
        message: String,
        code: Option<String>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Config(m) => write!(f, "invalid database config: {m}"),
            Error::Transport(m) => write!(f, "database request failed: {m}"),
            Error::Http { status, message } => {
                write!(f, "database returned HTTP {status}: {message}")
            }
            Error::Protocol(m) => write!(f, "unexpected database response: {m}"),
            Error::Sql { message, code } => match code {
                Some(c) => write!(f, "sql error [{c}]: {message}"),
                None => write!(f, "sql error: {message}"),
            },
            Error::Step { index, message, .. } => {
                write!(f, "statement {index} of the transaction failed: {message}")
            }
        }
    }
}

impl std::error::Error for Error {}

/// Klien untuk satu database Turso.
pub struct Client<T> {
    base_url: String,
    token: Option<String>,
    transport: T,
}

// Token tidak boleh muncul di log lewat `{:?}`.
impl<T> fmt::Debug for Client<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.base_url)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish_non_exhaustive()
    }
}

/// Normalisasi URL database. `libsql://` dan `wss://` dipetakan ke `https://`,
/// `ws://` ke `http://`; `http://` dipertahankan untuk `turso dev` lokal.
fn normalize_base_url(raw: &str) -> Result<String, Error> {
    let raw = raw.trim();
    let (scheme, rest) = if let Some(r) = raw.strip_prefix("libsql://") {
        ("https://", r)
    } else if let Some(r) = raw.strip_prefix("wss://") {
        ("https://", r)
    } else if let Some(r) = raw.strip_prefix("https://") {
        ("https://", r)
    } else if let Some(r) = raw.strip_prefix("ws://") {
        ("http://", r)
    } else if let Some(r) = raw.strip_prefix("http://") {
        ("http://", r)
    } else {
        return Err(Error::Config(
            "URL must start with libsql://, https:// or http://".into(),
        ));
    };
    let rest = rest.trim_end_matches('/');
    if rest.is_empty()
        || rest.starts_with('/')
        || rest.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(Error::Config("URL has no valid host".into()));
    }
    Ok(format!("{scheme}{rest}"))
}

/// Memotong pesan error dari server supaya tidak membengkak di log.
fn short(msg: &str) -> String {
    const MAX: usize = 200;
    let mut out: String = msg.chars().take(MAX).collect();
    if msg.chars().count() > MAX {
        out.push('…');
    }
    out
}

/// Susun batch transaksi seperti klien referensi `@libsql/client`:
/// `BEGIN IMMEDIATE`; tiap statement berjalan hanya jika langkah sebelumnya
/// ok; `COMMIT` hanya jika statement terakhir ok; `ROLLBACK` jika COMMIT tidak ok.
///
/// Indeks langkah: BEGIN = 0, statement ke-i = i + 1, COMMIT = n + 1, ROLLBACK = n + 2.
pub fn transaction_batch(stmts: Vec<Stmt>) -> Batch {
    let n = stmts.len();
    let mut steps = Vec::with_capacity(n + 3);
    steps.push(BatchStep {
        condition: None,
        stmt: Stmt::new("BEGIN IMMEDIATE").no_rows(),
    });
    for (i, stmt) in stmts.into_iter().enumerate() {
        steps.push(BatchStep {
            condition: Some(BatchCond::Ok { step: i as u32 }),
            stmt,
        });
    }
    steps.push(BatchStep {
        condition: Some(BatchCond::Ok { step: n as u32 }),
        stmt: Stmt::new("COMMIT").no_rows(),
    });
    steps.push(BatchStep {
        condition: Some(BatchCond::Not {
            cond: Box::new(BatchCond::Ok { step: n as u32 + 1 }),
        }),
        stmt: Stmt::new("ROLLBACK").no_rows(),
    });
    Batch { steps }
}

impl<T: Transport> Client<T> {
    /// `url`: `libsql://...`, `https://...`, atau `http://127.0.0.1:PORT`.
    /// `token` kosong dianggap tidak ada (database lokal tanpa auth).
    pub fn new(url: &str, token: Option<&str>, transport: T) -> Result<Self, Error> {
        Ok(Self {
            base_url: normalize_base_url(url)?,
            token: token
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_owned),
            transport,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Menjalankan satu statement.
    pub async fn execute(&self, stmt: Stmt) -> Result<StmtResult, Error> {
        let resp = self
            .pipeline(vec![StreamRequest::Execute { stmt }, StreamRequest::Close])
            .await?;
        match resp.results.into_iter().next() {
            Some(StreamResult::Ok {
                response: StreamResponse::Execute { result },
            }) => Ok(result),
            Some(StreamResult::Error { error }) => Err(Error::Sql {
                message: short(&error.message),
                code: error.code,
            }),
            _ => Err(Error::Protocol("expected an execute result".into())),
        }
    }

    /// Menjalankan beberapa statement dalam satu transaksi atomik. Hasilnya
    /// satu [`StmtResult`] per statement, berurutan. Jika ada yang gagal,
    /// semuanya di-rollback dan `Error::Step` menunjuk statement penyebabnya.
    pub async fn transaction(&self, stmts: Vec<Stmt>) -> Result<Vec<StmtResult>, Error> {
        if stmts.is_empty() {
            return Ok(Vec::new());
        }
        let n = stmts.len();
        let resp = self
            .pipeline(vec![
                StreamRequest::Batch {
                    batch: transaction_batch(stmts),
                },
                StreamRequest::Close,
            ])
            .await?;
        let result = match resp.results.into_iter().next() {
            Some(StreamResult::Ok {
                response: StreamResponse::Batch { result },
            }) => result,
            Some(StreamResult::Error { error }) => {
                return Err(Error::Sql {
                    message: short(&error.message),
                    code: error.code,
                })
            }
            _ => return Err(Error::Protocol("expected a batch result".into())),
        };

        let step_error = |i: usize| result.step_errors.get(i).and_then(Option::as_ref);

        if let Some(e) = step_error(0) {
            return Err(Error::Sql {
                message: short(&e.message),
                code: e.code.clone(),
            });
        }
        for i in 0..n {
            if let Some(e) = step_error(i + 1) {
                return Err(Error::Step {
                    index: i,
                    message: short(&e.message),
                    code: e.code.clone(),
                });
            }
        }
        if let Some(e) = step_error(n + 1) {
            return Err(Error::Sql {
                message: short(&e.message),
                code: e.code.clone(),
            });
        }

        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            match result.step_results.get(i + 1).and_then(Option::as_ref) {
                Some(r) => out.push(r.clone()),
                None => {
                    return Err(Error::Protocol(
                        "a statement of the transaction was not executed".into(),
                    ))
                }
            }
        }
        Ok(out)
    }

    async fn pipeline(&self, requests: Vec<StreamRequest>) -> Result<PipelineResponse, Error> {
        let body = serde_json::to_string(&PipelineRequest {
            baton: None,
            requests,
        })
        .map_err(|e| Error::Protocol(format!("cannot encode request: {e}")))?;

        let url = format!("{}/v2/pipeline", self.base_url);
        let resp = self
            .transport
            .post(&url, self.token.as_deref(), body)
            .await
            .map_err(Error::Transport)?;

        if !(200..300).contains(&resp.status) {
            // Server mengirim `{"message": ...}` (JSON) atau teks biasa.
            let message = serde_json::from_str::<serde_json::Value>(&resp.body)
                .ok()
                .and_then(|v| v.get("message").and_then(|m| m.as_str().map(str::to_owned)))
                .unwrap_or(resp.body);
            return Err(Error::Http {
                status: resp.status,
                message: short(message.trim()),
            });
        }

        serde_json::from_str(&resp.body).map_err(|e| Error::Protocol(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::Value;
    use futures_executor::block_on;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Recorded {
        url: String,
        bearer: Option<String>,
        body: String,
    }

    #[derive(Default)]
    struct Fake {
        sent: RefCell<Vec<Recorded>>,
        replies: RefCell<VecDeque<Result<HttpResponse, String>>>,
    }

    impl Fake {
        fn reply(self, status: u16, body: &str) -> Self {
            self.replies.borrow_mut().push_back(Ok(HttpResponse {
                status,
                body: body.to_owned(),
            }));
            self
        }
        fn fail(self, why: &str) -> Self {
            self.replies.borrow_mut().push_back(Err(why.to_owned()));
            self
        }
    }

    impl Transport for &Fake {
        fn post(
            &self,
            url: &str,
            bearer: Option<&str>,
            body: String,
        ) -> impl Future<Output = Result<HttpResponse, String>> {
            self.sent.borrow_mut().push(Recorded {
                url: url.to_owned(),
                bearer: bearer.map(str::to_owned),
                body,
            });
            let reply = self
                .replies
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Err("no canned reply".into()));
            async move { reply }
        }
    }

    const SELECT_OK: &str = r#"{"baton":null,"base_url":null,"results":[
        {"type":"ok","response":{"type":"execute","result":{
            "cols":[{"name":"id","decltype":"INTEGER"},{"name":"label","decltype":"TEXT"}],
            "rows":[[{"type":"integer","value":"7"},{"type":"text","value":"NixOS"}],
                    [{"type":"integer","value":"8"},{"type":"null"}]],
            "affected_row_count":0,"last_insert_rowid":null}}},
        {"type":"ok","response":{"type":"close"}}]}"#;

    #[test]
    fn url_normalization() {
        let n = |s| normalize_base_url(s);
        assert_eq!(
            n("libsql://db-org.turso.io").unwrap(),
            "https://db-org.turso.io"
        );
        assert_eq!(
            n(" https://db-org.turso.io/ ").unwrap(),
            "https://db-org.turso.io"
        );
        assert_eq!(n("wss://db.turso.io").unwrap(), "https://db.turso.io");
        assert_eq!(n("http://127.0.0.1:8081").unwrap(), "http://127.0.0.1:8081");
        assert_eq!(n("ws://localhost:8081/").unwrap(), "http://localhost:8081");
        for bad in [
            "",
            "ftp://x",
            "db.turso.io",
            "https://",
            "https:///x",
            "https://a b",
            "libsql://",
        ] {
            assert!(matches!(n(bad), Err(Error::Config(_))), "{bad:?}");
        }
    }

    #[test]
    fn debug_never_prints_the_token() {
        let fake = Fake::default();
        let c = Client::new("https://x.turso.io", Some("super-secret-token"), &fake).unwrap();
        let shown = format!("{c:?}");
        assert!(!shown.contains("super-secret-token"), "{shown}");
        assert!(shown.contains("redacted"));
    }

    #[test]
    fn empty_token_means_no_auth_header() {
        let fake = Fake::default().reply(200, SELECT_OK);
        let c = Client::new("http://127.0.0.1:8081", Some("  "), &fake).unwrap();
        block_on(c.execute(Stmt::new("SELECT 1"))).unwrap();
        assert_eq!(fake.sent.borrow()[0].bearer, None);
    }

    #[test]
    fn execute_sends_pipeline_and_parses_rows() {
        let fake = Fake::default().reply(200, SELECT_OK);
        let c = Client::new("libsql://db-org.turso.io", Some("tok"), &fake).unwrap();
        let res =
            block_on(c.execute(Stmt::new("SELECT id, label FROM t WHERE id > ?").bind(5))).unwrap();

        let sent = fake.sent.borrow();
        assert_eq!(sent[0].url, "https://db-org.turso.io/v2/pipeline");
        assert_eq!(sent[0].bearer.as_deref(), Some("tok"));
        let body: serde_json::Value = serde_json::from_str(&sent[0].body).unwrap();
        assert_eq!(body["requests"][0]["type"], "execute");
        assert_eq!(
            body["requests"][0]["stmt"]["args"][0],
            serde_json::json!({"type":"integer","value":"5"})
        );
        assert_eq!(body["requests"][1], serde_json::json!({"type":"close"}));
        assert!(body.get("baton").is_none());

        assert_eq!(res.rows.len(), 2);
        assert_eq!(res.get(0, "id"), Some(&Value::Integer(7)));
        assert_eq!(res.get(0, "label").and_then(Value::as_str), Some("NixOS"));
        assert!(res.get(1, "label").unwrap().is_null());
        assert_eq!(res.get(0, "nope"), None);
    }

    #[test]
    fn sql_error_is_reported_with_code() {
        let fake = Fake::default().reply(
            200,
            r#"{"results":[{"type":"error","error":{"message":"no such table: t","code":"SQLITE_ERROR"}}]}"#,
        );
        let c = Client::new("https://x.turso.io", None, &fake).unwrap();
        let err = block_on(c.execute(Stmt::new("SELECT * FROM t"))).unwrap_err();
        assert_eq!(
            err,
            Error::Sql {
                message: "no such table: t".into(),
                code: Some("SQLITE_ERROR".into())
            }
        );
    }

    #[test]
    fn http_errors_carry_status_and_short_message_but_not_the_token() {
        let long = "x".repeat(500);
        let fake = Fake::default()
            .reply(401, r#"{"message":"Unauthorized: invalid token"}"#)
            .reply(502, &long)
            .reply(200, "<html>not json</html>");
        let c = Client::new("https://x.turso.io", Some("tok-123"), &fake).unwrap();

        let e = block_on(c.execute(Stmt::new("SELECT 1"))).unwrap_err();
        assert_eq!(
            e,
            Error::Http {
                status: 401,
                message: "Unauthorized: invalid token".into()
            }
        );
        assert!(!e.to_string().contains("tok-123"));

        match block_on(c.execute(Stmt::new("SELECT 1"))).unwrap_err() {
            Error::Http {
                status: 502,
                message,
            } => assert_eq!(message.chars().count(), 201),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            block_on(c.execute(Stmt::new("SELECT 1"))).unwrap_err(),
            Error::Protocol(_)
        ));
    }

    #[test]
    fn network_failure_becomes_transport_error() {
        let fake = Fake::default().fail("connection reset");
        let c = Client::new("https://x.turso.io", None, &fake).unwrap();
        assert_eq!(
            block_on(c.execute(Stmt::new("SELECT 1"))).unwrap_err(),
            Error::Transport("connection reset".into())
        );
    }

    #[test]
    fn transaction_batch_has_the_reference_shape() {
        let b = transaction_batch(vec![Stmt::new("A"), Stmt::new("B")]);
        let sqls: Vec<&str> = b.steps.iter().map(|s| s.stmt.sql.as_str()).collect();
        assert_eq!(sqls, ["BEGIN IMMEDIATE", "A", "B", "COMMIT", "ROLLBACK"]);
        let conds: Vec<_> = b.steps.iter().map(|s| s.condition.clone()).collect();
        assert_eq!(conds[0], None);
        assert_eq!(conds[1], Some(BatchCond::Ok { step: 0 }));
        assert_eq!(conds[2], Some(BatchCond::Ok { step: 1 }));
        assert_eq!(conds[3], Some(BatchCond::Ok { step: 2 }));
        assert_eq!(
            conds[4],
            Some(BatchCond::Not {
                cond: Box::new(BatchCond::Ok { step: 3 })
            })
        );
    }

    fn batch_reply(results: &str, errors: &str) -> String {
        format!(
            r#"{{"results":[{{"type":"ok","response":{{"type":"batch","result":{{"step_results":{results},"step_errors":{errors}}}}}}},{{"type":"ok","response":{{"type":"close"}}}}]}}"#
        )
    }

    #[test]
    fn transaction_returns_one_result_per_statement() {
        let r = r#"{"cols":[],"rows":[],"affected_row_count":1,"last_insert_rowid":"42"}"#;
        let reply = batch_reply(
            &format!("[{r},{r},{r},{r},null]"),
            "[null,null,null,null,null]",
        );
        let fake = Fake::default().reply(200, &reply);
        let c = Client::new("https://x.turso.io", None, &fake).unwrap();
        let out = block_on(c.transaction(vec![Stmt::new("A"), Stmt::new("B")])).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].affected_row_count, 1);
        assert_eq!(out[1].last_insert_rowid, Some(42));
        let body: serde_json::Value = serde_json::from_str(&fake.sent.borrow()[0].body).unwrap();
        assert_eq!(body["requests"][0]["type"], "batch");
        assert_eq!(
            body["requests"][0]["batch"]["steps"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
    }

    #[test]
    fn transaction_failure_points_at_the_statement() {
        let ok = r#"{"cols":[],"rows":[],"affected_row_count":0}"#;
        // BEGIN ok, A ok, B gagal, COMMIT dilewati, ROLLBACK jalan.
        let reply = batch_reply(
            &format!("[{ok},{ok},null,null,{ok}]"),
            r#"[null,null,{"message":"UNIQUE constraint failed: t.id","code":"SQLITE_CONSTRAINT"},null,null]"#,
        );
        let fake = Fake::default().reply(200, &reply);
        let c = Client::new("https://x.turso.io", None, &fake).unwrap();
        let err = block_on(c.transaction(vec![Stmt::new("A"), Stmt::new("B")])).unwrap_err();
        assert_eq!(
            err,
            Error::Step {
                index: 1,
                message: "UNIQUE constraint failed: t.id".into(),
                code: Some("SQLITE_CONSTRAINT".into())
            }
        );
    }

    #[test]
    fn failed_commit_is_an_error_even_if_every_statement_succeeded() {
        let ok = r#"{"cols":[],"rows":[],"affected_row_count":0}"#;
        let reply = batch_reply(
            &format!("[{ok},{ok},null,{ok}]"),
            r#"[null,null,{"message":"database is locked"},null]"#,
        );
        let fake = Fake::default().reply(200, &reply);
        let c = Client::new("https://x.turso.io", None, &fake).unwrap();
        assert!(matches!(
            block_on(c.transaction(vec![Stmt::new("A")])).unwrap_err(),
            Error::Sql { .. }
        ));
    }

    #[test]
    fn empty_transaction_makes_no_request() {
        let fake = Fake::default();
        let c = Client::new("https://x.turso.io", None, &fake).unwrap();
        assert_eq!(block_on(c.transaction(vec![])).unwrap(), vec![]);
        assert!(fake.sent.borrow().is_empty());
    }
}
