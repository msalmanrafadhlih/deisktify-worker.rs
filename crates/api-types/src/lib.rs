//! Kontrak API antara `tquilla-api` (server) dan frontend `tquilla.is-a.dev`.
//!
//! Crate ini dipakai kedua sisi, jadi:
//! - hanya `serde` sebagai dependensi (ikut dikompilasi ke wasm32 di frontend),
//! - tidak ada kode yang bergantung pada runtime tertentu,
//! - perubahan yang tidak kompatibel = naikkan versi/tag, frontend memilih
//!   kapan ikut naik.
//!
//! Aturan keamanan: **server tidak boleh percaya klien**. Konstanta batas di
//! `limits` dipakai klien untuk validasi awal (UX), tetapi server memeriksa
//! ulang semuanya.

use serde::{Deserialize, Serialize};

/// Awalan semua endpoint versi 1.
pub const API_PREFIX: &str = "/api/v1";

/// Path endpoint. Satu-satunya tempat string path ditulis, supaya klien dan
/// server tidak bisa berbeda diam-diam.
pub mod paths {
    /// `GET`: status layanan dan database.
    pub const HEALTH: &str = "/api/v1/health";

    /// Semua path; tambahkan endpoint baru di sini juga supaya test prefix mencakupnya.
    pub const ALL: &[&str] = &[HEALTH];
}

/// Batas yang diberlakukan server (diisi bertahap, dimulai di D4).
pub mod limits {
    /// Ukuran maksimum body request JSON, dalam byte.
    pub const MAX_BODY_BYTES: usize = 16 * 1024;
}

/// Status keseluruhan layanan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceStatus {
    /// Layanan dan database berfungsi.
    Ok,
    /// Layanan hidup tetapi ada dependensi yang tidak berfungsi.
    Degraded,
}

/// Respons `GET /api/v1/health`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: ServiceStatus,
    /// Versi crate `api` yang berjalan.
    pub version: String,
    /// `true` jika `SELECT 1` ke database berhasil.
    pub db: bool,
}

/// Kode error yang stabil dan aman ditampilkan; pesan detail internal tidak
/// pernah dikirim ke klien.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    NotFound,
    MethodNotAllowed,
    PayloadTooLarge,
    RateLimited,
    Unavailable,
    Internal,
}

/// Isi error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiErrorDetail {
    pub code: ErrorCode,
    pub message: String,
}

/// Bentuk semua respons error: `{"error": {"code": "...", "message": "..."}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiErrorBody {
    pub error: ApiErrorDetail,
}

impl ApiErrorBody {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            error: ApiErrorDetail {
                code,
                message: message.into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_json_shape_is_stable() {
        let body = HealthResponse {
            status: ServiceStatus::Ok,
            version: "0.1.0".into(),
            db: true,
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({"status": "ok", "version": "0.1.0", "db": true})
        );
        let degraded: HealthResponse =
            serde_json::from_str(r#"{"status":"degraded","version":"x","db":false}"#).unwrap();
        assert_eq!(degraded.status, ServiceStatus::Degraded);
        assert!(!degraded.db);
    }

    #[test]
    fn error_json_shape_is_stable() {
        let body = ApiErrorBody::new(ErrorCode::RateLimited, "Too many requests.");
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({"error": {"code": "rate_limited", "message": "Too many requests."}})
        );
        let back: ApiErrorBody =
            serde_json::from_value(serde_json::to_value(&body).unwrap()).unwrap();
        assert_eq!(back, body);
    }

    #[test]
    fn every_path_lives_under_the_api_prefix() {
        for p in paths::ALL {
            assert!(p.starts_with(API_PREFIX), "{p}");
        }
    }

    #[test]
    fn unknown_error_code_is_rejected() {
        assert!(serde_json::from_str::<ErrorCode>("\"teapot\"").is_err());
    }
}
