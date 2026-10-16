//! Endpoint baca konten (D3): generations, bookmarks, radio, embiences, presets.
//!
//! Logika di sini murni terhadap [`Client`] sehingga dites dengan transport
//! palsu; handler HTTP (yang menyentuh `fetch` Workers) ada di `routes.rs`.
//!
//! Data dari database dianggap tidak sepenuhnya tepercaya (nanti ada panel admin,
//! D6): baris dengan URL/rute yang tidak lolos pemeriksaan dibuang dan dihitung
//! (`Loaded::skipped`), bukan dikirim ke klien dan bukan menggagalkan seluruh
//! endpoint. Tipe kolom yang salah tetap kegagalan keras (`ContentError::BadData`).

use api_types::validate::{is_http_url, is_route_or_url};
use api_types::{Bookmark, Embience, Generation, Preset, RadioStation};
use turso_http::{Client, Error, Stmt, StmtResult, Transport, Value};

/// SQL tiap endpoint ada di berkas `.sql` supaya `scripts/migrations.test.mjs`
/// menjalankan query yang persis sama di SQLite sungguhan.
const GENERATIONS: &str = include_str!("queries/generations.sql");
const BOOKMARKS: &str = include_str!("queries/bookmarks.sql");
const RADIO: &str = include_str!("queries/radio.sql");
const EMBIENCES: &str = include_str!("queries/embiences.sql");
const PRESETS: &str = include_str!("queries/presets.sql");

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentError {
    /// Database tidak terjangkau atau menolak query.
    Db(Error),
    /// Respons database tidak berbentuk yang diharapkan (kolom/tipe salah).
    BadData(&'static str),
}

impl std::fmt::Display for ContentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContentError::Db(e) => write!(f, "{e}"),
            ContentError::BadData(what) => write!(f, "unexpected data: {what}"),
        }
    }
}

impl std::error::Error for ContentError {}

impl From<Error> for ContentError {
    fn from(e: Error) -> Self {
        ContentError::Db(e)
    }
}

/// Hasil query: baris yang lolos, dan berapa yang dibuang karena tidak valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded<T> {
    pub items: Vec<T>,
    pub skipped: usize,
}

// --------------------------------------------------------------- baca baris

fn cell<'a>(res: &'a StmtResult, row: usize, col: &'static str) -> Result<&'a Value, ContentError> {
    res.get(row, col).ok_or(ContentError::BadData(col))
}

fn text(res: &StmtResult, row: usize, col: &'static str) -> Result<String, ContentError> {
    cell(res, row, col)?
        .as_str()
        .map(str::to_owned)
        .ok_or(ContentError::BadData(col))
}

fn opt_text(
    res: &StmtResult,
    row: usize,
    col: &'static str,
) -> Result<Option<String>, ContentError> {
    let v = cell(res, row, col)?;
    if v.is_null() {
        return Ok(None);
    }
    v.as_str()
        .map(|s| Some(s.to_owned()))
        .ok_or(ContentError::BadData(col))
}

fn id(res: &StmtResult, row: usize, col: &'static str) -> Result<u32, ContentError> {
    cell(res, row, col)?
        .as_i64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or(ContentError::BadData(col))
}

async fn query<T: Transport>(client: &Client<T>, sql: &str) -> Result<StmtResult, ContentError> {
    Ok(client.execute(Stmt::new(sql)).await?)
}

// ---------------------------------------------------------------- endpoint

pub async fn generations<T: Transport>(
    client: &Client<T>,
) -> Result<Loaded<Generation>, ContentError> {
    let res = query(client, GENERATIONS).await?;
    let mut out = Loaded {
        items: Vec::with_capacity(res.rows.len()),
        skipped: 0,
    };
    for row in 0..res.rows.len() {
        let g = Generation {
            number: id(&res, row, "number")?,
            label: text(&res, row, "label")?,
            link: text(&res, row, "link")?,
            kernel: text(&res, row, "kernel")?,
            built_on: text(&res, row, "built_on")?,
        };
        if is_route_or_url(&g.link) {
            out.items.push(g);
        } else {
            out.skipped += 1;
        }
    }
    Ok(out)
}

