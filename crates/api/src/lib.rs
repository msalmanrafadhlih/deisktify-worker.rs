//! Worker Cloudflare untuk tquilla-api: Axum di atas `workers-rs`.
//!
//! - `config`: binding Worker -> pengaturan (bagian murni dites native)
//! - `cors`  : allowlist origin + header keamanan
//! - `error` : error JSON seragam
//! - `db`    : transport `fetch` untuk `turso-http`
//! - `health`: `GET /api/v1/health`
//! - `content`/`routes`: endpoint baca konten (D3): query + validasi, lalu handler
//! - `chat`  : Guestbook (D4): validasi, pembatas kecepatan, baca/tulis/laporkan/hapus
//! - `admin` : otorisasi admin lewat `ADMIN_TOKEN`
//! - `panel` : dashboard admin (D6a): pembatas percobaan gagal, riwayat aksi, ringkasan
//! - `journal`/`github`: GitHub Journal (D5): Cron Trigger mengambil statistik dari
//!   GitHub, `GET /journal` membaca snapshot terakhir
//! - `radio_check`/`radio_probe`: pemeriksa stasiun radio (D7): Cron Trigger mengetuk
//!   stream, mencatat hasilnya, dan menandai stasiun yang basi
//! - `time`  : waktu UTC tanpa dependensi tanggal
//!
//! Kode di crate ini harus compile ke wasm32 (tanpa tokio, tanpa thread).

pub mod admin;
pub mod chat;
pub mod config;
pub mod content;
pub mod cors;
pub mod db;
pub mod error;
pub mod github;
pub mod health;
pub mod journal;
pub mod journal_admin;
pub mod moderation;
pub mod panel;
pub mod radio_check;
pub mod radio_probe;
pub mod routes;
pub mod tables;
pub mod time;

#[cfg(test)]
mod testing;

use std::sync::Arc;

use axum::{
    extract::DefaultBodyLimit,
    middleware,
    routing::{delete, get, post, put},
    Router,
};
use tower_service::Service;
use worker::{event, Context, Env, HttpRequest};

use crate::config::Config;

