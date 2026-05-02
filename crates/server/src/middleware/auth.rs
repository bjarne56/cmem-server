//! Bearer Token 鉴权中间件。
//!
//! 区分 user JWT 与 machine token(`cmt_` 前缀)。M2 阶段 machine token 验证已实现,
//! 但 machines 表条目要等 M3 才能创建。

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};

use crate::auth::tokens::{hash_machine_token, is_machine_token};
use crate::error::AppError;
use crate::state::AppState;

/// 当前请求的认证主体。
#[derive(Debug, Clone)]
pub enum Principal {
    /// 由 user JWT 鉴权。
    User { user_id: String, machine_id: Option<String> },
    /// 由 machine token 鉴权。
    Machine { user_id: String, machine_id: String },
}

impl Principal {
    pub fn user_id(&self) -> &str {
        match self {
            Self::User { user_id, .. } | Self::Machine { user_id, .. } => user_id,
        }
    }
}

/// 提取 `Authorization: Bearer xxx` 头。
fn extract_bearer(req: &Request) -> Result<String, AppError> {
    let auth = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .ok_or(AppError::Unauthorized)?
        .to_str()
        .map_err(|_| AppError::Unauthorized)?;
    let token = auth
        .strip_prefix("Bearer ")
        .or_else(|| auth.strip_prefix("bearer "))
        .ok_or(AppError::Unauthorized)?;
    if token.is_empty() {
        return Err(AppError::Unauthorized);
    }
    Ok(token.to_string())
}

/// require_auth:解析 Bearer 并把 `Principal` 注入 request extensions。
pub async fn require_auth(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let token = extract_bearer(&req)?;

    let principal = if is_machine_token(&token) {
        let hash = hash_machine_token(&token);
        let row = sqlx::query!(
            r#"
            SELECT user_id AS "user_id!: String", id AS "id!: String"
            FROM machines
            WHERE machine_token_hash = ?1 AND revoked = 0
            "#,
            hash,
        )
        .fetch_optional(&state.pool)
        .await
        .map_err(AppError::Db)?
        .ok_or(AppError::Unauthorized)?;
        Principal::Machine {
            user_id: row.user_id,
            machine_id: row.id,
        }
    } else {
        let claims = state
            .jwt
            .decode(&token)
            .map_err(|_| AppError::Unauthorized)?;
        if !matches!(claims.kind, crate::auth::TokenKind::Access) {
            return Err(AppError::Unauthorized);
        }
        Principal::User {
            user_id: claims.sub,
            machine_id: claims.mid,
        }
    };

    req.extensions_mut().insert(principal);
    Ok(next.run(req).await)
}
