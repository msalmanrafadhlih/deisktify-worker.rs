//! Runner migrasi SQL di atas [`Client`].
//!
//! Aturan (sama dengan `migrations/README.md`):
//! - Berkas bernama `NNNN_deskripsi` (4 digit, huruf kecil, `_` sebagai pemisah);
//!   nomor tidak boleh kembar dan dijalankan berurutan.
//! - Isi berkas hanya statement SQL; tanpa `BEGIN`/`COMMIT`/`ROLLBACK`/`SAVEPOINT`
//!   (runner membungkus satu migrasi dalam satu transaksi).
//! - Migrasi yang sudah tercatat tidak boleh berubah (checksum dibandingkan) dan
//!   tidak boleh hilang dari daftar; migrasi baru harus bernomor lebih besar dari
//!   yang terakhir dijalankan.
//!
//! Catatan terpisah dari transaksi: pencatatan di tabel `_migrations` ikut dalam
//! transaksi yang sama dengan isi migrasi, jadi migrasi yang gagal tidak
//! meninggalkan jejak setengah jalan, dan dua runner yang bersamaan tidak bisa
//! sama-sama berhasil (kunci primer `_migrations.name` menolak yang kedua).
//!
//! Tidak ada `sqlx`: tidak bisa ke wasm32 dan tidak bicara ke Turso.

use std::fmt;

use crate::client::{Client, Error, Transport};
use crate::proto::{Stmt, Value};

/// Satu berkas migrasi: `name` tanpa ekstensi (`0001_generations`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migration {
    pub name: String,
    pub sql: String,
}

impl Migration {
    pub fn new(name: impl Into<String>, sql: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            sql: sql.into(),
        }
    }

    /// Nomor urut dari nama, atau `None` jika nama tidak valid.
    pub fn number(&self) -> Option<u32> {
        parse_name(&self.name)
    }

    /// Sidik jari isi berkas (FNV-1a 64-bit, heksadesimal) untuk mendeteksi
    /// berkas yang diubah setelah dijalankan. Bukan pengaman terhadap pihak
    /// jahat. Akhir baris CRLF dianggap sama dengan LF supaya `autocrlf`
    /// di Windows tidak memicu alarm palsu.
    pub fn checksum(&self) -> String {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut bytes = self.sql.bytes().peekable();
        while let Some(b) = bytes.next() {
            if b == b'\r' && bytes.peek() == Some(&b'\n') {
                continue;
            }
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{hash:016x}")
    }
}

fn parse_name(name: &str) -> Option<u32> {
    let (digits, rest) = name.split_once('_')?;
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let ok = !rest.is_empty()
        && rest
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if !ok {
        return None;
    }
    digits.parse().ok()
}

/// Kegagalan migrasi. Pesan singkat; tidak memuat isi SQL atau rahasia.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrateError {
    /// Nama berkas tidak berbentuk `NNNN_deskripsi`.
    InvalidName(String),
    /// Dua migrasi bernomor sama.
    DuplicateNumber(u32),
    /// Daftar tidak terurut menurut nomor.
    Unsorted(String),
    /// Isi berkas tidak bisa dijalankan (kosong, kutip/komentar tak tertutup,
    /// atau memuat perintah transaksi).
    BadSql { name: String, reason: String },
    /// Migrasi yang sudah dijalankan isinya berubah.
    Modified(String),
    /// Database mencatat migrasi yang tidak ada di daftar (binary lebih lama?).
    Unknown(String),
    /// Ada migrasi baru bernomor lebih kecil dari yang sudah dijalankan.
    OutOfOrder(String),
    /// Database menolak. `name` kosong untuk langkah persiapan.
    Db { name: Option<String>, source: Error },
}

