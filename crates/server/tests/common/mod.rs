//! 集成测试共享 helper:启动一个内存数据库 + Router,提供 register/login/register_machine 等 helper。
//!
//! 不同测试 binary 用到的子集不一样,允许部分函数在某个测试里 dead。

#![allow(dead_code)]

use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    Router,
};
use cmem_server::{
    auth::JwtCodec,
    config::{AppConfig, AuthConfig, DatabaseConfig, ServerConfig},
    db,
    server::build_router,
    state::AppState,
};
use cmem_shared::api::{
    CreateMachineRequest, CreateMachineResponse, LoginRequest, LoginResponse, RegisterResponse,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use tower::ServiceExt;

/// 测试用 fast-argon2 配置(8 KiB / 1 iter,几毫秒就完成)。
pub fn fast_auth_config() -> AuthConfig {
    AuthConfig {
        jwt_secret: hex::encode([0u8; 32]),
        access_token_ttl_secs: 3600,
        refresh_token_ttl_secs: 86400,
        machine_token_ttl_secs: 86400,
        argon2_memory_kib: 8,
        argon2_iterations: 1,
        argon2_parallelism: 1,
        require_invite: false,
    }
}

pub fn fast_app_config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            bind: "127.0.0.1:0".to_string(),
        },
        database: DatabaseConfig {
            path: ":memory:".into(),
        },
        auth: fast_auth_config(),
    }
}

/// 起一个 in-memory sqlite + 跑 migration + 拼 router。
pub async fn make_app() -> (Router, SqlitePool) {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect in-memory sqlite");
    db::migrate(&pool).await.expect("migrate");
    let cfg = Arc::new(fast_app_config());
    let jwt = JwtCodec::new(&cfg.auth.jwt_secret).expect("init jwt codec");
    let state = AppState {
        pool: pool.clone(),
        jwt,
        config: cfg,
    };
    (build_router(state), pool)
}

pub async fn json_request<T: DeserializeOwned>(
    app: &Router,
    method: &str,
    uri: &str,
    body: Value,
    bearer: Option<&str>,
) -> (StatusCode, T) {
    let body_bytes = serde_json::to_vec(&body).expect("serialize body");
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = bearer {
        req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let req = req.body(Body::from(body_bytes)).expect("build req");
    let resp = app.clone().oneshot(req).await.expect("send req");
    let status = resp.status();
    let body = to_bytes(resp.into_body(), 16 * 1024 * 1024)
        .await
        .expect("read body");
    if body.is_empty() {
        // T 可能是 () - 但我们这里硬要 deserialize,留空时报错
        let parsed = serde_json::from_str::<T>("null").unwrap_or_else(|_| {
            panic!("empty body cannot deserialize as expected type for {uri}")
        });
        return (status, parsed);
    }
    let parsed: T = serde_json::from_slice(&body).unwrap_or_else(|e| {
        let txt = String::from_utf8_lossy(&body);
        panic!("response decode failed for {uri}: {e}\nbody: {txt}")
    });
    (status, parsed)
}

pub async fn empty_request(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    bearer: Option<&str>,
) -> StatusCode {
    let body_bytes = match body {
        Some(v) => serde_json::to_vec(&v).expect("serialize"),
        None => Vec::new(),
    };
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = bearer {
        req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let req = req.body(Body::from(body_bytes)).expect("build req");
    let resp = app.clone().oneshot(req).await.expect("send req");
    resp.status()
}

pub async fn register_user(app: &Router, username: &str) -> RegisterResponse {
    let (status, resp): (_, RegisterResponse) = json_request(
        app,
        "POST",
        "/api/auth/register",
        serde_json::json!({
            "username": username,
            "password": "correct horse battery staple",
        }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "register failed");
    resp
}

pub async fn login_user(app: &Router, username: &str) -> LoginResponse {
    let req = LoginRequest {
        username: username.to_string(),
        password: "correct horse battery staple".to_string(),
    };
    let body = serde_json::to_value(&req).unwrap();
    let (status, resp): (_, LoginResponse) =
        json_request(app, "POST", "/api/auth/login", body, None).await;
    assert_eq!(status, StatusCode::OK, "login failed");
    resp
}

pub async fn register_machine(
    app: &Router,
    access_token: &str,
    name: &str,
) -> CreateMachineResponse {
    let body = serde_json::to_value(CreateMachineRequest {
        name: name.to_string(),
        description: None,
    })
    .unwrap();
    let (status, resp): (_, CreateMachineResponse) = json_request(
        app,
        "POST",
        "/api/machines",
        body,
        Some(access_token),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "register machine failed");
    resp
}
