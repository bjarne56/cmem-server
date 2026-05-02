//! Admin web 表单 CSRF 防护(double-submit cookie 模式)。
//!
//! 设计:
//!
//! * GET /admin/* —— 中间件读 cookie `cmem_admin_csrf`,如果不存在就生成
//!   一个新的 random token 并 `Set-Cookie`。token 也通过 [`CsrfToken`]
//!   注入 request extensions,模板渲染时取出来塞进 `<input type=hidden
//!   name=_csrf>`。
//!
//! * POST /admin/* —— 中间件读 application/x-www-form-urlencoded body
//!   里的 `_csrf` 字段,与 cookie 比较。不一致 → 403。校验通过后,把原
//!   body 重新装回 request 让下游 handler 继续走 `Form(...)` 解析。
//!
//! 不影响:
//!
//! * `/api/admin/*` —— 纯 JSON API,Authorization Bearer JWT 已足。
//! * GET / OPTIONS / HEAD —— 不会修改服务器状态。
//!
//! 安全模型:
//!
//! 攻击者跨站提交表单,无法读到受害者的 cookie 值,因此填不出能匹配 cookie
//! 的 `_csrf`。只要 cookie 是 `SameSite=Strict + HttpOnly`(本项目对
//! 登录 cookie 已经如此,csrf cookie 因为前端要 inject 进表单不能 HttpOnly),
//! 跨站请求就构造不出合法 token。

use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use rand::{rngs::OsRng, RngCore};

use crate::state::AppState;

pub const CSRF_COOKIE_NAME: &str = "cmem_admin_csrf";
pub const CSRF_FORM_FIELD: &str = "_csrf";

/// 注入到 request extensions 供模板取用。
#[derive(Debug, Clone)]
pub struct CsrfToken(pub String);

impl CsrfToken {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 从 Cookie 头里取指定 cookie 值。复用 admin/middleware.rs 里的逻辑,
/// 但写在这里避免 cyclic import / pub 改动。
fn cookie_value(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    let h = headers.get(header::COOKIE)?.to_str().ok()?;
    for kv in h.split(';') {
        let kv = kv.trim();
        if let Some(rest) = kv.strip_prefix(&format!("{name}=")) {
            return Some(rest.to_string());
        }
    }
    None
}

fn generate_token() -> String {
    let mut buf = [0u8; 32];
    OsRng.fill_bytes(&mut buf);
    hex::encode(buf)
}

/// 从 url-encoded body 里 brute-force 找 `_csrf=...` 字段。
///
/// 自实现以避免引入 url crate;只支持 `&` 切片 + percent decode 16 进制。
/// CSRF token 字符集为 hex(`generate_token`),不需要 percent decode,
/// 直接字符串比较即可。
fn extract_csrf_from_form(body: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(body).ok()?;
    for pair in s.split('&') {
        if let Some(rest) = pair.strip_prefix(&format!("{CSRF_FORM_FIELD}=")) {
            return Some(rest.to_string());
        }
    }
    None
}

/// CSRF 中间件主入口。挂在 `/admin` 全部子路由上(包括 GET 与 POST)。
pub async fn csrf_protect(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    // CSRF 关闭时直接放行,但仍然给 GET 请求带上 token cookie + extension,
    // 方便模板照常渲染(双开关:配置关 = 不强制校验,但 token 一直有,运维
    // 切回 enabled=true 立刻生效)。
    let enabled = state.config.security.csrf_enabled;

    let method = req.method().clone();
    let is_state_changing =
        matches!(method, Method::POST | Method::PUT | Method::PATCH | Method::DELETE);

    // 读 / 生成 cookie token
    let (token, set_cookie) = match cookie_value(req.headers(), CSRF_COOKIE_NAME) {
        Some(t) if !t.is_empty() => (t, false),
        _ => (generate_token(), true),
    };

    if !is_state_changing {
        // GET / HEAD / OPTIONS:把 token 放 extensions 给模板用,需要时 set cookie。
        let mut req = req;
        req.extensions_mut().insert(CsrfToken(token.clone()));
        let mut resp = next.run(req).await;
        if set_cookie {
            // 注意 csrf cookie 不能 HttpOnly —— 模板需要把它塞进 form。
            // 但实际上 *form 是服务端渲染的*,这意味着我们可以让 cookie HttpOnly
            // 也无所谓(模板从 extension 取 token,而不是 JS 读 cookie)。
            // 留 HttpOnly 提升健壮性:即便有 XSS,token 也不会被脚本带走。
            let cookie = format!(
                "{CSRF_COOKIE_NAME}={token}; HttpOnly; Path=/admin; Max-Age=86400; SameSite=Strict"
            );
            if let Ok(v) = HeaderValue::from_str(&cookie) {
                resp.headers_mut().append(header::SET_COOKIE, v);
            }
        }
        return resp;
    }

    // POST / PUT / PATCH / DELETE:取 body 校验 _csrf 字段。
    // axum 0.7 Request<Body> 可以拆 body 出来 read。
    let (parts, body) = req.into_parts();
    let bytes = match to_bytes(body, 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return reject(parts.headers.clone(), enabled, "body read failed");
        }
    };

    let submitted = extract_csrf_from_form(&bytes);

    if enabled {
        let valid = matches!(submitted.as_deref(), Some(s) if s == token);
        if !valid {
            tracing::warn!(
                method = %parts.method,
                uri = %parts.uri,
                has_token = submitted.is_some(),
                "csrf token mismatch"
            );
            return reject(parts.headers.clone(), enabled, "csrf token mismatch");
        }
    }

    // 重组 request 给下游 handler。
    let mut req = Request::from_parts(parts, Body::from(bytes));
    req.extensions_mut().insert(CsrfToken(token.clone()));
    next.run(req).await
}

fn reject(_headers: axum::http::HeaderMap, _enabled: bool, _why: &'static str) -> Response {
    (
        StatusCode::FORBIDDEN,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        "<!doctype html><html><body><h1>403 Forbidden</h1>\
         <p>CSRF token missing or invalid. Please reload the page and try again.</p>\
         </body></html>",
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_finds_csrf_field() {
        let body = b"username=alice&_csrf=deadbeef&password=x";
        assert_eq!(
            extract_csrf_from_form(body).as_deref(),
            Some("deadbeef")
        );
    }

    #[test]
    fn extract_when_csrf_first() {
        let body = b"_csrf=abc123&x=y";
        assert_eq!(extract_csrf_from_form(body).as_deref(), Some("abc123"));
    }

    #[test]
    fn extract_returns_none_when_missing() {
        let body = b"username=alice&password=x";
        assert!(extract_csrf_from_form(body).is_none());
    }

    #[test]
    fn cookie_value_parses() {
        let mut h = axum::http::HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; cmem_admin_csrf=tok123; b=2"),
        );
        assert_eq!(
            cookie_value(&h, CSRF_COOKIE_NAME).as_deref(),
            Some("tok123")
        );
    }

    #[test]
    fn token_has_expected_length() {
        let t = generate_token();
        assert_eq!(t.len(), 64);
    }
}
