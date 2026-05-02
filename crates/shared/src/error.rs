//! API 错误响应结构体。
//!
//! 所有错误响应统一为 `{"error":{"code":"...","message":"..."}}`。

use serde::{Deserialize, Serialize};

/// 错误响应包络。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiErrorBody {
    pub error: ApiError,
}

/// 业务错误细节。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl ApiErrorBody {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: ApiError {
                code: code.into(),
                message: message.into(),
            },
        }
    }
}
