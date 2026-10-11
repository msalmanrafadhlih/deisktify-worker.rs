//! Worker Cloudflare untuk tquilla-api: Axum di atas `workers-rs`.
//!
//! - `config`: binding Worker -> pengaturan (bagian murni dites native)
//! - `cors`  : allowlist origin + header keamanan
//! - `error` : error JSON seragam
//! - `db`    : transport `fetch` untuk `turso-http`
//! - `health`: `GET /api/v1/health`
//!
//! Kode di crate ini harus compile ke wasm32 (tanpa tokio, tanpa thread).

pub mod config;
pub mod cors;
pub mod db;
pub mod error;
pub mod health;

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
}
