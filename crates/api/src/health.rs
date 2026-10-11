//! `GET /api/v1/health`: layanan hidup + `SELECT 1` ke database.
//! Respons tidak pernah memuat pesan error internal.

use std::sync::Arc;

use api_types::{HealthResponse, ServiceStatus};
use axum::{extract::State, http::StatusCode, Json};
use turso_http::{Client, Error, Stmt, Transport};

use crate::{config::Config, db};

/// Menjalankan `SELECT 1`. Dipisah dari handler supaya bisa dites dengan
/// transport palsu.
pub async fn ping<T: Transport>(client: &Client<T>) -> Result<(), Error> {
    let result = client.execute(Stmt::new("SELECT 1")).await?;
    match result
        .rows
        .first()
        .and_then(|r| r.first())
        .and_then(|v| v.as_i64())
    {
        Some(1) => Ok(()),
        _ => Err(Error::Protocol("SELECT 1 did not return 1".into())),
    }
}

pub fn response(db_ok: bool) -> (StatusCode, HealthResponse) {
    let (code, status) = if db_ok {
        (StatusCode::OK, ServiceStatus::Ok)
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, ServiceStatus::Degraded)
    };
    (
        code,
        HealthResponse {
            status,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            db: db_ok,
        },
    )
}

// `#[worker::send]`: future di wasm32 tidak `Send`, sedangkan axum memintanya.
#[worker::send]
pub async fn handler(State(config): State<Arc<Config>>) -> (StatusCode, Json<HealthResponse>) {
    let db_ok = match &config.db {
        None => false,
        Some(cfg) => match db::client(cfg) {
            Err(e) => {
                worker::console_error!("health: invalid database config: {e}");
                false
            }
            Ok(client) => match ping(&client).await {
                Ok(()) => true,
                Err(e) => {
                    worker::console_error!("health: database ping failed: {e}");
                    false
                }
            },
        },
    };
    let (code, body) = response(db_ok);
    (code, Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_executor::block_on;
    use std::future::Future;
    use turso_http::HttpResponse;

    struct Canned(Result<HttpResponse, String>);

    impl Transport for Canned {
        fn post(
            &self,
            _: &str,
            _: Option<&str>,
            _: String,
        ) -> impl Future<Output = Result<HttpResponse, String>> {
            let r = self.0.clone();
            async move { r }
        }
    }

    fn client(reply: Result<HttpResponse, String>) -> Client<Canned> {
        Client::new("https://x.turso.io", Some("tok"), Canned(reply)).unwrap()
    }

    const ONE: &str = r#"{"results":[{"type":"ok","response":{"type":"execute","result":{
        "cols":[{"name":"1"}],"rows":[[{"type":"integer","value":"1"}]],"affected_row_count":0}}}]}"#;

    #[test]
    fn ping_succeeds_only_when_select_1_returns_1() {
        let ok = client(Ok(HttpResponse {
            status: 200,
            body: ONE.into(),
        }));
        assert!(block_on(ping(&ok)).is_ok());

        let wrong = client(Ok(HttpResponse {
            status: 200,
            body: ONE.replace(r#""value":"1""#, r#""value":"2""#),
        }));
        assert!(block_on(ping(&wrong)).is_err());

        let unauthorized = client(Ok(HttpResponse {
            status: 401,
            body: r#"{"message":"Unauthorized"}"#.into(),
        }));
        assert!(matches!(
            block_on(ping(&unauthorized)),
            Err(Error::Http { status: 401, .. })
        ));

        assert!(block_on(ping(&client(Err("offline".into())))).is_err());
    }

    #[test]
    fn response_maps_db_state_to_status_code() {
        let (code, body) = response(true);
        assert_eq!(
            (code, body.status, body.db),
            (StatusCode::OK, ServiceStatus::Ok, true)
        );
        let (code, body) = response(false);
        assert_eq!(
            (code, body.status, body.db),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                ServiceStatus::Degraded,
                false
            )
        );
        assert_eq!(body.version, env!("CARGO_PKG_VERSION"));
    }
}
