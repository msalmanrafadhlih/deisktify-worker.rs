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

    /// `GET`: entri Guestbook (`?after=<id>` atau `?before=<id>`, `&limit=<n>`).
    /// `POST`: tulis entri (teks Markdown).
    pub const MESSAGES: &str = "/api/v1/messages";
    /// `POST`: laporkan sebuah entri (D4b). Templat rute Axum; klien memakai
    /// [`message_report`].
    pub const MESSAGE_REPORT: &str = "/api/v1/messages/{id}/report";
    /// `GET`: entri yang dilaporkan, paling banyak dilaporkan dulu. Hanya admin
    /// (`Authorization: Bearer`), tidak untuk browser.
    pub const ADMIN_REPORTS: &str = "/api/v1/admin/reports";
    /// `DELETE`: hapus (sembunyikan) sebuah entri. Hanya admin. Templat rute
    /// Axum; pemanggil memakai [`admin_message`].
    pub const ADMIN_MESSAGE: &str = "/api/v1/admin/messages/{id}";

    /// Semua path; tambahkan endpoint baru di sini juga supaya test prefix mencakupnya.
    pub const ALL: &[&str] = &[
        HEALTH,
        GENERATIONS,
        BOOKMARKS,
        RADIO,
        EMBIENCES,
        PRESETS,
        MESSAGES,
        MESSAGE_REPORT,
        ADMIN_REPORTS,
        ADMIN_MESSAGE,
    ];

    /// Path `POST` untuk melaporkan entri `id`.
    pub fn message_report(id: u32) -> String {
        format!("/api/v1/messages/{id}/report")
    }

    /// Path `DELETE` admin untuk entri `id`.
    pub fn admin_message(id: u32) -> String {
        format!("/api/v1/admin/messages/{id}")
    }
}

/// Batas yang diberlakukan server. Klien memakainya untuk `maxlength` dan
/// validasi awal; server tetap memeriksa ulang.
pub mod limits {
    /// Ukuran maksimum body request JSON, dalam byte.
    pub const MAX_BODY_BYTES: usize = 16 * 1024;
    /// Panjang nama pengguna Guestbook, dalam karakter (setelah dirapikan).
    pub const MAX_USERNAME_CHARS: usize = 32;
    /// Panjang entri Guestbook (teks Markdown), dalam karakter (setelah
    /// dirapikan). Kolom database membolehkan sampai 4000; 2000 cukup untuk
    /// sebuah "README mini" dan tetap muat di batas body 16 KiB.
    pub const MAX_MESSAGE_CHARS: usize = 2000;
    /// Jumlah baris maksimum dalam satu entri.
    pub const MAX_MESSAGE_LINES: usize = 40;
    /// Panjang tautan pengirim, dalam karakter.
    pub const MAX_LINK_CHARS: usize = 300;
    /// Jumlah entri default dan maksimum per `GET /messages`.
    pub const MESSAGES_DEFAULT_LIMIT: u32 = 50;
    pub const MESSAGES_MAX_LIMIT: u32 = 100;
    /// Jumlah entri terlapor yang dikembalikan `GET /admin/reports`.
    pub const REPORTS_LIMIT: u32 = 50;
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

