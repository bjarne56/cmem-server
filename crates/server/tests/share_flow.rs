//! M5 集成测试:三种共享 mode + 撤销 + 降级。
//!
//! 测试覆盖 docs/PROJECT_SHARING.md 的 8 个不变量(对应注释):
//!   #1 owner 永远拥有完整权限          → cannot_share_to_self / non_owner_cannot_share_again
//!   #2 read-only 不进 client observations → server 端 share_mode 字段返回正确,client 自处
//!     -- 这里测 server 返回的 share_mode 字符串正确
//!   #3 auto-copy server 标记 share_mode='auto-copy' → server 端字段正确
//!   #4 撤销共享只影响 shared_view  → revoke_does_not_delete_observations
//!   #5 mode 降级触发 share_mode_downgrades → mode_downgrade_records_pending_notice
//!   #6 fork 留 M6
//!   #7 fork 不同步 → 留 M6

mod common;

use axum::http::StatusCode;
use cmem_shared::api::{
    ForkObservationResponse, ForkProjectResponse, ListProjectsResponse,
    ListSharedProjectsResponse, PullResponse, PushResponse, ShareResponse,
};
use serde_json::json;

use common::*;

async fn alice_bob_setup(
) -> (axum::Router, sqlx::SqlitePool, String, String, String, String) {
    let (app, pool) = make_app().await;
    register_user(&app, "alice").await;
    register_user(&app, "bob").await;
    let alice = login_user(&app, "alice").await;
    let bob = login_user(&app, "bob").await;
    let alice_mac = register_machine(&app, &alice.access_token, "alice-mac").await;
    let bob_mac = register_machine(&app, &bob.access_token, "bob-mac").await;
    (
        app,
        pool,
        alice.access_token,
        alice_mac.machine_token,
        bob.access_token,
        bob_mac.machine_token,
    )
}

async fn alice_push_project(
    app: &axum::Router,
    alice_machine_token: &str,
    obs_id: &str,
    project_name: &str,
) -> String {
    let body = json!({
        "observations": [
            {
                "id": obs_id,
                "timestamp": 1_700_000_000_i64,
                "project_marker_id": null,
                "project_name": project_name,
                "project_path": format!("/Users/alice/work/{project_name}"),
                "content": "alice content",
                "obs_type": null,
                "metadata": null,
                "derived_from": null,
                "derivation_chain": null
            }
        ]
    });
    let (_, push): (_, PushResponse) = json_request(
        app,
        "POST",
        "/api/sync/push",
        body,
        Some(alice_machine_token),
    )
    .await;
    push.projects_resolved[0].project_id.clone().unwrap()
}

async fn alice_share(
    app: &axum::Router,
    alice_access: &str,
    project_id: &str,
    target_username: &str,
    mode: &str,
) -> ShareResponse {
    let (status, resp): (_, ShareResponse) = json_request(
        app,
        "POST",
        "/api/shares",
        json!({
            "project_id": project_id,
            "target_type": "user",
            "target_username": target_username,
            "share_mode": mode,
            "expires_in_secs": null
        }),
        Some(alice_access),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "share creation failed");
    resp
}

#[tokio::test]
async fn read_only_share_visible_to_bob_with_correct_mode() {
    let (app, _pool, alice_access, alice_machine, _bob_access, bob_machine) =
        alice_bob_setup().await;
    let pid = alice_push_project(
        &app,
        &alice_machine,
        "01900000-0000-7000-8000-000000000101",
        "nginx-rce",
    )
    .await;
    alice_share(&app, &alice_access, &pid, "bob", "read-only").await;

    let (_, bob_pull): (_, PullResponse) = json_request(
        &app,
        "POST",
        "/api/sync/pull",
        json!({"since_seq": 0, "include_shared": true}),
        Some(&bob_machine),
    )
    .await;
    assert_eq!(bob_pull.shared_observations.len(), 1, "bob sees 1 shared obs");
    assert_eq!(bob_pull.shared_observations[0].share_mode, "read-only");
    assert_eq!(bob_pull.shared_observations[0].project_name, "nginx-rce");
    assert_eq!(bob_pull.shared_observations[0].sharer_username, "alice");
    // bob 自己 own observations 应为空
    assert_eq!(bob_pull.own_observations.len(), 0);
}

