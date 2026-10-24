//! Editor data umum untuk dashboard admin (D6c).
//!
//! Satu **daftar putih** tabel ([`TABLES`]) menentukan apa yang boleh dilihat, diubah,
//! dan diekspor. Dari daftar itu dibuat SQL-nya (nama tabel dan kolom selalu dari
//! daftar ini, tidak pernah dari klien; nilai selalu parameter terikat) dan aturan
//! validasinya. Tidak ada SQL mentah dari klien, dan tidak ada operasi yang mengubah
//! skema: kolom baru hanya lewat `migrations/`.
//!
//! Server tidak memercayai klien: setiap nilai diperiksa ulang menurut jenis kolomnya,
//! kolom yang tidak dikenal ditolak, jumlah baris dibatasi (`max_rows`, ditegakkan di
//! SQL), dan nama kembar menjadi `Conflict`. SQL yang dibangun dicocokkan dengan berkas
//! emas `tests-data/tables_sql.json`, yang dijalankan di SQLite sungguhan oleh
//! `scripts/migrations.test.mjs`.

use api_types::journal::{clean_text, is_iso_date};
use api_types::validate::{is_http_url, is_route_or_url};
use serde::Serialize;
use serde_json::{json, Map, Value as Json};
use turso_http::{Client, Stmt, StmtResult, Transport, Value};

use crate::content;
use crate::panel::{write_error, PanelError};
use crate::time::format_utc;

const PRESET_ITEMS: &str = include_str!("queries/admin_preset_items.sql");
const PRESET_ITEMS_DELETE: &str = include_str!("queries/admin_preset_items_delete.sql");
const PRESET_ITEM_INSERT: &str = include_str!("queries/admin_preset_item_insert.sql");
const PRESET_EXISTS: &str = include_str!("queries/admin_preset_exists.sql");
const EMBIENCE_IDS: &str = include_str!("queries/admin_embience_ids.sql");
const MESSAGE_PURGE: &str = include_str!("queries/admin_message_purge.sql");
const MESSAGE_PURGE_REPORTS: &str = include_str!("queries/admin_message_purge_reports.sql");

/// Tabel kecil yang tidak berhalaman: satu respons memuat semuanya.
pub const SMALL_LIMIT: u32 = 500;
/// Paling banyak baris dalam satu ekspor (yang terbaru untuk tabel berhalaman).
pub const EXPORT_MAX: u32 = 5000;
/// Paling banyak embience dalam satu preset.
pub const MAX_PRESET_ITEMS: usize = 20;

// ------------------------------------------------------------------- definisi

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Satu baris teks; panjang dihitung dalam karakter.
    Text {
        min: usize,
        max: usize,
    },
    /// `http://` atau `https://`.
    Url,
    /// Rute internal (`/profile`) atau URL `http(s)`.
    RouteOrUrl,
    Int {
        min: i64,
        max: i64,
    },
    /// 0 atau 1 di database; `true`/`false` di JSON.
    Bool,
    /// `YYYY-MM-DD`.
    Date,
    /// UTC `YYYY-MM-DDTHH:MM:SSZ`. Hanya dibaca.
    Timestamp,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Text { .. } => "text",
            Kind::Url => "url",
            Kind::RouteOrUrl => "route",
            Kind::Int { .. } => "int",
            Kind::Bool => "bool",
            Kind::Date => "date",
            Kind::Timestamp => "timestamp",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Col {
    pub name: &'static str,
    pub label: &'static str,
    pub kind: Kind,
    pub nullable: bool,
}

const fn col(name: &'static str, label: &'static str, kind: Kind) -> Col {
    Col {
        name,
        label,
        kind,
        nullable: false,
    }
}

const fn opt(name: &'static str, label: &'static str, kind: Kind) -> Col {
    Col {
        name,
        label,
        kind,
        nullable: true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Edit,
    ReadOnly,
}

#[derive(Debug)]
pub struct Table {
    pub name: &'static str,
    pub label: &'static str,
    /// Kolom kunci (bilangan bulat, `PRIMARY KEY` tunggal).
    pub key: &'static str,
    pub key_label: &'static str,
    /// `true`: pengisi menentukan kunci saat menambah (mis. nomor generation).
    pub key_given: bool,
    pub access: Access,
    pub cols: &'static [Col],
    /// `ORDER BY` untuk daftar (tanpa kata kunci `ORDER BY`).
    pub order: &'static str,
    /// Berhalaman menurut kunci (menurun) untuk tabel yang bisa besar.
    pub paged: bool,
    pub max_rows: u32,
    /// Baris di tabel lain yang ikut dihapus bersama satu baris: `(tabel, kolom)`.
    pub cascade: &'static [(&'static str, &'static str)],
}

const TITLE_80: Kind = Kind::Text { min: 1, max: 80 };
const ORDER_NUM: Kind = Kind::Int {
    min: -1_000_000,
    max: 1_000_000,
};

pub static TABLES: &[Table] = &[
    Table {
        name: "generations",
        label: "Generasi boot",
        key: "number",
        key_label: "Nomor",
        key_given: true,
        access: Access::Edit,
        cols: &[
            col("label", "Nama", TITLE_80),
            col("link", "Tautan", Kind::RouteOrUrl),
            col("kernel", "Kernel", Kind::Text { min: 1, max: 32 }),
            col("built_on", "Tanggal build", Kind::Date),
        ],
        order: "number DESC",
        paged: false,
        max_rows: 20,
        cascade: &[],
    },
    Table {
        name: "bookmarks",
        label: "Bookmark",
        key: "id",
        key_label: "ID",
        key_given: false,
        access: Access::Edit,
        cols: &[
            col("label", "Nama", TITLE_80),
            col("url", "Alamat", Kind::Url),
            col("sort_order", "Urutan", ORDER_NUM),
        ],
        order: "sort_order, id",
        paged: false,
        max_rows: 50,
        cascade: &[],
    },
    Table {
        name: "radio_stations",
        label: "Stasiun radio",
        key: "id",
        key_label: "ID",
        key_given: false,
        access: Access::Edit,
        cols: &[
            col("label", "Nama", TITLE_80),
            col("slogan", "Slogan", Kind::Text { min: 0, max: 160 }),
            col("stream_url", "Alamat stream", Kind::Url),
            opt("official_link", "Situs resmi", Kind::Url),
            opt("image_url", "Gambar", Kind::Url),
            col("sort_order", "Urutan", ORDER_NUM),
            col("is_active", "Aktif", Kind::Bool),
        ],
        order: "sort_order, id",
        paged: false,
        max_rows: 60,
        cascade: &[],
    },
    Table {
        name: "embiences",
        label: "Suara latar",
        key: "id",
        key_label: "ID",
        key_given: false,
        access: Access::Edit,
        cols: &[
            col("name", "Nama", Kind::Text { min: 1, max: 40 }),
            col("icon_url", "Ikon", Kind::Url),
            col("stream_url", "Alamat audio", Kind::Url),
            col("sort_order", "Urutan", ORDER_NUM),
        ],
        order: "sort_order, id",
        paged: false,
        max_rows: 60,
        cascade: &[("embience_preset_items", "embience_id")],
    },
    Table {
        name: "embience_presets",
        label: "Preset suara latar",
        key: "id",
        key_label: "ID",
        key_given: false,
        access: Access::Edit,
        cols: &[
            col("label", "Nama", Kind::Text { min: 1, max: 40 }),
            col("sort_order", "Urutan", ORDER_NUM),
        ],
        order: "sort_order, id",
        paged: false,
        max_rows: 30,
        cascade: &[("embience_preset_items", "preset_id")],
    },
    Table {
        name: "chat_messages",
        label: "Entri Guestbook",
        key: "id",
        key_label: "ID",
        key_given: false,
        access: Access::ReadOnly,
        cols: &[
            col("username", "Nama", Kind::Text { min: 1, max: 64 }),
            opt("url", "Tautan", Kind::Url),
            col("message", "Pesan", Kind::Text { min: 1, max: 4000 }),
            col("created_at", "Dibuat", Kind::Timestamp),
            opt("deleted_at", "Disembunyikan", Kind::Timestamp),
        ],
        order: "id DESC",
        paged: true,
        max_rows: 0,
        cascade: &[],
    },
    Table {
        name: "admin_audit",
        label: "Riwayat admin",
        key: "id",
        key_label: "ID",
        key_given: false,
        access: Access::ReadOnly,
        cols: &[
            col("at", "Waktu", Kind::Timestamp),
            col("action", "Aksi", Kind::Text { min: 1, max: 40 }),
            opt("target", "Sasaran", Kind::Text { min: 1, max: 120 }),
        ],
        order: "id DESC",
        paged: true,
        max_rows: 0,
        cascade: &[],
    },
    Table {
        name: "journal_pinned_repos",
        label: "Repo sematan journal",
        key: "id",
        key_label: "ID",
        key_given: false,
        access: Access::ReadOnly,
        cols: &[
            col("position", "Posisi", ORDER_NUM),
            col("repo_name", "Repo", Kind::Text { min: 1, max: 100 }),
            opt("topic", "Topik", Kind::Text { min: 1, max: 60 }),
        ],
        order: "position, id",
        paged: false,
        max_rows: 0,
        cascade: &[],
    },
    Table {
        name: "journal_stack",
        label: "Stack journal",
        key: "id",
        key_label: "ID",
        key_given: false,
        access: Access::ReadOnly,
        cols: &[
            col("position", "Posisi", ORDER_NUM),
            col("name", "Nama", Kind::Text { min: 1, max: 40 }),
        ],
        order: "position, id",
        paged: false,
        max_rows: 0,
        cascade: &[],
    },
];

/// Mencari tabel menurut nama; `None` untuk apa pun di luar daftar putih.
pub fn find(name: &str) -> Option<&'static Table> {
    TABLES.iter().find(|t| t.name == name)
}

