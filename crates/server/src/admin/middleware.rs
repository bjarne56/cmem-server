//! `require_admin` 中间件:拒绝非 admin / 未登录用户。
//!
//! 鉴权流程:
//! 1. 从 `Cookie: cmem_admin_session=<JWT>` 优先解析(web 浏览器);
//! 2. 否则尝试 `Authorization: Bearer <JWT>`(API 客户端 / curl);
//! 3. 解码 JWT 拿 user_id,查 users 表确保 `is_admin=1 AND is_active=1`;
//! 4. 失败时区分客户端类型回响应:
//!    - 浏览器请求(Accept: text/html)→ 302 跳 `/admin/login`(GET 时)或 403 HTML(其他)
//!    - API 请求(Accept: application/json)→ 401/403 JSON
//!
//! 注意:本中间件不应应用到 `/admin/login` / `/admin/logout` / `/admin/__static`
//! 这类公开路由,否则会无限重定向。

use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
    Extension,
};

use crate::auth::TokenKind;
use crate::error::AppError;
use crate::middleware::Principal;
use crate::state::AppState;

pub const ADMIN_COOKIE_NAME: &str = "cmem_admin_session";

/// 把 admin user_id 注入 request extensions,供下游 handler 取用。
#[derive(Debug, Clone)]
pub struct AdminPrincipal {
    pub user_id: String,
    pub username: String,
}

/// 客户端是否偏好 HTML(用于 401/403 是否跳转登录页)。
fn wants_html(req: &Request) -> bool {
    req.headers()
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.contains("text/html"))
        .unwrap_or(false)
}

/// 从 Cookie 头里提取指定 cookie 值。
fn cookie_value<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> Option<&'a str> {
    let header = headers.get(header::COOKIE)?.to_str().ok()?;
    for kv in header.split(';') {
        let kv = kv.trim();
        if let Some(rest) = kv.strip_prefix(&format!("{name}=")) {
            return Some(rest);
        }
    }
    None
}

/// 从 Authorization 头里提取 Bearer token。
fn bearer_token(headers: &axum::http::HeaderMap) -> Option<&str> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    v.strip_prefix("Bearer ")
        .or_else(|| v.strip_prefix("bearer "))
}

/// 鉴权失败的响应(按 Accept 区分 HTML / JSON)。
fn auth_failure(req: &Request, status: StatusCode) -> Response {
    if wants_html(req) && req.method() == Method::GET {
        // GET HTML 请求且未登录 → 跳转登录页
        if status == StatusCode::UNAUTHORIZED {
            return Redirect::to("/admin/login").into_response();
        }
        // 其他失败(403)→ HTML 错误
        let body = format!(
            "<!doctype html><html><body style='font-family:system-ui;padding:2rem'><h1>{status}</h1><p><a href=\"/admin/login\">登录</a></p></body></html>"
        );
        let mut resp = (status, body).into_response();
        resp.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        return resp;
    }
    // JSON / 非 GET → AppError 路径
    match status {
        StatusCode::UNAUTHORIZED => AppError::Unauthorized.into_response(),
        StatusCode::FORBIDDEN => AppError::Forbidden.into_response(),
        _ => AppError::Unauthorized.into_response(),
    }
}

/// 鉴权中间件:见 [`crate::admin::middleware`] 模块文档。
pub async fn require_admin(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    // 1. 取 token(cookie 优先,Bearer 兜底)
    let token = cookie_value(req.headers(), ADMIN_COOKIE_NAME)
        .map(str::to_string)
        .or_else(|| bearer_token(req.headers()).map(str::to_string));
    let Some(token) = token else {
        return auth_failure(&req, StatusCode::UNAUTHORIZED);
    };

    // 2. 解码 JWT
    let Ok(claims) = state.jwt.decode(&token) else {
        return auth_failure(&req, StatusCode::UNAUTHORIZED);
    };
    if !matches!(claims.kind, TokenKind::Access) {
        return auth_failure(&req, StatusCode::UNAUTHORIZED);
    }

    // 3. 查 users:必须 admin + active
    let user = match crate::db::users::find_by_id(&state.pool, &claims.sub).await {
        Ok(Some(u)) => u,
        Ok(None) => return auth_failure(&req, StatusCode::UNAUTHORIZED),
        Err(e) => {
            tracing::error!(error = %e, "admin: db lookup failed");
            return AppError::Internal(e).into_response();
        }
    };
    if user.is_active == 0 {
        return auth_failure(&req, StatusCode::FORBIDDEN);
    }
    if user.is_admin == 0 {
        return auth_failure(&req, StatusCode::FORBIDDEN);
    }

    // 4. 注入 principal
    let admin = AdminPrincipal {
        user_id: user.id.clone(),
        username: user.username.clone(),
    };
    // 同时注入普通 Principal,方便复用 audit 等 handler
    req.extensions_mut().insert(Principal::User {
        user_id: user.id.clone(),
        machine_id: None,
    });
    req.extensions_mut().insert(Extension(admin.clone()));
    req.extensions_mut().insert(admin);

    next.run(req).await
}