/// Router lengkap. Dipisah dari `fetch` supaya bisa dites tanpa runtime Workers.
pub fn app(config: Arc<Config>) -> Router {
    let origins = Arc::new(config.allowed_origins.clone());
    Router::new()
        .route(api_types::paths::HEALTH, get(health::handler))
        .route(api_types::paths::GENERATIONS, get(routes::generations))
        .route(api_types::paths::BOOKMARKS, get(routes::bookmarks))
        .route(api_types::paths::RADIO, get(routes::radio))
        .route(api_types::paths::EMBIENCES, get(routes::embiences))
        .route(api_types::paths::PRESETS, get(routes::presets))
        .route(
            api_types::paths::MESSAGES,
            get(routes::messages_get).post(routes::messages_post),
        )
        .route(
            api_types::paths::MESSAGE_REPORT,
            post(routes::message_report),
        )
        .route(api_types::paths::ADMIN_REPORTS, get(routes::admin_reports))
        .route(
            api_types::paths::ADMIN_MESSAGE,
            delete(routes::admin_delete),
        )
        .route(api_types::paths::JOURNAL, get(routes::journal_get))
        .route(
            api_types::paths::ADMIN_JOURNAL_REFRESH,
            post(routes::admin_journal_refresh),
        )
        .route(api_types::paths::ADMIN_LOGIN, post(routes::admin_login))
        .route(
            api_types::paths::ADMIN_OVERVIEW,
            get(routes::admin_overview),
        )
        .route(api_types::paths::ADMIN_AUDIT, get(routes::admin_audit))
        .route(api_types::paths::ADMIN_JOURNAL, get(routes::admin_journal))
        .route(
            api_types::paths::ADMIN_JOURNAL_PROFILE,
            put(routes::admin_journal_profile),
        )
        .route(
            api_types::paths::ADMIN_JOURNAL_REPOS,
            post(routes::admin_journal_repo_add),
        )
        .route(
            api_types::paths::ADMIN_JOURNAL_REPOS_ORDER,
            put(routes::admin_journal_repos_order),
        )
        .route(
            api_types::paths::ADMIN_JOURNAL_REPO,
            put(routes::admin_journal_repo_edit).delete(routes::admin_journal_repo_delete),
        )
        .route(
            api_types::paths::ADMIN_JOURNAL_STACK,
            post(routes::admin_journal_stack_add),
        )
        .route(
            api_types::paths::ADMIN_JOURNAL_STACK_ORDER,
            put(routes::admin_journal_stack_order),
        )
        .route(
            api_types::paths::ADMIN_JOURNAL_STACK_ITEM,
            put(routes::admin_journal_stack_edit).delete(routes::admin_journal_stack_delete),
        )
        .route(
            api_types::paths::ADMIN_MESSAGES,
            get(routes::admin_messages),
        )
        .route(
            api_types::paths::ADMIN_MESSAGE_RESTORE,
            post(routes::admin_message_restore),
        )
        .route(
            api_types::paths::ADMIN_MESSAGE_REPORTS,
            delete(routes::admin_message_dismiss),
        )
        .route(api_types::paths::ADMIN_TABLES, get(routes::admin_tables))
        .route(
            api_types::paths::ADMIN_TABLE_ROWS,
            get(routes::admin_table_rows).post(routes::admin_table_insert),
        )
        .route(
            api_types::paths::ADMIN_TABLE_ROW,
            put(routes::admin_table_update).delete(routes::admin_table_delete),
        )
        .route(
            api_types::paths::ADMIN_TABLE_EXPORT,
            get(routes::admin_table_export),
        )
        .route(
            api_types::paths::ADMIN_PRESET_ITEMS,
            get(routes::admin_preset_items),
        )
        .route(
            api_types::paths::ADMIN_PRESET_ITEMS_SET,
            put(routes::admin_preset_items_set),
        )
        .route(
            api_types::paths::ADMIN_MESSAGE_PURGE,
            post(routes::admin_message_purge),
        )
        .route(api_types::paths::ADMIN_RADIO, get(routes::admin_radio))
        .route(
            api_types::paths::ADMIN_RADIO_CHECK,
            post(routes::admin_radio_check),
        )
        .fallback(error::not_found)
        .method_not_allowed_fallback(error::method_not_allowed)
        .with_state(config)
        .layer(DefaultBodyLimit::max(api_types::limits::MAX_BODY_BYTES))
        // Terluar: CORS dan header keamanan berlaku juga untuk 404/405.
        .layer(middleware::from_fn_with_state(origins, cors::middleware))
}

#[event(fetch)]
async fn fetch(
    req: HttpRequest,
    env: Env,
    _ctx: Context,
) -> worker::Result<axum::http::Response<axum::body::Body>> {
    console_error_panic_hook::set_once();
    let config = Arc::new(Config::from_env(&env));
    Ok(app(config).call(req).await?)
}

