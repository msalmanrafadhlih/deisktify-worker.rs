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
    /// `GET`: menu boot halaman utama, terbaru dulu (`number` menurun).
    pub const GENERATIONS: &str = "/api/v1/generations";
    /// `GET`: bookmark bawaan jendela Browser.
    pub const BOOKMARKS: &str = "/api/v1/bookmarks";
    /// `GET`: stasiun radio yang aktif.
    pub const RADIO: &str = "/api/v1/radio";
    /// `GET`: suara latar (Embience).
    pub const EMBIENCES: &str = "/api/v1/embiences";
    /// `GET`: preset Embience beserta isinya.
    pub const PRESETS: &str = "/api/v1/presets";

    /// Semua path; tambahkan endpoint baru di sini juga supaya test prefix mencakupnya.
    pub const ALL: &[&str] = &[HEALTH, GENERATIONS, BOOKMARKS, RADIO, EMBIENCES, PRESETS];
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

/// Pemeriksaan tautan yang dipakai server (sebelum mengirim) **dan** klien
/// (sebelum memakai), karena klien tidak boleh percaya respons begitu saja.
pub mod validate {
    fn clean(s: &str, max: usize) -> bool {
        !s.is_empty() && s.len() <= max && !s.chars().any(|c| c.is_whitespace() || c.is_control())
    }

    /// `http://` atau `https://` diikuti host, tanpa spasi/kontrol, maksimal 2048 byte.
    /// Hanya skema huruf kecil yang diterima.
    pub fn is_http_url(s: &str) -> bool {
        let rest = s
            .strip_prefix("https://")
            .or_else(|| s.strip_prefix("http://"));
        matches!(rest, Some(r) if !r.is_empty() && !r.starts_with('/')) && clean(s, 2048)
    }

    /// Rute internal (`/profile`; bukan `//host` atau `/\`) atau URL http(s).
    pub fn is_route_or_url(s: &str) -> bool {
        let internal = s.starts_with('/') && !s.starts_with("//") && !s.starts_with("/\\");
        (internal && clean(s, 2048)) || is_http_url(s)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn http_urls_are_checked_strictly() {
            for ok in ["https://a.b", "http://127.0.0.1:8080/x?y=1"] {
                assert!(is_http_url(ok), "{ok}");
            }
            assert!(!is_http_url("HTTPS://A.B"));
            for bad in [
                "",
                "https://",
                "https:///x",
                "javascript:alert(1)",
                "data:text/html,x",
                "//evil.example",
                "ftp://a.b",
                "https://a.b/c d",
                "https://a.b/\n",
                "/profile",
            ] {
                assert!(!is_http_url(bad), "{bad:?}");
            }
            assert!(!is_http_url(&format!("https://a.b/{}", "x".repeat(2100))));
        }

        #[test]
        fn routes_must_be_internal_paths_or_http_urls() {
            for ok in ["/profile", "/deisktify", "https://a.b/c", "/"] {
                assert!(is_route_or_url(ok), "{ok}");
            }
            for bad in [
                "//evil.example",
                "/\\evil.example",
                "profile",
                "javascript:x",
                "/a b",
                "",
            ] {
                assert!(!is_route_or_url(bad), "{bad:?}");
            }
        }
    }
}

/// Pembungkus semua respons daftar: `{"items": [...]}`. Objek (bukan array
/// telanjang) supaya metadata, mis. penomoran halaman, bisa ditambah nanti
/// tanpa merusak klien lama.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Items<T> {
    pub items: Vec<T>,
}

/// Satu baris menu boot. `link` adalah rute internal (`/deisktify`) atau URL
/// `http(s)://`; klien tetap harus memeriksanya sebelum membuka.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Generation {
    pub number: u32,
    pub label: String,
    pub link: String,
    pub kernel: String,
    /// Tanggal `YYYY-MM-DD`.
    pub built_on: String,
}

/// Bookmark bawaan di jendela Browser. `url` selalu `http(s)://`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: u32,
    pub label: String,
    pub url: String,
}

/// Stasiun radio. Semua URL `http(s)://`; `official_link` dan `image_url` boleh kosong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RadioStation {
    pub id: u32,
    pub label: String,
    pub slogan: String,
    pub stream_url: String,
    pub official_link: Option<String>,
    pub image_url: Option<String>,
}

/// Satu suara latar. `icon_url` dan `stream_url` selalu `http(s)://`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Embience {
    pub id: u32,
    pub name: String,
    pub icon_url: String,
    pub stream_url: String,
}

/// Preset suara latar. `embience_ids` berurutan menurut `position` dan hanya
/// berisi id yang ada di `GET /api/v1/embiences`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    pub id: u32,
    pub label: String,
    pub embience_ids: Vec<u32>,
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
    fn list_responses_are_wrapped_in_items() {
        let body = Items {
            items: vec![Generation {
                number: 5,
                label: "Go to Desktop".into(),
                link: "/deisktify".into(),
                kernel: "6.12.7".into(),
                built_on: "2026-08-30".into(),
            }],
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({"items": [{
                "number": 5, "label": "Go to Desktop", "link": "/deisktify",
                "kernel": "6.12.7", "built_on": "2026-08-30"
            }]})
        );
    }

    #[test]
    fn content_shapes_are_stable() {
        let radio = RadioStation {
            id: 1,
            label: "Delta FM".into(),
            slogan: "Hits".into(),
            stream_url: "https://a.b/s".into(),
            official_link: None,
            image_url: Some("https://a.b/i.png".into()),
        };
        assert_eq!(
            serde_json::to_value(&radio).unwrap(),
            serde_json::json!({
                "id": 1, "label": "Delta FM", "slogan": "Hits", "stream_url": "https://a.b/s",
                "official_link": null, "image_url": "https://a.b/i.png"
            })
        );
        let preset: Preset =
            serde_json::from_str(r#"{"id":3,"label":"Busy","embience_ids":[9,6,12]}"#).unwrap();
        assert_eq!(preset.embience_ids, [9, 6, 12]);
        let bookmark: Bookmark =
            serde_json::from_str(r#"{"id":1,"label":"x","url":"https://x.y"}"#).unwrap();
        assert_eq!(bookmark.id, 1);
        let embience: Embience = serde_json::from_str(
            r#"{"id":2,"name":"Storm","icon_url":"https://x/y.svg","stream_url":"https://x/y.mp3"}"#,
        )
        .unwrap();
        assert_eq!(embience.name, "Storm");
    }

    #[test]
    fn newer_servers_may_add_fields_without_breaking_old_clients() {
        let g: Generation = serde_json::from_str(
            r#"{"number":1,"label":"a","link":"/x","kernel":"1","built_on":"2026-01-01","new":true}"#,
        )
        .unwrap();
        assert_eq!(g.number, 1);
    }

    #[test]
    fn unknown_error_code_is_rejected() {
        assert!(serde_json::from_str::<ErrorCode>("\"teapot\"").is_err());
    }
}