impl Table {
    pub fn editable(&self) -> bool {
        self.access == Access::Edit
    }

    /// Kunci diikuti kolom, urutan yang sama dengan `SELECT` dan ekspor.
    fn all_names(&self) -> Vec<&'static str> {
        std::iter::once(self.key)
            .chain(self.cols.iter().map(|c| c.name))
            .collect()
    }
}

// ------------------------------------------------------------------------ SQL

/// Seluruh SQL sebuah tabel. Dibangun dari daftar putih saja.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct TableSql {
    pub list: String,
    pub exists: String,
    pub insert: String,
    pub update: String,
    pub delete: Vec<String>,
}

pub fn sql_for(t: &Table) -> TableSql {
    let names = t.all_names().join(", ");
    let list = format!(
        "SELECT {names} FROM {} WHERE (?1 IS NULL OR {} < ?1) ORDER BY {} LIMIT ?2",
        t.name, t.key, t.order
    );
    let exists = format!("SELECT {} FROM {} WHERE {} = ?1", t.key, t.name, t.key);

    // Menambah: nilai kolom (dan kunci bila diberikan pengisi) lalu batas jumlah baris.
    let insert_cols: Vec<&str> = if t.key_given {
        t.all_names()
    } else {
        t.cols.iter().map(|c| c.name).collect()
    };
    let marks: Vec<String> = (1..=insert_cols.len()).map(|i| format!("?{i}")).collect();
    let insert = format!(
        "INSERT INTO {} ({}) SELECT {} WHERE (SELECT count(*) FROM {}) < ?{} RETURNING {}",
        t.name,
        insert_cols.join(", "),
        marks.join(", "),
        t.name,
        insert_cols.len() + 1,
        t.key
    );

    // Mengubah: ?1 = kunci, ?2.. = kolom.
    let sets: Vec<String> = t
        .cols
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{} = ?{}", c.name, i + 2))
        .collect();
    let update = format!(
        "UPDATE {} SET {} WHERE {} = ?1 RETURNING {}",
        t.name,
        sets.join(", "),
        t.key,
        t.key
    );

    // Menghapus: dulu baris turunan, baris utama terakhir (hasilnya menentukan 404).
    let mut delete: Vec<String> = t
        .cascade
        .iter()
        .map(|(table, column)| format!("DELETE FROM {table} WHERE {column} = ?1"))
        .collect();
    delete.push(format!(
        "DELETE FROM {} WHERE {} = ?1 RETURNING {}",
        t.name, t.key, t.key
    ));

    TableSql {
        list,
        exists,
        insert,
        update,
        delete,
    }
}

/// Satu baris berisi jumlah baris tiap tabel, alias = nama tabel.
pub fn counts_sql() -> String {
    let parts: Vec<String> = TABLES
        .iter()
        .map(|t| format!("(SELECT count(*) FROM {0}) AS {0}", t.name))
        .collect();
    format!("SELECT {}", parts.join(", "))
}

/// Isi berkas emas: dicocokkan oleh tes Rust, dijalankan oleh tes node.
pub fn golden() -> Json {
    let mut tables = Map::new();
    for t in TABLES {
        let sql = sql_for(t);
        tables.insert(
            t.name.to_owned(),
            json!({
                "key": t.key,
                "key_given": t.key_given,
                "editable": t.editable(),
                "paged": t.paged,
                "max_rows": t.max_rows,
                "columns": t.cols.iter().map(|c| json!({
                    "name": c.name,
                    "kind": c.kind.name(),
                    "nullable": c.nullable,
                    "min": kind_min(c.kind),
                    "max": kind_max(c.kind),
                })).collect::<Vec<_>>(),
                "sql": sql,
            }),
        );
    }
    json!({ "counts": counts_sql(), "tables": tables })
}

fn kind_min(k: Kind) -> Json {
    match k {
        Kind::Text { min, .. } => json!(min),
        Kind::Int { min, .. } => json!(min),
        _ => Json::Null,
    }
}

fn kind_max(k: Kind) -> Json {
    match k {
        Kind::Text { max, .. } => json!(max),
        Kind::Int { max, .. } => json!(max),
        _ => Json::Null,
    }
}

// ------------------------------------------------------------------ validasi

const BAD_VALUE: PanelError = PanelError::Invalid("A value is not valid.");

