//! Dashboard admin (D6a): pembatas percobaan masuk yang gagal, riwayat aksi, dan
//! ringkasan. Seperti `chat.rs`, logikanya murni terhadap [`Client`] dan dites
//! dengan transport palsu; handler HTTP ada di `routes.rs`.
//!
//! - Pembatas: sepuluh percobaan **gagal** dalam satu jendela 15 menit mengunci
//!   kunci itu (hash IP) sampai jendelanya berganti. Selama terkunci, token yang
//!   benar pun ditolak, supaya tebakan tidak bisa diteruskan tanpa batas.
//! - Riwayat: hanya aksi yang berhasil, berisi jenis dan sasaran, bukan isi data.
//! - Tidak ada IP asli di database, hanya hash dengan `RATE_LIMIT_SALT`.

use serde::Serialize;
use turso_http::{Client, Error, Stmt, StmtResult, Transport, Value};

use crate::content::{self, ContentError};
use crate::time::format_utc;

const ATTEMPTS_COUNT: &str = include_str!("queries/admin_attempts_count.sql");
const ATTEMPTS_HIT: &str = include_str!("queries/admin_attempts_hit.sql");
const ATTEMPTS_CLEANUP: &str = include_str!("queries/admin_attempts_cleanup.sql");
const AUDIT_INSERT: &str = include_str!("queries/admin_audit_insert.sql");
const AUDIT_PRUNE: &str = include_str!("queries/admin_audit_prune.sql");
const AUDIT_LIST: &str = include_str!("queries/admin_audit_list.sql");
const OVERVIEW: &str = include_str!("queries/admin_overview.sql");

/// Kegagalan yang diizinkan per jendela sebelum kunci ditolak.
pub const MAX_FAILURES: i64 = 10;
/// Lebar jendela pembatas (detik).
pub const LOCK_WINDOW_SECS: i64 = 900;
/// Baris riwayat yang disimpan.
pub const AUDIT_KEEP: i64 = 500;
pub const AUDIT_DEFAULT_LIMIT: u32 = 50;
pub const AUDIT_MAX_LIMIT: u32 = 100;

/// Nama tabel data yang dihitung di ringkasan, urut tampil. Setiap nama harus
/// menjadi alias kolom di `queries/admin_overview.sql` (ada tes-nya). Tabel
/// internal (`chat_rate_limits`, `admin_attempts`, `_migrations`) tidak ditampilkan.
pub const TABLES: [&str; 12] = [
    "generations",
    "bookmarks",
    "radio_stations",
    "embiences",
    "embience_presets",
    "embience_preset_items",
    "chat_messages",
    "message_reports",
    "journal_snapshots",
    "journal_pinned_repos",
    "journal_stack",
    "admin_audit",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelError {
    /// Database tidak terjangkau atau menolak query.
    Db(Error),
    /// Respons database tidak berbentuk yang diharapkan.
    BadData(&'static str),
    /// Masukan admin ditolak (pesan tetap, tidak memuat masukannya).
    Invalid(&'static str),
    /// Baris yang dituju tidak ada.
    NotFound,
    /// Nilai itu sudah ada (batas UNIQUE).
    Conflict,
}

impl std::fmt::Display for PanelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PanelError::Db(e) => write!(f, "{e}"),
            PanelError::BadData(what) => write!(f, "unexpected data: {what}"),
            PanelError::Invalid(why) => write!(f, "invalid input: {why}"),
            PanelError::NotFound => f.write_str("not found"),
            PanelError::Conflict => f.write_str("already exists"),
        }
    }
}

impl std::error::Error for PanelError {}

impl From<Error> for PanelError {
    fn from(e: Error) -> Self {
        PanelError::Db(e)
    }
}

impl From<ContentError> for PanelError {
    fn from(e: ContentError) -> Self {
        match e {
            ContentError::Db(e) => PanelError::Db(e),
            ContentError::BadData(w) => PanelError::BadData(w),
        }
    }
}