impl fmt::Display for MigrateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrateError::InvalidName(n) => {
                write!(
                    f,
                    "invalid migration name {n:?} (expected NNNN_description)"
                )
            }
            MigrateError::DuplicateNumber(n) => write!(f, "two migrations share number {n:04}"),
            MigrateError::Unsorted(n) => write!(f, "migration {n} is out of order in the list"),
            MigrateError::BadSql { name, reason } => write!(f, "migration {name}: {reason}"),
            MigrateError::Modified(n) => write!(
                f,
                "migration {n} was changed after it was applied; add a new migration instead"
            ),
            MigrateError::Unknown(n) => write!(
                f,
                "database has migration {n} which is not in this build (older binary?)"
            ),
            MigrateError::OutOfOrder(n) => write!(
                f,
                "new migration {n} is numbered below an already applied one; renumber it"
            ),
            MigrateError::Db {
                name: Some(n),
                source,
            } => write!(f, "migration {n} failed and was rolled back: {source}"),
            MigrateError::Db { name: None, source } => write!(f, "{source}"),
        }
    }
}

impl std::error::Error for MigrateError {}

/// Hasil `status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub applied: Vec<String>,
    pub pending: Vec<String>,
}

// ---------------------------------------------------------------------------
// Pemecah statement
// ---------------------------------------------------------------------------

/// Memecah skrip SQL menjadi statement. Memahami kutip `'` `"` `` ` `` `[ ]`,
/// komentar `--` dan `/* */` (dibuang), serta badan `CREATE TRIGGER ... BEGIN ... END`
/// (titik koma di dalamnya bukan pemisah, `CASE ... END` tidak mengakhiri trigger).
/// Statement kosong dibuang.
pub fn split_statements(sql: &str) -> Result<Vec<String>, String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut words: Vec<String> = Vec::new(); // kata-kata statement berjalan (huruf besar)
    let mut word = String::new();
    let mut in_trigger_body = false;
    let mut case_depth = 0u32;
    let mut trigger_done = false;
    let mut i = 0;

    // Menutup kata yang sedang dibaca dan memperbarui keadaan trigger.
    fn finish_word(
        word: &mut String,
        words: &mut Vec<String>,
        in_body: &mut bool,
        case_depth: &mut u32,
        trigger_done: &mut bool,
    ) {
        if word.is_empty() {
            return;
        }
        let w = word.to_ascii_uppercase();
        word.clear();
        let is_trigger = words.first().map(String::as_str) == Some("CREATE")
            && (words.get(1).map(String::as_str) == Some("TRIGGER")
                || (matches!(words.get(1).map(String::as_str), Some("TEMP" | "TEMPORARY"))
                    && words.get(2).map(String::as_str) == Some("TRIGGER")));
        if is_trigger {
            if !*in_body && w == "BEGIN" {
                *in_body = true;
            } else if *in_body {
                match w.as_str() {
                    "CASE" => *case_depth += 1,
                    "END" if *case_depth > 0 => *case_depth -= 1,
                    "END" => *trigger_done = true,
                    _ => {}
                }
            }
        }
        words.push(w);
    }

    macro_rules! flush {
        () => {{
            let stmt = cur.trim();
            if !stmt.is_empty() {
                out.push(stmt.to_owned());
            }
            cur.clear();
            words.clear();
            in_trigger_body = false;
            case_depth = 0;
            trigger_done = false;
        }};
    }

    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();

        match c {
            '-' if next == Some('-') => {
                finish_word(
                    &mut word,
                    &mut words,
                    &mut in_trigger_body,
                    &mut case_depth,
                    &mut trigger_done,
                );
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                cur.push('\n');
            }
            '/' if next == Some('*') => {
                finish_word(
                    &mut word,
                    &mut words,
                    &mut in_trigger_body,
                    &mut case_depth,
                    &mut trigger_done,
                );
                i += 2;
                loop {
                    if i + 1 >= chars.len() {
                        return Err("unterminated /* comment".into());
                    }
                    if chars[i] == '*' && chars[i + 1] == '/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                cur.push(' ');
                continue;
            }
            '\'' | '"' | '`' | '[' => {
                finish_word(
                    &mut word,
                    &mut words,
                    &mut in_trigger_body,
                    &mut case_depth,
                    &mut trigger_done,
                );
                let close = if c == '[' { ']' } else { c };
                cur.push(c);
                i += 1;
                loop {
                    let Some(&d) = chars.get(i) else {
                        return Err(format!("unterminated {c} quote"));
                    };
                    cur.push(d);
                    i += 1;
                    if d == close {
                        // kutip ganda (`''`) adalah karakter kutip itu sendiri
                        if close != ']' && chars.get(i) == Some(&close) {
                            cur.push(close);
                            i += 1;
                            continue;
                        }
                        break;
                    }
                }
                // Isi literal bukan kata kunci.
                words.push(String::from("\u{0}"));
                continue;
            }
            ';' => {
                finish_word(
                    &mut word,
                    &mut words,
                    &mut in_trigger_body,
                    &mut case_depth,
                    &mut trigger_done,
                );
                let in_open_trigger = in_trigger_body && !trigger_done;
                if in_open_trigger {
                    cur.push(';');
                } else {
                    flush!();
                }
            }
            c if c.is_ascii_alphanumeric() || c == '_' => {
                word.push(c);
                cur.push(c);
            }
            c => {
                finish_word(
                    &mut word,
                    &mut words,
                    &mut in_trigger_body,
                    &mut case_depth,
                    &mut trigger_done,
                );
                cur.push(c);
            }
        }
        i += 1;
    }
    finish_word(
        &mut word,
        &mut words,
        &mut in_trigger_body,
        &mut case_depth,
        &mut trigger_done,
    );
    if in_trigger_body && !trigger_done {
        return Err("CREATE TRIGGER without a closing END".into());
    }
    let last = cur.trim();
    if !last.is_empty() {
        out.push(last.to_owned());
    }
    Ok(out)
}