/// Memeriksa satu nilai dari klien menurut jenis kolomnya. `Ok(None)` = NULL.
fn check_value(c: &Col, raw: Option<&Json>) -> Result<Value, PanelError> {
    let null_ok = c.nullable;
    let raw = match raw {
        None | Some(Json::Null) if null_ok => return Ok(Value::Null),
        None | Some(Json::Null) => return Err(BAD_VALUE),
        Some(v) => v,
    };
    match c.kind {
        Kind::Text { min, max } => {
            let s = clean_text(raw.as_str().ok_or(BAD_VALUE)?, usize::MAX);
            if s.is_empty() && null_ok {
                return Ok(Value::Null);
            }
            let n = s.chars().count();
            if n < min || n > max {
                return Err(BAD_VALUE);
            }
            Ok(Value::from(s))
        }
        Kind::Url | Kind::RouteOrUrl => {
            let s = raw.as_str().ok_or(BAD_VALUE)?.trim();
            if s.is_empty() && null_ok {
                return Ok(Value::Null);
            }
            let ok = if c.kind == Kind::Url {
                is_http_url(s)
            } else {
                is_route_or_url(s)
            };
            if ok {
                Ok(Value::from(s))
            } else {
                Err(BAD_VALUE)
            }
        }
        Kind::Int { min, max } => {
            let n = raw.as_i64().ok_or(BAD_VALUE)?;
            if (min..=max).contains(&n) {
                Ok(Value::from(n))
            } else {
                Err(BAD_VALUE)
            }
        }
        Kind::Bool => match raw {
            Json::Bool(b) => Ok(Value::from(i64::from(*b))),
            other => match other.as_i64() {
                Some(n @ (0 | 1)) => Ok(Value::from(n)),
                _ => Err(BAD_VALUE),
            },
        },
        Kind::Date => {
            let s = raw.as_str().ok_or(BAD_VALUE)?.trim();
            if is_iso_date(s) {
                Ok(Value::from(s))
            } else {
                Err(BAD_VALUE)
            }
        }
        // Tidak pernah ditulis dari klien (hanya tabel read-only memakainya).
        Kind::Timestamp => Err(BAD_VALUE),
    }
}

/// Isi baris yang sudah diperiksa: kunci (bila diberikan) dan nilai tiap kolom.
#[derive(Debug, PartialEq)]
pub struct CleanRow {
    pub key: Option<u32>,
    pub values: Vec<Value>,
}

/// Memeriksa badan permintaan tambah (`creating`) atau ubah. Kolom yang tidak
/// dikenal, kunci pada perubahan, dan tabel read-only ditolak.
pub fn clean_row(
    t: &Table,
    body: &Map<String, Json>,
    creating: bool,
) -> Result<CleanRow, PanelError> {
    if !t.editable() {
        return Err(PanelError::Invalid("This table is read-only."));
    }
    let key_allowed = creating && t.key_given;
    for name in body.keys() {
        let known = t.cols.iter().any(|c| c.name == name) || (key_allowed && name == t.key);
        if !known {
            return Err(PanelError::Invalid("Unknown field."));
        }
    }
    let key = if key_allowed {
        let n = body.get(t.key).and_then(Json::as_i64).ok_or(BAD_VALUE)?;
        Some(u32::try_from(n).ok().filter(|k| *k >= 1).ok_or(BAD_VALUE)?)
    } else {
        None
    };
    let values = t
        .cols
        .iter()
        .map(|c| check_value(c, body.get(c.name)))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CleanRow { key, values })
}

// -------------------------------------------------------------------- keluaran

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ColumnInfo {
    pub name: &'static str,
    pub label: &'static str,
    pub kind: &'static str,
    pub nullable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableInfo {
    pub name: &'static str,
    pub label: &'static str,
    pub editable: bool,
    pub key: &'static str,
    pub key_label: &'static str,
    pub key_given: bool,
    pub paged: bool,
    pub max_rows: u32,
    pub rows: u32,
    pub columns: Vec<ColumnInfo>,
}

fn column_info(c: &Col) -> ColumnInfo {
    let (min, max) = match c.kind {
        Kind::Text { min, max } => (Some(min as i64), Some(max as i64)),
        Kind::Int { min, max } => (Some(min), Some(max)),
        _ => (None, None),
    };
    ColumnInfo {
        name: c.name,
        label: c.label,
        kind: c.kind.name(),
        nullable: c.nullable,
        min,
        max,
    }
}

/// Daftar tabel beserta skemanya dan jumlah barisnya (satu query).
pub async fn overview<T: Transport>(client: &Client<T>) -> Result<Vec<TableInfo>, PanelError> {
    let res = client.execute(Stmt::new(counts_sql())).await?;
    TABLES
        .iter()
        .map(|t| {
            Ok(TableInfo {
                name: t.name,
                label: t.label,
                editable: t.editable(),
                key: t.key,
                key_label: t.key_label,
                key_given: t.key_given,
                paged: t.paged,
                max_rows: t.max_rows,
                rows: content::id(&res, 0, t.name)?,
                columns: t.cols.iter().map(column_info).collect(),
            })
        })
        .collect()
}

fn cell_json(
    res: &StmtResult,
    row: usize,
    name: &'static str,
    kind: Kind,
    nullable: bool,
) -> Result<Json, PanelError> {
    let v = res.get(row, name).ok_or(PanelError::BadData(name))?;
    if v.is_null() {
        return if nullable {
            Ok(Json::Null)
        } else {
            Err(PanelError::BadData(name))
        };
    }
    match kind {
        Kind::Int { .. } => v.as_i64().map(Json::from).ok_or(PanelError::BadData(name)),
        Kind::Bool => match v.as_i64() {
            Some(0) => Ok(Json::Bool(false)),
            Some(1) => Ok(Json::Bool(true)),
            _ => Err(PanelError::BadData(name)),
        },
        _ => v.as_str().map(Json::from).ok_or(PanelError::BadData(name)),
    }
}

/// Satu baris sebagai objek JSON: kunci lalu kolom. Isi apa adanya (admin perlu
/// melihat baris yang rusak untuk memperbaikinya).
pub fn row_json(t: &Table, res: &StmtResult, row: usize) -> Result<Map<String, Json>, PanelError> {
    let mut out = Map::new();
    let key = res
        .get(row, t.key)
        .and_then(|v| v.as_i64())
        .ok_or(PanelError::BadData("key"))?;
    out.insert(t.key.to_owned(), Json::from(key));
    for c in t.cols {
        out.insert(
            c.name.to_owned(),
            cell_json(res, row, c.name, c.kind, c.nullable)?,
        );
    }
    Ok(out)
}

fn rows_json(t: &Table, res: &StmtResult) -> Result<Vec<Map<String, Json>>, PanelError> {
    (0..res.rows.len()).map(|i| row_json(t, res, i)).collect()
}

// ------------------------------------------------------------------- operasi

/// Satu halaman baris. Tabel yang tidak berhalaman selalu dimuat penuh.
pub async fn list<T: Transport>(
    client: &Client<T>,
    t: &Table,
    before: Option<u32>,
    limit: u32,
) -> Result<Vec<Map<String, Json>>, PanelError> {
    let (before, limit) = if t.paged {
        (before, limit)
    } else {
        (None, SMALL_LIMIT)
    };
    let res = client
        .execute(
            Stmt::new(sql_for(t).list)
                .bind(before.map_or(Value::Null, Value::from))
                .bind(limit),
        )
        .await?;
    rows_json(t, &res)
}

/// Menambah baris. Mengembalikan kunci baru.
pub async fn insert<T: Transport>(
    client: &Client<T>,
    t: &Table,
    body: &Map<String, Json>,
) -> Result<u32, PanelError> {
    let row = clean_row(t, body, true)?;
    let mut stmt = Stmt::new(sql_for(t).insert);
    if let Some(key) = row.key {
        stmt = stmt.bind(key);
    }
    for v in row.values {
        stmt = stmt.bind(v);
    }
    stmt = stmt.bind(t.max_rows);
    let res = client.execute(stmt).await.map_err(write_error)?;
    if res.rows.is_empty() {
        // Satu-satunya syarat INSERT selain constraint: batas jumlah baris.
        return Err(PanelError::Invalid("The table is full."));
    }
    Ok(content::id(&res, 0, t.key)?)
}

/// Mengubah baris yang ada. Kunci tidak bisa diubah.
pub async fn update<T: Transport>(
    client: &Client<T>,
    t: &Table,
    id: u32,
    body: &Map<String, Json>,
) -> Result<(), PanelError> {
    let row = clean_row(t, body, false)?;
    let mut stmt = Stmt::new(sql_for(t).update).bind(id);
    for v in row.values {
        stmt = stmt.bind(v);
    }
    let res = client.execute(stmt).await.map_err(write_error)?;
    if res.rows.is_empty() {
        return Err(PanelError::NotFound);
    }
    Ok(())
}

/// Menghapus baris beserta turunannya dalam satu transaksi.
pub async fn delete<T: Transport>(
    client: &Client<T>,
    t: &Table,
    id: u32,
) -> Result<(), PanelError> {
    if !t.editable() {
        return Err(PanelError::Invalid("This table is read-only."));
    }
    let stmts: Vec<Stmt> = sql_for(t)
        .delete
        .into_iter()
        .map(|sql| Stmt::new(sql).bind(id))
        .collect();
    let results = client.transaction(stmts).await?;
    match results.last() {
        Some(last) if !last.rows.is_empty() => Ok(()),
        _ => Err(PanelError::NotFound),
    }
}

// --------------------------------------------------------------------- ekspor

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Csv,
}

