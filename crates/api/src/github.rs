//! Pengirim `fetch` Cloudflare ke GitHub GraphQL. Hanya jalan di runtime
//! Workers (wasm32); di host (test) tidak dipanggil. Logikanya ada di
//! `journal.rs`.

use std::future::Future;

use turso_http::HttpResponse;
use worker::{Fetch, Headers, Method, Request, RequestInit};

use crate::journal::GraphQl;

const ENDPOINT: &str = "https://api.github.com/graphql";

/// Token GitHub (`GITHUB_TOKEN`) dikirim hanya sebagai header `Authorization`.
/// Tidak ikut `Debug`, tidak masuk body, tidak masuk pesan error.
pub struct GithubClient {
    token: String,
}

impl GithubClient {
    pub fn new(token: &str) -> Self {
        Self {
            token: token.to_owned(),
        }
    }
}

impl std::fmt::Debug for GithubClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GithubClient(<redacted>)")
    }
}

impl GraphQl for GithubClient {
    fn post(&self, body: String) -> impl Future<Output = Result<HttpResponse, String>> {
        let token = self.token.clone();
        async move {
            let err = |e: worker::Error| e.to_string();

            let headers = Headers::new();
            headers
                .set("content-type", "application/json")
                .map_err(err)?;
            headers
                .set("authorization", &format!("Bearer {token}"))
                .map_err(err)?;
            // GitHub menolak permintaan tanpa User-Agent.
            headers.set("user-agent", "tquilla-api").map_err(err)?;

            let mut init = RequestInit::new();
            init.with_method(Method::Post)
                .with_headers(headers)
                .with_body(Some(worker::wasm_bindgen::JsValue::from_str(&body)));

            let request = Request::new_with_init(ENDPOINT, &init).map_err(err)?;
            let mut response = Fetch::Request(request).send().await.map_err(err)?;
            let status = response.status_code();
            let body = response.text().await.map_err(err)?;
            Ok(HttpResponse { status, body })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_hides_the_token() {
        let c = GithubClient::new("ghp_supersecret");
        assert!(!format!("{c:?}").contains("supersecret"));
    }
}