    /// Menjadikan input pengguna sebuah URL `http(s)`: `a.dev` -> `https://a.dev`,
    /// skema dijadikan huruf kecil. Input dengan skema lain (`javascript:`,
    /// `data:`, `mailto:`, `ftp://`), berspasi, atau kosong -> `None`.
    /// `host:port` tanpa skema (`a.dev:8080`) dianggap host.
    pub fn normalize_url(raw: &str) -> Option<String> {
        let t = raw.trim();
        if t.is_empty() || t.contains(char::is_whitespace) {
            return None;
        }
        let lower = t.to_ascii_lowercase();
        let url = if lower.starts_with("https://") {
            format!("https://{}", &t["https://".len()..])
        } else if lower.starts_with("http://") {
            format!("http://{}", &t["http://".len()..])
        } else {
            if let Some((before, after)) = t.split_once(':') {
                let scheme_like = before.starts_with(|c: char| c.is_ascii_alphabetic())
                    && before
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
                if scheme_like && !after.starts_with(|c: char| c.is_ascii_digit()) {
                    return None;
                }
            }
            format!("https://{t}")
        };
        is_http_url(&url).then_some(url)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn user_urls_become_http_urls_or_nothing() {
            assert_eq!(normalize_url("a.dev").as_deref(), Some("https://a.dev"));
            assert_eq!(
                normalize_url("  a.dev/x ").as_deref(),
                Some("https://a.dev/x")
            );
            assert_eq!(
                normalize_url("HTTP://A.DEV").as_deref(),
                Some("http://A.DEV")
            );
            assert_eq!(
                normalize_url("https://a.dev/Path?q=1").as_deref(),
                Some("https://a.dev/Path?q=1")
            );
            assert_eq!(
                normalize_url("a.dev:8080/x").as_deref(),
                Some("https://a.dev:8080/x")
            );
            for bad in [
                "",
                "   ",
                "a b.dev",
                "javascript:alert(1)",
                "JaVaScRiPt:alert(1)",
                "data:text/html,<b>x</b>",
                "vbscript:x",
                "mailto:a@b.c",
                "ftp://a.b",
                "//evil.example",
                "https://",
                "http:///x",
            ] {
                assert_eq!(normalize_url(bad), None, "{bad:?}");
            }
        }

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

/// Satu entri Guestbook. `message` adalah teks Markdown mentah (server tidak
/// pernah mengubahnya menjadi HTML; klien yang merender dan harus
/// mengabaikan HTML mentah di dalamnya). `url` (kalau ada) selalu `http(s)://`;
/// klien tetap memeriksanya. Avatar dan lampiran sengaja tidak ada: avatar
/// dibuat klien dari `username`, dan unggahan tidak didukung.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: u32,
    pub username: String,
    pub url: Option<String>,
    pub message: String,
    /// UTC, `YYYY-MM-DDTHH:MM:SSZ`.
    pub created_at: String,
}

/// Satu entri yang dilaporkan, untuk admin (`GET /api/v1/admin/reports`).
/// Hanya dipakai alat admin; browser publik tidak pernah menerimanya.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportedMessage {
    pub id: u32,
    pub username: String,
    pub url: Option<String>,
    pub message: String,
    /// UTC, `YYYY-MM-DDTHH:MM:SSZ`.
    pub created_at: String,
    /// Jumlah pelapor berbeda.
    pub reports: u32,
    /// UTC, laporan terbaru.
    pub last_reported_at: String,
}

/// Body `POST /api/v1/messages`. Server merapikan dan memvalidasi ulang semuanya
/// (lihat [`chat`]); field yang tidak dikenal diabaikan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewMessage {
    pub username: String,
    #[serde(default)]
    pub url: Option<String>,
    pub message: String,
}

/// Perapian dan pemeriksaan teks Guestbook. Dipakai server (yang berwenang) dan
/// klien (hanya untuk pratinjau/tombol kirim), supaya aturannya tidak berbeda.
pub mod chat {
    use super::{limits, validate};