/// `format=json|csv` (bawaan `json`); `None` untuk nilai lain.
pub fn parse_format(raw: Option<&str>) -> Option<Format> {
    let mut format = Format::Json;
    for pair in raw.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == "format" {
            format = match value {
                "json" => Format::Json,
                "csv" => Format::Csv,
                _ => return None,
            };
        }
    }
    Some(format)
}

pub struct Exported {
    pub content_type: &'static str,
    pub filename: String,
    pub body: String,
}

/// Satu sel CSV (RFC 4180). Teks yang diawali `=`, `+`, `-`, `@`, tab, atau CR diberi
/// awalan `'` supaya program spreadsheet tidak menjalankannya sebagai rumus.
pub fn csv_cell(v: &Json) -> String {
    let text = match v {
        Json::Null => return String::new(),
        Json::Bool(b) => return if *b { "1" } else { "0" }.to_owned(),
        Json::Number(n) => return n.to_string(),
        Json::String(s) => s.clone(),
        other => other.to_string(),
    };
    let guarded = if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{text}")
    } else {
        text
    };
    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

pub fn to_csv(t: &Table, rows: &[Map<String, Json>]) -> String {
    let names = t.all_names();
    let mut out = names.join(",");
    out.push_str("\r\n");
    for row in rows {
        let cells: Vec<String> = names
            .iter()
            .map(|n| csv_cell(row.get(*n).unwrap_or(&Json::Null)))
            .collect();
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }
    out
}

/// Mengekspor paling banyak [`EXPORT_MAX`] baris (yang terbaru untuk tabel berhalaman).
pub async fn export<T: Transport>(
    client: &Client<T>,
    t: &Table,
    format: Format,
    now_secs: i64,
) -> Result<Exported, PanelError> {
    let res = client
        .execute(
            Stmt::new(sql_for(t).list)
                .bind(Value::Null)
                .bind(EXPORT_MAX + 1),
        )
        .await?;
    let mut rows = rows_json(t, &res)?;
    let truncated = rows.len() > EXPORT_MAX as usize;
    rows.truncate(EXPORT_MAX as usize);
    Ok(match format {
        Format::Csv => Exported {
            content_type: "text/csv; charset=utf-8",
            filename: format!("{}.csv", t.name),
            body: to_csv(t, &rows),
        },
        Format::Json => Exported {
            content_type: "application/json; charset=utf-8",
            filename: format!("{}.json", t.name),
            body: serde_json::to_string_pretty(&json!({
                "table": t.name,
                "exported_at": format_utc(now_secs),
                "truncated": truncated,
                "rows": rows,
            }))
            .map_err(|_| PanelError::BadData("export"))?,
        },
    })
}

// --------------------------------------------------------------- isi preset

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PresetItem {
    pub preset_id: u32,
    pub embience_id: u32,
    pub position: u32,
}

pub async fn preset_items<T: Transport>(client: &Client<T>) -> Result<Vec<PresetItem>, PanelError> {
    let res = client.execute(Stmt::new(PRESET_ITEMS)).await?;
    (0..res.rows.len())
        .map(|i| {
            Ok(PresetItem {
                preset_id: content::id(&res, i, "preset_id")?,
                embience_id: content::id(&res, i, "embience_id")?,
                position: content::id(&res, i, "position")?,
            })
        })
        .collect()
}

/// Mengganti seluruh isi satu preset dengan `ids` (urutan = urutan putar). Setiap id
/// harus embience yang ada, tanpa kembar; preset-nya harus ada.
pub async fn set_preset_items<T: Transport>(
    client: &Client<T>,
    preset_id: u32,
    ids: &[u32],
) -> Result<(), PanelError> {
    let mut sorted = ids.to_vec();
    sorted.sort_unstable();
    if ids.len() > MAX_PRESET_ITEMS || sorted.windows(2).any(|w| w[0] == w[1]) {
        return Err(PanelError::Invalid("The list is not valid."));
    }
    let checks = client
        .transaction(vec![
            Stmt::new(PRESET_EXISTS).bind(preset_id),
            Stmt::new(EMBIENCE_IDS),
        ])
        .await?;
    let [preset, embiences] =
        <[_; 2]>::try_from(checks).map_err(|_| PanelError::BadData("preset check"))?;
    if preset.rows.is_empty() {
        return Err(PanelError::NotFound);
    }
    let known = (0..embiences.rows.len())
        .map(|i| content::id(&embiences, i, "id"))
        .collect::<Result<Vec<_>, _>>()?;
    if ids.iter().any(|id| !known.contains(id)) {
        return Err(PanelError::Invalid("The list is not valid."));
    }
    let mut stmts = vec![Stmt::new(PRESET_ITEMS_DELETE).bind(preset_id).no_rows()];
    for (id, position) in ids.iter().zip(0u32..) {
        stmts.push(
            Stmt::new(PRESET_ITEM_INSERT)
                .bind(preset_id)
                .bind(*id)
                .bind(position)
                .no_rows(),
        );
    }
    client.transaction(stmts).await?;
    Ok(())
}

// ------------------------------------------------------------ hapus permanen

/// Teks konfirmasi yang harus dikirim klien untuk menghapus permanen entri `id`.
pub fn purge_confirmation(id: u32) -> String {
    format!("purge:{id}")
}