fn first_word(stmt: &str) -> String {
    stmt.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .find(|w| !w.is_empty())
        .unwrap_or("")
        .to_ascii_uppercase()
}

/// Memecah dan memeriksa skrip; mengembalikan statement siap kirim.
fn prepare(name: &str, sql: &str) -> Result<Vec<String>, MigrateError> {
    let bad = |reason: String| MigrateError::BadSql {
        name: name.to_owned(),
        reason,
    };
    let stmts = split_statements(sql).map_err(bad)?;
    if stmts.is_empty() {
        return Err(bad("contains no SQL statements".into()));
    }
    for s in &stmts {
        let w = first_word(s);
        if matches!(
            w.as_str(),
            "BEGIN" | "COMMIT" | "END" | "ROLLBACK" | "SAVEPOINT" | "RELEASE"
        ) {
            return Err(bad(format!(
                "must not contain {w} (the runner wraps the file in a transaction)"
            )));
        }
    }
    Ok(stmts)
}

/// Memeriksa daftar migrasi tanpa menyentuh database.
pub fn validate(migrations: &[Migration]) -> Result<(), MigrateError> {
    let mut last: Option<u32> = None;
    for m in migrations {
        let n = m
            .number()
            .ok_or_else(|| MigrateError::InvalidName(m.name.clone()))?;
        match last {
            Some(prev) if n == prev => return Err(MigrateError::DuplicateNumber(n)),
            Some(prev) if n < prev => return Err(MigrateError::Unsorted(m.name.clone())),
            _ => {}
        }
        last = Some(n);
        prepare(&m.name, &m.sql)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

const CREATE_TABLE: &str = "CREATE TABLE IF NOT EXISTS _migrations (\
    name TEXT PRIMARY KEY, \
    checksum TEXT NOT NULL, \
    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')))";

fn db(name: Option<&str>, source: Error) -> MigrateError {
    MigrateError::Db {
        name: name.map(str::to_owned),
        source,
    }
}

/// Migrasi tercatat di database: `(name, checksum)`, urut menurut nama.
async fn load_applied<T: Transport>(
    client: &Client<T>,
    create_table: bool,
) -> Result<Vec<(String, String)>, MigrateError> {
    if create_table {
        client
            .execute(Stmt::new(CREATE_TABLE).no_rows())
            .await
            .map_err(|e| db(None, e))?;
    } else {
        let exists = client
            .execute(Stmt::new(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_migrations'",
            ))
            .await
            .map_err(|e| db(None, e))?;
        if exists.rows.is_empty() {
            return Ok(Vec::new());
        }
    }
    let res = client
        .execute(Stmt::new(
            "SELECT name, checksum FROM _migrations ORDER BY name",
        ))
        .await
        .map_err(|e| db(None, e))?;
    let mut out = Vec::with_capacity(res.rows.len());
    for row in 0..res.rows.len() {
        let text = |col: &str| {
            res.get(row, col)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| {
                    db(
                        None,
                        Error::Protocol("_migrations has an unexpected shape".into()),
                    )
                })
        };
        out.push((text("name")?, text("checksum")?));
    }
    Ok(out)
}

/// Memisahkan daftar menjadi yang sudah dan belum dijalankan, sambil memeriksa
/// kecocokan dengan catatan di database.
fn plan<'a>(
    migrations: &'a [Migration],
    applied: &[(String, String)],
) -> Result<(Vec<&'a Migration>, Vec<&'a Migration>), MigrateError> {
    for (name, checksum) in applied {
        match migrations.iter().find(|m| &m.name == name) {
            None => return Err(MigrateError::Unknown(name.clone())),
            Some(m) if &m.checksum() != checksum => {
                return Err(MigrateError::Modified(name.clone()))
            }
            Some(_) => {}
        }
    }
    let (done, pending): (Vec<_>, Vec<_>) = migrations
        .iter()
        .partition(|m| applied.iter().any(|(n, _)| n == &m.name));
    if let Some(max) = done.iter().filter_map(|m| m.number()).max() {
        if let Some(m) = pending.iter().find(|m| m.number().is_some_and(|n| n < max)) {
            return Err(MigrateError::OutOfOrder(m.name.clone()));
        }
    }
    Ok((done, pending))
}

