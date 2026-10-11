//! CORS: hanya origin yang terdaftar di `ALLOWED_ORIGINS`. Tanpa cookie dan
//! tanpa `Access-Control-Allow-Credentials`; otorisasi admin (D6) memakai header.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderName, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::Response,
};

/// Header CORS untuk sebuah request. Kosong jika origin tidak diizinkan
/// (browser lalu memblokir sendiri). `Vary: Origin` selalu ada agar cache
/// tidak mencampur jawaban antar origin.
pub fn cors_headers(
    allowed: &[String],
    origin: Option<&str>,
    preflight: bool,
) -> Vec<(HeaderName, String)> {
    let mut out = vec![(header::VARY, "Origin".to_owned())];
    let Some(origin) = origin.filter(|o| allowed.iter().any(|a| a == o)) else {
        return out;
    };
    out.push((header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.to_owned()));
    if preflight {
        out.push((
            header::ACCESS_CONTROL_ALLOW_METHODS,
            "GET, POST, OPTIONS".into(),
        ));
        out.push((header::ACCESS_CONTROL_ALLOW_HEADERS, "content-type".into()));
        out.push((header::ACCESS_CONTROL_MAX_AGE, "600".into()));
    }
    out
}

fn apply(res: &mut Response, headers: Vec<(HeaderName, String)>) {
    for (name, value) in headers {
        if let Ok(v) = HeaderValue::from_str(&value) {
            // `append` untuk Vary supaya tidak menimpa nilai dari lapisan lain.
            if name == header::VARY {
                res.headers_mut().append(name, v);
            } else {
                res.headers_mut().insert(name, v);
            }
        }
    }
}

/// Header yang berlaku untuk semua respons API.
fn harden(res: &mut Response) {
    let h = res.headers_mut();
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.entry(header::CACHE_CONTROL)
        .or_insert(HeaderValue::from_static("no-store"));
}

pub async fn middleware(
    State(allowed): State<Arc<Vec<String>>>,
    req: Request,
    next: Next,
) -> Response {
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let is_preflight = req.method() == Method::OPTIONS
        && req
            .headers()
            .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD);

    let mut res = if is_preflight {
        let mut r = Response::new(Body::empty());
        *r.status_mut() = StatusCode::NO_CONTENT;
        r
    } else {
        next.run(req).await
    };

    apply(
        &mut res,
        cors_headers(&allowed, origin.as_deref(), is_preflight),
    );
    harden(&mut res);
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed() -> Vec<String> {
        vec![
            "https://tquilla.is-a.dev".into(),
            "http://localhost:8080".into(),
        ]
    }

    fn get(h: &[(HeaderName, String)], name: HeaderName) -> Option<&str> {
        h.iter().find(|(n, _)| *n == name).map(|(_, v)| v.as_str())
    }

    #[test]
    fn allowed_origin_is_echoed_exactly() {
        let h = cors_headers(&allowed(), Some("https://tquilla.is-a.dev"), false);
        assert_eq!(
            get(&h, header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some("https://tquilla.is-a.dev")
        );
        assert_eq!(get(&h, header::VARY), Some("Origin"));
        assert!(get(&h, header::ACCESS_CONTROL_ALLOW_METHODS).is_none());
    }

    #[test]
    fn unknown_missing_and_lookalike_origins_get_no_access() {
        for origin in [
            None,
            Some("null"),
            Some("https://evil.example"),
            Some("https://tquilla.is-a.dev.evil.example"),
            Some("http://tquilla.is-a.dev"),
            Some("https://tquilla.is-a.dev/"),
            Some("*"),
        ] {
            let h = cors_headers(&allowed(), origin, false);
            assert!(
                get(&h, header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none(),
                "{origin:?}"
            );
            assert_eq!(get(&h, header::VARY), Some("Origin"));
        }
    }

    #[test]
    fn empty_allowlist_allows_nobody() {
        let h = cors_headers(&[], Some("https://tquilla.is-a.dev"), true);
        assert!(get(&h, header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
    }

    #[test]
    fn preflight_lists_methods_headers_and_max_age_but_no_credentials() {
        let h = cors_headers(&allowed(), Some("http://localhost:8080"), true);
        assert_eq!(
            get(&h, header::ACCESS_CONTROL_ALLOW_METHODS),
            Some("GET, POST, OPTIONS")
        );
        assert_eq!(
            get(&h, header::ACCESS_CONTROL_ALLOW_HEADERS),
            Some("content-type")
        );
        assert_eq!(get(&h, header::ACCESS_CONTROL_MAX_AGE), Some("600"));
        assert!(get(&h, header::ACCESS_CONTROL_ALLOW_CREDENTIALS).is_none());
    }
}