/// Menghapus permanen entri Guestbook **yang sudah disembunyikan**, beserta
/// laporannya. `NotFound` bila tidak ada atau masih tampil.
pub async fn purge_message<T: Transport>(
    client: &Client<T>,
    id: u32,
    confirm: &str,
) -> Result<(), PanelError> {
    if confirm != purge_confirmation(id) {
        return Err(PanelError::Invalid("Confirmation does not match."));
    }
    let results = client
        .transaction(vec![
            Stmt::new(MESSAGE_PURGE).bind(id),
            Stmt::new(MESSAGE_PURGE_REPORTS).bind(id).no_rows(),
        ])
        .await?;
    match results.first() {
        Some(first) if !first.rows.is_empty() => Ok(()),
        _ => Err(PanelError::NotFound),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{client, execute_error, execute_reply, result, tx_reply, Script};

    fn table(name: &str) -> &'static Table {
        find(name).unwrap()
    }

    fn body(v: Json) -> Map<String, Json> {
        v.as_object().unwrap().clone()
    }

    fn int(n: i64) -> Json {
        json!({"type": "integer", "value": n.to_string()})
    }

    fn text(s: &str) -> Json {
        json!({"type": "text", "value": s})
    }

    fn radio() -> Json {
        json!({
            "label": "  Delta   FM ", "slogan": "", "stream_url": "https://a.example/s",
            "official_link": "", "sort_order": 3, "is_active": true
        })
    }

    // ------------------------------------------------------------- daftar putih

    #[test]
    fn the_registry_is_consistent() {
        let mut names = std::collections::HashSet::new();
        for t in TABLES {
            assert!(names.insert(t.name), "duplicate table {}", t.name);
            let ident =
                |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
            assert!(ident(t.name) && ident(t.key), "{}", t.name);
            let mut cols = std::collections::HashSet::new();
            for c in t.cols {
                assert!(ident(c.name), "{}.{}", t.name, c.name);
                assert!(
                    cols.insert(c.name),
                    "duplicate column {}.{}",
                    t.name,
                    c.name
                );
                assert_ne!(c.name, t.key, "{} repeats its key", t.name);
                assert!(!c.label.is_empty());
                if t.editable() {
                    assert_ne!(
                        c.kind,
                        Kind::Timestamp,
                        "{}.{} cannot be written",
                        t.name,
                        c.name
                    );
                }
            }
            if t.editable() {
                assert!(t.max_rows > 0, "{} needs a cap", t.name);
            } else {
                assert!(t.cascade.is_empty(), "{} is read-only", t.name);
            }
            for (other, column) in t.cascade {
                assert!(ident(other) && ident(column));
            }
            // ORDER BY memakai kolom yang ada.
            for part in t.order.split(',') {
                let name = part.split_whitespace().next().unwrap();
                assert!(t.all_names().contains(&name), "{}: order by {name}", t.name);
            }
            if t.paged {
                assert!(
                    t.order.starts_with(&format!("{} DESC", t.key)),
                    "{}",
                    t.name
                );
            }
        }
    }

    #[test]
    fn only_listed_tables_can_be_found() {
        assert!(find("radio_stations").is_some());
        for bad in [
            "",
            "_migrations",
            "sqlite_master",
            "admin_attempts",
            "radio_stations; DROP TABLE x",
            "RADIO_STATIONS",
            "message_reports",
            "journal_snapshots",
        ] {
            assert!(find(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_overview_list_matches_the_admin_dashboard_tables() {
        // Setiap tabel yang bisa diubah juga muncul di ringkasan jumlah baris.
        for t in TABLES.iter().filter(|t| t.editable()) {
            assert!(crate::panel::TABLES.contains(&t.name), "{}", t.name);
        }
    }

    // ----------------------------------------------------------------------- SQL

    #[test]
    fn generated_sql_is_what_we_expect() {
        let b = sql_for(table("bookmarks"));
        assert_eq!(
            b.list,
            "SELECT id, label, url, sort_order FROM bookmarks WHERE (?1 IS NULL OR id < ?1) ORDER BY sort_order, id LIMIT ?2"
        );
        assert_eq!(
            b.insert,
            "INSERT INTO bookmarks (label, url, sort_order) SELECT ?1, ?2, ?3 WHERE (SELECT count(*) FROM bookmarks) < ?4 RETURNING id"
        );
        assert_eq!(
            b.update,
            "UPDATE bookmarks SET label = ?2, url = ?3, sort_order = ?4 WHERE id = ?1 RETURNING id"
        );
        assert_eq!(
            b.delete,
            ["DELETE FROM bookmarks WHERE id = ?1 RETURNING id"]
        );
        let g = sql_for(table("generations"));
        assert!(g.insert.starts_with("INSERT INTO generations (number, label, link, kernel, built_on) SELECT ?1, ?2, ?3, ?4, ?5 WHERE"));
        assert!(g.insert.contains("< ?6 RETURNING number"));
        let e = sql_for(table("embiences"));
        assert_eq!(
            e.delete,
            [
                "DELETE FROM embience_preset_items WHERE embience_id = ?1",
                "DELETE FROM embiences WHERE id = ?1 RETURNING id"
            ]
        );
    }

    #[test]
    fn sql_never_contains_values_only_placeholders() {
        for t in TABLES {
            let s = sql_for(t);
            for sql in [&s.list, &s.exists, &s.insert, &s.update]
                .into_iter()
                .chain(s.delete.iter())
            {
                assert!(!sql.contains('\''), "{sql}");
                assert!(sql.contains("?1"), "{sql}");
            }
        }
    }

    #[test]
    fn the_golden_file_matches_the_generated_sql() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests-data/tables_sql.json");
        let fresh = serde_json::to_string_pretty(&golden()).unwrap() + "\n";
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::write(path, &fresh).unwrap();
        }
        let saved = std::fs::read_to_string(path).expect("run with UPDATE_GOLDEN=1 to create it");
        assert_eq!(
            saved, fresh,
            "tests-data/tables_sql.json is stale: UPDATE_GOLDEN=1 cargo test -p api tables"
        );
    }

    // ------------------------------------------------------------------ validasi

    #[test]
    fn a_valid_radio_row_is_cleaned() {
        let row = clean_row(table("radio_stations"), &body(radio()), true).unwrap();
        assert_eq!(row.key, None);
        let want = vec![
            Value::from("Delta FM"),
            Value::from(""),
            Value::from("https://a.example/s"),
            Value::Null,
            Value::Null,
            Value::from(3_i64),
            Value::from(1_i64),
        ];
        assert_eq!(row.values, want);
    }

    #[test]
    fn nullable_columns_may_be_missing_but_required_ones_may_not() {
        let mut v = radio();
        v.as_object_mut().unwrap().remove("official_link");
        v.as_object_mut().unwrap().remove("image_url");
        assert!(clean_row(table("radio_stations"), &body(v.clone()), true).is_ok());
        for required in ["label", "slogan", "stream_url", "sort_order", "is_active"] {
            let mut w = v.clone();
            w.as_object_mut().unwrap().remove(required);
            assert!(
                clean_row(table("radio_stations"), &body(w), true).is_err(),
                "{required}"
            );
        }
        let mut n = v;
        n["label"] = Json::Null;
        assert!(clean_row(table("radio_stations"), &body(n), true).is_err());
    }

    #[test]
    fn bad_values_are_rejected_by_kind() {
        let t = table("radio_stations");
        let with = |k: &str, v: Json| {
            let mut r = radio();
            r[k] = v;
            clean_row(t, &body(r), true)
        };
        assert!(with("label", json!("")).is_err());
        assert!(with("label", json!(" \n ")).is_err());
        assert!(with("label", json!("x".repeat(81))).is_err());
        assert!(with("label", json!("x".repeat(80))).is_ok());
        assert!(with("label", json!(5)).is_err());
        assert!(with("slogan", json!("s".repeat(161))).is_err());
        for url in [
            "javascript:alert(1)",
            "ftp://a.example",
            "//a.example",
            "https://",
            "https://a b",
            "data:text/html,x",
            "/relative",
        ] {
            assert!(with("stream_url", json!(url)).is_err(), "{url}");
            assert!(with("image_url", json!(url)).is_err(), "{url}");
        }
        assert!(with("stream_url", json!("")).is_err(), "required url");
        assert!(with("image_url", json!("https://i.example/a.png")).is_ok());
        assert!(with("sort_order", json!(1.5)).is_err());
        assert!(with("sort_order", json!("3")).is_err());
        assert!(with("sort_order", json!(1_000_001)).is_err());
        assert!(with("sort_order", json!(-1_000_000)).is_ok());
        assert!(with("is_active", json!(2)).is_err());
        assert!(with("is_active", json!("yes")).is_err());
        assert!(with("is_active", json!(0)).is_ok());
        assert!(with("is_active", json!(false)).is_ok());
    }

    #[test]
    fn hidden_characters_never_reach_the_database() {
        let r = clean_row(
            table("bookmarks"),
            &body(json!({"label": "a\u{202e}b\u{200b}c\nd", "url": "https://a.example", "sort_order": 0})),
            true,
        )
        .unwrap();
        assert_eq!(r.values[0], Value::from("abc d"));
    }

    #[test]
    fn unknown_fields_and_the_key_are_handled_strictly() {
        let t = table("bookmarks");
        let ok = json!({"label": "a", "url": "https://a.example", "sort_order": 0});
        for extra in ["id", "foo", "deleted_at", "rowid", "label2"] {
            let mut v = ok.clone();
            v[extra] = json!(1);
            assert!(
                clean_row(t, &body(v.clone()), true).is_err(),
                "create {extra}"
            );
            assert!(clean_row(t, &body(v), false).is_err(), "update {extra}");
        }
        // Tabel dengan kunci yang diberikan: wajib saat menambah, dilarang saat mengubah.
        let g = table("generations");
        let gen = json!({"number": 6, "label": "x", "link": "/deisktify", "kernel": "6.1", "built_on": "2026-10-01"});
        assert_eq!(clean_row(g, &body(gen.clone()), true).unwrap().key, Some(6));
        assert!(
            clean_row(g, &body(gen.clone()), false).is_err(),
            "key cannot change"
        );
        let mut no_key = gen.clone();
        no_key.as_object_mut().unwrap().remove("number");
        assert!(clean_row(g, &body(no_key.clone()), true).is_err());
        assert!(clean_row(g, &body(no_key), false).is_ok());
        for bad in [
            json!(0),
            json!(-1),
            json!(1.5),
            json!("6"),
            json!(4_294_967_296_i64),
        ] {
            let mut v = gen.clone();
            v["number"] = bad;
            assert!(clean_row(g, &body(v), true).is_err());
        }
    }

    #[test]
    fn generation_links_and_dates_are_checked() {
        let g = table("generations");
        let with = |k: &str, v: &str| {
            let mut r = json!({"label": "x", "link": "/deisktify", "kernel": "6.1", "built_on": "2026-10-01"});
            r[k] = json!(v);
            clean_row(g, &body(r), false)
        };
        assert!(with("link", "https://a.example").is_ok());
        assert!(with("link", "/profile").is_ok());
        for bad in ["//evil.example", "/\\evil", "javascript:x", "", "/a b"] {
            assert!(with("link", bad).is_err(), "{bad}");
        }
        for bad in ["2026-13-01", "2026-1-1", "01/02/2026", "", "2026-10-32"] {
            assert!(with("built_on", bad).is_err(), "{bad}");
        }
        assert!(with("built_on", "2026-02-28").is_ok());
    }

    #[test]
    fn read_only_tables_refuse_every_write() {
        for name in [
            "chat_messages",
            "admin_audit",
            "journal_stack",
            "journal_pinned_repos",
        ] {
            let t = table(name);
            assert!(clean_row(t, &Map::new(), true).is_err(), "{name}");
            futures_executor::block_on(async {
                let s = Script::new(vec![]);
                let c = client(&s);
                assert!(insert(&c, t, &Map::new()).await.is_err());
                assert!(update(&c, t, 1, &Map::new()).await.is_err());
                assert!(delete(&c, t, 1).await.is_err());
                assert_eq!(s.requests(), 0, "{name} never reaches the database");
            });
        }
    }

    // ----------------------------------------------------------------- operasi

    #[test]
    fn the_overview_reads_counts_in_one_query() {
        futures_executor::block_on(async {
            let cols: Vec<&str> = TABLES.iter().map(|t| t.name).collect();
            let vals: Vec<Json> = (0..TABLES.len()).map(|i| json!(i as i64)).collect();
            let s = Script::new(vec![execute_reply(result(&cols, &[vals]))]);
            let info = overview(&client(&s)).await.unwrap();
            assert_eq!(s.requests(), 1);
            assert_eq!(info.len(), TABLES.len());
            assert_eq!(info[1].name, "bookmarks");
            assert_eq!(info[1].rows, 1);
            assert!(info[0].editable && !info.last().unwrap().editable);
            let radio = info.iter().find(|t| t.name == "radio_stations").unwrap();
            assert_eq!(radio.columns.len(), 7);
            assert_eq!(radio.columns[6].kind, "bool");
            assert_eq!(radio.columns[0].max, Some(80));
            let g = info.iter().find(|t| t.name == "generations").unwrap();
            assert!(g.key_given);
            assert_eq!(g.columns[1].kind, "route");
        });
    }

    #[test]
    fn listing_returns_typed_objects_and_binds_the_cursor_only_for_paged_tables() {
        futures_executor::block_on(async {
            let cols = [
                "id",
                "label",
                "slogan",
                "stream_url",
                "official_link",
                "image_url",
                "sort_order",
                "is_active",
            ];
            let row = vec![
                json!(4),
                json!("Delta"),
                json!(""),
                json!("https://a.example"),
                json!(null),
                json!("https://i.example/x"),
                json!(2),
                json!(1),
            ];
            let s = Script::new(vec![execute_reply(result(&cols, &[row]))]);
            let rows = list(&client(&s), table("radio_stations"), Some(99), 10)
                .await
                .unwrap();
            assert_eq!(
                s.args(0, 0),
                vec![json!({"type": "null"}), int(500)],
                "small tables ignore the cursor and limit"
            );
            assert_eq!(rows[0]["id"], json!(4));
            assert_eq!(rows[0]["is_active"], json!(true));
            assert_eq!(rows[0]["official_link"], Json::Null);
            assert_eq!(rows[0]["sort_order"], json!(2));

            let cols = [
                "id",
                "username",
                "url",
                "message",
                "created_at",
                "deleted_at",
            ];
            let s = Script::new(vec![execute_reply(result(&cols, &[]))]);
            list(&client(&s), table("chat_messages"), Some(99), 10)
                .await
                .unwrap();
            assert_eq!(s.args(0, 0), vec![int(99), int(10)]);
        });
    }

    #[test]
    fn a_wrongly_typed_cell_is_bad_data_not_a_panic() {
        futures_executor::block_on(async {
            let cols = ["id", "label", "url", "sort_order"];
            let s = Script::new(vec![execute_reply(result(
                &cols,
                &[vec![
                    json!(1),
                    json!("a"),
                    json!("https://a.example"),
                    json!("not a number"),
                ]],
            ))]);
            assert!(matches!(
                list(&client(&s), table("bookmarks"), None, 10).await,
                Err(PanelError::BadData(_))
            ));
            let s = Script::new(vec![execute_reply(result(
                &cols,
                &[vec![
                    json!(1),
                    json!(null),
                    json!("https://a.example"),
                    json!(1),
                ]],
            ))]);
            assert!(matches!(
                list(&client(&s), table("bookmarks"), None, 10).await,
                Err(PanelError::BadData(_))
            ));
        });
    }

    #[test]
    fn insert_binds_values_then_the_cap_and_returns_the_new_key() {
        futures_executor::block_on(async {
            let s = Script::new(vec![execute_reply(result(&["id"], &[vec![json!(8)]]))]);
            let id = insert(
                &client(&s),
                table("bookmarks"),
                &body(json!({"label": " Gh ", "url": "https://gh.example", "sort_order": 4})),
            )
            .await
            .unwrap();
            assert_eq!(id, 8);
            assert_eq!(
                s.args(0, 0),
                vec![text("Gh"), text("https://gh.example"), int(4), int(50)]
            );

            let s = Script::new(vec![execute_reply(result(&["number"], &[vec![json!(6)]]))]);
            let n = insert(&client(&s), table("generations"), &body(json!({"number": 6, "label": "x", "link": "/p", "kernel": "6", "built_on": "2026-01-01"}))).await.unwrap();
            assert_eq!(n, 6);
            assert_eq!(s.args(0, 0)[0], int(6));
            assert_eq!(s.args(0, 0)[5], int(20));
        });
    }

    #[test]
    fn insert_reports_a_full_table_and_a_duplicate_differently() {
        futures_executor::block_on(async {
            let ok = json!({"name": "Rain", "icon_url": "https://i.example/a", "stream_url": "https://s.example/a", "sort_order": 0});
            let s = Script::new(vec![execute_reply(result(&["id"], &[]))]);
            assert_eq!(
                insert(&client(&s), table("embiences"), &body(ok.clone())).await,
                Err(PanelError::Invalid("The table is full."))
            );
            let s = Script::new(vec![execute_error(
                "UNIQUE constraint failed: embiences.name",
            )]);
            assert_eq!(
                insert(&client(&s), table("embiences"), &body(ok.clone())).await,
                Err(PanelError::Conflict)
            );
            let s = Script::new(vec![execute_error("CHECK constraint failed: x")]);
            assert!(matches!(
                insert(&client(&s), table("embiences"), &body(ok)).await,
                Err(PanelError::Db(_))
            ));
        });
    }

    #[test]
    fn invalid_writes_never_reach_the_database() {
        futures_executor::block_on(async {
            let s = Script::new(vec![]);
            let c = client(&s);
            assert!(insert(
                &c,
                table("bookmarks"),
                &body(json!({"label": "a", "url": "javascript:1", "sort_order": 0}))
            )
            .await
            .is_err());
            assert!(
                update(&c, table("bookmarks"), 1, &body(json!({"label": "a"})))
                    .await
                    .is_err()
            );
            assert_eq!(s.requests(), 0);
        });
    }

    #[test]
    fn update_binds_the_key_first_and_reports_missing_rows() {
        futures_executor::block_on(async {
            let s = Script::new(vec![
                execute_reply(result(&["id"], &[vec![json!(3)]])),
                execute_reply(result(&["id"], &[])),
            ]);
            let c = client(&s);
            let b = body(json!({"label": "L", "url": "https://u.example", "sort_order": -2}));
            update(&c, table("bookmarks"), 3, &b).await.unwrap();
            assert_eq!(
                s.args(0, 0),
                vec![int(3), text("L"), text("https://u.example"), int(-2)]
            );
            assert_eq!(
                update(&c, table("bookmarks"), 9, &b).await,
                Err(PanelError::NotFound)
            );
        });
    }

    #[test]
    fn deleting_removes_children_first_in_one_transaction() {
        futures_executor::block_on(async {
            let s = Script::new(vec![tx_reply(vec![
                result(&[], &[]),
                result(&["id"], &[vec![json!(5)]]),
            ])]);
            delete(&client(&s), table("embiences"), 5).await.unwrap();
            assert_eq!(s.requests(), 1);
            let body = s.body(0);
            assert!(
                body.find("embience_preset_items").unwrap() < body.find("FROM embiences").unwrap()
            );
            assert_eq!(s.args(0, 1), vec![int(5)]);
            assert_eq!(s.args(0, 2), vec![int(5)]);

            let s = Script::new(vec![tx_reply(vec![result(&["id"], &[])])]);
            assert_eq!(
                delete(&client(&s), table("bookmarks"), 5).await,
                Err(PanelError::NotFound)
            );
        });
    }

    // ------------------------------------------------------------------- ekspor

    #[test]
    fn csv_cells_follow_rfc_4180_and_defuse_spreadsheet_formulas() {
        assert_eq!(csv_cell(&json!("plain")), "plain");
        assert_eq!(csv_cell(&json!("a,b")), "\"a,b\"");
        assert_eq!(csv_cell(&json!("say \"hi\"")), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_cell(&json!("two\nlines")), "\"two\nlines\"");
        assert_eq!(csv_cell(&Json::Null), "");
        assert_eq!(csv_cell(&json!(true)), "1");
        assert_eq!(csv_cell(&json!(false)), "0");
        assert_eq!(csv_cell(&json!(-5)), "-5", "numbers are not text");
        for formula in ["=1+1", "+1", "-1", "@SUM(A1)", "\tx", "\rx"] {
            assert!(
                csv_cell(&json!(formula)).starts_with('\'')
                    || csv_cell(&json!(formula)).starts_with("\"'"),
                "{formula}: {}",
                csv_cell(&json!(formula))
            );
        }
        assert_eq!(
            csv_cell(&json!("=HYPERLINK(\"http://x\",\"y\")")),
            "\"'=HYPERLINK(\"\"http://x\"\",\"\"y\"\")\""
        );
        assert_eq!(
            csv_cell(&json!("a=b")),
            "a=b",
            "only a leading sign matters"
        );
    }

    #[test]
    fn export_formats_are_parsed_strictly() {
        assert_eq!(parse_format(None), Some(Format::Json));
        assert_eq!(parse_format(Some("format=csv")), Some(Format::Csv));
        assert_eq!(parse_format(Some("x=1&format=json")), Some(Format::Json));
        for bad in ["format=", "format=xml", "format=CSV", "format=csv%20"] {
            assert_eq!(parse_format(Some(bad)), None, "{bad}");
        }
    }

    #[test]
    fn exporting_builds_a_file_named_after_the_table() {
        futures_executor::block_on(async {
            let cols = ["id", "label", "url", "sort_order"];
            let rows = vec![
                vec![
                    json!(1),
                    json!("=evil()"),
                    json!("https://a.example"),
                    json!(0),
                ],
                vec![json!(2), json!("a,b"), json!("https://b.example"), json!(1)],
            ];
            let s = Script::new(vec![
                execute_reply(result(&cols, &rows)),
                execute_reply(result(&cols, &rows)),
            ]);
            let c = client(&s);
            let csv = export(&c, table("bookmarks"), Format::Csv, 0)
                .await
                .unwrap();
            assert_eq!(csv.filename, "bookmarks.csv");
            assert!(csv.content_type.starts_with("text/csv"));
            assert_eq!(
                csv.body,
                "id,label,url,sort_order\r\n1,'=evil(),https://a.example,0\r\n2,\"a,b\",https://b.example,1\r\n"
            );
            assert_eq!(s.args(0, 0), vec![json!({"type": "null"}), int(5001)]);
            let js = export(&c, table("bookmarks"), Format::Json, 0)
                .await
                .unwrap();
            assert_eq!(js.filename, "bookmarks.json");
            let parsed: Json = serde_json::from_str(&js.body).unwrap();
            assert_eq!(parsed["table"], "bookmarks");
            assert_eq!(parsed["exported_at"], "1970-01-01T00:00:00Z");
            assert_eq!(parsed["truncated"], false);
            assert_eq!(parsed["rows"][1]["label"], "a,b");
            assert_eq!(
                parsed["rows"][0]["label"], "=evil()",
                "JSON keeps the original text"
            );
        });
    }

    #[test]
    fn a_huge_table_is_cut_and_flagged() {
        futures_executor::block_on(async {
            let cols = ["id", "at", "action", "target"];
            let rows: Vec<Vec<Json>> = (0..=EXPORT_MAX as i64)
                .rev()
                .map(|i| {
                    vec![
                        json!(i + 1),
                        json!("2026-01-01T00:00:00Z"),
                        json!("login"),
                        json!(null),
                    ]
                })
                .collect();
            assert_eq!(rows.len(), EXPORT_MAX as usize + 1);
            let s = Script::new(vec![execute_reply(result(&cols, &rows))]);
            let out = export(&client(&s), table("admin_audit"), Format::Json, 0)
                .await
                .unwrap();
            let parsed: Json = serde_json::from_str(&out.body).unwrap();
            assert_eq!(parsed["truncated"], true);
            assert_eq!(
                parsed["rows"].as_array().unwrap().len(),
                EXPORT_MAX as usize
            );
        });
    }

    // ------------------------------------------------------------------- preset

    fn checks(preset_exists: bool, embiences: &[i64]) -> Result<turso_http::HttpResponse, String> {
        let preset_rows = if preset_exists {
            vec![vec![json!(2)]]
        } else {
            vec![]
        };
        let emb: Vec<Vec<Json>> = embiences.iter().map(|i| vec![json!(i)]).collect();
        tx_reply(vec![result(&["id"], &preset_rows), result(&["id"], &emb)])
    }

    #[test]
    fn preset_items_are_replaced_in_the_given_order() {
        futures_executor::block_on(async {
            let s = Script::new(vec![
                checks(true, &[1, 2, 3]),
                tx_reply(vec![result(&[], &[]), result(&[], &[]), result(&[], &[])]),
            ]);
            set_preset_items(&client(&s), 2, &[3, 1]).await.unwrap();
            assert_eq!(s.requests(), 2);
            assert_eq!(s.args(0, 1), vec![int(2)], "preset checked first");
            assert!(s.body(1).contains("DELETE FROM embience_preset_items"));
            assert_eq!(s.args(1, 1), vec![int(2)]);
            assert_eq!(s.args(1, 2), vec![int(2), int(3), int(0)]);
            assert_eq!(s.args(1, 3), vec![int(2), int(1), int(1)]);
        });
    }

    #[test]
    fn preset_items_are_refused_when_anything_is_off_and_nothing_is_written() {
        futures_executor::block_on(async {
            let bad = Err(PanelError::Invalid("The list is not valid."));
            // kembar dan terlalu banyak: tanpa menyentuh database
            let s = Script::new(vec![]);
            assert_eq!(set_preset_items(&client(&s), 2, &[1, 1]).await, bad);
            let many: Vec<u32> = (1..=MAX_PRESET_ITEMS as u32 + 1).collect();
            assert_eq!(set_preset_items(&client(&s), 2, &many).await, bad);
            assert_eq!(s.requests(), 0);
            // embience yang tidak ada
            let s = Script::new(vec![checks(true, &[1, 2])]);
            assert_eq!(set_preset_items(&client(&s), 2, &[1, 9]).await, bad);
            assert_eq!(s.requests(), 1);
            // preset yang tidak ada
            let s = Script::new(vec![checks(false, &[1, 2])]);
            assert_eq!(
                set_preset_items(&client(&s), 7, &[1]).await,
                Err(PanelError::NotFound)
            );
            assert_eq!(s.requests(), 1);
        });
    }

    #[test]
    fn a_preset_can_be_emptied() {
        futures_executor::block_on(async {
            let s = Script::new(vec![checks(true, &[1]), tx_reply(vec![result(&[], &[])])]);
            set_preset_items(&client(&s), 2, &[]).await.unwrap();
            assert_eq!(s.requests(), 2);
        });
    }

    #[test]
    fn listing_preset_items_reads_every_column() {
        futures_executor::block_on(async {
            let s = Script::new(vec![execute_reply(result(
                &["preset_id", "embience_id", "position"],
                &[
                    vec![json!(1), json!(6), json!(0)],
                    vec![json!(1), json!(12), json!(1)],
                ],
            ))]);
            let items = preset_items(&client(&s)).await.unwrap();
            assert_eq!(
                items[1],
                PresetItem {
                    preset_id: 1,
                    embience_id: 12,
                    position: 1
                }
            );
        });
    }

    // ---------------------------------------------------------- hapus permanen

    #[test]
    fn purging_needs_the_exact_confirmation_and_only_hits_hidden_entries() {
        futures_executor::block_on(async {
            for wrong in ["", "purge", "purge:13", "PURGE:12", "purge:12 ", "12"] {
                let s = Script::new(vec![]);
                assert!(
                    purge_message(&client(&s), 12, wrong).await.is_err(),
                    "{wrong:?}"
                );
                assert_eq!(s.requests(), 0);
            }
            let s = Script::new(vec![tx_reply(vec![
                result(&["id"], &[vec![json!(12)]]),
                result(&[], &[]),
            ])]);
            purge_message(&client(&s), 12, "purge:12").await.unwrap();
            let body = s.body(0);
            assert!(body.contains("deleted_at IS NOT NULL"));
            assert!(
                body.find("DELETE FROM chat_messages").unwrap()
                    < body.find("DELETE FROM message_reports").unwrap()
            );
            // Entri yang masih tampil atau tidak ada: tidak ada baris yang terhapus.
            let s = Script::new(vec![tx_reply(vec![result(&["id"], &[]), result(&[], &[])])]);
            assert_eq!(
                purge_message(&client(&s), 12, "purge:12").await,
                Err(PanelError::NotFound)
            );
        });
    }
}
