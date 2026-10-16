//! Error API: selalu JSON `{"error": {"code", "message"}}`, dengan pesan umum.
//! Detail internal hanya masuk log Worker, tidak pernah ke klien.

use api_types::{ApiErrorBody, ErrorCode};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppError {
    pub status: StatusCode,
    pub code: ErrorCode,
    pub message: &'static str,
}

impl AppError {
    pub const NOT_FOUND: Self = Self {
        status: StatusCode::NOT_FOUND,
        code: ErrorCode::NotFound,
        message: "Not found.",
    };
    pub const METHOD_NOT_ALLOWED: Self = Self {
        status: StatusCode::METHOD_NOT_ALLOWED,
        code: ErrorCode::MethodNotAllowed,
        message: "Method not allowed.",
    };
    pub const UNAVAILABLE: Self = Self {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: ErrorCode::Unavailable,
        message: "Service temporarily unavailable.",
    };
    pub const INTERNAL: Self = Self {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: ErrorCode::Internal,
        message: "Something went wrong.",
    };
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ApiErrorBody::new(self.code, self.message)),
        )
            .into_response()
    }
}

pub async fn not_found() -> AppError {
    AppError::NOT_FOUND
}

pub async fn method_not_allowed() -> AppError {
    AppError::METHOD_NOT_ALLOWED
}