/// Galat tulis dari database: pelanggaran UNIQUE (nama atau kunci kembar) menjadi
/// `Conflict`; selain itu tetap galat database.
pub fn write_error(e: Error) -> PanelError {
    let unique = match &e {
        Error::Sql { message, code } | Error::Step { message, code, .. } => {
            message.contains("UNIQUE constraint failed")
                || code.as_deref().is_some_and(|c| c.contains("UNIQUE"))
        }
        _ => false,
    };
    if unique {
        PanelError::Conflict
    } else {
        PanelError::Db(e)
    }
}

// ------------------------------------------------------------------ pembatas

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Token benar dan kunci tidak terkunci.
    Granted,
    /// Token salah; kegagalan sudah dicatat.
    Denied,
    /// Terlalu banyak kegagalan di jendela ini.
    Locked { retry_after_secs: u32 },
}

/// Memutuskan apakah sebuah permintaan admin boleh lewat. `authorized` adalah
/// hasil pemeriksaan token (`admin::authorized`); keputusan terkunci didahulukan
/// dari token yang benar. Hanya kegagalan yang menulis ke database.
pub async fn check_access<T: Transport>(
    client: &Client<T>,
    key: &str,
    now_secs: i64,
    authorized: bool,
) -> Result<Access, PanelError> {
    let window = now_secs.div_euclid(LOCK_WINDOW_SECS);
    let res = client
        .execute(Stmt::new(ATTEMPTS_COUNT).bind(key).bind(window))
        .await?;
    let hits = if res.rows.is_empty() {
        0
    } else {
        res.get(0, "hits")
            .and_then(Value::as_i64)
            .ok_or(PanelError::BadData("hits"))?
    };
    if hits >= MAX_FAILURES {
        let left = LOCK_WINDOW_SECS - now_secs.rem_euclid(LOCK_WINDOW_SECS);
        return Ok(Access::Locked {
            retry_after_secs: u32::try_from(left).unwrap_or(60),
        });
    }
    if authorized {
        return Ok(Access::Granted);
    }
    // Catat kegagalan dan buang jendela yang sudah dua jendela lalu.
    client
        .transaction(vec![
            Stmt::new(ATTEMPTS_HIT).bind(key).bind(window),
            Stmt::new(ATTEMPTS_CLEANUP).bind(window - 1).no_rows(),
        ])
        .await?;
    Ok(Access::Denied)
}

// ------------------------------------------------------------------- riwayat

/// Delapan karakter pertama hash: cukup untuk membedakan perangkat, tidak cukup
/// untuk dijadikan kunci pembatas.
pub fn short_who(who: &str) -> String {
    who.chars().take(8).collect()
}

/// Mencatat satu aksi yang berhasil lalu memangkas riwayat lama (satu transaksi).
/// `action` dan `target` dibuat server (bukan input klien) dan pendek.
pub async fn record<T: Transport>(
    client: &Client<T>,
    who: &str,
    now_secs: i64,
    action: &str,
    target: Option<&str>,
) -> Result<(), PanelError> {
    let insert = Stmt::new(AUDIT_INSERT)
        .bind(format_utc(now_secs))
        .bind(action)
        .bind(target.map_or(Value::Null, Value::from))
        .bind(who);
    let prune = Stmt::new(AUDIT_PRUNE).bind(AUDIT_KEEP).no_rows();
    client.transaction(vec![insert, prune]).await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditEntry {
    pub id: u32,
    pub at: String,
    pub action: String,
    pub target: Option<String>,
    /// Delapan karakter pertama hash pelaku ([`short_who`]).
    pub who: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditQuery {
    /// Hanya entri dengan id lebih kecil dari ini (halaman lebih lama).
    pub before: Option<u32>,
    pub limit: u32,
}

/// `before=<id>&limit=<n>`. `None` jika nilainya tidak valid; parameter lain
/// diabaikan.
pub fn parse_audit_query(raw: Option<&str>) -> Option<AuditQuery> {
    let mut before = None;
    let mut limit = AUDIT_DEFAULT_LIMIT;
    for pair in raw.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let digits = !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit());
        match key {
            "before" => before = Some(value.parse::<u32>().ok().filter(|_| digits)?),
            "limit" => {
                limit = value
                    .parse::<u32>()
                    .ok()
                    .filter(|n| digits && (1..=AUDIT_MAX_LIMIT).contains(n))?;
            }
            _ => {}
        }
    }
    Some(AuditQuery { before, limit })
}

