//! M2 集成测试:admin CLI 子命令(直接调用 lib 函数,绕过 HTTP)。
//!
//! 覆盖:
//!   - user create / list / promote / demote / disable / enable / delete
//!   - delete 触发 ON DELETE CASCADE(machines / projects 一并清理)
//!   - invite create 输出 nanoid 32 字符
//!   - invite consume 后 use_count 增长
//!   - require_invite=true 时 register 必须带有效码
//!   - audit_log 记录 admin.* 事件

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use cmem_server::{
    auth::JwtCodec,
    commands::admin::{
        invite_create, invite_revoke, user_create, user_delete, user_set_active, user_set_admin,
    },
    config::AppConfig,
    db::{self, audit, invites, machines, projects, stats, users},
    server::build_router,
    state::AppState,
};
use cmem_shared::api::{CreateProjectRequest, RegisterResponse};
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

use common::*;

/// 起一个 in-memory pool + cfg(测试 admin CLI 函数用)。
async fn make_admin_ctx() -> (SqlitePool, AppConfig) {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect in-memory sqlite");
    db::migrate(&pool).await.expect("migrate");
    (pool, fast_app_config())
}

#[tokio::test]
async fn admin_user_create_then_list_visible() {
    let (pool, cfg) = make_admin_ctx().await;

    user_create(&pool, &cfg, "alice", "supersecret", Some("alice@x.io"), false)
        .await
        .expect("create alice");

    let rows = users::list_all(&pool).await.expect("list users");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].username, "alice");
    assert_eq!(rows[0].email.as_deref(), Some("alice@x.io"));
    assert_eq!(rows[0].is_admin, 0);
    assert_eq!(rows[0].is_active, 1);

    // duplicate must fail
    let dup = user_create(&pool, &cfg, "alice", "x", None, false).await;
    assert!(dup.is_err(), "duplicate username should fail");
}

#[tokio::test]
async fn admin_user_promote_demote_toggles_is_admin() {
    let (pool, cfg) = make_admin_ctx().await;
    user_create(&pool, &cfg, "alice", "supersecret", None, false)
        .await
        .expect("create");

    user_set_admin(&pool, "alice", true).await.expect("promote");
    let rows = users::list_all(&pool).await.expect("list");
    assert_eq!(rows[0].is_admin, 1);

    user_set_admin(&pool, "alice", false).await.expect("demote");
    let rows = users::list_all(&pool).await.expect("list");
    assert_eq!(rows[0].is_admin, 0);
}

#[tokio::test]
async fn admin_user_disable_enable_toggles_is_active() {
    let (pool, cfg) = make_admin_ctx().await;
    user_create(&pool, &cfg, "alice", "supersecret", None, false)
        .await
        .expect("create");

    user_set_active(&pool, "alice", false)
        .await
        .expect("disable");
    let rows = users::list_all(&pool).await.expect("list");
    assert_eq!(rows[0].is_active, 0);

    user_set_active(&pool, "alice", true).await.expect("enable");
    let rows = users::list_all(&pool).await.expect("list");
    assert_eq!(rows[0].is_active, 1);
}

#[tokio::test]
async fn admin_user_delete_cascades_machines_and_projects() {
    // 用 HTTP 起 alice + 1 台机器 + 1 个项目,然后用 admin 命令删 alice。
    let (app, pool) = make_app().await;
    register_user(&app, "alice").await;
    let login = login_user(&app, "alice").await;
    let mac = register_machine(&app, &login.access_token, "alice-mac").await;
    // 创建 1 个项目
    let body = serde_json::to_value(CreateProjectRequest {
        name: "demo".into(),
        description: None,
    })
    .expect("ser");
    let (status, _proj): (_, serde_json::Value) =
        json_request(&app, "POST", "/api/projects", body, Some(&login.access_token)).await;
    assert_eq!(status, StatusCode::CREATED);

    // 确认存在
    let alice_row = users::find_by_username(&pool, "alice")
        .await
        .expect("find alice")
        .expect("alice exists");
    let macs = machines::list_by_user(&pool, &alice_row.id)
        .await
        .expect("list macs");
    let projs = projects::list_by_user(&pool, &alice_row.id)
        .await
        .expect("list projs");
    assert_eq!(macs.len(), 1);
    assert_eq!(projs.len(), 1);
    let mac_id = mac.machine.id.clone();

    // 删除
    user_delete(&pool, "alice").await.expect("delete alice");

    // user 没了
    assert!(users::find_by_username(&pool, "alice")
        .await
        .expect("re-lookup")
        .is_none());
    // machine 也走 CASCADE 没了
    assert!(machines::find_by_id(&pool, &mac_id)
        .await
        .expect("lookup machine")
        .is_none());
    // 项目也没了
    let projs2 = projects::list_by_user(&pool, &alice_row.id)
        .await
        .expect("list");
    assert!(projs2.is_empty());

    // delete 不存在的用户应当报错
    let again = user_delete(&pool, "alice").await;
    assert!(again.is_err());
}

