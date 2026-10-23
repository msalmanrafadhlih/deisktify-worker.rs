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
    pub const FORBIDDEN: Self = Self {
        status: StatusCode::FORBIDDEN,
        code: ErrorCode::Forbidden,
        message: "Request not allowed from this origin.",
    };
    pub const UNAUTHORIZED: Self = Self {
        status: StatusCode::UNAUTHORIZED,
        code: ErrorCode::Unauthorized,
        message: "Unauthorized.",
    };
    pub const PAYLOAD_TOO_LARGE: Self = Self {
        status: StatusCode::PAYLOAD_TOO_LARGE,
        code: ErrorCode::PayloadTooLarge,
        message: "Request body is too large.",
    };
    pub const RATE_LIMITED: Self = Self {
        status: StatusCode::TOO_MANY_REQUESTS,
        code: ErrorCode::RateLimited,
        message: "Too many requests, try again in a moment.",
    };
    pub const UNAVAILABLE: Self = Self {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: ErrorCode::Unavailable,
        message: "Service temporarily unavailable.",
    };
    /// Data jurnal belum pernah dibuat (cron belum jalan sekali pun).
    pub const NOT_READY: Self = Self {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: ErrorCode::Unavailable,
        message: "The journal has not been generated yet.",
    };
    /// Endpoint admin yang butuh pengaturan Worker yang belum diisi.
    pub const NOT_CONFIGURED: Self = Self {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: ErrorCode::Unavailable,
        message: "This feature is not configured.",
    };
    /// Layanan luar (GitHub) gagal.
    pub const UPSTREAM: Self = Self {
        status: StatusCode::BAD_GATEWAY,
        code: ErrorCode::Unavailable,
        message: "Could not reach GitHub.",
    };
    /// Admin: nilai itu sudah ada (nama repo atau stack kembar).
    pub const CONFLICT: Self = Self {
        status: StatusCode::CONFLICT,
        code: ErrorCode::BadRequest,
        message: "That already exists.",
    };
    pub const INTERNAL: Self = Self {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: ErrorCode::Internal,
        message: "Something went wrong.",
    };
}

impl AppError {
    /// 400 dengan pesan tetap (tidak pernah memuat input pengguna).
    pub const fn bad_request(message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: ErrorCode::BadRequest,
            message,
        }
    }
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
