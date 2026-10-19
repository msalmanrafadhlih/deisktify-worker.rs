//! Konfigurasi dari binding Worker (`[vars]` di wrangler.toml, secret, atau
//! `.dev.vars` saat `wrangler dev`). Bagian murninya terpisah supaya bisa dites.

use worker::Env;

/// Sambungan ke Turso.
#[derive(Clone, PartialEq, Eq)]
pub struct DbConfig {
    pub url: String,
    pub token: Option<String>,
}

// Token tidak boleh ikut ter-log lewat `{:?}`.
impl std::fmt::Debug for DbConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbConfig")
            .field("url", &self.url)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Nilai rahasia yang tidak boleh ikut ter-log lewat `{:?}`.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// Origin browser yang boleh memanggil API (CORS). Kosong = tidak ada.
    pub allowed_origins: Vec<String>,
    /// `None` jika `TURSO_DATABASE_URL` belum diisi.
    pub db: Option<DbConfig>,
    /// Garam untuk hash IP pembatas kecepatan chat (`RATE_LIMIT_SALT`). Kosong
    /// tetap berfungsi, tetapi hash IP jadi lebih mudah ditebak.
    pub rate_salt: Secret,
    /// Kata terlarang di chat (`BANNED_WORDS`, dipisah koma), huruf kecil.
    /// Sengaja dari konfigurasi, bukan dari repo publik.
    pub banned_words: Vec<String>,
    /// Token admin (`ADMIN_TOKEN`, secret). Kosong atau pendek = endpoint admin
    /// tertutup untuk semua orang (lihat `admin.rs`).
    pub admin_token: Secret,
    /// Token GitHub untuk Cron Trigger jurnal (`GITHUB_TOKEN`, secret, scope
    /// `read:user` cukup). Kosong = jurnal tidak bisa diperbarui.
    pub github_token: Secret,
}

/// Daftar kata terlarang dipisah koma atau baris baru. Kosong dan terlalu
/// panjang dibuang; paling banyak 500 kata.
pub fn parse_banned_words(raw: &str) -> Vec<String> {
    raw.split([',', '\n'])
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty() && s.chars().count() <= 64)
        .take(500)
        .collect()
}

/// `https://host[:port]` atau `http://host[:port]`, tanpa path, tanpa wildcard.
fn is_valid_origin(s: &str) -> bool {
    let Some(host) = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
    else {
        return false;
    };
    !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
}

/// Mem-parsing daftar origin dipisah koma. Entri tidak valid (wildcard `*`,
/// ada path, tanpa skema, dst.) dibuang, bukan ditebak.
pub fn parse_origins(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| is_valid_origin(s))
        .collect()
}

impl Config {
    pub fn from_values(
        origins: Option<&str>,
        db_url: Option<&str>,
        db_token: Option<&str>,
    ) -> Self {
        fn non_empty(s: Option<&str>) -> Option<&str> {
            s.map(str::trim).filter(|s| !s.is_empty())
        }
        Self {
            allowed_origins: origins.map(parse_origins).unwrap_or_default(),
            db: non_empty(db_url).map(|url| DbConfig {
                url: url.to_owned(),
                token: non_empty(db_token).map(str::to_owned),
            }),
            rate_salt: Secret::default(),
            banned_words: Vec::new(),
            admin_token: Secret::default(),
            github_token: Secret::default(),
        }
    }

    /// Token admin (opsional).
    #[must_use]
    pub fn with_admin(mut self, token: Option<&str>) -> Self {
        self.admin_token = Secret::new(token.map(str::trim).unwrap_or_default());
        self
    }

    /// Token GitHub untuk jurnal (opsional).
    #[must_use]
    pub fn with_github(mut self, token: Option<&str>) -> Self {
        self.github_token = Secret::new(token.map(str::trim).unwrap_or_default());
        self
    }

    /// Pengaturan Live Chat (opsional).
    #[must_use]
    pub fn with_chat(mut self, salt: Option<&str>, banned_words: Option<&str>) -> Self {
        self.rate_salt = Secret::new(salt.map(str::trim).unwrap_or_default());
        self.banned_words = banned_words.map(parse_banned_words).unwrap_or_default();
        self
    }

