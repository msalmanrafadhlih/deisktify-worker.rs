//! Handler HTTP untuk endpoint baca konten (D3). Query dan validasinya ada di
//! `content.rs` (dites native); di sini hanya menyambungkan konfigurasi,
//! klien database, dan respons.

use std::future::Future;
use std::sync::Arc;

use api_types::{ChatMessage, Items, NewMessage};
use axum::{
    body::Bytes,
    extract::{rejection::BytesRejection, RawQuery, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

use crate::{
    chat::{self, ChatError},
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

// ------------------------------------------------------------- Live Chat (D4a)

/// Pesan error umum; detail hanya ke log Worker.
pub fn chat_error(e: &ChatError) -> Response {
    use turso_http::Error;
    match e {
        ChatError::RateLimited { retry_after_secs } => {
            let mut res = AppError::RATE_LIMITED.into_response();
            if let Ok(v) = HeaderValue::from_str(&retry_after_secs.to_string()) {
                res.headers_mut().insert(header::RETRY_AFTER, v);
            }
            res
        }
        ChatError::Db(Error::Config(_) | Error::Transport(_) | Error::Http { .. }) => {
            AppError::UNAVAILABLE.into_response()
        }
        ChatError::Db(_) | ChatError::BadData(_) => AppError::INTERNAL.into_response(),
    }
}

fn db_client(
    config: &Config,
    name: &'static str,
) -> Result<turso_http::Client<WorkerTransport>, AppError> {
    let Some(cfg) = &config.db else {
        return Err(AppError::UNAVAILABLE);
    };
    db::client(cfg).map_err(|e| {
        worker::console_error!("{name}: invalid database config: {e}");
        AppError::UNAVAILABLE
    })
}

/// `GET /api/v1/messages?after=<id>&limit=<n>`. Dipanggil berulang (polling),
/// jadi tidak pernah di-cache.
#[worker::send]
pub async fn messages_get(
    State(config): State<Arc<Config>>,
    RawQuery(query): RawQuery,
) -> Result<Response, AppError> {
    let q = chat::parse_list_query(query.as_deref())
        .ok_or(AppError::bad_request("Invalid query parameters."))?;
    let client = db_client(&config, "messages")?;
    match chat::list(&client, &q).await {
        Ok(items) => Ok(Json(Items { items }).into_response()),
        Err(e) => {
            worker::console_error!("messages: {e}");
            Ok(chat_error(&e))
        }
    }
}

/// `true` untuk `application/json` (boleh diikuti `; charset=...`).
fn is_json(content_type: Option<&str>) -> bool {
    content_type
        .map(|v| {
            v.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .is_some_and(|m| m == "application/json")
}

/// Browser yang menyebut `Origin` harus berasal dari daftar yang diizinkan.
/// Tanpa `Origin` (curl, server lain) diteruskan: pembatas kecepatan berlaku
/// sama untuk semua.
fn origin_ok(allowed: &[String], headers: &HeaderMap) -> bool {
    match headers.get(header::ORIGIN) {
        None => true,
        Some(v) => v
            .to_str()
            .map(|o| allowed.iter().any(|a| a == o))
            .unwrap_or(false),
    }
}

fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get(HeaderName::from_static("cf-connecting-ip"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().chars().take(64).collect::<String>())
        .filter(|s| !s.is_empty())
        // Saat `wrangler dev` header ini tidak ada: satu ember bersama.
        .unwrap_or_else(|| "local".to_owned())
}

/// `POST /api/v1/messages`. Urutan sengaja murah dulu, database terakhir:
/// origin, tipe konten, isi body, validasi, baru pembatas kecepatan dan simpan.
#[worker::send]
pub async fn messages_post(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, AppError> {
    if !origin_ok(&config.allowed_origins, &headers) {
        return Err(AppError::FORBIDDEN);
    }
    if !is_json(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
    ) {
        return Err(AppError::bad_request(
            "Content-Type must be application/json.",
        ));
    }
    let body = body.map_err(|e| {
        if e.into_response().status() == StatusCode::PAYLOAD_TOO_LARGE {
            AppError::PAYLOAD_TOO_LARGE
        } else {
            AppError::bad_request("Could not read the request body.")
        }
    })?;
    let new: NewMessage = serde_json::from_slice(&body)
        .map_err(|_| AppError::bad_request("The request body is not valid."))?;
    let clean = chat::prepare(&new, &config.banned_words)
        .map_err(|reject| AppError::bad_request(reject.message()))?;

    let client = db_client(&config, "messages")?;
    let now_secs = i64::try_from(worker::Date::now().as_millis() / 1000).unwrap_or(0);
    let ip = client_ip(&headers);
    let sender = chat::Sender {
        ip: &ip,
        salt: config.rate_salt.expose(),
        now_secs,
    };
    match chat::submit(&client, &sender, &clean).await {
        Ok(message) => Ok(created(message)),
        Err(e) => {
            if !matches!(e, ChatError::RateLimited { .. }) {
                worker::console_error!("messages: {e}");
            }
            Ok(chat_error(&e))
        }
    }
}

fn created(message: ChatMessage) -> Response {
    (StatusCode::CREATED, Json(message)).into_response()
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