pub async fn bookmarks<T: Transport>(client: &Client<T>) -> Result<Loaded<Bookmark>, ContentError> {
    let res = query(client, BOOKMARKS).await?;
    let mut out = Loaded {
        items: Vec::with_capacity(res.rows.len()),
        skipped: 0,
    };
    for row in 0..res.rows.len() {
        let b = Bookmark {
            id: id(&res, row, "id")?,
            label: text(&res, row, "label")?,
            url: text(&res, row, "url")?,
        };
        if is_http_url(&b.url) {
            out.items.push(b);
        } else {
            out.skipped += 1;
        }
    }
    Ok(out)
}

pub async fn radio<T: Transport>(client: &Client<T>) -> Result<Loaded<RadioStation>, ContentError> {
    let res = query(client, RADIO).await?;
    let mut out = Loaded {
        items: Vec::with_capacity(res.rows.len()),
        skipped: 0,
    };
    for row in 0..res.rows.len() {
        let mut station = RadioStation {
            id: id(&res, row, "id")?,
            label: text(&res, row, "label")?,
            slogan: text(&res, row, "slogan")?,
            stream_url: text(&res, row, "stream_url")?,
            official_link: opt_text(&res, row, "official_link")?,
            image_url: opt_text(&res, row, "image_url")?,
        };
        if !is_http_url(&station.stream_url) {
            out.skipped += 1;
            continue;
        }
        // Tautan/gambar yang buruk hanya dibuang, stasiunnya tetap bisa diputar.
        station.official_link.take_if(|u| !is_http_url(u));
        station.image_url.take_if(|u| !is_http_url(u));
        out.items.push(station);
    }
    Ok(out)
}

pub async fn embiences<T: Transport>(client: &Client<T>) -> Result<Loaded<Embience>, ContentError> {
    let res = query(client, EMBIENCES).await?;
    let mut out = Loaded {
        items: Vec::with_capacity(res.rows.len()),
        skipped: 0,
    };
    for row in 0..res.rows.len() {
        let e = Embience {
            id: id(&res, row, "id")?,
            name: text(&res, row, "name")?,
            icon_url: text(&res, row, "icon_url")?,
            stream_url: text(&res, row, "stream_url")?,
        };
        if is_http_url(&e.icon_url) && is_http_url(&e.stream_url) {
            out.items.push(e);
        } else {
            out.skipped += 1;
        }
    }
    Ok(out)
}

