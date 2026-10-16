//! Handler HTTP untuk endpoint baca konten (D3). Query dan validasinya ada di
//! `content.rs` (dites native); di sini hanya menyambungkan konfigurasi,
//! klien database, dan respons.

use std::future::Future;
use std::sync::Arc;

use api_types::Items;
use axum::{
    extract::State,
    http::{header, HeaderValue},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

use crate::{
    config::Config,
    content::{self, ContentError, Loaded},
    db::{self, WorkerTransport},
    error::AppError,
};

/// Konten jarang berubah: browser boleh memakai salinan 1 menit dan menyajikan
/// yang basi sambil memperbarui selama 5 menit. Hanya untuk respons sukses;
/// error tidak pernah di-cache (`no-store` dari middleware).
const CONTENT_CACHE: &str = "public, max-age=60, stale-while-revalidate=300";

/// Pesan error umum; detail hanya ke log Worker.
pub fn app_error(e: &ContentError) -> AppError {
    use turso_http::Error;
    match e {
        // Database tidak terjangkau / menolak koneksi: sementara.
        ContentError::Db(Error::Config(_) | Error::Transport(_) | Error::Http { .. }) => {
            AppError::UNAVAILABLE
        }
        // SQL gagal, protokol aneh, atau data berbentuk salah: bug/skema.
        ContentError::Db(_) | ContentError::BadData(_) => AppError::INTERNAL,
    }
}

fn ok<T: Serialize>(items: Vec<T>) -> Response {
    let mut res = Json(Items { items }).into_response();
    res.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(CONTENT_CACHE),
    );
    res
}

async fn respond<T, Fut>(
    config: &Config,
    name: &'static str,
    run: impl FnOnce(turso_http::Client<WorkerTransport>) -> Fut,
) -> Result<Response, AppError>
where
    T: Serialize,
    Fut: Future<Output = Result<Loaded<T>, ContentError>>,
{
    let Some(cfg) = &config.db else {
        return Err(AppError::UNAVAILABLE);
    };
    let client = db::client(cfg).map_err(|e| {
        worker::console_error!("{name}: invalid database config: {e}");
        AppError::UNAVAILABLE
    })?;
    match run(client).await {
        Ok(loaded) => {
            if loaded.skipped > 0 {
                worker::console_error!("{name}: skipped {} invalid row(s)", loaded.skipped);
            }
            Ok(ok(loaded.items))
        }
        Err(e) => {
            worker::console_error!("{name}: {e}");
            Err(app_error(&e))
        }
    }
}

// `#[worker::send]`: future di wasm32 tidak `Send`, sedangkan axum memintanya.

#[worker::send]
pub async fn generations(State(config): State<Arc<Config>>) -> Result<Response, AppError> {
    respond(&config, "generations", |c| async move {
        content::generations(&c).await
    })
    .await
}

#[worker::send]
pub async fn bookmarks(State(config): State<Arc<Config>>) -> Result<Response, AppError> {
    respond(&config, "bookmarks", |c| async move {
        content::bookmarks(&c).await
    })
    .await
}

#[worker::send]
pub async fn radio(State(config): State<Arc<Config>>) -> Result<Response, AppError> {
    respond(
        &config,
        "radio",
        |c| async move { content::radio(&c).await },
    )
    .await
}

#[worker::send]
pub async fn embiences(State(config): State<Arc<Config>>) -> Result<Response, AppError> {
    respond(&config, "embiences", |c| async move {
        content::embiences(&c).await
    })
    .await
}

#[worker::send]
pub async fn presets(State(config): State<Arc<Config>>) -> Result<Response, AppError> {
    respond(
        &config,
        "presets",
        |c| async move { content::presets(&c).await },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use turso_http::Error;

    #[test]
    fn database_outages_are_503_and_bugs_are_500() {
        for e in [
            Error::Config("x".into()),
            Error::Transport("offline".into()),
            Error::Http {
                status: 401,
                message: "Unauthorized".into(),
            },
        ] {
            assert_eq!(app_error(&ContentError::Db(e)), AppError::UNAVAILABLE);
        }
        for e in [
            ContentError::Db(Error::Sql {
                message: "no such table".into(),
                code: None,
            }),
            ContentError::Db(Error::Protocol("x".into())),
            ContentError::BadData("id"),
        ] {
            assert_eq!(app_error(&e), AppError::INTERNAL);
        }
    }

    #[test]
    fn client_messages_never_echo_internal_details() {
        let e = app_error(&ContentError::Db(Error::Sql {
            message: "no such table: radio_stations".into(),
            code: None,
        }));
        assert!(!e.message.contains("radio_stations"));
        assert_eq!(e.message, "Something went wrong.");
    }

    #[test]
    fn success_responses_are_cacheable_json_lists() {
        let res = ok(vec![1, 2]);
        assert_eq!(res.headers()[header::CACHE_CONTROL], CONTENT_CACHE);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
    }
}
