//! Live Chat (D4a): membaca dan mengirim pesan.
//!
//! Seperti `content.rs`, logika di sini murni terhadap [`Client`] sehingga dites
//! dengan transport palsu; handler HTTP (yang menyentuh `fetch` dan `Date` Workers)
//! ada di `routes.rs`.
//!
//! Prinsip: server tidak percaya klien. Semua teks dirapikan dan divalidasi ulang
//! (`api_types::chat`), tautan dinormalkan ulang, kata terlarang dicek, dan
//! kecepatan kirim dibatasi per pengirim lewat tabel `chat_rate_limits` (akurat dan
//! sama di semua lokasi Cloudflare, tidak seperti binding Rate Limiting yang
//! hanya per lokasi dan "eventually consistent").

use api_types::chat::{self, Reject};
use api_types::validate::is_http_url;
use api_types::{limits, ChatMessage, NewMessage};
use sha2::{Digest, Sha256};
use turso_http::{Client, Error, Stmt, StmtResult, Transport, Value};

use crate::content::{self, ContentError};

const RECENT: &str = include_str!("queries/messages_recent.sql");
const AFTER: &str = include_str!("queries/messages_after.sql");
const INSERT: &str = include_str!("queries/messages_insert.sql");
const RATE_HIT: &str = include_str!("queries/rate_hit.sql");
const RATE_CLEANUP: &str = include_str!("queries/rate_cleanup.sql");

/// Pesan per pengirim: per menit dan per jam (jendela tetap).
pub const PER_MINUTE: i64 = 5;
pub const PER_HOUR: i64 = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatError {
    /// Database tidak terjangkau atau menolak query.
    Db(Error),
    /// Respons database tidak berbentuk yang diharapkan.
    BadData(&'static str),
    /// Terlalu sering mengirim.
    RateLimited { retry_after_secs: u32 },
}

impl std::fmt::Display for ChatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChatError::Db(e) => write!(f, "{e}"),
            ChatError::BadData(what) => write!(f, "unexpected data: {what}"),
            ChatError::RateLimited { retry_after_secs } => {
                write!(f, "rate limited ({retry_after_secs}s)")
            }
        }
    }
}

impl std::error::Error for ChatError {}

impl From<Error> for ChatError {
    fn from(e: Error) -> Self {
        ChatError::Db(e)
    }
}

impl From<ContentError> for ChatError {
    fn from(e: ContentError) -> Self {
        match e {
            ContentError::Db(e) => ChatError::Db(e),
            ContentError::BadData(w) => ChatError::BadData(w),
        }
    }
}

// ------------------------------------------------------------------ membaca

/// Parameter `GET /messages`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListQuery {
    /// Hanya pesan dengan id lebih besar dari ini (untuk polling).
    pub after: Option<u32>,
    pub limit: u32,
}

/// Mem-parsing query string (`after=12&limit=30`). `None` jika nilainya tidak
/// valid; parameter yang tidak dikenal diabaikan.
pub fn parse_list_query(raw: Option<&str>) -> Option<ListQuery> {
    let mut after = None;
    let mut limit = limits::MESSAGES_DEFAULT_LIMIT;
    for pair in raw.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let digits = !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit());
        match key {
            "after" => after = Some(value.parse::<u32>().ok().filter(|_| digits)?),
            "limit" => {
                limit = value
                    .parse::<u32>()
                    .ok()
                    .filter(|n| digits && (1..=limits::MESSAGES_MAX_LIMIT).contains(n))?;
            }
            _ => {}
        }
    }
    Some(ListQuery { after, limit })
}

fn https_only(url: Option<String>) -> Option<String> {
    url.filter(|u| u.starts_with("https://") && is_http_url(u))
}

fn http_only(url: Option<String>) -> Option<String> {
    url.filter(|u| is_http_url(u))
}

/// Satu baris -> pesan. Isi database dianggap tidak sepenuhnya tepercaya:
/// tautan yang tidak lolos dibuang dari pesan (pesannya tetap tampil).
fn message_row(res: &StmtResult, row: usize) -> Result<ChatMessage, ChatError> {
    Ok(ChatMessage {
        id: content::id(res, row, "id")?,
        username: content::text(res, row, "username")?,
        url: http_only(content::opt_text(res, row, "url")?),
        avatar_url: https_only(content::opt_text(res, row, "avatar_url")?),
        message: content::text(res, row, "message")?,
        attachment_url: https_only(content::opt_text(res, row, "attachment_url")?),
        created_at: content::text(res, row, "created_at")?,
    })
}

