//! Admin web 后台集成测试。
//!
//! 覆盖:
//! - require_admin middleware:无 cookie / 无 Bearer → 401(JSON 路径)
//! - require_admin:非 admin 用户 → 403
//! - 防最后一个 active admin 删除
//! - 防最后一个 active admin 降级
//! - admin 创建用户 → users.csv 导出能看到该行 + audit_log 写入 admin.user_create
//! - admin 重置密码 → 旧密码 login 失败
//! - admin 软删 observation → admin search include_deleted=true 仍能看到 deleted_at

mod common;

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    Router,
};
use cmem_server::db::{audit, observations, users};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use tower::ServiceExt;

use common::*;

/// 起 in-memory pool + router + 顺手提升一个 admin。
async fn make_app_with_admin(username: &str) -> (Router, SqlitePool, String, String) {
    let (app, pool) = make_app().await;
    register_user(&app, username).await;
    promote_to_admin(&pool, username).await;
    let login = login_user(&app, username).await;
    let user_id = users::find_by_username(&pool, username)
        .await
        .expect("lookup")
        .expect("user")
        .id;
    (app, pool, user_id, login.access_token)
}

async fn promote_to_admin(pool: &SqlitePool, username: &str) {
    let user = users::find_by_username(pool, username)
        .await
        .expect("lookup")
        .expect("user");
    users::set_admin(pool, &user.id, true).await.expect("promote");
}

/// 简化 helper:对 /api/admin/* 发请求,带 Bearer。
async fn admin_request(
    app: &Router,
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Vec<u8>) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::ACCEPT, "application/json");
    if let Some(token) = bearer {
        req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let body = match body {
        Some(v) => {
            req = req.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&v).unwrap())
        }
        None => Body::empty(),
    };
    let req = req.body(body).expect("build req");
    let resp = app.clone().oneshot(req).await.expect("send");
    let status = resp.status();
    let body = to_bytes(resp.into_body(), 16 * 1024 * 1024)
        .await
        .expect("read body");
    (status, body.to_vec())
}

// ---------- 测试 1:无 cookie / 无 bearer → 401 ----------