pub async fn presets<T: Transport>(client: &Client<T>) -> Result<Loaded<Preset>, ContentError> {
    let res = query(client, PRESETS).await?;
    let mut items: Vec<Preset> = Vec::new();
    for row in 0..res.rows.len() {
        let preset_id = id(&res, row, "preset_id")?;
        // Satu baris per (preset, embience); `embience_id` NULL = preset kosong
        // atau item yang menunjuk embience yang sudah tidak ada.
        let embience = match cell(&res, row, "embience_id")? {
            v if v.is_null() => None,
            _ => Some(id(&res, row, "embience_id")?),
        };
        if items.last().map(|p| p.id) != Some(preset_id) {
            items.push(Preset {
                id: preset_id,
                label: text(&res, row, "label")?,
                embience_ids: Vec::new(),
            });
        }
        if let (Some(e), Some(preset)) = (embience, items.last_mut()) {
            if !preset.embience_ids.contains(&e) {
                preset.embience_ids.push(e);
            }
        }
    }
    Ok(Loaded { items, skipped: 0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_executor::block_on;
    use serde_json::{json, Value as Json};
    use std::cell::RefCell;
    use std::future::Future;
    use turso_http::HttpResponse;

    /// Transport yang menjawab satu hasil `execute` dan mencatat SQL-nya.
    struct Canned {
        reply: Result<HttpResponse, String>,
        sql: RefCell<Vec<String>>,
    }

    impl Transport for &Canned {
        fn post(
            &self,
            _: &str,
            _: Option<&str>,
            body: String,
        ) -> impl Future<Output = Result<HttpResponse, String>> {
            let req: Json = serde_json::from_str(&body).unwrap();
            self.sql
                .borrow_mut()
                .push(req["requests"][0]["stmt"]["sql"].as_str().unwrap().into());
            let r = self.reply.clone();
            async move { r }
        }
    }

    fn cell_json(v: &Json) -> Json {
        match v {
            Json::Null => json!({"type": "null"}),
            Json::Number(n) => json!({"type": "integer", "value": n.to_string()}),
            Json::String(s) => json!({"type": "text", "value": s}),
            other => panic!("unsupported test value {other}"),
        }
    }

    fn rows_reply(cols: &[&str], rows: Vec<Vec<Json>>) -> Canned {
        let body = json!({"results": [
            {"type": "ok", "response": {"type": "execute", "result": {
                "cols": cols.iter().map(|c| json!({"name": c})).collect::<Vec<_>>(),
                "rows": rows.iter().map(|r| r.iter().map(cell_json).collect::<Vec<_>>()).collect::<Vec<_>>(),
                "affected_row_count": 0
            }}},
            {"type": "ok", "response": {"type": "close"}}
        ]});
        Canned {
            reply: Ok(HttpResponse {
                status: 200,
                body: body.to_string(),
            }),
            sql: RefCell::new(Vec::new()),
        }
    }

    fn client(c: &Canned) -> Client<&Canned> {
        Client::new("https://x.turso.io", Some("tok"), c).unwrap()
    }

    // ------------------------------------------------------------ endpoint

    #[test]
    fn generations_keep_query_order_and_drop_unsafe_links() {
        let c = rows_reply(
            &["number", "label", "link", "kernel", "built_on"],
            vec![
                vec![
                    json!(5),
                    json!("Go to Desktop"),
                    json!("/deisktify"),
                    json!("6.12.7"),
                    json!("2026-08-30"),
                ],
                vec![
                    json!(4),
                    json!("Evil"),
                    json!("javascript:alert(1)"),
                    json!("1"),
                    json!("2026-01-01"),
                ],
                vec![
                    json!(3),
                    json!("LinkedIn"),
                    json!("https://linkedin.com/in/x"),
                    json!("6.11.9"),
                    json!("2026-05-02"),
                ],
            ],
        );
        let out = block_on(generations(&client(&c))).unwrap();
        assert_eq!(out.skipped, 1);
        let numbers: Vec<_> = out.items.iter().map(|g| g.number).collect();
        assert_eq!(numbers, [5, 3]);
        assert_eq!(out.items[0].built_on, "2026-08-30");
        let sql = c.sql.borrow();
        assert!(sql[0].contains("ORDER BY number DESC"), "{}", sql[0]);
    }

    #[test]
    fn bookmarks_drop_non_http_urls() {
        let c = rows_reply(
            &["id", "label", "url"],
            vec![
                vec![json!(1), json!("Github"), json!("https://github.com/x")],
                vec![json!(2), json!("Bad"), json!("javascript:alert(1)")],
            ],
        );
        let out = block_on(bookmarks(&client(&c))).unwrap();
        assert_eq!(out.items.len(), 1);
        assert_eq!(out.skipped, 1);
        assert_eq!(out.items[0].url, "https://github.com/x");
        assert!(c.sql.borrow()[0].contains("ORDER BY sort_order, id"));
    }

    #[test]
    fn radio_keeps_stations_but_blanks_bad_optional_links() {
        let c = rows_reply(
            &[
                "id",
                "label",
                "slogan",
                "stream_url",
                "official_link",
                "image_url",
            ],
            vec![
                vec![
                    json!(1),
                    json!("A"),
                    json!("s"),
                    json!("https://a.b/s"),
                    json!("https://a.b"),
                    json!("https://a.b/i.png"),
                ],
                vec![
                    json!(2),
                    json!("B"),
                    json!(""),
                    json!("https://b.c/s"),
                    json!(null),
                    json!("javascript:x"),
                ],
                vec![
                    json!(3),
                    json!("C"),
                    json!(""),
                    json!("javascript:x"),
                    json!(null),
                    json!(null),
                ],
            ],
        );
        let out = block_on(radio(&client(&c))).unwrap();
        assert_eq!(out.skipped, 1, "station with a bad stream is dropped");
        assert_eq!(out.items.len(), 2);
        assert_eq!(out.items[0].official_link.as_deref(), Some("https://a.b"));
        assert_eq!(out.items[1].official_link, None);
        assert_eq!(out.items[1].image_url, None, "bad image URL is blanked");
        assert!(c.sql.borrow()[0].contains("is_active = 1"));
    }

    #[test]
    fn embiences_need_both_urls() {
        let c = rows_reply(
            &["id", "name", "icon_url", "stream_url"],
            vec![
                vec![
                    json!(1),
                    json!("Rain"),
                    json!("https://x/rain.svg"),
                    json!("https://x/rain.mp3"),
                ],
                vec![
                    json!(2),
                    json!("Bad"),
                    json!("https://x/b.svg"),
                    json!("file:///etc/passwd"),
                ],
            ],
        );
        let out = block_on(embiences(&client(&c))).unwrap();
        assert_eq!(out.items.iter().map(|e| e.id).collect::<Vec<_>>(), [1]);
        assert_eq!(out.skipped, 1);
    }

    #[test]
    fn presets_group_rows_in_order_and_survive_empty_or_dangling_items() {
        let c = rows_reply(
            &["preset_id", "label", "embience_id"],
            vec![
                vec![json!(1), json!("Learning"), json!(6)],
                vec![json!(1), json!("Learning"), json!(12)],
                vec![json!(2), json!("Empty"), json!(null)],
                vec![json!(3), json!("Busy"), json!(9)],
                vec![json!(3), json!("Busy"), json!(6)],
                vec![json!(3), json!("Busy"), json!(6)],
            ],
        );
        let out = block_on(presets(&client(&c))).unwrap();
        let shape: Vec<_> = out
            .items
            .iter()
            .map(|p| (p.id, p.label.as_str(), p.embience_ids.clone()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (1, "Learning", vec![6, 12]),
                (2, "Empty", vec![]),
                (3, "Busy", vec![9, 6]),
            ]
        );
        assert!(c.sql.borrow()[0].contains("i.position"));
    }

    #[test]
    fn wrong_column_types_or_missing_columns_are_hard_errors() {
        let c = rows_reply(
            &["id", "label", "url"],
            vec![vec![json!("one"), json!("x"), json!("https://a.b")]],
        );
        assert_eq!(
            block_on(bookmarks(&client(&c))),
            Err(ContentError::BadData("id"))
        );
        let c = rows_reply(&["id", "label"], vec![vec![json!(1), json!("x")]]);
        assert_eq!(
            block_on(bookmarks(&client(&c))),
            Err(ContentError::BadData("url"))
        );
        let c = rows_reply(
            &["id", "label", "url"],
            vec![vec![json!(-1), json!("x"), json!("https://a.b")]],
        );
        assert_eq!(
            block_on(bookmarks(&client(&c))),
            Err(ContentError::BadData("id"))
        );
    }

    #[test]
    fn database_failures_are_db_errors() {
        let c = Canned {
            reply: Ok(HttpResponse {
                status: 401,
                body: r#"{"message":"Unauthorized"}"#.into(),
            }),
            sql: RefCell::new(Vec::new()),
        };
        assert!(matches!(
            block_on(radio(&client(&c))),
            Err(ContentError::Db(Error::Http { status: 401, .. }))
        ));
        let c = Canned {
            reply: Err("offline".into()),
            sql: RefCell::new(Vec::new()),
        };
        assert!(matches!(
            block_on(generations(&client(&c))),
            Err(ContentError::Db(Error::Transport(_)))
        ));
    }

    #[test]
    fn empty_tables_are_empty_lists_not_errors() {
        let c = rows_reply(&["id", "label", "url"], vec![]);
        let out = block_on(bookmarks(&client(&c))).unwrap();
        assert!(out.items.is_empty());
        assert_eq!(out.skipped, 0);
    }
}
