//! Pengetuk stream radio lewat `fetch` Cloudflare. Hanya jalan di runtime
//! Workers (wasm32); di host (test) tidak dipanggil. Logikanya (URL mana yang
//! boleh, redirect, menilai status) ada di `radio_check.rs`.
//!
//! - `redirect: manual`: `fetch` tidak pernah mengikuti redirect sendiri; tiap
//!   lompatan diputuskan `radio_check::probe_url` (dan dihitung sebagai
//!   subrequest tersendiri oleh Cloudflare).
//! - Hanya header yang dibaca. Begitu header diterima, `AbortController`
//!   memutus koneksi sehingga badan stream tidak diunduh.
//! - Batas waktu [`TIMEOUT_MS`] (waktu dinding, bukan CPU).
//! - Pesan error `fetch` tidak diteruskan: bisa memuat URL dan tokennya.

use std::future::Future;
use std::time::Duration;

use futures_util::future::{select, Either};
use worker::{
    AbortController, Delay, Fetch, Headers, Method, Request, RequestInit, RequestRedirect,
};

use crate::radio_check::{Probe, ProbeError, Reply};

/// Batas menunggu header satu permintaan.
pub const TIMEOUT_MS: u64 = 8_000;

#[derive(Debug, Clone, Copy, Default)]
pub struct WorkerProbe;

impl Probe for WorkerProbe {
    fn get(&self, url: &str) -> impl Future<Output = Result<Reply, ProbeError>> {
        let url = url.to_owned();
        async move {
            let net = |_: worker::Error| ProbeError::Network;

            let headers = Headers::new();
            headers
                .set("user-agent", "tquilla-api radio-check")
                .map_err(net)?;
            headers.set("accept", "*/*").map_err(net)?;
            // Server Shoutcast/Icecast menyisipkan metadata di badan kalau diminta.
            headers.set("icy-metadata", "0").map_err(net)?;

            let mut init = RequestInit::new();
            init.with_method(Method::Get)
                .with_headers(headers)
                .with_redirect(RequestRedirect::Manual);
            let request = Request::new_with_init(&url, &init).map_err(net)?;

            let controller = AbortController::default();
            let signal = controller.signal();
            let fetch = Fetch::Request(request);
            let send = Box::pin(fetch.send_with_signal(&signal));
            let timeout = Box::pin(Delay::from(Duration::from_millis(TIMEOUT_MS)));

            let response = match select(send, timeout).await {
                Either::Left((Ok(response), _)) => response,
                Either::Left((Err(_), _)) => return Err(ProbeError::Network),
                Either::Right(_) => {
                    controller.abort();
                    return Err(ProbeError::Timeout);
                }
            };

            let reply = Reply {
                status: response.status_code(),
                location: response.headers().get("location").ok().flatten(),
            };
            // Header cukup: putuskan koneksi, jangan unduh stream yang tak berujung.
            controller.abort();
            Ok(reply)
        }
    }
}