#[tokio::test]
async fn admin_api_without_token_returns_401() {
    let (app, _pool) = make_app().await;
    let (status, _) = admin_request(&app, "GET", "/api/admin/stats", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---------- 测试 2:非 admin → 403 ----------

#[tokio::test]
async fn admin_api_with_non_admin_user_returns_403() {
    let (app, _pool) = make_app().await;
    register_user(&app, "alice").await;
    let login = login_user(&app, "alice").await;
    let (status, _) =
        admin_request(&app, "GET", "/api/admin/stats", Some(&login.access_token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ---------- 测试 3:admin 能拿到 stats ----------

#[tokio::test]
async fn admin_api_with_admin_user_returns_stats() {
    let (app, _pool, _id, token) = make_app_with_admin("root").await;
    let (status, body) = admin_request(&app, "GET", "/api/admin/stats", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_slice(&body).expect("json");
    assert!(v["users"].as_i64().unwrap() >= 1, "expected at least 1 user");
    assert!(v.get("recent").is_some());
}

// ---------- 测试 4:防止删除最后一个活跃 admin ----------

#[tokio::test]
async fn cannot_delete_last_active_admin() {
    let (app, pool, root_id, token) = make_app_with_admin("root").await;

    // root 自己不能删自己 → 409
    let (status, _) = admin_request(
        &app,
        "DELETE",
        &format!("/api/admin/users/{root_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // 再注册一个普通用户,把它降级成普通用户(默认就是)。提升它为 admin。
    register_user(&app, "second").await;
    let second_id = users::find_by_username(&pool, "second")
        .await
        .unwrap()
        .unwrap()
        .id;
    let (s, _) = admin_request(
        &app,
        "PATCH",
        &format!("/api/admin/users/{second_id}"),
        Some(&token),
        Some(json!({"is_admin": true})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // 用 second admin 的 token 删 root(2 个 admin → 删一个仍剩 1)
    let second_login = login_user(&app, "second").await;
    let (s, _) = admin_request(
        &app,
        "DELETE",
        &format!("/api/admin/users/{root_id}"),
        Some(&second_login.access_token),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}

// ---------- 测试 5:防止降级最后一个活跃 admin ----------

#[tokio::test]
async fn cannot_demote_last_active_admin() {
    let (app, _pool, root_id, token) = make_app_with_admin("root").await;
    let (status, body) = admin_request(
        &app,
        "PATCH",
        &format!("/api/admin/users/{root_id}"),
        Some(&token),
        Some(json!({ "is_admin": false })),
    )
    .await;
    let bs = String::from_utf8_lossy(&body);
    assert_eq!(status, StatusCode::CONFLICT, "body: {bs}");
}

// ---------- 测试 6:admin 创建 user → audit_log + users.csv 都能看到 ----------

#[tokio::test]
async fn admin_create_user_writes_audit_and_appears_in_csv_export() {
    let (app, pool, root_id, token) = make_app_with_admin("root").await;

    // POST /api/admin/users
    let (status, _) = admin_request(
        &app,
        "POST",
        "/api/admin/users",
        Some(&token),
        Some(json!({
            "username": "carol",
            "password": "supersecretpw",
            "email": "carol@x.io",
            "is_admin": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // audit_log 应有 admin.user_create
    let rows = audit::search(&pool, Some(&root_id), Some("admin."), None, None, 50)
        .await
        .expect("audit search");
    assert!(
        rows.iter().any(|r| r.action == "admin.user_create"),
        "audit_log should contain admin.user_create"
    );

    // GET /api/admin/export/users.csv 应能看到 carol
    let (status, body) = admin_request(
        &app,
        "GET",
        "/api/admin/export/users.csv",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let csv = String::from_utf8(body).expect("utf8");
    assert!(csv.contains("carol"), "csv should mention carol: {csv}");
    assert!(
        csv.contains("carol@x.io"),
        "csv should mention email: {csv}"
    );
}

// ---------- 测试 7:admin 重置密码 → 旧密码 login 失败 ----------

#[tokio::test]
async fn admin_reset_password_invalidates_old_password() {
    let (app, pool, _root_id, token) = make_app_with_admin("root").await;
    register_user(&app, "dave").await;
    // 此时 dave 用 "correct horse battery staple" 能登录
    let _ = login_user(&app, "dave").await;

    let dave_id = users::find_by_username(&pool, "dave")
        .await
        .unwrap()
        .unwrap()
        .id;

    // PATCH /api/admin/users/:id { password: "newone..." }
    let (status, _) = admin_request(
        &app,
        "PATCH",
        &format!("/api/admin/users/{dave_id}"),
        Some(&token),
        Some(json!({ "password": "newpassword!!" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 旧密码应该 login 失败 → 401
    let req = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "username": "dave",
                "password": "correct horse battery staple"
            }))
            .unwrap(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.expect("send");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ---------- 测试 8:soft delete observation 工作正常 ----------

#[tokio::test]
async fn admin_soft_delete_observation_marks_deleted_at() {
    let (app, pool, _root_id, token) = make_app_with_admin("root").await;

    // 让 root 注册 machine + push 一条 observation
    let root_login = login_user(&app, "root").await;
    let m = register_machine(&app, &root_login.access_token, "laptop").await;
    let push_body = json!({
        "observations": [{
            "id": "obs-1",
            "timestamp": 1000,
            "project_marker_id": null,
            "project_name": "demo",
            "project_path": "/x/demo",
            "content": "hello world",
            "obs_type": "note",
            "metadata": null,
            "derived_from": null,
            "derivation_chain": null,
        }]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/sync/push")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {}", m.machine_token))
        .body(Body::from(serde_json::to_vec(&push_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.expect("push");
    assert_eq!(resp.status(), StatusCode::OK);

    // soft-delete via admin API
    let (status, _) = admin_request(
        &app,
        "DELETE",
        "/api/admin/observations/obs-1",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // 通过 admin search 能看到(include_deleted=true 默认行为是 web UI)
    let rows = observations::admin_search(&pool, None, None, None, None, None, None, true, 100, 0)
        .await
        .expect("admin search");
    let found = rows.iter().find(|r| r.id == "obs-1").expect("obs-1");
    assert!(found.deleted_at.is_some(), "obs-1 should now be deleted");
}

