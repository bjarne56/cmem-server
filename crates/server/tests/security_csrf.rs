//! Admin web CSRF 防护端到端测试。
//!
//! 覆盖:
//! * `GET /admin/login` 返回 `Set-Cookie: cmem_admin_csrf=...`,token 写进 cookie。
//! * `POST /admin/login` 不带 _csrf 字段 → 403。
//! * `POST /admin/login` 带错误 _csrf 字段 → 403。
//! * `POST /admin/login` 带与 cookie 一致的 _csrf 字段 → 进入下游 handler
//!   (handler 自己的 401 InvalidCredentials 是预期,因为我们用的是空账号 —
//!   关键是 *没被 CSRF 中间件拦截到 403*)。
//! * `csrf_enabled=false` 时不强制(回归测试,确保不破坏配置开关)。

mod common;

use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    Router,
};
use cmem_server::{
    auth::JwtCodec,
    config::{AppConfig, AuthConfig, DatabaseConfig, SecurityConfig, ServerConfig},
    db,
    server::build_router,
    state::AppState,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tower::ServiceExt;

async fn make_app_with_csrf(enabled: bool) -> Router {
    let opts = SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect");
    db::migrate(&pool).await.expect("migrate");
    let cfg = AppConfig {
        server: ServerConfig {
            bind: "127.0.0.1:0".to_string(),
        },
        database: DatabaseConfig {
            path: ":memory:".into(),
        },
        auth: AuthConfig {
            jwt_secret: hex::encode([0u8; 32]),
            access_token_ttl_secs: 3600,
            refresh_token_ttl_secs: 86400,
            machine_token_ttl_secs: 86400,
            argon2_memory_kib: 8,
            argon2_iterations: 1,
            argon2_parallelism: 1,
            require_invite: false,
        },
        security: SecurityConfig {
            // 高配额避免限速干扰本测试
            trusted_proxies: vec!["0.0.0.0/0".into(), "::/0".into()],
            login_rate_per_minute: 10_000,
            api_rate_per_minute: 10_000,
            csrf_enabled: enabled,
        },
    };
    let cfg = Arc::new(cfg);
    let jwt = JwtCodec::new(&cfg.auth.jwt_secret).expect("jwt");
    let state = AppState {
        pool,
        jwt,
        config: cfg,
    };
    build_router(state)
}

/// 从 Set-Cookie 头里抓 csrf token。
fn extract_csrf_cookie(headers: &axum::http::HeaderMap) -> Option<String> {
    for v in headers.get_all(header::SET_COOKIE).iter() {
        let s = v.to_str().ok()?;
        if let Some(rest) = s.strip_prefix("cmem_admin_csrf=") {
            // 取 ; 之前
            let val = rest.split(';').next()?;
            return Some(val.to_string());
        }
    }
    None
}

#[tokio::test]
async fn get_login_page_sets_csrf_cookie() {
    let app = make_app_with_csrf(true).await;
    let req = Request::builder()
        .method("GET")
        .uri("/admin/login")
        .header(header::ACCEPT, "text/html")
        .body(Body::empty())
        .expect("build");
    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK);
    let token = extract_csrf_cookie(resp.headers());
    assert!(token.is_some(), "csrf cookie should be set on GET /admin/login");
    assert_eq!(
        token.as_ref().map(|t| t.len()),
        Some(64),
        "csrf token must be 64 hex chars (32 random bytes)"
    );
}

#[tokio::test]
async fn post_login_without_csrf_is_forbidden() {
    let app = make_app_with_csrf(true).await;
    // 1. 先 GET 拿到 cookie(模拟用户访问登录页)
    let get_req = Request::builder()
        .method("GET")
        .uri("/admin/login")
        .body(Body::empty())
        .expect("build");
    let get_resp = app.clone().oneshot(get_req).await.expect("oneshot");
    let token = extract_csrf_cookie(get_resp.headers()).expect("token");

    // 2. POST 携带 cookie,但 form body 不含 _csrf → 403
    let body = "username=alice&password=correct horse";
    let post_req = Request::builder()
        .method("POST")
        .uri("/admin/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, format!("cmem_admin_csrf={token}"))
        .body(Body::from(body))
        .expect("build");
    let resp = app.oneshot(post_req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn post_login_with_wrong_csrf_is_forbidden() {
    let app = make_app_with_csrf(true).await;
    let get_req = Request::builder()
        .method("GET")
        .uri("/admin/login")
        .body(Body::empty())
        .expect("build");
    let get_resp = app.clone().oneshot(get_req).await.expect("oneshot");
    let token = extract_csrf_cookie(get_resp.headers()).expect("token");

    let bad = "abc123def456".to_string();
    assert_ne!(bad, token);

    let body = format!("username=alice&password=foo&_csrf={bad}");
    let post_req = Request::builder()
        .method("POST")
        .uri("/admin/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, format!("cmem_admin_csrf={token}"))
        .body(Body::from(body))
        .expect("build");
    let resp = app.oneshot(post_req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn post_login_with_correct_csrf_passes_csrf_layer() {
    // 关键检查:CSRF 通过后,handler 自己的 InvalidCredentials 401 / 200
    // 都算"CSRF 没拦截"。我们这里因为没创建用户,期望 401,不是 403。
    let app = make_app_with_csrf(true).await;
    let get_req = Request::builder()
        .method("GET")
        .uri("/admin/login")
        .body(Body::empty())
        .expect("build");
    let get_resp = app.clone().oneshot(get_req).await.expect("oneshot");
    let token = extract_csrf_cookie(get_resp.headers()).expect("token");

    // 模拟 form 提交,_csrf 必须 url-encoded 一致 — 我们用 hex 字符,不需 encode。
    let body = format!("username=ghost&password=foo123456&_csrf={token}");
    let post_req = Request::builder()
        .method("POST")
        .uri("/admin/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, format!("cmem_admin_csrf={token}"))
        .body(Body::from(body))
        .expect("build");
    let resp = app.oneshot(post_req).await.expect("oneshot");
    let status = resp.status();
    let body = to_bytes(resp.into_body(), 16 * 1024).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    // 不应是 CSRF 拒绝(403 with CSRF body)
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "CSRF should pass; got {status} body={body_str}"
    );
    // 实际 handler 因为 ghost 用户不存在,会返回 401 + login page
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::OK,
        "expected handler-level 401 / OK after CSRF accepts; got {status}"
    );
}

#[tokio::test]
async fn csrf_disabled_skips_validation() {
    // 配置 csrf_enabled=false → POST 不带 _csrf 也允许通过(交给 handler)
    let app = make_app_with_csrf(false).await;
    let body = "username=alice&password=correct horse";
    let post_req = Request::builder()
        .method("POST")
        .uri("/admin/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .expect("build");
    let resp = app.oneshot(post_req).await.expect("oneshot");
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "with csrf_enabled=false, POST should not be CSRF-rejected"
    );
}
