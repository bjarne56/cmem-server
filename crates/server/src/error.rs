//! HTTP 层统一错误。
//!
//! 所有 handler 返回 `Result<T, AppError>`,中间件转 JSON 响应。

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use cmem_shared::ApiErrorBody;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("validation failed: {0}")]
    Validation(String),

    #[error("invalid credentials")]
    InvalidCredentials,

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden")]
    Forbidden,

    #[error("not found")]
    NotFound,

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("rate limited")]
    RateLimited,

    /// 内部错误:不向用户暴露细节。
    #[error("internal: {0}")]
    Internal(#[from] anyhow::Error),

    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
}

impl AppError {
    fn code_status(&self) -> (&'static str, StatusCode) {
        match self {
            Self::Validation(_) => ("VALIDATION_FAILED", StatusCode::BAD_REQUEST),
            Self::InvalidCredentials => ("INVALID_CREDENTIALS", StatusCode::UNAUTHORIZED),
            Self::Unauthorized => ("UNAUTHORIZED", StatusCode::UNAUTHORIZED),
            Self::Forbidden => ("FORBIDDEN", StatusCode::FORBIDDEN),
            Self::NotFound => ("NOT_FOUND", StatusCode::NOT_FOUND),
            Self::Conflict(_) => ("CONFLICT", StatusCode::CONFLICT),
            Self::RateLimited => ("RATE_LIMITED", StatusCode::TOO_MANY_REQUESTS),
            Self::Internal(_) | Self::Db(_) => ("INTERNAL", StatusCode::INTERNAL_SERVER_ERROR),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (code, status) = self.code_status();
        // 内部错误打 log,但不把细节回给客户端
        let public_message = match &self {
            AppError::Internal(e) => {
                tracing::error!(error = %e, "internal error");
                "internal server error".to_string()
            }
            AppError::Db(e) => {
                tracing::error!(error = %e, "database error");
                "internal server error".to_string()
            }
            other => other.to_string(),
        };
        let body = ApiErrorBody::new(code, public_message);
        (status, Json(body)).into_response()
    }
}