/// Mana yang sudah dan belum dijalankan. Tidak mengubah database.
pub async fn status<T: Transport>(
    client: &Client<T>,
    migrations: &[Migration],
) -> Result<Status, MigrateError> {
    validate(migrations)?;
    let applied = load_applied(client, false).await?;
    let (done, pending) = plan(migrations, &applied)?;
    Ok(Status {
        applied: done.iter().map(|m| m.name.clone()).collect(),
        pending: pending.iter().map(|m| m.name.clone()).collect(),
    })
}

/// Menjalankan semua migrasi yang belum dijalankan, satu transaksi per migrasi.
/// Mengembalikan nama migrasi yang baru dijalankan (kosong bila sudah mutakhir).
pub async fn run<T: Transport>(
    client: &Client<T>,
    migrations: &[Migration],
) -> Result<Vec<String>, MigrateError> {
    validate(migrations)?;
    let applied = load_applied(client, true).await?;
    let (_, pending) = plan(migrations, &applied)?;

    let mut ran = Vec::with_capacity(pending.len());
    for m in pending {
        let mut stmts: Vec<Stmt> = prepare(&m.name, &m.sql)?
            .into_iter()
            .map(|s| Stmt::new(s).no_rows())
            .collect();
        stmts.push(
            Stmt::new("INSERT INTO _migrations (name, checksum) VALUES (?, ?)")
                .bind(m.name.as_str())
                .bind(m.checksum())
                .no_rows(),
        );
        client
            .transaction(stmts)
            .await
            .map_err(|e| db(Some(&m.name), e))?;
        ran.push(m.name.clone());
    }
    Ok(ran)
}