    /// Alasan sebuah input ditolak. `message()` aman dikirim ke klien.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Reject {
        UsernameLength,
        UsernameChars,
        MessageLength,
        MessageChars,
        Link,
        /// Dipakai server untuk daftar kata terlarang; pesannya sengaja umum.
        Blocked,
    }

    impl Reject {
        pub fn message(self) -> &'static str {
            match self {
                Reject::UsernameLength => "Username must be 1 to 32 characters.",
                Reject::UsernameChars => "Username has characters that are not allowed.",
                Reject::MessageLength => {
                    "Message must be 1 to 2000 characters and at most 40 lines."
                }
                Reject::MessageChars => "Message has characters that are not allowed.",
                Reject::Link => "The link must be a valid web address.",
                Reject::Blocked => "This message was rejected.",
            }
        }
    }

    /// Karakter kontrol dan karakter tak terlihat/pembalik arah teks
    /// (zero-width, U+202E, dst.) yang dipakai untuk menyamar.
    fn hidden(c: char) -> bool {
        c.is_control()
            || matches!(
                c,
                '\u{00AD}'
                    | '\u{200B}'..='\u{200F}'
                    | '\u{202A}'..='\u{202E}'
                    | '\u{2060}'..='\u{2064}'
                    | '\u{2066}'..='\u{2069}'
                    | '\u{FEFF}'
            )
    }

    /// Spasi berurutan (juga tab/baris baru) menjadi satu spasi; 1..=32 karakter.
    pub fn clean_username(raw: &str) -> Result<String, Reject> {
        let s = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        if s.chars().any(hidden) {
            return Err(Reject::UsernameChars);
        }
        let n = s.chars().count();
        if n == 0 || n > limits::MAX_USERNAME_CHARS {
            return Err(Reject::UsernameLength);
        }
        Ok(s)
    }

    /// Karakter yang ditolak di isi entri. Sama dengan [`hidden`] kecuali
    /// U+200D (zero-width joiner): emoji majemuk (keluarga, profesi, bendera
    /// bergender) tersusun darinya. Pencocokan kata terlarang membuang semua
    /// non-alfanumerik, jadi ZWJ tidak bisa dipakai menyelinap lewat filter.
    fn hidden_in_message(c: char) -> bool {
        c != '\u{200D}' && hidden(c)
    }

    /// Dirapikan untuk disimpan sebagai Markdown: `\r\n` menjadi `\n`, tab
    /// menjadi empat spasi (Markdown menghitung tab sebagai kelipatan empat,
    /// dan tab mentah merusak blok kode dan daftar bersarang), spasi di
    /// ujung baris dibuang, lebih dari satu baris kosong berturut-turut
    /// diringkas; 1..=2000 karakter dan paling banyak 40 baris.
    pub fn clean_message(raw: &str) -> Result<String, Reject> {
        let unified = raw.replace("\r\n", "\n").replace('\r', "\n");
        let mut out = String::with_capacity(unified.len());
        for c in unified.chars() {
            match c {
                '\n' => out.push('\n'),
                '\t' => out.push_str("    "),
                c if hidden_in_message(c) => return Err(Reject::MessageChars),
                c => out.push(c),
            }
        }
        let mut lines: Vec<&str> = Vec::new();
        for line in out.trim().split('\n') {
            let line = line.trim_end();
            // Paling banyak satu baris kosong berturut-turut.
            if line.is_empty() && lines.last().is_some_and(|l| l.is_empty()) {
                continue;
            }
            lines.push(line);
        }
        let s = lines.join("\n");
        let n = s.chars().count();
        if n == 0 || n > limits::MAX_MESSAGE_CHARS || lines.len() > limits::MAX_MESSAGE_LINES {
            return Err(Reject::MessageLength);
        }
        Ok(s)
    }

    /// Tautan opsional: kosong -> `None`; selain itu harus jadi URL `http(s)`
    /// (lihat [`validate::normalize_url`]) dan paling panjang 300 karakter.
    pub fn clean_link(raw: &str) -> Result<Option<String>, Reject> {
        if raw.trim().is_empty() {
            return Ok(None);
        }
        match validate::normalize_url(raw) {
            Some(url) if url.chars().count() <= limits::MAX_LINK_CHARS => Ok(Some(url)),
            _ => Err(Reject::Link),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn usernames_are_collapsed_and_bounded() {
            assert_eq!(clean_username("  Moch   T \n").as_deref(), Ok("Moch T"));
            assert_eq!(clean_username("名前").as_deref(), Ok("名前"));
            assert_eq!(clean_username("   "), Err(Reject::UsernameLength));
            assert_eq!(clean_username(&"a".repeat(33)), Err(Reject::UsernameLength));
            assert!(clean_username(&"a".repeat(32)).is_ok());
            for bad in ["a\u{0}b", "adm\u{202E}in", "a\u{200B}b", "a\u{FEFF}"] {
                assert_eq!(clean_username(bad), Err(Reject::UsernameChars), "{bad:?}");
            }
        }

        #[test]
        fn messages_are_normalised_and_bounded() {
            assert_eq!(clean_message("  hi  ").as_deref(), Ok("hi"));
            assert_eq!(clean_message("a\r\nb\rc").as_deref(), Ok("a\nb\nc"));
            assert_eq!(clean_message("a\tb").as_deref(), Ok("a    b"));
            assert_eq!(
                clean_message("- a\n\t- b  \n").as_deref(),
                Ok("- a\n    - b"),
                "nested list keeps its indent; trailing spaces go"
            );
            assert_eq!(clean_message("a\n\n\n\n\nb").as_deref(), Ok("a\n\nb"));
            assert_eq!(clean_message(" \n "), Err(Reject::MessageLength));
            assert_eq!(clean_message(&"a".repeat(2001)), Err(Reject::MessageLength));
            assert!(clean_message(&"a".repeat(2000)).is_ok());
            assert!(
                clean_message(&"é".repeat(2000)).is_ok(),
                "counted in chars, not bytes"
            );
            let many = (0..41).map(|_| "x").collect::<Vec<_>>().join("\n");
            assert_eq!(clean_message(&many), Err(Reject::MessageLength));
            let forty = (0..40).map(|_| "x").collect::<Vec<_>>().join("\n");
            assert!(clean_message(&forty).is_ok());
            for bad in ["a\u{0}b", "a\u{1b}[31m", "x\u{202E}y", "a\u{200B}b"] {
                assert_eq!(clean_message(bad), Err(Reject::MessageChars), "{bad:?}");
            }
        }

        #[test]
        fn compound_emoji_survive_but_usernames_still_reject_zero_width_joiners() {
            let family = "hi \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
            assert_eq!(clean_message(family).as_deref(), Ok(family));
            assert_eq!(
                clean_username("a\u{200D}b"),
                Err(Reject::UsernameChars),
                "a joiner has no business in a name"
            );
        }

        #[test]
        fn markdown_syntax_is_stored_verbatim() {
            let md = "# Hi\n\n**bold** _it_ ~~x~~ `c`\n\n- [x] done\n\n```rs\nfn main() {}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |";
            assert_eq!(clean_message(md).as_deref(), Ok(md));
        }

        #[test]
        fn links_are_optional_and_always_http() {
            assert_eq!(clean_link(""), Ok(None));
            assert_eq!(clean_link("  "), Ok(None));
            assert_eq!(clean_link("a.dev"), Ok(Some("https://a.dev".to_owned())));
            assert_eq!(clean_link("javascript:alert(1)"), Err(Reject::Link));
            assert_eq!(clean_link("a b"), Err(Reject::Link));
            assert_eq!(
                clean_link(&format!("a.dev/{}", "x".repeat(300))),
                Err(Reject::Link)
            );
        }
    }
}

