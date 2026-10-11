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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// Origin browser yang boleh memanggil API (CORS). Kosong = tidak ada.
    pub allowed_origins: Vec<String>,
    /// `None` jika `TURSO_DATABASE_URL` belum diisi.
    pub db: Option<DbConfig>,
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
        }
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
        Self::from_values(origins.as_deref(), url.as_deref(), token.as_deref())
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
