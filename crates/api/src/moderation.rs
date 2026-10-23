//! Moderasi Guestbook dari dashboard admin (D6b): daftar entri (dilaporkan,
//! disembunyikan, semua), tampilkan lagi, dan tolak laporan. Menyembunyikan
//! memakai `chat::hide` yang sudah ada.
//!
//! Entri tidak pernah dihapus dari sini (soft delete saja); penghapusan keras
//! menunggu D6c dengan konfirmasi.

use api_types::validate::is_http_url;
use serde::Serialize;
use turso_http::{Client, Stmt, Transport, Value};

use crate::content;
use crate::panel::PanelError;

const LIST: &str = include_str!("queries/admin_messages_list.sql");
const RESTORE: &str = include_str!("queries/admin_message_restore.sql");
const DISMISS: &str = include_str!("queries/admin_reports_dismiss.sql");

pub const DEFAULT_LIMIT: u32 = 30;
pub const MAX_LIMIT: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    /// Belum disembunyikan dan punya laporan.
    Reported,
    /// Sudah disembunyikan.
    Hidden,
    All,
}

impl Filter {
    fn as_sql(self) -> &'static str {
        match self {
            Filter::Reported => "reported",
            Filter::Hidden => "hidden",
            Filter::All => "all",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModQuery {
    pub filter: Filter,
    /// Hanya entri dengan id lebih kecil dari ini (halaman lebih lama).
    pub before: Option<u32>,
    pub limit: u32,
}

/// `filter=reported|hidden|all&before=<id>&limit=<n>`; bawaan `reported`.
/// `None` jika ada nilai yang tidak valid; parameter lain diabaikan.
pub fn parse_query(raw: Option<&str>) -> Option<ModQuery> {
    let mut q = ModQuery {
        filter: Filter::Reported,
        before: None,
        limit: DEFAULT_LIMIT,
    };
    for pair in raw.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let digits = !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit());
        match key {
            "filter" => {
                q.filter = match value {
                    "reported" => Filter::Reported,
                    "hidden" => Filter::Hidden,
                    "all" => Filter::All,
                    _ => return None,
                }
            }
            "before" => q.before = Some(value.parse::<u32>().ok().filter(|_| digits)?),
            "limit" => {
                q.limit = value
                    .parse::<u32>()
                    .ok()
                    .filter(|n| digits && (1..=MAX_LIMIT).contains(n))?;
            }
            _ => {}
        }
    }
    Some(q)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModMessage {
    pub id: u32,
    pub username: String,
    pub url: Option<String>,
    /// Teks Markdown mentah; dashboard menampilkannya sebagai teks biasa.
    pub message: String,
    pub created_at: String,
    /// Waktu disembunyikan; `None` kalau masih tampil.
    pub hidden_at: Option<String>,
    pub reports: u32,
    pub last_reported_at: Option<String>,
}

/// Satu halaman, terbaru dulu; halaman berikutnya memakai id terkecil sebagai
/// `before`.
pub async fn list<T: Transport>(
    client: &Client<T>,
    q: &ModQuery,
) -> Result<Vec<ModMessage>, PanelError> {
    let res = client
        .execute(
            Stmt::new(LIST)
                .bind(q.before.map_or(Value::Null, Value::from))
                .bind(q.filter.as_sql())
                .bind(q.limit),
        )
        .await?;
    (0..res.rows.len())
        .map(|row| {
            Ok(ModMessage {
                id: content::id(&res, row, "id")?,
                username: content::text(&res, row, "username")?,
                url: content::opt_text(&res, row, "url")?.filter(|u| is_http_url(u)),
                message: content::text(&res, row, "message")?,
                created_at: content::text(&res, row, "created_at")?,
                hidden_at: content::opt_text(&res, row, "deleted_at")?,
                reports: content::id(&res, row, "reports")?,
                last_reported_at: content::opt_text(&res, row, "last_reported_at")?,
            })
        })
        .collect()
}

/// Menampilkan lagi entri yang disembunyikan. `NotFound` jika tidak ada atau
/// sudah tampil.
pub async fn restore<T: Transport>(client: &Client<T>, id: u32) -> Result<(), PanelError> {
    let res = client.execute(Stmt::new(RESTORE).bind(id)).await?;
    if res.rows.is_empty() {
        Err(PanelError::NotFound)
    } else {
        Ok(())
    }
}

/// Menolak laporan: menghapus semua laporan sebuah entri (entrinya tetap).
/// `NotFound` jika tidak ada laporan.
pub async fn dismiss_reports<T: Transport>(client: &Client<T>, id: u32) -> Result<(), PanelError> {
    let res = client.execute(Stmt::new(DISMISS).bind(id)).await?;
    if res.rows.is_empty() {
        Err(PanelError::NotFound)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{client, execute_reply, result, Script};
    use serde_json::json;

    const COLS: [&str; 8] = [
        "id",
        "username",
        "url",
        "message",
        "created_at",
        "deleted_at",
        "reports",
        "last_reported_at",
    ];

    #[test]
    fn queries_are_parsed_strictly_with_safe_defaults() {
        let d = parse_query(None).unwrap();
        assert_eq!(
            d,
            ModQuery {
                filter: Filter::Reported,
                before: None,
                limit: 30
            }
        );
        let q = parse_query(Some("filter=all&before=50&limit=100&x=1")).unwrap();
        assert_eq!((q.filter, q.before, q.limit), (Filter::All, Some(50), 100));
        assert_eq!(
            parse_query(Some("filter=hidden")).unwrap().filter,
            Filter::Hidden
        );
        for bad in [
            "filter=",
            "filter=ALL",
            "filter=deleted",
            "before=",
            "before=-1",
            "before=1.5",
            "before=99999999999",
            "limit=0",
            "limit=101",
            "limit=abc",
        ] {
            assert!(parse_query(Some(bad)).is_none(), "{bad}");
        }
    }

    #[test]
    fn listing_binds_cursor_filter_and_limit_and_reads_every_column() {
        futures_executor::block_on(async {
            let s = Script::new(vec![execute_reply(result(
                &COLS,
                &[
                    vec![
                        json!(7),
                        json!("ani"),
                        json!("https://a.example"),
                        json!("halo *dunia*"),
                        json!("2026-01-02T00:00:00Z"),
                        json!(null),
                        json!(3),
                        json!("2026-01-03T00:00:00Z"),
                    ],
                    vec![
                        json!(6),
                        json!("budi"),
                        json!("javascript:alert(1)"),
                        json!("x"),
                        json!("2026-01-01T00:00:00Z"),
                        json!("2026-01-04T00:00:00Z"),
                        json!(0),
                        json!(null),
                    ],
                ],
            ))]);
            let q = ModQuery {
                filter: Filter::All,
                before: Some(9),
                limit: 20,
            };
            let got = list(&client(&s), &q).await.unwrap();
            let args = s.args(0, 0);
            assert_eq!(args[0], json!({"type": "integer", "value": "9"}));
            assert_eq!(args[1], json!({"type": "text", "value": "all"}));
            assert_eq!(args[2], json!({"type": "integer", "value": "20"}));
            assert_eq!(got.len(), 2);
            assert_eq!(got[0].reports, 3);
            assert_eq!(got[0].hidden_at, None);
            assert_eq!(got[0].url.as_deref(), Some("https://a.example"));
            // Tautan yang bukan http(s) dibuang, teksnya tetap.
            assert_eq!(got[1].url, None);
            assert_eq!(got[1].hidden_at.as_deref(), Some("2026-01-04T00:00:00Z"));
            assert_eq!(got[1].last_reported_at, None);
        });
    }

    #[test]
    fn the_first_page_sends_a_null_cursor() {
        futures_executor::block_on(async {
            let s = Script::new(vec![execute_reply(result(&COLS, &[]))]);
            let got = list(&client(&s), &parse_query(None).unwrap())
                .await
                .unwrap();
            assert!(got.is_empty());
            assert_eq!(s.args(0, 0)[0], json!({"type": "null"}));
        });
    }

    #[test]
    fn restore_and_dismiss_report_not_found_when_nothing_changed() {
        futures_executor::block_on(async {
            let s = Script::new(vec![
                execute_reply(result(&["id"], &[vec![json!(4)]])),
                execute_reply(result(&["id"], &[])),
                execute_reply(result(&["message_id"], &[vec![json!(4)]])),
                execute_reply(result(&["message_id"], &[])),
            ]);
            let c = client(&s);
            assert!(restore(&c, 4).await.is_ok());
            assert_eq!(restore(&c, 4).await, Err(PanelError::NotFound));
            assert!(dismiss_reports(&c, 4).await.is_ok());
            assert_eq!(dismiss_reports(&c, 4).await, Err(PanelError::NotFound));
            assert_eq!(s.args(0, 0), vec![json!({"type": "integer", "value": "4"})]);
        });
    }
}