#[tokio::test]
async fn admin_invite_create_emits_nanoid_and_consume_increments() {
    let (pool, _cfg) = make_admin_ctx().await;

    invite_create(&pool, 3, Some(7)).await.expect("create");
    let rows = invites::list_all(&pool).await.expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].max_uses, 3);
    assert_eq!(rows[0].use_count, 0);
    // nanoid!(32) 长度 = 32
    assert_eq!(rows[0].code.chars().count(), 32);
    // expires_at 应该约 7 天后
    let expires = rows[0].expires_at.expect("expires set");
    let now = chrono::Utc::now().timestamp();
    let delta = expires - now;
    assert!(
        (6 * 86_400..=8 * 86_400).contains(&delta),
        "expires roughly 7d, got delta={delta}"
    );

    // 模拟 register 消费一次。invite_codes.used_by 有 FK→users(id),需要真实用户。
    let cfg = fast_app_config();
    user_create(&pool, &cfg, "alice", "supersecret", None, false)
        .await
        .expect("create alice");
    let alice = users::find_by_username(&pool, "alice")
        .await
        .expect("look")
        .expect("present");
    invites::consume(&pool, &rows[0].code, &alice.id, chrono::Utc::now().timestamp())
        .await
        .expect("consume");
    let after = invites::find_by_code(&pool, &rows[0].code)
        .await
        .expect("find")
        .expect("present");
    assert_eq!(after.use_count, 1);
    assert_eq!(after.used_by.as_deref(), Some(alice.id.as_str()));

    // revoke 应当删除
    invite_revoke(&pool, &rows[0].code).await.expect("revoke");
    assert!(invites::find_by_code(&pool, &rows[0].code)
        .await
        .expect("look")
        .is_none());
}

#[tokio::test]
async fn admin_writes_audit_log_entries() {
    let (pool, cfg) = make_admin_ctx().await;
    user_create(&pool, &cfg, "alice", "supersecret", None, false)
        .await
        .expect("create");
    user_set_admin(&pool, "alice", true).await.expect("promote");
    invite_create(&pool, 1, None).await.expect("invite create");

    let rows = audit::list_recent(&pool, None, 50).await.expect("list");
    let actions: Vec<&str> = rows.iter().map(|r| r.action.as_str()).collect();
    assert!(actions.contains(&"admin.user_create"));
    assert!(actions.contains(&"admin.user_promote"));
    assert!(actions.contains(&"admin.invite_create"));
}

#[tokio::test]
async fn admin_stats_counts_users() {
    let (pool, cfg) = make_admin_ctx().await;
    user_create(&pool, &cfg, "alice", "supersecret", None, false)
        .await
        .expect("create alice");
    user_create(&pool, &cfg, "bob", "supersecret", None, true)
        .await
        .expect("create bob");

    let s = stats::collect(&pool).await.expect("collect");
    assert_eq!(s.users, 2);
    assert_eq!(s.machines, 0);
    assert_eq!(s.projects, 0);
    assert_eq!(s.observations, 0);
}

/// require_invite=true 时,register 必须带有效 invite_code。
#[tokio::test]
async fn register_with_require_invite_enforces_invite_code() {
    // 自定义 cfg 打开 require_invite。
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect");
    db::migrate(&pool).await.expect("migrate");

    let mut cfg = fast_app_config();
    cfg.auth.require_invite = true;
    let cfg = Arc::new(cfg);
    let jwt = JwtCodec::new(&cfg.auth.jwt_secret).expect("jwt");
    let state = AppState {
        pool: pool.clone(),
        jwt,
        config: cfg.clone(),
    };
    let app = build_router(state);

    // 1) 无 invite_code → 400
    let (status, _v): (_, serde_json::Value) = json_request(
        &app,
        "POST",
        "/api/auth/register",
        json!({"username": "alice", "password": "correct horse battery staple"}),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 2) 无效 invite_code → 400
    let (status, _v): (_, serde_json::Value) = json_request(
        &app,
        "POST",
        "/api/auth/register",
        json!({"username": "alice", "password": "correct horse battery staple", "invite_code": "nope"}),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 3) 创建一个 max_uses=1 的 invite,注册成功;再注册同 code 失败(exhausted)
    invite_create(&pool, 1, None).await.expect("create");
    let invite = invites::list_all(&pool).await.expect("list")[0].clone();

    let (status, _resp): (_, RegisterResponse) = json_request(
        &app,
        "POST",
        "/api/auth/register",
        json!({
            "username": "alice",
            "password": "correct horse battery staple",
            "invite_code": invite.code,
        }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let after = invites::find_by_code(&pool, &invite.code)
        .await
        .expect("look")
        .expect("present");
    assert_eq!(after.use_count, 1);

    // 第二次同 code → exhausted
    let (status, _v): (_, serde_json::Value) = json_request(
        &app,
        "POST",
        "/api/auth/register",
        json!({
            "username": "bob",
            "password": "correct horse battery staple",
            "invite_code": invite.code,
        }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// 旧用法回归:不带任何 invite_code、require_invite=false 时 register 仍能成功。
#[tokio::test]
async fn register_without_invite_when_optional_still_works() {
    let (app, _pool) = make_app().await;
    let _ = register_user(&app, "alice").await;
    let _login = login_user(&app, "alice").await;
}

/// 即便 require_invite=false,显式传一个无效 invite_code 也应被拒绝。
#[tokio::test]
async fn register_with_bad_invite_when_optional_still_rejected() {
    let (app, _pool) = make_app().await;
    let (status, _v): (_, serde_json::Value) = json_request(
        &app,
        "POST",
        "/api/auth/register",
        json!({"username": "carol", "password": "correct horse battery staple", "invite_code": "bogus"}),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

