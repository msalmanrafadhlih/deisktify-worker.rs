//! Transport `fetch` Cloudflare untuk `turso-http`, dan pembuat klien.
//! Hanya jalan di runtime Workers (wasm32); di host (test) tidak dipanggil.

use std::future::Future;

use turso_http::{Client, Error, HttpResponse, Transport};
use worker::{Fetch, Headers, Method, Request, RequestInit};

use crate::config::DbConfig;

#[derive(Debug, Clone, Copy, Default)]
pub struct WorkerTransport;

impl Transport for WorkerTransport {
    fn post(
        &self,
        url: &str,
        bearer: Option<&str>,
        body: String,
    ) -> impl Future<Output = Result<HttpResponse, String>> {
        let url = url.to_owned();
        let bearer = bearer.map(str::to_owned);
        async move {
            let err = |e: worker::Error| e.to_string();

            let headers = Headers::new();
            headers
                .set("content-type", "application/json")
                .map_err(err)?;
            if let Some(token) = bearer {
                headers
                    .set("authorization", &format!("Bearer {token}"))
                    .map_err(err)?;
            }

            let mut init = RequestInit::new();
            init.with_method(Method::Post)
                .with_headers(headers)
                .with_body(Some(worker::wasm_bindgen::JsValue::from_str(&body)));

            let request = Request::new_with_init(&url, &init).map_err(err)?;
            let mut response = Fetch::Request(request).send().await.map_err(err)?;
            let status = response.status_code();
            let body = response.text().await.map_err(err)?;
            Ok(HttpResponse { status, body })
        }
    }
}

pub fn client(cfg: &DbConfig) -> Result<Client<WorkerTransport>, Error> {
    Client::new(&cfg.url, cfg.token.as_deref(), WorkerTransport)
}