/// Cron Trigger (`[triggers] crons` di wrangler.toml). Dua pekerjaan, dibedakan
/// dari ekspresi cron-nya; masing-masing eksekusi punya anggaran subrequest
/// dan CPU sendiri:
///
/// - `radio_check::CRON`: ketuk stasiun radio aktif dan catat hasilnya (D7).
/// - lainnya (`0 */6 * * *`): ambil statistik GitHub dan simpan sebagai
///   snapshot jurnal. Gagal = snapshot lama tetap dipakai.
///
/// Hasilnya hanya masuk log Worker (tanpa URL: URL stream memuat token).
#[event(scheduled)]
async fn scheduled(event: worker::ScheduledEvent, env: Env, _ctx: worker::ScheduleContext) {
    console_error_panic_hook::set_once();
    let config = Config::from_env(&env);
    let now_secs = i64::try_from(worker::Date::now().as_millis() / 1000).unwrap_or(0);
    if event.cron() == radio_check::CRON {
        match radio_check::run(&config, now_secs).await {
            Ok(report) => worker::console_log!("radio: {report}"),
            Err(e) => worker::console_error!("radio: check failed: {e}"),
        }
        return;
    }
    match journal::run(&config, now_secs).await {
        Ok(done) => worker::console_log!("journal: refreshed at {}", done.fetched_at),
        Err(e) => worker::console_error!("journal: refresh failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::{header, Method, Request, StatusCode},
    };
    use tower::ServiceExt;

    fn config() -> Arc<Config> {
        // Tanpa database: /health tidak menyentuh `fetch` JS (tidak ada di host).
        Arc::new(Config::from_values(
            Some("https://tquilla.is-a.dev,http://localhost:8080"),
            None,
            None,
        ))
    }

    async fn call(req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, serde_json::Value) {
        let res = app(config()).oneshot(req).await.unwrap();
        let (parts, body) = res.into_parts();
        let bytes = to_bytes(body, 64 * 1024).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (parts.status, parts.headers, json)
    }

    fn req(method: Method, uri: &str) -> axum::http::request::Builder {
        Request::builder().method(method).uri(uri)
    }

    #[test]
    fn health_without_database_is_degraded_503_and_never_leaks_details() {
        futures_executor::block_on(async {
            let (status, headers, json) = call(
                req(Method::GET, "/api/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(json["status"], "degraded");
            assert_eq!(json["db"], false);
            assert_eq!(
                json.as_object().unwrap().len(),
                3,
                "only status, version, db"
            );
            assert_eq!(headers[header::CACHE_CONTROL], "no-store");
            assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        });
    }

    #[test]
    fn every_content_endpoint_is_503_json_without_a_database_and_never_cached() {
        futures_executor::block_on(async {
            for path in [
                api_types::paths::GENERATIONS,
                api_types::paths::BOOKMARKS,
                api_types::paths::RADIO,
                api_types::paths::EMBIENCES,
                api_types::paths::PRESETS,
                api_types::paths::JOURNAL,
            ] {
                let (status, headers, json) =
                    call(req(Method::GET, path).body(Body::empty()).unwrap()).await;
                assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{path}");
                assert_eq!(json["error"]["code"], "unavailable", "{path}");
                assert_eq!(headers[header::CACHE_CONTROL], "no-store", "{path}");

                let (status, _, json) =
                    call(req(Method::POST, path).body(Body::empty()).unwrap()).await;
                assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{path}");
                assert_eq!(json["error"]["code"], "method_not_allowed");
            }
        });
    }

    #[test]
    fn all_declared_paths_are_routed() {
        futures_executor::block_on(async {
            for path in api_types::paths::ALL {
                let (status, _, _) =
                    call(req(Method::GET, path).body(Body::empty()).unwrap()).await;
                assert_ne!(status, StatusCode::NOT_FOUND, "{path} has no route");
            }
        });
    }

    #[test]
    fn unknown_path_is_a_json_404() {
        futures_executor::block_on(async {
            let (status, _, json) =
                call(req(Method::GET, "/nope").body(Body::empty()).unwrap()).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(json["error"]["code"], "not_found");
        });
    }

    #[test]
    fn wrong_method_is_a_json_405() {
        futures_executor::block_on(async {
            let (status, _, json) = call(
                req(Method::DELETE, "/api/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
            assert_eq!(json["error"]["code"], "method_not_allowed");
        });
    }

    #[test]
    fn cors_headers_appear_for_the_site_and_also_on_errors() {
        futures_executor::block_on(async {
            let (_, headers, _) = call(
                req(Method::GET, "/nope")
                    .header(header::ORIGIN, "https://tquilla.is-a.dev")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(
                headers[header::ACCESS_CONTROL_ALLOW_ORIGIN],
                "https://tquilla.is-a.dev"
            );
            assert_eq!(headers[header::VARY], "Origin");

            let (_, headers, _) = call(
                req(Method::GET, "/api/v1/health")
                    .header(header::ORIGIN, "https://evil.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert!(!headers.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
        });
    }

    #[test]
    fn preflight_is_answered_without_reaching_the_routes() {
        futures_executor::block_on(async {
            let (status, headers, _) = call(
                req(Method::OPTIONS, "/api/v1/anything")
                    .header(header::ORIGIN, "http://localhost:8080")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            assert_eq!(
                headers[header::ACCESS_CONTROL_ALLOW_ORIGIN],
                "http://localhost:8080"
            );
            assert_eq!(
                headers[header::ACCESS_CONTROL_ALLOW_METHODS],
                "GET, POST, OPTIONS"
            );

            let (status, headers, _) = call(
                req(Method::OPTIONS, "/api/v1/anything")
                    .header(header::ORIGIN, "https://evil.example")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            assert!(!headers.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
        });
    }

    // ---------------------------------------------------------- Guestbook (D4)

    async fn call_with(
        cfg: Arc<Config>,
        req: Request<Body>,
    ) -> (StatusCode, axum::http::HeaderMap, serde_json::Value) {
        let res = app(cfg).oneshot(req).await.unwrap();
        let (parts, body) = res.into_parts();
        let bytes = to_bytes(body, 64 * 1024).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (parts.status, parts.headers, json)
    }

    fn post(origin: Option<&str>, content_type: &str, body: impl Into<Body>) -> Request<Body> {
        let mut b =
            req(Method::POST, "/api/v1/messages").header(header::CONTENT_TYPE, content_type);
        if let Some(o) = origin {
            b = b.header(header::ORIGIN, o);
        }
        b.body(body.into()).unwrap()
    }

    const SITE: &str = "https://tquilla.is-a.dev";
    const GOOD: &str = r#"{"username":"moch","url":"a.dev","message":"hello"}"#;

    #[test]
    fn posting_from_a_foreign_origin_is_forbidden_before_anything_else() {
        futures_executor::block_on(async {
            let (status, _, json) =
                call(post(Some("https://evil.example"), "application/json", GOOD)).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(json["error"]["code"], "forbidden");
        });
    }

    #[test]
    fn only_json_bodies_are_accepted_which_also_forces_a_cors_preflight() {
        futures_executor::block_on(async {
            for ct in [
                "text/plain",
                "application/x-www-form-urlencoded",
                "multipart/form-data",
            ] {
                let (status, _, json) = call(post(Some(SITE), ct, GOOD)).await;
                assert_eq!(status, StatusCode::BAD_REQUEST, "{ct}");
                assert_eq!(json["error"]["code"], "bad_request");
            }
            let (status, _, _) =
                call(post(Some(SITE), "Application/JSON; charset=utf-8", GOOD)).await;
            assert_eq!(
                status,
                StatusCode::SERVICE_UNAVAILABLE,
                "valid body, no database"
            );
        });
    }

    #[test]
    fn malformed_or_invalid_messages_are_400_with_a_safe_message() {
        futures_executor::block_on(async {
            for body in [
                "",
                "not json",
                "[]",
                r#"{"username":"x"}"#,
                r#"{"username":1,"message":"x"}"#,
            ] {
                let (status, _, json) = call(post(Some(SITE), "application/json", body)).await;
                assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
                assert_eq!(json["error"]["code"], "bad_request");
            }
            for (body, needle) in [
                (r#"{"username":" ","message":"x"}"#, "Username"),
                (r#"{"username":"a","message":"  "}"#, "Message"),
                (
                    r#"{"username":"a","message":"x","url":"javascript:alert(1)"}"#,
                    "link",
                ),
                (r#"{"username":"a\u0000b","message":"x"}"#, "Username"),
            ] {
                let (status, _, json) = call(post(Some(SITE), "application/json", body)).await;
                assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
                let msg = json["error"]["message"].as_str().unwrap();
                assert!(msg.contains(needle), "{msg}");
                assert!(!msg.contains("alert"), "input must not be echoed");
            }
        });
    }

    #[test]
    fn a_valid_message_without_a_database_is_503_and_a_missing_origin_is_allowed() {
        futures_executor::block_on(async {
            for origin in [Some(SITE), None] {
                let (status, headers, json) = call(post(origin, "application/json", GOOD)).await;
                assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
                assert_eq!(json["error"]["code"], "unavailable");
                assert_eq!(headers[header::CACHE_CONTROL], "no-store");
            }
        });
    }

    #[test]
    fn oversized_bodies_are_a_json_413() {
        futures_executor::block_on(async {
            let big = format!(r#"{{"username":"a","message":"{}"}}"#, "x".repeat(20_000));
            let (status, _, json) = call(post(Some(SITE), "application/json", big)).await;
            assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
            assert_eq!(json["error"]["code"], "payload_too_large");
        });
    }

    #[test]
    fn banned_words_from_the_configuration_reject_the_message() {
        futures_executor::block_on(async {
            let cfg = Arc::new(
                Config::from_values(Some(SITE), None, None).with_chat(None, Some("Forbidden")),
            );
            let body = r#"{"username":"a","message":"this is f.o.r.b.i.d.d.e.n"}"#;
            let (status, _, json) =
                call_with(cfg, post(Some(SITE), "application/json", body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(json["error"]["message"], "This message was rejected.");
        });
    }

    #[test]
    fn listing_messages_validates_the_query_and_needs_the_database() {
        futures_executor::block_on(async {
            for bad in [
                "limit=0",
                "limit=101",
                "after=x",
                "after=-1",
                "before=x",
                "after=1&before=2",
            ] {
                let uri = format!("/api/v1/messages?{bad}");
                let (status, _, json) =
                    call(req(Method::GET, &uri).body(Body::empty()).unwrap()).await;
                assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
                assert_eq!(json["error"]["code"], "bad_request");
            }
            let (status, headers, _) = call(
                req(Method::GET, "/api/v1/messages?after=3&limit=10")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        });
    }

    #[test]
    fn messages_only_allow_get_and_post() {
        futures_executor::block_on(async {
            for method in [Method::PUT, Method::DELETE, Method::PATCH] {
                let (status, _, json) =
                    call(req(method, "/api/v1/messages").body(Body::empty()).unwrap()).await;
                assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
                assert_eq!(json["error"]["code"], "method_not_allowed");
            }
        });
    }
    // ------------------------------------------------- moderasi (D4b)

    const ADMIN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

    fn with_admin() -> Arc<Config> {
        Arc::new(Config::from_values(Some(SITE), None, None).with_admin(Some(ADMIN)))
    }

    fn report_req(origin: Option<&str>, content_type: Option<&str>, id: &str) -> Request<Body> {
        let mut b = req(Method::POST, &format!("/api/v1/messages/{id}/report"));
        if let Some(o) = origin {
            b = b.header(header::ORIGIN, o);
        }
        if let Some(c) = content_type {
            b = b.header(header::CONTENT_TYPE, c);
        }
        b.body(Body::from("{}")).unwrap()
    }

    fn admin_req(method: Method, path: &str, token: Option<&str>) -> Request<Body> {
        let mut b = req(method, path);
        if let Some(t) = token {
            b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        b.body(Body::empty()).unwrap()
    }

    #[test]
    fn reporting_follows_the_same_origin_and_json_rules_as_writing() {
        futures_executor::block_on(async {
            let (status, _, json) = call(report_req(
                Some("https://evil.example"),
                Some("application/json"),
                "3",
            ))
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(json["error"]["code"], "forbidden");

            for ct in [None, Some("text/plain"), Some("multipart/form-data")] {
                let (status, _, _) = call(report_req(Some(SITE), ct, "3")).await;
                assert_eq!(status, StatusCode::BAD_REQUEST, "{ct:?}");
            }
        });
    }

    #[test]
    fn a_report_needs_a_plain_positive_id_and_then_the_database() {
        futures_executor::block_on(async {
            for bad in ["0", "x", "-1", "1.5", "99999999999"] {
                let (status, _, json) =
                    call(report_req(Some(SITE), Some("application/json"), bad)).await;
                assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
                assert_eq!(json["error"]["code"], "bad_request");
            }
            for origin in [Some(SITE), None] {
                let (status, headers, json) =
                    call(report_req(origin, Some("application/json"), "3")).await;
                assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "no database");
                assert_eq!(json["error"]["code"], "unavailable");
                assert_eq!(headers[header::CACHE_CONTROL], "no-store");
            }
        });
    }

    #[test]
    fn report_only_allows_post() {
        futures_executor::block_on(async {
            for method in [Method::GET, Method::PUT, Method::DELETE] {
                let (status, _, _) = call(
                    req(method, "/api/v1/messages/3/report")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
                assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
            }
        });
    }

    #[test]
    fn admin_endpoints_are_closed_without_the_right_token() {
        futures_executor::block_on(async {
            for cfg in [config(), with_admin()] {
                for token in [None, Some("wrong"), Some("")] {
                    let (status, _, json) = call_with(
                        cfg.clone(),
                        admin_req(Method::GET, "/api/v1/admin/reports", token),
                    )
                    .await;
                    assert_eq!(status, StatusCode::UNAUTHORIZED, "{token:?}");
                    assert_eq!(json["error"]["code"], "unauthorized");
                    let (status, _, _) = call_with(
                        cfg.clone(),
                        admin_req(Method::DELETE, "/api/v1/admin/messages/1", token),
                    )
                    .await;
                    assert_eq!(status, StatusCode::UNAUTHORIZED, "{token:?}");
                }
            }
            // Admin tidak dikonfigurasi: bahkan "token" yang sama dengan nilai
            // kosong tidak membuka apa pun.
            let (status, _, _) =
                call(admin_req(Method::GET, "/api/v1/admin/reports", Some(ADMIN))).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        });
    }

    #[test]
    fn a_valid_admin_token_passes_the_gate_and_then_needs_the_database() {
        futures_executor::block_on(async {
            for (method, path) in [
                (Method::GET, "/api/v1/admin/reports"),
                (Method::DELETE, "/api/v1/admin/messages/7"),
                // Gerbang (token + pembatas) jalan lebih dulu daripada validasi id.
                (Method::DELETE, "/api/v1/admin/messages/abc"),
            ] {
                let (status, _, json) =
                    call_with(with_admin(), admin_req(method, path, Some(ADMIN))).await;
                assert_eq!(
                    status,
                    StatusCode::SERVICE_UNAVAILABLE,
                    "{path}: no database"
                );
                assert_eq!(json["error"]["code"], "unavailable");
            }
        });
    }

    #[test]
    fn the_token_is_checked_before_the_id_so_ids_cannot_be_probed() {
        futures_executor::block_on(async {
            let (status, _, _) = call_with(
                with_admin(),
                admin_req(Method::DELETE, "/api/v1/admin/messages/abc", None),
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        });
    }

    #[test]
    fn browsers_cannot_be_granted_admin_access_through_cors() {
        futures_executor::block_on(async {
            let (status, headers, _) = call(
                req(Method::OPTIONS, "/api/v1/admin/messages/1")
                    .header(header::ORIGIN, SITE)
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "DELETE")
                    .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            let methods = headers[header::ACCESS_CONTROL_ALLOW_METHODS]
                .to_str()
                .unwrap();
            let allowed_headers = headers[header::ACCESS_CONTROL_ALLOW_HEADERS]
                .to_str()
                .unwrap();
            assert!(!methods.contains("DELETE"), "{methods}");
            assert!(
                !allowed_headers.contains("authorization"),
                "{allowed_headers}"
            );
        });
    }
    // ----------------------------------------------------------- jurnal (D5)

    fn with_admin_and_github() -> Arc<Config> {
        Arc::new(
            Config::from_values(Some(SITE), None, None)
                .with_admin(Some(ADMIN))
                .with_github(Some("ghp_test")),
        )
    }

    #[test]
    fn the_journal_is_read_only_and_public() {
        futures_executor::block_on(async {
            for method in [Method::POST, Method::PUT, Method::DELETE] {
                let (status, _, json) = call(
                    req(method, api_types::paths::JOURNAL)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
                assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
                assert_eq!(json["error"]["code"], "method_not_allowed");
            }
        });
    }

    #[test]
    fn manual_journal_refresh_is_admin_only_and_post_only() {
        futures_executor::block_on(async {
            let path = api_types::paths::ADMIN_JOURNAL_REFRESH;
            for cfg in [config(), with_admin(), with_admin_and_github()] {
                for token in [None, Some("wrong"), Some("")] {
                    let (status, _, json) =
                        call_with(cfg.clone(), admin_req(Method::POST, path, token)).await;
                    assert_eq!(status, StatusCode::UNAUTHORIZED, "{token:?}");
                    assert_eq!(json["error"]["code"], "unauthorized");
                }
            }
            for method in [Method::GET, Method::PUT, Method::DELETE] {
                let (status, _, _) =
                    call_with(with_admin(), admin_req(method, path, Some(ADMIN))).await;
                assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
            }
        });
    }

    #[test]
    fn manual_refresh_with_a_valid_token_needs_the_database_and_never_echoes_secrets() {
        futures_executor::block_on(async {
            let path = api_types::paths::ADMIN_JOURNAL_REFRESH;
            for cfg in [with_admin(), with_admin_and_github()] {
                let (status, _, json) =
                    call_with(cfg, admin_req(Method::POST, path, Some(ADMIN))).await;
                assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
                assert_eq!(json["error"]["code"], "unavailable");
                let text = json.to_string();
                assert!(!text.contains("ghp_test") && !text.contains(ADMIN));
            }
        });
    }

    #[test]
    fn browsers_cannot_be_granted_journal_refresh_through_cors() {
        futures_executor::block_on(async {
            let (status, headers, _) = call(
                req(Method::OPTIONS, api_types::paths::ADMIN_JOURNAL_REFRESH)
                    .header(header::ORIGIN, SITE)
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                    .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            let allowed_headers = headers[header::ACCESS_CONTROL_ALLOW_HEADERS]
                .to_str()
                .unwrap();
            assert!(
                !allowed_headers.contains("authorization"),
                "{allowed_headers}"
            );
        });
    }

    // ------------------------------------------------- dashboard admin (D6a)

    const PANEL_PATHS: [(Method, &str); 3] = [
        (Method::POST, api_types::paths::ADMIN_LOGIN),
        (Method::GET, api_types::paths::ADMIN_OVERVIEW),
        (Method::GET, api_types::paths::ADMIN_AUDIT),
    ];

    #[test]
    fn dashboard_endpoints_are_closed_without_the_right_token() {
        futures_executor::block_on(async {
            for cfg in [config(), with_admin()] {
                for (method, path) in PANEL_PATHS {
                    for token in [None, Some("wrong"), Some("")] {
                        let (status, _, json) =
                            call_with(cfg.clone(), admin_req(method.clone(), path, token)).await;
                        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path} {token:?}");
                        assert_eq!(json["error"]["code"], "unauthorized");
                    }
                }
            }
            // Admin mati: token yang sama dengan nilai kosong pun tidak membuka apa pun.
            for (method, path) in PANEL_PATHS {
                let (status, _, _) = call(admin_req(method, path, Some(ADMIN))).await;
                assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
            }
        });
    }

    #[test]
    fn dashboard_endpoints_with_a_valid_token_need_the_database() {
        futures_executor::block_on(async {
            for (method, path) in PANEL_PATHS {
                let (status, headers, json) =
                    call_with(with_admin(), admin_req(method, path, Some(ADMIN))).await;
                assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{path}");
                assert_eq!(json["error"]["code"], "unavailable");
                assert_eq!(headers[header::CACHE_CONTROL], "no-store");
            }
        });
    }

    #[test]
    fn dashboard_endpoints_only_allow_their_own_method() {
        futures_executor::block_on(async {
            for (allowed, path) in PANEL_PATHS {
                for method in [Method::GET, Method::POST, Method::PUT, Method::DELETE] {
                    if method == allowed {
                        continue;
                    }
                    let (status, _, _) =
                        call_with(with_admin(), admin_req(method.clone(), path, Some(ADMIN))).await;
                    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{method} {path}");
                }
            }
        });
    }

    #[test]
    fn browsers_cannot_be_granted_dashboard_endpoints_through_cors() {
        futures_executor::block_on(async {
            for (method, path) in PANEL_PATHS {
                let (status, headers, _) = call(
                    req(Method::OPTIONS, path)
                        .header(header::ORIGIN, SITE)
                        .header(header::ACCESS_CONTROL_REQUEST_METHOD, method.as_str())
                        .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
                assert_eq!(status, StatusCode::NO_CONTENT);
                let allowed = headers[header::ACCESS_CONTROL_ALLOW_HEADERS]
                    .to_str()
                    .unwrap();
                assert!(!allowed.contains("authorization"), "{path}: {allowed}");
            }
        });
    }
    // ------------------------------------------ journal dan moderasi (D6b)

    /// Path (dengan `{id}` diisi 1) dan metode yang diizinkan.
    fn editor_paths() -> Vec<(String, Vec<Method>)> {
        use api_types::paths as p;
        let fill = |path: &str| path.replace("{id}", "1").replace("{table}", "bookmarks");
        vec![
            (fill(p::ADMIN_JOURNAL), vec![Method::GET]),
            (fill(p::ADMIN_JOURNAL_PROFILE), vec![Method::PUT]),
            (fill(p::ADMIN_JOURNAL_REPOS), vec![Method::POST]),
            (fill(p::ADMIN_JOURNAL_REPOS_ORDER), vec![Method::PUT]),
            (
                fill(p::ADMIN_JOURNAL_REPO),
                vec![Method::PUT, Method::DELETE],
            ),
            (fill(p::ADMIN_JOURNAL_STACK), vec![Method::POST]),
            (fill(p::ADMIN_JOURNAL_STACK_ORDER), vec![Method::PUT]),
            (
                fill(p::ADMIN_JOURNAL_STACK_ITEM),
                vec![Method::PUT, Method::DELETE],
            ),
            (fill(p::ADMIN_MESSAGES), vec![Method::GET]),
            (fill(p::ADMIN_MESSAGE_RESTORE), vec![Method::POST]),
            (fill(p::ADMIN_MESSAGE_REPORTS), vec![Method::DELETE]),
            (fill(p::ADMIN_TABLES), vec![Method::GET]),
            (fill(p::ADMIN_TABLE_ROWS), vec![Method::GET, Method::POST]),
            (fill(p::ADMIN_TABLE_ROW), vec![Method::PUT, Method::DELETE]),
            (fill(p::ADMIN_TABLE_EXPORT), vec![Method::GET]),
            (fill(p::ADMIN_PRESET_ITEMS), vec![Method::GET]),
            (fill(p::ADMIN_PRESET_ITEMS_SET), vec![Method::PUT]),
            (fill(p::ADMIN_MESSAGE_PURGE), vec![Method::POST]),
            (fill(p::ADMIN_RADIO), vec![Method::GET]),
            (fill(p::ADMIN_RADIO_CHECK), vec![Method::POST]),
        ]
    }

    #[test]
    fn editor_endpoints_are_closed_without_the_right_token() {
        futures_executor::block_on(async {
            for cfg in [config(), with_admin()] {
                for (path, methods) in editor_paths() {
                    for method in &methods {
                        for token in [None, Some("wrong"), Some("")] {
                            let (status, _, json) =
                                call_with(cfg.clone(), admin_req(method.clone(), &path, token))
                                    .await;
                            assert_eq!(
                                status,
                                StatusCode::UNAUTHORIZED,
                                "{method} {path} {token:?}"
                            );
                            assert_eq!(json["error"]["code"], "unauthorized");
                        }
                    }
                }
            }
            for (path, methods) in editor_paths() {
                for method in methods {
                    let (status, _, _) = call(admin_req(method.clone(), &path, Some(ADMIN))).await;
                    assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
                }
            }
        });
    }

    #[test]
    fn editor_endpoints_with_a_valid_token_need_the_database() {
        futures_executor::block_on(async {
            for (path, methods) in editor_paths() {
                for method in methods {
                    let (status, headers, json) =
                        call_with(with_admin(), admin_req(method.clone(), &path, Some(ADMIN)))
                            .await;
                    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{method} {path}");
                    assert_eq!(json["error"]["code"], "unavailable");
                    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
                }
            }
        });
    }

    #[test]
    fn editor_endpoints_only_allow_their_own_methods() {
        futures_executor::block_on(async {
            for (path, allowed) in editor_paths() {
                for method in [Method::GET, Method::POST, Method::PUT, Method::DELETE] {
                    if allowed.contains(&method) {
                        continue;
                    }
                    let (status, _, _) =
                        call_with(with_admin(), admin_req(method.clone(), &path, Some(ADMIN)))
                            .await;
                    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{method} {path}");
                }
            }
        });
    }

    #[test]
    fn the_order_routes_are_not_swallowed_by_the_id_routes() {
        futures_executor::block_on(async {
            // "order" harus sampai ke handler urutan (503 tanpa database), bukan
            // ditolak sebagai id tidak valid (400) oleh rute `{id}`.
            for path in [
                api_types::paths::ADMIN_JOURNAL_REPOS_ORDER,
                api_types::paths::ADMIN_JOURNAL_STACK_ORDER,
            ] {
                let (status, _, _) =
                    call_with(with_admin(), admin_req(Method::PUT, path, Some(ADMIN))).await;
                assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{path}");
            }
        });
    }

    #[test]
    fn browsers_cannot_be_granted_editor_endpoints_through_cors() {
        futures_executor::block_on(async {
            for (path, methods) in editor_paths() {
                for method in methods {
                    let (status, headers, _) = call(
                        req(Method::OPTIONS, &path)
                            .header(header::ORIGIN, SITE)
                            .header(header::ACCESS_CONTROL_REQUEST_METHOD, method.as_str())
                            .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await;
                    assert_eq!(status, StatusCode::NO_CONTENT);
                    let allowed = headers[header::ACCESS_CONTROL_ALLOW_HEADERS]
                        .to_str()
                        .unwrap();
                    assert!(!allowed.contains("authorization"), "{path}: {allowed}");
                    let methods = headers[header::ACCESS_CONTROL_ALLOW_METHODS]
                        .to_str()
                        .unwrap();
                    assert!(!methods.contains("PUT"), "{path}: {methods}");
                    assert!(!methods.contains("DELETE"), "{path}: {methods}");
                }
            }
        });
    }
}