pub async fn audit_list<T: Transport>(
    client: &Client<T>,
    q: &AuditQuery,
) -> Result<Vec<AuditEntry>, PanelError> {
    let res = client
        .execute(
            Stmt::new(AUDIT_LIST)
                .bind(q.before.map_or(Value::Null, Value::from))
                .bind(q.limit),
        )
        .await?;
    (0..res.rows.len())
        .map(|row| {
            Ok(AuditEntry {
                id: content::id(&res, row, "id")?,
                at: content::text(&res, row, "at")?,
                action: content::text(&res, row, "action")?,
                target: content::opt_text(&res, row, "target")?,
                who: short_who(&content::text(&res, row, "who")?),
            })
        })
        .collect()
}

// ------------------------------------------------------------------ ringkasan

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableCount {
    pub name: &'static str,
    pub rows: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuestbookStatus {
    pub total: u64,
    pub hidden: u64,
    /// Entri yang masih tampil dan pernah dilaporkan.
    pub reported: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JournalStatus {
    pub fetched_at: Option<String>,
    pub source: Option<String>,
    pub snapshots: u64,
    /// `GITHUB_TOKEN` terisi (nilainya tidak pernah dikirim).
    pub github_configured: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationStatus {
    pub applied: u64,
    pub latest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Overview {
    pub now: String,
    /// Hash pendek perangkat ini, untuk mengenali baris riwayat sendiri.
    pub me: String,
    pub tables: Vec<TableCount>,
    pub guestbook: GuestbookStatus,
    pub journal: JournalStatus,
    pub migrations: MigrationStatus,
}

/// Konteks permintaan yang ikut ke ringkasan.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    pub who: &'a str,
    pub now_secs: i64,
    pub github_configured: bool,
}

fn count(res: &StmtResult, col: &'static str) -> Result<u64, PanelError> {
    res.get(0, col)
        .and_then(Value::as_i64)
        .and_then(|n| u64::try_from(n).ok())
        .ok_or(PanelError::BadData(col))
}

pub async fn overview<T: Transport>(
    client: &Client<T>,
    ctx: &Context<'_>,
) -> Result<Overview, PanelError> {
    let res = client.execute(Stmt::new(OVERVIEW)).await?;
    if res.rows.len() != 1 {
        return Err(PanelError::BadData("overview"));
    }
    let tables = TABLES
        .iter()
        .map(|name| {
            Ok(TableCount {
                name,
                rows: count(&res, name)?,
            })
        })
        .collect::<Result<Vec<_>, PanelError>>()?;
    Ok(Overview {
        now: format_utc(ctx.now_secs),
        me: short_who(ctx.who),
        guestbook: GuestbookStatus {
            total: count(&res, "chat_messages")?,
            hidden: count(&res, "messages_hidden")?,
            reported: count(&res, "messages_reported")?,
        },
        journal: JournalStatus {
            fetched_at: content::opt_text(&res, 0, "journal_fetched_at")?,
            source: content::opt_text(&res, 0, "journal_source")?,
            snapshots: count(&res, "journal_snapshots")?,
            github_configured: ctx.github_configured,
        },
        migrations: MigrationStatus {
            applied: count(&res, "migrations_applied")?,
            latest: content::opt_text(&res, 0, "migrations_latest")?,
        },
        tables,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{client, execute_reply, result, tx_reply, Script};
    use serde_json::{json, Value as Json};

    const KEY: &str = "0123456789abcdef0123456789abcdef";

    fn hits(n: i64) -> Result<turso_http::HttpResponse, String> {
        execute_reply(result(&["hits"], &[vec![json!(n)]]))
    }

    fn no_hits() -> Result<turso_http::HttpResponse, String> {
        execute_reply(result(&["hits"], &[]))
    }

    fn recorded() -> Result<turso_http::HttpResponse, String> {
        tx_reply(vec![result(&["hits"], &[vec![json!(1)]]), result(&[], &[])])
    }

    #[test]
    fn a_good_token_costs_one_read_and_no_write() {
        futures_executor::block_on(async {
            let s = Script::new(vec![no_hits()]);
            let out = check_access(&client(&s), KEY, 1_700_000_000, true).await;
            assert_eq!(out, Ok(Access::Granted));
            assert_eq!(s.requests(), 1);
            assert!(s.body(0).contains("admin_attempts"));
            assert_eq!(s.args(0, 0)[0]["value"], KEY);
            assert_eq!(
                s.args(0, 0)[1]["value"],
                (1_700_000_000_i64 / LOCK_WINDOW_SECS).to_string()
            );
        });
    }

    #[test]
    fn a_wrong_token_is_counted_and_old_windows_are_dropped_in_one_transaction() {
        futures_executor::block_on(async {
            let s = Script::new(vec![hits(3), recorded()]);
            let now = 1_700_000_000_i64;
            let out = check_access(&client(&s), KEY, now, false).await;
            assert_eq!(out, Ok(Access::Denied));
            assert_eq!(s.requests(), 2);
            let body = s.body(1);
            assert!(body.contains("INSERT INTO admin_attempts"), "{body}");
            assert!(body.contains("DELETE FROM admin_attempts"), "{body}");
            let window = now / LOCK_WINDOW_SECS;
            // Langkah 1 = hit (key, window); langkah 2 = cleanup (window - 1).
            assert_eq!(s.args(1, 1)[0]["value"], KEY);
            assert_eq!(s.args(1, 1)[1]["value"], window.to_string());
            assert_eq!(s.args(1, 2)[0]["value"], (window - 1).to_string());
        });
    }

    #[test]
    fn the_tenth_failure_locks_even_a_correct_token_without_writing() {
        futures_executor::block_on(async {
            for authorized in [true, false] {
                let s = Script::new(vec![hits(MAX_FAILURES)]);
                let now = 900 * 1000 + 100; // 100 detik masuk jendela
                let out = check_access(&client(&s), KEY, now, authorized).await;
                assert_eq!(
                    out,
                    Ok(Access::Locked {
                        retry_after_secs: 800
                    })
                );
                assert_eq!(s.requests(), 1, "a locked key is never written");
            }
        });
    }

    #[test]
    fn nine_failures_still_get_the_normal_answer() {
        futures_executor::block_on(async {
            let s = Script::new(vec![hits(MAX_FAILURES - 1)]);
            assert_eq!(
                check_access(&client(&s), KEY, 5, true).await,
                Ok(Access::Granted)
            );
            let s = Script::new(vec![hits(MAX_FAILURES - 1), recorded()]);
            assert_eq!(
                check_access(&client(&s), KEY, 5, false).await,
                Ok(Access::Denied)
            );
        });
    }

    #[test]
    fn the_lock_ends_when_the_window_changes() {
        futures_executor::block_on(async {
            // Jendela baru: belum ada baris untuk kunci ini.
            let s = Script::new(vec![no_hits()]);
            let next = 900 * 1001;
            assert_eq!(
                check_access(&client(&s), KEY, next, true).await,
                Ok(Access::Granted)
            );
        });
    }

    #[test]
    fn database_trouble_is_an_error_not_a_grant() {
        futures_executor::block_on(async {
            let s = Script::new(vec![Err("offline".into())]);
            assert!(matches!(
                check_access(&client(&s), KEY, 5, true).await,
                Err(PanelError::Db(_))
            ));
            let s = Script::new(vec![execute_reply(result(&["hits"], &[vec![json!("x")]]))]);
            assert_eq!(
                check_access(&client(&s), KEY, 5, true).await,
                Err(PanelError::BadData("hits"))
            );
        });
    }

    #[test]
    fn recording_an_action_inserts_then_prunes_with_the_hash_not_an_ip() {
        futures_executor::block_on(async {
            let s = Script::new(vec![tx_reply(vec![
                result(&["id"], &[vec![json!(7)]]),
                result(&[], &[]),
            ])]);
            record(
                &client(&s),
                KEY,
                1_700_000_000,
                "message.hide",
                Some("message:12"),
            )
            .await
            .unwrap();
            let body = s.body(0);
            assert!(body.contains("INSERT INTO admin_audit"), "{body}");
            assert!(body.contains("DELETE FROM admin_audit"), "{body}");
            let a = s.args(0, 1);
            assert_eq!(a[0]["value"], "2023-11-14T22:13:20Z");
            assert_eq!(a[1]["value"], "message.hide");
            assert_eq!(a[2]["value"], "message:12");
            assert_eq!(a[3]["value"], KEY);
            assert_eq!(s.args(0, 2)[0]["value"], AUDIT_KEEP.to_string());

            let s = Script::new(vec![tx_reply(vec![
                result(&["id"], &[vec![json!(8)]]),
                result(&[], &[]),
            ])]);
            record(&client(&s), KEY, 0, "login", None).await.unwrap();
            assert_eq!(s.args(0, 1)[2]["type"], "null");
        });
    }

    #[test]
    fn a_failed_write_is_reported_to_the_caller() {
        futures_executor::block_on(async {
            let s = Script::new(vec![Err("offline".into())]);
            assert!(record(&client(&s), KEY, 0, "login", None).await.is_err());
        });
    }

    #[test]
    fn audit_queries_are_parsed_strictly() {
        let q = |raw| parse_audit_query(Some(raw));
        assert_eq!(
            parse_audit_query(None),
            Some(AuditQuery {
                before: None,
                limit: AUDIT_DEFAULT_LIMIT
            })
        );
        assert_eq!(
            q("before=42&limit=10"),
            Some(AuditQuery {
                before: Some(42),
                limit: 10
            })
        );
        assert_eq!(q("limit=100").unwrap().limit, 100);
        assert_eq!(q("x=1&limit=5").unwrap().limit, 5);
        for bad in [
            "limit=0",
            "limit=101",
            "limit=",
            "limit=x",
            "before=x",
            "before=-1",
            "before=",
            "before=4294967296",
        ] {
            assert_eq!(q(bad), None, "{bad}");
        }
    }

    #[test]
    fn listing_binds_the_cursor_and_shortens_the_hash() {
        futures_executor::block_on(async {
            let cols = ["id", "at", "action", "target", "who"];
            let s = Script::new(vec![execute_reply(result(
                &cols,
                &[
                    vec![
                        json!(9),
                        json!("2026-10-22T01:02:03Z"),
                        json!("message.hide"),
                        json!("message:3"),
                        json!(KEY),
                    ],
                    vec![
                        json!(8),
                        json!("2026-10-22T01:00:00Z"),
                        json!("login"),
                        Json::Null,
                        json!(KEY),
                    ],
                ],
            ))]);
            let q = AuditQuery {
                before: Some(10),
                limit: 2,
            };
            let items = audit_list(&client(&s), &q).await.unwrap();
            assert_eq!(items.len(), 2);
            assert_eq!(items[0].id, 9);
            assert_eq!(items[0].who, "01234567");
            assert_eq!(items[0].target.as_deref(), Some("message:3"));
            assert_eq!(items[1].target, None);
            assert_eq!(s.args(0, 0)[0]["value"], "10");
            assert_eq!(s.args(0, 0)[1]["value"], "2");

            let s = Script::new(vec![execute_reply(result(&cols, &[]))]);
            let q = AuditQuery {
                before: None,
                limit: 5,
            };
            assert!(audit_list(&client(&s), &q).await.unwrap().is_empty());
            assert_eq!(s.args(0, 0)[0]["type"], "null");
        });
    }

    #[test]
    fn a_wrongly_typed_audit_row_is_a_hard_error() {
        futures_executor::block_on(async {
            let cols = ["id", "at", "action", "target", "who"];
            let s = Script::new(vec![execute_reply(result(
                &cols,
                &[vec![
                    json!("nine"),
                    json!("t"),
                    json!("a"),
                    Json::Null,
                    json!("w"),
                ]],
            ))]);
            let q = AuditQuery {
                before: None,
                limit: 5,
            };
            assert_eq!(
                audit_list(&client(&s), &q).await,
                Err(PanelError::BadData("id"))
            );
        });
    }

    #[test]
    fn the_overview_sql_names_every_table_it_reports() {
        for name in TABLES {
            assert!(
                OVERVIEW.contains(&format!("AS {name}"))
                    || OVERVIEW.contains(&format!("AS  {name}")),
                "{name} is not an alias in admin_overview.sql"
            );
            assert!(
                OVERVIEW.contains(&format!("FROM {name})")),
                "{name} is not counted"
            );
        }
    }

    fn overview_row() -> (Vec<&'static str>, Vec<Json>) {
        let mut cols: Vec<&'static str> = TABLES.to_vec();
        let mut vals: Vec<Json> = (1..=TABLES.len() as i64).map(|n| json!(n * 10)).collect();
        for (c, v) in [
            ("messages_hidden", json!(2)),
            ("messages_reported", json!(1)),
            ("journal_fetched_at", json!("2026-10-22T00:00:00Z")),
            ("journal_source", json!("github")),
            ("migrations_applied", json!(10)),
            ("migrations_latest", json!("0010_admin")),
        ] {
            cols.push(c);
            vals.push(v);
        }
        (cols, vals)
    }

    #[test]
    fn the_overview_maps_every_column_and_never_sends_the_github_token() {
        futures_executor::block_on(async {
            let (cols, vals) = overview_row();
            let s = Script::new(vec![execute_reply(result(&cols, &[vals]))]);
            let ctx = Context {
                who: KEY,
                now_secs: 1_700_000_000,
                github_configured: true,
            };
            let o = overview(&client(&s), &ctx).await.unwrap();
            assert_eq!(o.now, "2023-11-14T22:13:20Z");
            assert_eq!(o.me, "01234567");
            assert_eq!(o.tables.len(), TABLES.len());
            assert_eq!(o.tables[0].name, "generations");
            assert_eq!(o.tables[0].rows, 10);
            assert_eq!(o.tables[6].name, "chat_messages");
            assert_eq!(o.guestbook.total, 70);
            assert_eq!(o.guestbook.hidden, 2);
            assert_eq!(o.guestbook.reported, 1);
            assert_eq!(o.journal.snapshots, 90);
            assert_eq!(o.journal.source.as_deref(), Some("github"));
            assert!(o.journal.github_configured);
            assert_eq!(o.migrations.latest.as_deref(), Some("0010_admin"));
            let json = serde_json::to_string(&o).unwrap();
            assert!(!json.contains(KEY), "only the short hash leaves the server");
        });
    }

    #[test]
    fn an_empty_journal_and_migration_table_show_up_as_nulls() {
        futures_executor::block_on(async {
            let (cols, mut vals) = overview_row();
            let n = vals.len();
            vals[n - 4] = Json::Null; // journal_fetched_at
            vals[n - 3] = Json::Null; // journal_source
            vals[n - 1] = Json::Null; // migrations_latest
            let s = Script::new(vec![execute_reply(result(&cols, &[vals]))]);
            let ctx = Context {
                who: KEY,
                now_secs: 0,
                github_configured: false,
            };
            let o = overview(&client(&s), &ctx).await.unwrap();
            assert_eq!(o.journal.fetched_at, None);
            assert_eq!(o.journal.source, None);
            assert_eq!(o.migrations.latest, None);
            assert!(!o.journal.github_configured);
        });
    }

    #[test]
    fn a_missing_column_or_row_is_a_hard_error() {
        futures_executor::block_on(async {
            let ctx = Context {
                who: KEY,
                now_secs: 0,
                github_configured: false,
            };
            let (cols, vals) = overview_row();
            let s = Script::new(vec![execute_reply(result(
                &cols[..5],
                &[vals[..5].to_vec()],
            ))]);
            assert!(matches!(
                overview(&client(&s), &ctx).await,
                Err(PanelError::BadData(_))
            ));
            let s = Script::new(vec![execute_reply(result(&cols, &[]))]);
            assert_eq!(
                overview(&client(&s), &ctx).await,
                Err(PanelError::BadData("overview"))
            );
        });
    }

    #[test]
    fn short_hashes_are_eight_characters() {
        assert_eq!(short_who(KEY), "01234567");
        assert_eq!(short_who("ab"), "ab");
        assert_eq!(short_who(""), "");
    }
}