/// Menjalankan satu skrip (mis. seed) dalam satu transaksi, tanpa mencatatnya.
/// Skrip harus idempotent (`INSERT OR IGNORE`, dst.). Mengembalikan jumlah statement.
pub async fn run_script<T: Transport>(
    client: &Client<T>,
    name: &str,
    sql: &str,
) -> Result<usize, MigrateError> {
    let stmts = prepare(name, sql)?;
    let count = stmts.len();
    client
        .transaction(stmts.into_iter().map(|s| Stmt::new(s).no_rows()).collect())
        .await
        .map_err(|e| db(Some(name), e))?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::HttpResponse;
    use futures_executor::block_on;
    use std::cell::RefCell;
    use std::future::Future;

    // --------------------------------------------------------- pemecah

    fn split(sql: &str) -> Vec<String> {
        split_statements(sql).unwrap()
    }

    #[test]
    fn splits_on_semicolons_and_drops_comments_and_blanks() {
        let s = split("-- header; with semicolon\nCREATE TABLE a (x);;\n/* c; */ INSERT INTO a VALUES (1) -- tail\n;\n  \n");
        assert_eq!(s, vec!["CREATE TABLE a (x)", "INSERT INTO a VALUES (1)"]);
    }

    #[test]
    fn last_statement_needs_no_semicolon() {
        assert_eq!(split("SELECT 1"), vec!["SELECT 1"]);
    }

    #[test]
    fn semicolons_inside_quotes_do_not_split() {
        let s =
            split("INSERT INTO t VALUES ('a;b', \"c;d\", `e;f`, 'it''s;'); SELECT [x;y] FROM t;");
        assert_eq!(s.len(), 2);
        assert!(s[0].contains("'it''s;'"));
        assert!(s[1].contains("[x;y]"));
    }

    #[test]
    fn comment_markers_inside_strings_are_kept() {
        let s = split("INSERT INTO t VALUES ('-- not a comment', '/* nor this */');");
        assert_eq!(
            s,
            vec!["INSERT INTO t VALUES ('-- not a comment', '/* nor this */')"]
        );
    }

    #[test]
    fn trigger_body_is_one_statement() {
        let sql = "CREATE TRIGGER bump AFTER INSERT ON t BEGIN\n\
                   UPDATE c SET n = n + 1;\n\
                   UPDATE d SET m = CASE WHEN n > 1 THEN 1 ELSE 0 END;\n\
                   END;\n\
                   SELECT 1;";
        let s = split(sql);
        assert_eq!(s.len(), 2, "{s:?}");
        assert!(s[0].starts_with("CREATE TRIGGER bump"));
        assert!(s[0].ends_with("END"));
        assert_eq!(s[1], "SELECT 1");
    }

    #[test]
    fn temp_trigger_is_recognised_too() {
        let s = split("CREATE TEMP TRIGGER x AFTER DELETE ON t BEGIN SELECT 1; END; SELECT 2;");
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn identifiers_containing_keywords_do_not_confuse_the_splitter() {
        let s = split("CREATE TABLE begin_end (end_at TEXT, begin_at TEXT); SELECT 1;");
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn unterminated_input_is_an_error() {
        assert!(split_statements("SELECT 'oops").is_err());
        assert!(split_statements("SELECT 1 /* oops").is_err());
        assert!(split_statements("SELECT [oops").is_err());
        assert!(split_statements("CREATE TRIGGER t AFTER INSERT ON x BEGIN SELECT 1;").is_err());
    }

    #[test]
    fn transaction_control_is_rejected() {
        for sql in [
            "BEGIN; SELECT 1;",
            "SELECT 1; COMMIT;",
            "rollback",
            "SAVEPOINT a",
            "release a",
            "END",
        ] {
            let err = prepare("0001_x", sql).unwrap_err();
            assert!(matches!(err, MigrateError::BadSql { .. }), "{sql}: {err}");
        }
        assert!(prepare("0001_x", "-- only a comment\n").is_err());
        // BEGIN di dalam trigger boleh
        assert!(prepare(
            "0001_x",
            "CREATE TRIGGER t AFTER INSERT ON a BEGIN SELECT 1; END;"
        )
        .is_ok());
    }

    // ------------------------------------------------- nama dan checksum

    #[test]
    fn names_follow_the_convention() {
        for ok in ["0001_init", "0042_add_users_2", "9999_x"] {
            assert!(parse_name(ok).is_some(), "{ok}");
        }
        for bad in [
            "1_init",
            "00001_init",
            "0001init",
            "0001_",
            "0001_Init",
            "0001_a-b",
            "0001_a.sql",
            "abcd_x",
            "",
        ] {
            assert!(parse_name(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn checksum_ignores_crlf_but_not_content() {
        let a = Migration::new("0001_a", "SELECT 1;\nSELECT 2;\n");
        let b = Migration::new("0001_a", "SELECT 1;\r\nSELECT 2;\r\n");
        let c = Migration::new("0001_a", "SELECT 1;\nSELECT 3;\n");
        assert_eq!(a.checksum(), b.checksum());
        assert_ne!(a.checksum(), c.checksum());
        assert_eq!(a.checksum().len(), 16);
    }

    #[test]
    fn validate_catches_bad_lists() {
        let m = |n: &str| Migration::new(n, "SELECT 1;");
        assert!(validate(&[m("0001_a"), m("0002_b")]).is_ok());
        assert_eq!(
            validate(&[m("0001_a"), m("0001_b")]),
            Err(MigrateError::DuplicateNumber(1))
        );
        assert_eq!(
            validate(&[m("0002_a"), m("0001_b")]),
            Err(MigrateError::Unsorted("0001_b".into()))
        );
        assert_eq!(
            validate(&[m("one_a")]),
            Err(MigrateError::InvalidName("one_a".into()))
        );
        assert!(matches!(
            validate(&[Migration::new("0001_a", "BEGIN;")]),
            Err(MigrateError::BadSql { .. })
        ));
    }

    // ------------------------------------------------------ database palsu

    /// Meniru bagian protokol yang dipakai runner dengan keadaan sendiri:
    /// tabel `_migrations` sebagai daftar, batch sukses/gagal sesuai skenario.
    #[derive(Default)]
    struct FakeDb {
        has_table: RefCell<bool>,
        rows: RefCell<Vec<(String, String)>>,
        /// Batch yang mengandung potongan SQL ini gagal pada statement tsb.
        fail_when_sql_contains: RefCell<Option<String>>,
        /// Semua SQL yang dikirim, berurutan (tiap statement satu entri).
        log: RefCell<Vec<String>>,
        batches: RefCell<usize>,
    }

    fn execute_reply(cols: &[&str], rows: &[Vec<&str>]) -> String {
        let cols: Vec<String> = cols
            .iter()
            .map(|c| format!(r#"{{"name":"{c}","decltype":"TEXT"}}"#))
            .collect();
        let rows: Vec<String> = rows
            .iter()
            .map(|r| {
                let cells: Vec<String> = r
                    .iter()
                    .map(|v| format!(r#"{{"type":"text","value":"{v}"}}"#))
                    .collect();
                format!("[{}]", cells.join(","))
            })
            .collect();
        format!(
            r#"{{"results":[{{"type":"ok","response":{{"type":"execute","result":{{"cols":[{}],"rows":[{}],"affected_row_count":0}}}}}},{{"type":"ok","response":{{"type":"close"}}}}]}}"#,
            cols.join(","),
            rows.join(",")
        )
    }

    impl Transport for &FakeDb {
        fn post(
            &self,
            _url: &str,
            _bearer: Option<&str>,
            body: String,
        ) -> impl Future<Output = Result<HttpResponse, String>> {
            let req: serde_json::Value = serde_json::from_str(&body).unwrap();
            let first = &req["requests"][0];
            let reply = if first["type"] == "execute" {
                let sql = first["stmt"]["sql"].as_str().unwrap().to_owned();
                self.log.borrow_mut().push(sql.clone());
                if sql.starts_with("CREATE TABLE IF NOT EXISTS _migrations") {
                    *self.has_table.borrow_mut() = true;
                    execute_reply(&[], &[])
                } else if sql.contains("sqlite_master") {
                    if *self.has_table.borrow() {
                        execute_reply(&["1"], &[vec!["1"]])
                    } else {
                        execute_reply(&["1"], &[])
                    }
                } else if sql.starts_with("SELECT name, checksum FROM _migrations") {
                    let rows = self.rows.borrow();
                    let mut rows: Vec<_> = rows.clone();
                    rows.sort();
                    let cells: Vec<Vec<&str>> = rows
                        .iter()
                        .map(|(n, c)| vec![n.as_str(), c.as_str()])
                        .collect();
                    execute_reply(&["name", "checksum"], &cells)
                } else {
                    panic!("unexpected execute: {sql}");
                }
            } else {
                *self.batches.borrow_mut() += 1;
                let steps = first["batch"]["steps"].as_array().unwrap();
                let stmts: Vec<&serde_json::Value> = steps[1..steps.len() - 2]
                    .iter()
                    .map(|s| &s["stmt"])
                    .collect();
                for s in &stmts {
                    self.log
                        .borrow_mut()
                        .push(s["sql"].as_str().unwrap().to_owned());
                }
                let n = stmts.len();
                let ok = r#"{"cols":[],"rows":[],"affected_row_count":0}"#;
                let fail_at = self
                    .fail_when_sql_contains
                    .borrow()
                    .as_ref()
                    .and_then(|needle| {
                        stmts
                            .iter()
                            .position(|s| s["sql"].as_str().unwrap().contains(needle.as_str()))
                    });
                let (results, errors) = match fail_at {
                    Some(i) => {
                        let mut results = vec![ok.to_owned()];
                        results.extend(std::iter::repeat_n(ok.to_owned(), i));
                        results.extend(std::iter::repeat_n("null".to_owned(), n - i + 1));
                        results.push(ok.to_owned()); // ROLLBACK
                        let mut errors = vec!["null".to_owned(); i + 1];
                        errors.push(r#"{"message":"boom","code":"SQLITE_ERROR"}"#.into());
                        errors.extend(std::iter::repeat_n("null".to_owned(), n - i + 1));
                        (results, errors)
                    }
                    None => {
                        // Catat baris _migrations yang di-INSERT.
                        let last = stmts[n - 1];
                        if last["sql"]
                            .as_str()
                            .unwrap()
                            .starts_with("INSERT INTO _migrations")
                        {
                            let arg =
                                |i: usize| last["args"][i]["value"].as_str().unwrap().to_owned();
                            self.rows.borrow_mut().push((arg(0), arg(1)));
                        }
                        let mut results = vec![ok.to_owned(); n + 2];
                        results.push("null".into());
                        (results, vec!["null".to_owned(); n + 3])
                    }
                };
                format!(
                    r#"{{"results":[{{"type":"ok","response":{{"type":"batch","result":{{"step_results":[{}],"step_errors":[{}]}}}}}},{{"type":"ok","response":{{"type":"close"}}}}]}}"#,
                    results.join(","),
                    errors.join(",")
                )
            };
            async move {
                Ok(HttpResponse {
                    status: 200,
                    body: reply,
                })
            }
        }
    }

    fn client(db: &FakeDb) -> Client<&FakeDb> {
        Client::new("http://127.0.0.1:8081", None, db).unwrap()
    }

    fn list() -> Vec<Migration> {
        vec![
            Migration::new("0001_a", "CREATE TABLE a (x);\nCREATE INDEX a_x ON a (x);"),
            Migration::new("0002_b", "CREATE TABLE b (y);"),
        ]
    }

    // ---------------------------------------------------------- runner

    #[test]
    fn runs_pending_migrations_once_each_in_their_own_transaction() {
        let db = FakeDb::default();
        let ran = block_on(run(&client(&db), &list())).unwrap();
        assert_eq!(ran, vec!["0001_a", "0002_b"]);
        assert_eq!(*db.batches.borrow(), 2);
        assert_eq!(db.rows.borrow().len(), 2);

        let log = db.log.borrow();
        let create = log
            .iter()
            .position(|s| s.contains("CREATE TABLE a"))
            .unwrap();
        let index = log.iter().position(|s| s.contains("CREATE INDEX")).unwrap();
        let record = log
            .iter()
            .position(|s| s.starts_with("INSERT INTO _migrations"))
            .unwrap();
        assert!(create < index && index < record, "{log:?}");
    }

    #[test]
    fn second_run_is_a_no_op() {
        let db = FakeDb::default();
        block_on(run(&client(&db), &list())).unwrap();
        let ran = block_on(run(&client(&db), &list())).unwrap();
        assert!(ran.is_empty());
        assert_eq!(*db.batches.borrow(), 2);
    }

    #[test]
    fn only_new_migrations_run_after_an_update() {
        let db = FakeDb::default();
        block_on(run(&client(&db), &list()[..1])).unwrap();
        let ran = block_on(run(&client(&db), &list())).unwrap();
        assert_eq!(ran, vec!["0002_b"]);
    }

    #[test]
    fn status_does_not_create_the_table() {
        let db = FakeDb::default();
        let s = block_on(status(&client(&db), &list())).unwrap();
        assert!(s.applied.is_empty());
        assert_eq!(s.pending, vec!["0001_a", "0002_b"]);
        assert!(!*db.has_table.borrow());
        assert_eq!(*db.batches.borrow(), 0);

        block_on(run(&client(&db), &list()[..1])).unwrap();
        let s = block_on(status(&client(&db), &list())).unwrap();
        assert_eq!(s.applied, vec!["0001_a"]);
        assert_eq!(s.pending, vec!["0002_b"]);
    }

    #[test]
    fn modified_applied_migration_is_refused_before_running_anything() {
        let db = FakeDb::default();
        block_on(run(&client(&db), &list()[..1])).unwrap();
        let mut changed = list();
        changed[0].sql.push_str("\nCREATE TABLE sneaky (z);");
        let before = *db.batches.borrow();
        let err = block_on(run(&client(&db), &changed)).unwrap_err();
        assert_eq!(err, MigrateError::Modified("0001_a".into()));
        assert_eq!(*db.batches.borrow(), before);
    }

    #[test]
    fn database_ahead_of_the_binary_is_refused() {
        let db = FakeDb::default();
        block_on(run(&client(&db), &list())).unwrap();
        let err = block_on(run(&client(&db), &list()[..1])).unwrap_err();
        assert_eq!(err, MigrateError::Unknown("0002_b".into()));
    }

    #[test]
    fn new_migration_numbered_below_an_applied_one_is_refused() {
        let db = FakeDb::default();
        block_on(run(&client(&db), &[list()[1].clone()])).unwrap();
        let err = block_on(run(&client(&db), &list())).unwrap_err();
        assert_eq!(err, MigrateError::OutOfOrder("0001_a".into()));
    }

    #[test]
    fn failed_migration_is_reported_by_name_and_not_recorded() {
        let db = FakeDb::default();
        *db.fail_when_sql_contains.borrow_mut() = Some("CREATE INDEX".into());
        let err = block_on(run(&client(&db), &list())).unwrap_err();
        match err {
            MigrateError::Db {
                name: Some(n),
                source: Error::Step { index, .. },
            } => {
                assert_eq!(n, "0001_a");
                assert_eq!(index, 1);
            }
            other => panic!("{other:?}"),
        }
        assert!(db.rows.borrow().is_empty());
        // 0002 tidak dicoba setelah 0001 gagal
        assert_eq!(*db.batches.borrow(), 1);
        assert!(MigrateError::Db {
            name: Some("0001_a".into()),
            source: Error::Config("x".into())
        }
        .to_string()
        .contains("rolled back"));
    }

    #[test]
    fn invalid_list_never_touches_the_database() {
        let db = FakeDb::default();
        let bad = vec![Migration::new("0001_a", "COMMIT;")];
        assert!(block_on(run(&client(&db), &bad)).is_err());
        assert!(db.log.borrow().is_empty());
    }

    #[test]
    fn scripts_run_in_one_transaction_without_recording() {
        let db = FakeDb::default();
        let n = block_on(run_script(
            &client(&db),
            "0001_content",
            "INSERT OR IGNORE INTO a VALUES (1);\nINSERT OR IGNORE INTO a VALUES (2);",
        ))
        .unwrap();
        assert_eq!(n, 2);
        assert_eq!(*db.batches.borrow(), 1);
        assert!(db.rows.borrow().is_empty());
        assert!(!*db.has_table.borrow());
    }

    #[test]
    fn error_messages_do_not_echo_sql() {
        let e = MigrateError::BadSql {
            name: "0001_a".into(),
            reason: "contains no SQL statements".into(),
        };
        assert!(!e.to_string().contains("SELECT"));
    }
}