#[tokio::test]
async fn fork_allowed_share_marks_mode_correctly() {
    let (app, _pool, alice_access, alice_machine, _, bob_machine) =
        alice_bob_setup().await;
    let pid = alice_push_project(
        &app,
        &alice_machine,
        "01900000-0000-7000-8000-000000000201",
        "fork-able",
    )
    .await;
    alice_share(&app, &alice_access, &pid, "bob", "fork-allowed").await;

    let (_, bob_pull): (_, PullResponse) = json_request(
        &app,
        "POST",
        "/api/sync/pull",
        json!({"since_seq": 0}),
        Some(&bob_machine),
    )
    .await;
    assert_eq!(bob_pull.shared_observations[0].share_mode, "fork-allowed");
}

#[tokio::test]
async fn auto_copy_share_marks_mode_correctly() {
    let (app, _pool, alice_access, alice_machine, _, bob_machine) =
        alice_bob_setup().await;
    let pid = alice_push_project(
        &app,
        &alice_machine,
        "01900000-0000-7000-8000-000000000301",
        "auto-cp",
    )
    .await;
    alice_share(&app, &alice_access, &pid, "bob", "auto-copy").await;

    let (_, bob_pull): (_, PullResponse) = json_request(
        &app,
        "POST",
        "/api/sync/pull",
        json!({"since_seq": 0}),
        Some(&bob_machine),
    )
    .await;
    assert_eq!(bob_pull.shared_observations[0].share_mode, "auto-copy");
    // server 端 auto-copy 只是 JOIN 返回,不会主动改 observation owner;
    // observation.user_id 仍是 alice 的 user_id(UUID),sharer_username 是 "alice"。
    assert_eq!(bob_pull.shared_observations[0].sharer_username, "alice");
    assert_eq!(
        bob_pull.shared_observations[0].observation.user_id,
        bob_pull.shared_observations[0].sharer_user_id,
        "auto-copy 副本的生成由 client 完成,server 不改 owner"
    );
}

