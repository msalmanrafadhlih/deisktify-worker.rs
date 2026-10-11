//! Tipe kawat (wire) protokol Hrana over HTTP (JSON).

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

// ---------------------------------------------------------------------------
// Nilai
// ---------------------------------------------------------------------------

/// Satu nilai SQLite.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    Float(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// Bentuk di kabel. Integer selalu string (supaya tidak kehilangan presisi),
/// float adalah angka JSON, blob memakai kunci `base64`.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Wire {
    Null,
    Integer {
        #[serde(with = "i64_as_string")]
        value: i64,
    },
    Float {
        value: f64,
    },
    Text {
        value: String,
    },
    Blob {
        base64: String,
    },
}

mod i64_as_string {
    use super::*;

    pub fn serialize<S: Serializer>(v: &i64, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(v)
    }

    /// Menerima string (bentuk resmi) maupun angka (toleran).
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum StrOrNum {
            Str(String),
            Num(i64),
        }
        match StrOrNum::deserialize(d)? {
            StrOrNum::Num(n) => Ok(n),
            StrOrNum::Str(s) => s.parse().map_err(serde::de::Error::custom),
        }
    }
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Null => Wire::Null,
            Value::Integer(i) => Wire::Integer { value: *i },
            // NaN dan tak-hingga tidak ada di JSON (dan SQLite menyimpan NaN
            // sebagai NULL), jadi dikirim sebagai NULL.
            Value::Float(f) if !f.is_finite() => Wire::Null,
            Value::Float(f) => Wire::Float { value: *f },
            Value::Text(t) => Wire::Text { value: t.clone() },
            Value::Blob(b) => Wire::Blob {
                base64: STANDARD.encode(b),
            },
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match Wire::deserialize(d)? {
            Wire::Null => Value::Null,
            Wire::Integer { value } => Value::Integer(value),
            Wire::Float { value } => Value::Float(value),
            Wire::Text { value } => Value::Text(value),
            Wire::Blob { base64 } => Value::Blob(
                STANDARD
                    .decode(base64.as_bytes())
                    .map_err(serde::de::Error::custom)?,
            ),
        })
    }
}

impl Value {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_blob(&self) -> Option<&[u8]> {
        match self {
            Value::Blob(b) => Some(b),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Integer(v)
    }
}
impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Integer(v.into())
    }
}
impl From<u32> for Value {
    fn from(v: u32) -> Self {
        Value::Integer(v.into())
    }
}
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Integer(i64::from(v))
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Text(v.to_owned())
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Text(v)
    }
}
impl From<Vec<u8>> for Value {
    fn from(v: Vec<u8>) -> Self {
        Value::Blob(v)
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Value::Null, Into::into)
    }
}

// ---------------------------------------------------------------------------
// Statement dan batch (request)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NamedArg {
    pub name: String,
    pub value: Value,
}

/// Satu statement SQL dengan parameternya. Selalu pakai parameter (`?` atau
/// `:nama`), jangan menyisipkan input pengguna ke string SQL.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stmt {
    pub sql: String,
    pub args: Vec<Value>,
    pub named_args: Vec<NamedArg>,
    pub want_rows: bool,
}

impl Stmt {
    pub fn new(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            args: Vec::new(),
            named_args: Vec::new(),
            want_rows: true,
        }
    }

    /// Menambah parameter posisional (`?`).
    #[must_use]
    pub fn bind(mut self, v: impl Into<Value>) -> Self {
        self.args.push(v.into());
        self
    }

    /// Menambah parameter bernama (`:nama`, `@nama`, atau `$nama`).
    #[must_use]
    pub fn bind_named(mut self, name: impl Into<String>, v: impl Into<Value>) -> Self {
        self.named_args.push(NamedArg {
            name: name.into(),
            value: v.into(),
        });
        self
    }

    /// Untuk INSERT/UPDATE/DELETE: server tidak perlu mengirim baris.
    #[must_use]
    pub fn no_rows(mut self) -> Self {
        self.want_rows = false;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BatchCond {
    Ok { step: u32 },
    Error { step: u32 },
    Not { cond: Box<BatchCond> },
    And { conds: Vec<BatchCond> },
    Or { conds: Vec<BatchCond> },
    IsAutocommit,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BatchStep {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<BatchCond>,
    pub stmt: Stmt,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Batch {
    pub steps: Vec<BatchStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamRequest {
    Execute { stmt: Stmt },
    Batch { batch: Batch },
    Close,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PipelineRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baton: Option<String>,
    pub requests: Vec<StreamRequest>,
}

// ---------------------------------------------------------------------------
// Respons
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Col {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub decltype: Option<String>,
}

/// Error dari server untuk satu request/langkah.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ServerError {
    pub message: String,
    #[serde(default)]
    pub code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StmtResult {
    #[serde(default)]
    pub cols: Vec<Col>,
    #[serde(default)]
    pub rows: Vec<Vec<Value>>,
    #[serde(default)]
    pub affected_row_count: u64,
    #[serde(default, deserialize_with = "opt_i64_from_string")]
    pub last_insert_rowid: Option<i64>,
}

fn opt_i64_from_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StrOrNum {
        Str(String),
        Num(i64),
    }
    Ok(match Option::<StrOrNum>::deserialize(d)? {
        None => None,
        Some(StrOrNum::Num(n)) => Some(n),
        Some(StrOrNum::Str(s)) => Some(s.parse().map_err(serde::de::Error::custom)?),
    })
}

impl StmtResult {
    /// Indeks kolom menurut nama (pertama yang cocok).
    pub fn col_index(&self, name: &str) -> Option<usize> {
        self.cols
            .iter()
            .position(|c| c.name.as_deref() == Some(name))
    }

    /// Nilai di `row` pada kolom bernama `col`.
    pub fn get(&self, row: usize, col: &str) -> Option<&Value> {
        self.rows.get(row)?.get(self.col_index(col)?)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BatchResult {
    #[serde(default)]
    pub step_results: Vec<Option<StmtResult>>,
    #[serde(default)]
    pub step_errors: Vec<Option<ServerError>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamResponse {
    Close,
    Execute {
        result: StmtResult,
    },
    Batch {
        result: BatchResult,
    },
    /// Tipe lain (`sequence`, `describe`, ...) yang tidak kita pakai.
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamResult {
    Ok { response: StreamResponse },
    Error { error: ServerError },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PipelineResponse {
    #[serde(default)]
    pub baton: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    pub results: Vec<StreamResult>,
}