/// Pesan terbaru (tanpa `after`) atau yang lebih baru dari `after`; selalu
/// urut dari yang paling lama.
pub async fn list<T: Transport>(
    client: &Client<T>,
    q: &ListQuery,
) -> Result<Vec<ChatMessage>, ChatError> {
    let stmt = match q.after {
        Some(after) => Stmt::new(AFTER).bind(after).bind(q.limit),
        None => Stmt::new(RECENT).bind(q.limit),
    };
    let res = client.execute(stmt).await?;
    (0..res.rows.len()).map(|i| message_row(&res, i)).collect()
}

// ------------------------------------------------------------------ mengirim

/// Pesan yang sudah dirapikan dan lolos validasi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clean {
    pub username: String,
    pub url: Option<String>,
    pub message: String,
}

/// Huruf kecil, hanya huruf dan angka: "B.a d" cocok dengan kata "bad".
fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// `true` jika salah satu kata terlarang ada di salah satu teks. Daftar kata
/// ada di konfigurasi (`BANNED_WORDS`), bukan di repo.
pub fn contains_banned(banned: &[String], texts: &[&str]) -> bool {
    let squashed: Vec<String> = texts.iter().map(|t| squash(t)).collect();
    banned.iter().any(|word| {
        let w = squash(word);
        !w.is_empty() && squashed.iter().any(|t| t.contains(&w))
    })
}

/// Rapikan dan validasi isi permintaan (tanpa menyentuh database).
pub fn prepare(new: &NewMessage, banned: &[String]) -> Result<Clean, Reject> {
    let username = chat::clean_username(&new.username)?;
    let message = chat::clean_message(&new.message)?;
    let url = chat::clean_link(new.url.as_deref().unwrap_or(""))?;
    if contains_banned(banned, &[&username, &message, url.as_deref().unwrap_or("")]) {
        return Err(Reject::Blocked);
    }
    Ok(Clean {
        username,
        url,
        message,
    })
}

/// Kunci pembatas dari IP pengirim: SHA-256 dengan garam rahasia, 32 heks.
/// IP asli tidak pernah disimpan.
pub fn ip_key(ip: &str, salt: &str) -> String {
    let mut h = Sha256::new();
    h.update(salt.as_bytes());
    h.update(b"|");
    h.update(ip.as_bytes());
    h.finalize()
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Tambah hitungan jendela menit lalu jam; tolak jika melewati batas. Jendela
/// menit diperiksa dulu dan, kalau penuh, jam tidak ikut bertambah.
pub async fn check_rate<T: Transport>(
    client: &Client<T>,
    key: &str,
    now_secs: i64,
) -> Result<(), ChatError> {
    for (scope, size, limit) in [("m", 60_i64, PER_MINUTE), ("h", 3600, PER_HOUR)] {
        let stmt = Stmt::new(RATE_HIT)
            .bind(scope)
            .bind(key)
            .bind(now_secs.div_euclid(size));
        let res = client.execute(stmt).await?;
        let hits = res
            .get(0, "hits")
            .and_then(Value::as_i64)
            .ok_or(ChatError::BadData("hits"))?;
        if hits > limit {
            let retry = u32::try_from(size - now_secs.rem_euclid(size)).unwrap_or(60);
            return Err(ChatError::RateLimited {
                retry_after_secs: retry,
            });
        }
    }
    Ok(())
}

/// Tahap sebuah pengiriman yang butuh konteks permintaan.
#[derive(Debug, Clone, Copy)]
pub struct Sender<'a> {
    /// IP dari `CF-Connecting-IP` (atau penanda lokal saat development).
    pub ip: &'a str,
    pub salt: &'a str,
    pub now_secs: i64,
}

/// Cek kecepatan, simpan pesan, bersihkan hitungan lama (satu transaksi).
pub async fn submit<T: Transport>(
    client: &Client<T>,
    sender: &Sender<'_>,
    clean: &Clean,
) -> Result<ChatMessage, ChatError> {
    check_rate(client, &ip_key(sender.ip, sender.salt), sender.now_secs).await?;

    let url = clean.url.as_deref().map_or(Value::Null, Value::from);
    let insert = Stmt::new(INSERT)
        .bind(clean.username.as_str())
        .bind(url)
        .bind(clean.message.as_str())
        .bind(format_utc(sender.now_secs));
    // Hitungan yang sudah lewat dari jendela terbesar yang dipakai dibuang.
    let cleanup = Stmt::new(RATE_CLEANUP)
        .bind(sender.now_secs.div_euclid(60) - 10)
        .bind(sender.now_secs.div_euclid(3600) - 3)
        .no_rows();
    let results = client.transaction(vec![insert, cleanup]).await?;
    let first = results.first().ok_or(ChatError::BadData("insert"))?;
    if first.rows.len() != 1 {
        return Err(ChatError::BadData("insert returned no row"));
    }
    message_row(first, 0)
}

