//! Worker Cloudflare untuk tquilla-api: Axum di atas `workers-rs`.
//!
//! - `config`: binding Worker -> pengaturan (bagian murni dites native)
//! - `cors`  : allowlist origin + header keamanan
//! - `error` : error JSON seragam
//! - `db`    : transport `fetch` untuk `turso-http`
//! - `health`: `GET /api/v1/health`
//! - `content`/`routes`: endpoint baca konten (D3): query + validasi, lalu handler
//! - `chat`  : Live Chat (D4a): validasi, pembatas kecepatan, baca/kirim pesan
//!
//! Kode di crate ini harus compile ke wasm32 (tanpa tokio, tanpa thread).

pub mod chat;
pub mod config;
pub mod content;
pub mod cors;
pub mod db;
pub mod error;
pub mod health;
pub mod routes;

use std::sync::Arc;

use axum::{extract::DefaultBodyLimit, middleware, routing::get, Router};
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

    // ---------------------------------------------------------- Live Chat (D4a)

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
            for bad in ["limit=0", "limit=101", "after=x", "after=-1"] {
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
}