/// 不变量 #4:撤销共享只影响 shared_view 显示,不删 observations。
#[tokio::test]
async fn revoke_does_not_delete_observations() {
    let (app, _pool, alice_access, alice_machine, _, bob_machine) =
        alice_bob_setup().await;
    let pid = alice_push_project(
        &app,
        &alice_machine,
        "01900000-0000-7000-8000-000000000401",
        "to-revoke",
    )
    .await;
    let share_resp = alice_share(&app, &alice_access, &pid, "bob", "read-only").await;

    // bob 第一次 pull 看到
    let (_, bob_pull1): (_, PullResponse) = json_request(
        &app,
        "POST",
        "/api/sync/pull",
        json!({"since_seq": 0}),
        Some(&bob_machine),
    )
    .await;
    assert_eq!(bob_pull1.shared_observations.len(), 1);

    // alice 撤销
    let status = empty_request(
        &app,
        "DELETE",
        &format!("/api/shares/{}", share_resp.share.id),
        None,
        Some(&alice_access),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // bob 再 pull,shared_observations 为空,但 revoked_shares 有提示
    let (_, bob_pull2): (_, PullResponse) = json_request(
        &app,
        "POST",
        "/api/sync/pull",
        json!({"since_seq": 0}),
        Some(&bob_machine),
    )
    .await;
    assert_eq!(bob_pull2.shared_observations.len(), 0);
    assert_eq!(bob_pull2.revoked_shares.len(), 1);
    assert_eq!(bob_pull2.revoked_shares[0].project_id, pid);
    assert_eq!(bob_pull2.revoked_shares[0].owner_username, "alice");

    // alice 自己的 observation 仍在(她 pull 自己的 own_observations)
    let (_, alice_pull): (_, PullResponse) = json_request(
        &app,
        "POST",
        "/api/sync/pull",
        json!({"since_seq": 0}),
        Some(&alice_machine),
    )
    .await;
    assert_eq!(alice_pull.own_observations.len(), 1);
}

/// 不变量 #5:mode 降级(fork-allowed → read-only)写入 share_mode_downgrades 并通知 bob。
#[tokio::test]
async fn mode_downgrade_records_pending_notice() {
    let (app, _pool, alice_access, alice_machine, bob_access, bob_machine) =
        alice_bob_setup().await;
    let pid = alice_push_project(
        &app,
        &alice_machine,
        "01900000-0000-7000-8000-000000000501",
        "downgrade-test",
    )
    .await;
    let share = alice_share(&app, &alice_access, &pid, "bob", "fork-allowed").await;

    // alice patch → read-only(降级)
    let (status, _patched): (_, ShareResponse) = json_request(
        &app,
        "PATCH",
        &format!("/api/shares/{}", share.share.id),
        json!({"share_mode": "read-only"}),
        Some(&alice_access),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // bob pull,pending_downgrades 有 1 条
    let (_, bob_pull): (_, PullResponse) = json_request(
        &app,
        "POST",
        "/api/sync/pull",
        json!({"since_seq": 0}),
        Some(&bob_machine),
    )
    .await;
    assert_eq!(bob_pull.pending_downgrades.len(), 1);
    let notice = &bob_pull.pending_downgrades[0];
    assert_eq!(notice.old_mode, "fork-allowed");
    assert_eq!(notice.new_mode, "read-only");
    assert_eq!(notice.project_id, pid);

    // bob ack
    let ack_status = empty_request(
        &app,
        "POST",
        "/api/shared/notifications/ack",
        Some(json!({"downgrade_ids": [notice.id]})),
        Some(&bob_access),
    )
    .await;
    assert_eq!(ack_status, StatusCode::NO_CONTENT);

    // 再 pull,pending_downgrades 为空
    let (_, bob_pull2): (_, PullResponse) = json_request(
        &app,
        "POST",
        "/api/sync/pull",
        json!({"since_seq": 0}),
        Some(&bob_machine),
    )
    .await;
    assert_eq!(bob_pull2.pending_downgrades.len(), 0);
    // shared_observations 仍能看,只是变成 read-only
    assert_eq!(bob_pull2.shared_observations[0].share_mode, "read-only");
}

/// upgrade(read-only → fork-allowed)不应该写降级记录。
#[tokio::test]
async fn mode_upgrade_does_not_record_downgrade() {
    let (app, _pool, alice_access, alice_machine, _, bob_machine) =
        alice_bob_setup().await;
    let pid = alice_push_project(
        &app,
        &alice_machine,
        "01900000-0000-7000-8000-000000000601",
        "upgrade-test",
    )
    .await;
    let share = alice_share(&app, &alice_access, &pid, "bob", "read-only").await;

    json_request::<ShareResponse>(
        &app,
        "PATCH",
        &format!("/api/shares/{}", share.share.id),
        json!({"share_mode": "fork-allowed"}),
        Some(&alice_access),
    )
    .await;

    let (_, bob_pull): (_, PullResponse) = json_request(
        &app,
        "POST",
        "/api/sync/pull",
        json!({"since_seq": 0}),
        Some(&bob_machine),
    )
    .await;
    assert_eq!(
        bob_pull.pending_downgrades.len(),
        0,
        "upgrade should not generate a downgrade notice"
    );
    assert_eq!(bob_pull.shared_observations[0].share_mode, "fork-allowed");
}

/// 不变量 #1:非 owner 不能 share 别人的项目;owner 不能 share 给自己。
#[tokio::test]
async fn non_owner_cannot_share() {
    let (app, _pool, alice_access, alice_machine, bob_access, _bob_machine) =
        alice_bob_setup().await;
    let pid = alice_push_project(
        &app,
        &alice_machine,
        "01900000-0000-7000-8000-000000000701",
        "owned-by-alice",
    )
    .await;

    // bob 尝试 share alice 的项目 → 404 (因为他查不到 (bob, pid))
    let status = empty_request(
        &app,
        "POST",
        "/api/shares",
        Some(json!({
            "project_id": pid,
            "target_type": "user",
            "target_username": "alice",
            "share_mode": "read-only",
        })),
        Some(&bob_access),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // alice 不能 share 给自己
    let status2 = empty_request(
        &app,
        "POST",
        "/api/shares",
        Some(json!({
            "project_id": pid,
            "target_type": "user",
            "target_username": "alice",
            "share_mode": "read-only",
        })),
        Some(&alice_access),
    )
    .await;
    assert_eq!(status2, StatusCode::BAD_REQUEST);
}

/// link share 强制 read-only(匿名访客无法 fork)。
#[tokio::test]
async fn link_share_forced_to_read_only() {
    let (app, _pool, alice_access, alice_machine, _, _) = alice_bob_setup().await;
    let pid = alice_push_project(
        &app,
        &alice_machine,
        "01900000-0000-7000-8000-000000000801",
        "link-share",
    )
    .await;
    let (status, resp): (_, ShareResponse) = json_request(
        &app,
        "POST",
        "/api/shares",
        json!({
            "project_id": pid,
            "target_type": "link",
            "share_mode": "fork-allowed", // 故意传非 read-only
        }),
        Some(&alice_access),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(resp.share.share_mode, "read-only", "link 强制 read-only");
    assert!(resp.share.share_token.is_some());
    assert!(resp.share_url.is_some());
}

/// 不变量 #6 + #7:Bob fork 整个项目后,新项目归 Bob,内容是 alice 副本(derived_from 链)。
#[tokio::test]
async fn fork_allowed_lets_bob_fork_whole_project() {
    let (app, _pool, alice_access, alice_machine, bob_access, bob_machine) =
        alice_bob_setup().await;
    let pid = alice_push_project(
        &app,
        &alice_machine,
        "01900000-0000-7000-8000-000000001001",
        "fork-target",
    )
    .await;
    // alice 再 push 第二条,确认 fork 会拷多条
    json_request::<PushResponse>(
        &app,
        "POST",
        "/api/sync/push",
        json!({"observations": [{
            "id": "01900000-0000-7000-8000-000000001002",
            "timestamp": 1_700_000_100_i64,
            "project_marker_id": null,
            "project_name": "fork-target",
            "project_path": "/Users/alice/work/fork-target",
            "content": "second alice obs",
            "obs_type": null, "metadata": null,
            "derived_from": null, "derivation_chain": null
        }]}),
        Some(&alice_machine),
    )
    .await;

    alice_share(&app, &alice_access, &pid, "bob", "fork-allowed").await;

    // bob fork
    let (status, fork_resp): (_, ForkProjectResponse) = json_request(
        &app,
        "POST",
        &format!("/api/projects/{pid}/fork"),
        json!({}),
        Some(&bob_access),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(fork_resp.copied_observations, 2);
    assert!(fork_resp.project.name.starts_with("fork-target-fork-of-alice"));
    assert_eq!(fork_resp.project.forked_from_project.as_deref(), Some(pid.as_str()));

    // bob 看自己的 own_observations,应该有 2 条带 derived_from 的副本
    let (_, bob_pull): (_, PullResponse) = json_request(
        &app,
        "POST",
        "/api/sync/pull",
        json!({"since_seq": 0}),
        Some(&bob_machine),
    )
    .await;
    assert_eq!(bob_pull.own_observations.len(), 2);
    for obs in &bob_pull.own_observations {
        assert!(obs.derived_from.is_some(), "fork 副本必须有 derived_from");
        // derivation_chain 为 array
        let chain = obs.derivation_chain.as_ref().expect("chain present");
        assert!(chain.is_array());
    }

    // bob 在 GET /api/projects 应该看到 fork 后的项目
    let (_, list): (_, ListProjectsResponse) = json_request(
        &app,
        "GET",
        "/api/projects",
        json!({}),
        Some(&bob_access),
    )
    .await;
    assert_eq!(list.projects.len(), 1);
    assert_eq!(list.projects[0].forked_from_project.as_deref(), Some(pid.as_str()));
    assert_eq!(list.projects[0].observation_count, 2);
}

/// 不变量 #4(强化版):alice 撤销 share 后,bob 已 fork 的副本仍在 own_observations 中。
#[tokio::test]
async fn revoke_after_fork_keeps_bobs_copies() {
    let (app, _pool, alice_access, alice_machine, bob_access, bob_machine) =
        alice_bob_setup().await;
    let pid = alice_push_project(
        &app,
        &alice_machine,
        "01900000-0000-7000-8000-000000001101",
        "to-fork-and-revoke",
    )
    .await;
    let share = alice_share(&app, &alice_access, &pid, "bob", "fork-allowed").await;

    // bob fork
    let (_, fork): (_, ForkProjectResponse) = json_request(
        &app,
        "POST",
        &format!("/api/projects/{pid}/fork"),
        json!({}),
        Some(&bob_access),
    )
    .await;
    assert_eq!(fork.copied_observations, 1);

    // alice 撤销 share
    empty_request(
        &app,
        "DELETE",
        &format!("/api/shares/{}", share.share.id),
        None,
        Some(&alice_access),
    )
    .await;

    // bob 的 own_observations 仍含 fork 副本
    let (_, bob_pull): (_, PullResponse) = json_request(
        &app,
        "POST",
        "/api/sync/pull",
        json!({"since_seq": 0}),
        Some(&bob_machine),
    )
    .await;
    assert_eq!(
        bob_pull.own_observations.len(),
        1,
        "撤销 share 不影响 bob 已 fork 的副本"
    );
    assert!(bob_pull.shared_observations.is_empty());
    assert_eq!(bob_pull.revoked_shares.len(), 1);
}

/// read-only 模式不允许 fork(权限矩阵)。
#[tokio::test]
async fn read_only_share_forbids_fork() {
    let (app, _pool, alice_access, alice_machine, bob_access, _bob_machine) =
        alice_bob_setup().await;
    let pid = alice_push_project(
        &app,
        &alice_machine,
        "01900000-0000-7000-8000-000000001201",
        "read-only-no-fork",
    )
    .await;
    alice_share(&app, &alice_access, &pid, "bob", "read-only").await;

    let status = empty_request(
        &app,
        "POST",
        &format!("/api/projects/{pid}/fork"),
        Some(json!({})),
        Some(&bob_access),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// 单条 fork:fork-allowed 模式 OK,目标 project 必须是 forker 自己的。
#[tokio::test]
async fn fork_single_observation_into_bob_project() {
    let (app, _pool, alice_access, alice_machine, bob_access, bob_machine) =
        alice_bob_setup().await;
    let pid = alice_push_project(
        &app,
        &alice_machine,
        "01900000-0000-7000-8000-000000001301",
        "single-fork-src",
    )
    .await;
    alice_share(&app, &alice_access, &pid, "bob", "fork-allowed").await;

    // bob 先 push 一条,创建一个目标项目
    let (_, bob_push): (_, PushResponse) = json_request(
        &app,
        "POST",
        "/api/sync/push",
        json!({"observations":[{
            "id": "01900000-0000-7000-8000-000000001302",
            "timestamp": 1_700_000_500_i64,
            "project_marker_id": null,
            "project_name": "bobs-research",
            "project_path": "/home/bob/research",
            "content": "bob own obs",
            "obs_type": null, "metadata": null,
            "derived_from": null, "derivation_chain": null
        }]}),
        Some(&bob_machine),
    )
    .await;
    let bob_pid = bob_push.projects_resolved[0].project_id.clone().unwrap();

    // 单条 fork
    let (status, fork): (_, ForkObservationResponse) = json_request(
        &app,
        "POST",
        "/api/observations/01900000-0000-7000-8000-000000001301/fork",
        json!({"to_project_id": bob_pid}),
        Some(&bob_access),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(fork.observation.project_id.as_deref(), Some(bob_pid.as_str()));
    assert_eq!(
        fork.observation.derived_from.as_deref(),
        Some("01900000-0000-7000-8000-000000001301")
    );
}

/// GET /api/shared 列出别人共享给我的。
#[tokio::test]
async fn shared_endpoint_lists_received_projects() {
    let (app, _pool, alice_access, alice_machine, bob_access, _) =
        alice_bob_setup().await;
    let pid = alice_push_project(
        &app,
        &alice_machine,
        "01900000-0000-7000-8000-000000000901",
        "received-test",
    )
    .await;
    alice_share(&app, &alice_access, &pid, "bob", "read-only").await;

    let (status, resp): (_, ListSharedProjectsResponse) = json_request(
        &app,
        "GET",
        "/api/shared",
        json!({}),
        Some(&bob_access),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp.shared_projects.len(), 1);
    assert_eq!(resp.shared_projects[0].project_name, "received-test");
    assert_eq!(resp.shared_projects[0].owner_username, "alice");
    assert_eq!(resp.shared_projects[0].share_mode, "read-only");
    assert_eq!(resp.shared_projects[0].observation_count, 1);
}
