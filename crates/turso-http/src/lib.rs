//! Klien HTTP minimal untuk Turso (libSQL) tanpa runtime async tertentu.
//!
//! Kenapa tidak memakai crate `libsql`/`libsql-client`: `libsql` membawa kode
//! native (tidak bisa ke wasm32) dan `libsql-client` (backend Workers) sudah
//! deprecated. Protokolnya sederhana (JSON lewat `POST /v2/pipeline`), jadi
//! cukup ditulis sendiri, dengan transport yang bisa diganti:
//! - Worker: `fetch` Cloudflare,
//! - test: transport palsu.
//!
//! Bentuk JSON mengikuti klien referensi resmi `@libsql/hrana-client`;
//! `tests/golden.rs` membandingkan keluaran kita dengan hasil encoder
//! referensi itu (lihat `scripts/gen_golden.mjs`).

pub mod client;
pub mod migrate;
pub mod proto;

pub use client::{Client, Error, HttpResponse, Transport};
pub use proto::{Stmt, StmtResult, Value};