/// Kode error yang stabil dan aman ditampilkan; pesan detail internal tidak
/// pernah dikirim ke klien.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    NotFound,
    MethodNotAllowed,
    /// Request ditolak karena asalnya (mis. `Origin` yang tidak diizinkan).
    Forbidden,
    /// Endpoint admin: token tidak ada atau salah.
    Unauthorized,
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
    fn path_helpers_fill_in_the_route_templates() {
        assert_eq!(paths::message_report(42), "/api/v1/messages/42/report");
        assert_eq!(
            paths::message_report(42),
            paths::MESSAGE_REPORT.replace("{id}", "42")
        );
        assert_eq!(paths::admin_message(7), "/api/v1/admin/messages/7");
        assert_eq!(
            paths::admin_message(7),
            paths::ADMIN_MESSAGE.replace("{id}", "7")
        );
    }

    #[test]
    fn unauthorized_error_code_is_snake_case_on_the_wire() {
        let body = ApiErrorBody::new(ErrorCode::Unauthorized, "No.");
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({"error": {"code": "unauthorized", "message": "No."}})
        );
    }

    #[test]
    fn reported_message_json_shape_is_stable() {
        let r = ReportedMessage {
            id: 3,
            username: "u".into(),
            url: None,
            message: "m".into(),
            created_at: "2026-10-18T00:00:00Z".into(),
            reports: 2,
            last_reported_at: "2026-10-18T01:00:00Z".into(),
        };
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            serde_json::json!({
                "id": 3, "username": "u", "url": null, "message": "m",
                "created_at": "2026-10-18T00:00:00Z", "reports": 2,
                "last_reported_at": "2026-10-18T01:00:00Z"
            })
        );
    }

    #[test]
    fn guestbook_entry_json_shape_is_stable_and_has_no_image_fields() {
        let m = ChatMessage {
            id: 4,
            username: "moch".into(),
            url: Some("https://a.dev".into()),
            message: "# hi".into(),
            created_at: "2026-10-18T00:00:00Z".into(),
        };
        assert_eq!(
            serde_json::to_value(&m).unwrap(),
            serde_json::json!({
                "id": 4, "username": "moch", "url": "https://a.dev",
                "message": "# hi", "created_at": "2026-10-18T00:00:00Z"
            })
        );
    }

    #[test]
    fn clients_and_servers_from_the_chat_era_still_understand_each_other() {
        // Server lama (v0.3) masih mengirim kolom gambar; klien baru mengabaikannya.
        let old_server = r#"{"id":1,"username":"a","url":null,"avatar_url":"https://x/a.png",
            "message":"m","attachment_url":null,"created_at":"2026-01-01T00:00:00Z"}"#;
        let m: ChatMessage = serde_json::from_str(old_server).unwrap();
        assert_eq!(m.id, 1);

        // Klien lama (v0.3) membaca bentuk baru: kolom gambar adalah `Option`,
        // jadi yang hilang jatuh ke `None` dan tidak membuat decode gagal.
        #[derive(Deserialize)]
        struct V03 {
            id: u32,
            avatar_url: Option<String>,
            attachment_url: Option<String>,
        }
        let new_shape = serde_json::to_string(&m).unwrap();
        let old_client: V03 = serde_json::from_str(&new_shape).unwrap();
        assert_eq!(old_client.id, 1);
        assert_eq!(
            (old_client.avatar_url, old_client.attachment_url),
            (None, None)
        );
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