    /// Variabel biasa dulu, lalu secret (URL database boleh disimpan sebagai
    /// salah satunya; token seharusnya selalu secret).
    pub fn from_env(env: &Env) -> Self {
        let text = |name: &str| {
            env.var(name)
                .map(|v| v.to_string())
                .or_else(|_| env.secret(name).map(|v| v.to_string()))
                .ok()
        };
        let origins = text("ALLOWED_ORIGINS");
        let url = text("TURSO_DATABASE_URL");
        let token = env.secret("TURSO_AUTH_TOKEN").map(|v| v.to_string()).ok();
        let salt = text("RATE_LIMIT_SALT");
        let banned = text("BANNED_WORDS");
        // Hanya secret: token admin tidak boleh jatuh ke `[vars]` yang ada di repo.
        let admin = env.secret("ADMIN_TOKEN").map(|v| v.to_string()).ok();
        // Hanya secret, sama alasannya.
        let github = env.secret("GITHUB_TOKEN").map(|v| v.to_string()).ok();
        Self::from_values(origins.as_deref(), url.as_deref(), token.as_deref())
            .with_chat(salt.as_deref(), banned.as_deref())
            .with_admin(admin.as_deref())
            .with_github(github.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_are_parsed_and_sanitized() {
        assert_eq!(
            parse_origins(" https://Tquilla.is-a.dev ,http://localhost:8080,https://[::1]:8080"),
            [
                "https://tquilla.is-a.dev",
                "http://localhost:8080",
                "https://[::1]:8080"
            ]
        );
        for bad in [
            "*",
            "https://*.is-a.dev",
            "tquilla.is-a.dev",
            "https://tquilla.is-a.dev/",
            "https://tquilla.is-a.dev/path",
            "https://user@host",
            "ftp://host",
            "https://",
            "",
        ] {
            assert!(parse_origins(bad).is_empty(), "{bad:?} must be dropped");
        }
    }

    #[test]
    fn banned_words_are_trimmed_lowercased_and_bounded() {
        assert_eq!(
            parse_banned_words(" Foo , bar\n\nBAZ,, "),
            ["foo", "bar", "baz"]
        );
        assert!(parse_banned_words("").is_empty());
        assert!(parse_banned_words(&"x".repeat(65)).is_empty());
        let many = (0..600)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(parse_banned_words(&many).len(), 500);
    }

    #[test]
    fn chat_settings_default_to_empty_and_the_salt_stays_out_of_debug_output() {
        let c = Config::from_values(None, None, None);
        assert_eq!(c.rate_salt.expose(), "");
        assert!(c.banned_words.is_empty());
        let c = c.with_chat(Some(" s3cret "), Some("a,b"));
        assert_eq!(c.rate_salt.expose(), "s3cret");
        assert_eq!(c.banned_words, ["a", "b"]);
        assert!(!format!("{c:?}").contains("s3cret"));
    }

    #[test]
    fn the_admin_token_defaults_to_empty_and_stays_out_of_debug_output() {
        let c = Config::from_values(None, None, None);
        assert_eq!(c.admin_token.expose(), "");
        let c = c.with_admin(Some("  tok-tok-tok  "));
        assert_eq!(c.admin_token.expose(), "tok-tok-tok");
        assert!(!format!("{c:?}").contains("tok-tok-tok"));
    }

    #[test]
    fn the_github_token_defaults_to_empty_and_stays_out_of_debug_output() {
        let c = Config::from_values(None, None, None);
        assert_eq!(c.github_token.expose(), "");
        let c = c.with_github(Some("  ghp_tok-tok  "));
        assert_eq!(c.github_token.expose(), "ghp_tok-tok");
        assert!(!format!("{c:?}").contains("ghp_tok-tok"));
    }

    #[test]
    fn missing_values_mean_closed_by_default() {
        let c = Config::from_values(None, None, None);
        assert!(c.allowed_origins.is_empty());
        assert!(c.db.is_none());
        let c = Config::from_values(Some(""), Some("  "), Some("tok"));
        assert!(c.allowed_origins.is_empty());
        assert!(c.db.is_none(), "a token without a url is not a database");
    }

    #[test]
    fn db_config_keeps_url_and_optional_token() {
        let c = Config::from_values(None, Some("libsql://x.turso.io"), Some(" "));
        assert_eq!(
            c.db,
            Some(DbConfig {
                url: "libsql://x.turso.io".into(),
                token: None
            })
        );
    }

    #[test]
    fn debug_output_hides_the_token() {
        let c = Config::from_values(None, Some("http://127.0.0.1:8081"), Some("super-secret"));
        let shown = format!("{c:?}");
        assert!(!shown.contains("super-secret"), "{shown}");
    }
}