// --------------------------------------------------------------------- waktu

/// Detik Unix -> `YYYY-MM-DDTHH:MM:SSZ` (UTC), tanpa dependensi tanggal.
pub fn format_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Hari sipil dari hari Unix (algoritma Howard Hinnant, `civil_from_days`).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value as Json};
    use std::cell::RefCell;
    use std::future::Future;
    use turso_http::HttpResponse;

    /// Menjawab setiap POST dengan balasan berikutnya dari antrian dan mencatat
    /// isi permintaannya.
    struct Script {
        replies: RefCell<Vec<Result<HttpResponse, String>>>,
        seen: RefCell<Vec<Json>>,
    }

    impl Script {
        fn new(replies: Vec<Result<HttpResponse, String>>) -> Self {
            let mut replies = replies;
            replies.reverse();
            Self {
                replies: RefCell::new(replies),
                seen: RefCell::new(Vec::new()),
            }
        }
        fn sql(&self, i: usize) -> String {
            self.seen.borrow()[i]["requests"][0]["stmt"]["sql"]
                .as_str()
                .unwrap_or("")
                .to_owned()
        }
        fn requests(&self) -> usize {
            self.seen.borrow().len()
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

    fn client(s: &Script) -> Client<&Script> {
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

    fn result(cols: &[&str], rows: &[Vec<Json>]) -> Json {
        json!({
            "cols": cols.iter().map(|c| json!({"name": c})).collect::<Vec<_>>(),
            "rows": rows.iter().map(|r| r.iter().map(cell).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "affected_row_count": 0
        })
    }

    fn ok(body: Json) -> Result<HttpResponse, String> {
        Ok(HttpResponse {
            status: 200,
            body: body.to_string(),
        })
    }

    fn execute_reply(res: Json) -> Result<HttpResponse, String> {
        ok(json!({"results": [
            {"type": "ok", "response": {"type": "execute", "result": res}},
            {"type": "ok", "response": {"type": "close"}}
        ]}))
    }

    fn hits(n: i64) -> Result<HttpResponse, String> {
        execute_reply(result(&["hits"], &[vec![json!(n)]]))
    }

    const MSG_COLS: [&str; 7] = [
        "id",
        "username",
        "url",
        "avatar_url",
        "message",
        "attachment_url",
        "created_at",
    ];

    fn msg_row(id: i64, user: &str, url: Json, avatar: Json, text: &str, att: Json) -> Vec<Json> {
        vec![
            json!(id),
            json!(user),
            url,
            avatar,
            json!(text),
            att,
            json!("2026-10-17T05:30:00Z"),
        ]
    }

    /// Balasan `transaction([insert, cleanup])` yang berhasil.
    fn tx_reply(inserted: Vec<Vec<Json>>) -> Result<HttpResponse, String> {
        ok(json!({"results": [
            {"type": "ok", "response": {"type": "batch", "result": {
                "step_results": [
                    result(&[], &[]),
                    result(&MSG_COLS, &inserted),
                    result(&[], &[]),
                    null, null
                ],
                "step_errors": [null, null, null, null, null]
            }}},
            {"type": "ok", "response": {"type": "close"}}
        ]}))
    }

    fn sender() -> Sender<'static> {
        Sender {
            ip: "203.0.113.9",
            salt: "pepper",
            now_secs: 1_700_000_000,
        }
    }

    fn clean() -> Clean {
        Clean {
            username: "moch".into(),
            url: Some("https://a.dev".into()),
            message: "hello".into(),
        }
    }

    #[test]
    fn utc_timestamps_match_known_dates() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(format_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_utc(951_868_800), "2000-03-01T00:00:00Z");
        assert_eq!(format_utc(4_102_444_799), "2099-12-31T23:59:59Z");
        assert_eq!(format_utc(-1), "1969-12-31T23:59:59Z");
        // Bentuk yang dituntut CHECK di tabel.
        let s = format_utc(1_792_000_000);
        assert_eq!(s.len(), 20);
        assert!(s.ends_with('Z') && s.as_bytes()[10] == b'T');
    }

    #[test]
    fn list_queries_are_parsed_strictly() {
        let q = |s: Option<&str>| parse_list_query(s);
        assert_eq!(
            q(None),
            Some(ListQuery {
                after: None,
                limit: 50
            })
        );
        assert_eq!(
            q(Some("after=12&limit=30")),
            Some(ListQuery {
                after: Some(12),
                limit: 30
            })
        );
        assert_eq!(q(Some("x=1&after=0")).unwrap().after, Some(0));
        assert_eq!(q(Some("limit=100")).unwrap().limit, 100);
        for bad in [
            "limit=0",
            "limit=101",
            "limit=abc",
            "limit=",
            "limit=-1",
            "limit=+5",
            "after=-1",
            "after=1.5",
            "after=",
            "after=99999999999",
            "after",
        ] {
            assert_eq!(q(Some(bad)), None, "{bad}");
        }
    }

    #[test]
    fn requests_are_cleaned_and_validated() {
        let new = |u: &str, l: Option<&str>, m: &str| NewMessage {
            username: u.into(),
            url: l.map(Into::into),
            message: m.into(),
        };
        assert_eq!(
            prepare(&new("  Moch  ", Some("a.dev"), " hi \r\nthere "), &[]),
            Ok(Clean {
                username: "Moch".into(),
                url: Some("https://a.dev".into()),
                message: "hi\nthere".into()
            })
        );
        assert_eq!(prepare(&new("m", None, "x"), &[]).unwrap().url, None);
        assert_eq!(prepare(&new("m", Some(" "), "x"), &[]).unwrap().url, None);
        assert_eq!(
            prepare(&new("", None, "x"), &[]),
            Err(Reject::UsernameLength)
        );
        assert_eq!(
            prepare(&new("m", None, " "), &[]),
            Err(Reject::MessageLength)
        );
        assert_eq!(
            prepare(&new("m", Some("javascript:alert(1)"), "x"), &[]),
            Err(Reject::Link)
        );
    }

    #[test]
    fn banned_words_match_across_case_and_separators() {
        let banned = vec!["Badword".to_owned(), "  ".to_owned()];
        assert!(contains_banned(&banned, &["hello", "a BADWORD here"]));
        assert!(contains_banned(&banned, &["b.a-d w o r d"]));
        assert!(contains_banned(&banned, &["x", "https://bad-word.example"]));
        assert!(!contains_banned(&banned, &["hello", "world"]));
        assert!(!contains_banned(&[], &["anything"]));
        let n = NewMessage {
            username: "ok".into(),
            url: None,
            message: "this has BadWord".into(),
        };
        assert_eq!(prepare(&n, &banned), Err(Reject::Blocked));
        assert!(prepare(&n, &[]).is_ok());
    }

    #[test]
    fn the_ip_is_hashed_with_the_salt_and_never_stored_raw() {
        let a = ip_key("203.0.113.9", "pepper");
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(!a.contains("203"));
        assert_eq!(a, ip_key("203.0.113.9", "pepper"));
        assert_ne!(a, ip_key("203.0.113.10", "pepper"));
        assert_ne!(a, ip_key("203.0.113.9", "other"));
    }

    #[test]
    fn listing_without_after_uses_the_recent_query_and_after_uses_the_cursor_query() {
        let rows = vec![msg_row(1, "a", json!(null), json!(null), "hi", json!(null))];
        let s = Script::new(vec![execute_reply(result(&MSG_COLS, &rows))]);
        let got = futures_executor::block_on(list(
            &client(&s),
            &ListQuery {
                after: None,
                limit: 30,
            },
        ))
        .unwrap();
        assert_eq!(got.len(), 1);
        assert!(s.sql(0).contains("DESC"));
        let args = &s.seen.borrow()[0]["requests"][0]["stmt"]["args"];
        assert_eq!(args[0]["value"], "30");

        let s = Script::new(vec![execute_reply(result(&MSG_COLS, &[]))]);
        let got = futures_executor::block_on(list(
            &client(&s),
            &ListQuery {
                after: Some(7),
                limit: 10,
            },
        ))
        .unwrap();
        assert!(got.is_empty());
        assert!(s.sql(0).contains("WHERE id > ?1"));
        let args = &s.seen.borrow()[0]["requests"][0]["stmt"]["args"];
        assert_eq!(
            (args[0]["value"].clone(), args[1]["value"].clone()),
            (json!("7"), json!("10"))
        );
    }

    #[test]
    fn rows_from_the_database_lose_unsafe_links_but_keep_the_message() {
        let rows = vec![msg_row(
            3,
            "x",
            json!("javascript:alert(1)"),
            json!("http://insecure/avatar.png"),
            "text",
            json!("data:image/png;base64,AAAA"),
        )];
        let s = Script::new(vec![execute_reply(result(&MSG_COLS, &rows))]);
        let got = futures_executor::block_on(list(
            &client(&s),
            &ListQuery {
                after: None,
                limit: 5,
            },
        ))
        .unwrap();
        assert_eq!(got[0].message, "text");
        assert_eq!(
            (
                got[0].url.clone(),
                got[0].avatar_url.clone(),
                got[0].attachment_url.clone()
            ),
            (None, None, None)
        );
    }

    #[test]
    fn a_wrongly_typed_column_is_a_hard_error() {
        let mut row = msg_row(1, "a", json!(null), json!(null), "hi", json!(null));
        row[0] = json!("not a number");
        let s = Script::new(vec![execute_reply(result(&MSG_COLS, &[row]))]);
        let err = futures_executor::block_on(list(
            &client(&s),
            &ListQuery {
                after: None,
                limit: 5,
            },
        ))
        .unwrap_err();
        assert_eq!(err, ChatError::BadData("id"));
    }

    #[test]
    fn a_normal_submit_checks_minute_then_hour_then_inserts_in_one_transaction() {
        let row = msg_row(
            9,
            "moch",
            json!("https://a.dev"),
            json!(null),
            "hello",
            json!(null),
        );
        let s = Script::new(vec![hits(1), hits(1), tx_reply(vec![row])]);
        let got = futures_executor::block_on(submit(&client(&s), &sender(), &clean())).unwrap();
        assert_eq!((got.id, got.username.as_str()), (9, "moch"));
        assert_eq!(s.requests(), 3);
        assert!(s.sql(0).contains("chat_rate_limits"));
        let first = s.seen.borrow()[0]["requests"][0]["stmt"]["args"].clone();
        assert_eq!(first[0]["value"], "m");
        assert_eq!(first[2]["value"], (1_700_000_000_i64 / 60).to_string());
        let second = s.seen.borrow()[1]["requests"][0]["stmt"]["args"].clone();
        assert_eq!(second[0]["value"], "h");
        // Kunci adalah hash, bukan IP.
        let key = first[1]["value"].as_str().unwrap().to_owned();
        assert_eq!(key, ip_key("203.0.113.9", "pepper"));
        // Transaksi: BEGIN, INSERT (dengan waktu server), DELETE, COMMIT, ROLLBACK.
        let batch = &s.seen.borrow()[2]["requests"][0]["batch"]["steps"];
        assert_eq!(batch[1]["stmt"]["args"][3]["value"], "2023-11-14T22:13:20Z");
        assert!(batch[2]["stmt"]["sql"]
            .as_str()
            .unwrap()
            .starts_with("DELETE"));
    }

    #[test]
    fn the_sixth_message_in_a_minute_is_refused_without_touching_the_hour_or_the_table() {
        let s = Script::new(vec![hits(PER_MINUTE + 1)]);
        let err = futures_executor::block_on(submit(&client(&s), &sender(), &clean())).unwrap_err();
        // 1_700_000_000 % 60 == 20 -> 40 detik sampai jendela berikutnya.
        assert_eq!(
            err,
            ChatError::RateLimited {
                retry_after_secs: 40
            }
        );
        assert_eq!(s.requests(), 1, "no hour check, no insert");
    }

    #[test]
    fn the_hour_limit_applies_even_when_the_minute_is_fine() {
        let s = Script::new(vec![hits(1), hits(PER_HOUR + 1)]);
        let err = futures_executor::block_on(submit(&client(&s), &sender(), &clean())).unwrap_err();
        // 1_700_000_000 % 3600 == 800 -> 2800 detik.
        assert_eq!(
            err,
            ChatError::RateLimited {
                retry_after_secs: 2800
            }
        );
        assert_eq!(s.requests(), 2);
    }

    #[test]
    fn exactly_at_the_limit_is_still_allowed() {
        let row = msg_row(1, "a", json!(null), json!(null), "x", json!(null));
        let s = Script::new(vec![hits(PER_MINUTE), hits(PER_HOUR), tx_reply(vec![row])]);
        assert!(futures_executor::block_on(submit(&client(&s), &sender(), &clean())).is_ok());
    }

    #[test]
    fn database_failures_surface_as_db_errors_and_nothing_is_inserted() {
        let s = Script::new(vec![Err("offline".into())]);
        let err = futures_executor::block_on(submit(&client(&s), &sender(), &clean())).unwrap_err();
        assert!(matches!(err, ChatError::Db(Error::Transport(_))));
        assert_eq!(s.requests(), 1);
    }

    #[test]
    fn a_transaction_that_returns_no_row_is_bad_data() {
        let s = Script::new(vec![hits(1), hits(1), tx_reply(vec![])]);
        let err = futures_executor::block_on(submit(&client(&s), &sender(), &clean())).unwrap_err();
        assert!(matches!(err, ChatError::BadData(_)));
    }
}
