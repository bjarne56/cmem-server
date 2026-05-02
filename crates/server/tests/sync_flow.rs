//! M3 集成测试:机器注册 + push/pull(无项目识别)。

mod common;

use axum::http::StatusCode;
use cmem_shared::api::{ListMachinesResponse, PullResponse, PushResponse};
use serde_json::json;

use common::*;

#[tokio::test]
async fn machine_register_lists_and_revokes() {
    let (app, _pool) = make_app().await;
    let _ = register_user(&app, "alice").await;
    let login = login_user(&app, "alice").await;

    let m = register_machine(&app, &login.access_token, "alice-mac").await;
    assert!(m.machine_token.starts_with("cmt_"));
    assert_eq!(m.machine.name, "alice-mac");

    let (status, list): (_, ListMachinesResponse) = json_request(
        &app,
        "GET",
        "/api/machines",
        json!({}),
        Some(&login.access_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.machines.len(), 1);
    assert_eq!(list.machines[0].id, m.machine.id);

    let revoke_status = empty_request(
        &app,
        "DELETE",
        &format!("/api/machines/{}", m.machine.id),
        None,
        Some(&login.access_token),
    )
    .await;
    assert_eq!(revoke_status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn two_machines_push_and_cross_pull() {
    let (app, _pool) = make_app().await;
    let _ = register_user(&app, "alice").await;
    let login = login_user(&app, "alice").await;

    let mac = register_machine(&app, &login.access_token, "alice-mac").await;
    let linux = register_machine(&app, &login.access_token, "alice-linux").await;

    // Mac push 1 条
    let push_body = json!({
        "observations": [
            {
                "id": "01900000-0000-7000-8000-000000000001",
                "timestamp": 1_700_000_000,
                "project_marker_id": null,
                "project_name": null,
                "project_path": null,
                "content": "decision from mac",
                "obs_type": "decision",
                "metadata": null,
                "derived_from": null,
                "derivation_chain": null
            }
        ]
    });
    let (status, push_resp): (_, PushResponse) = json_request(
        &app,
        "POST",
        "/api/sync/push",
        push_body,
        Some(&mac.machine_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(push_resp.accepted, 1);
    assert_eq!(push_resp.duplicates, 0);

    // Linux push 1 条
    let push_body2 = json!({
        "observations": [
            {
                "id": "01900000-0000-7000-8000-000000000002",
                "timestamp": 1_700_000_100,
                "project_marker_id": null,
                "project_name": null,
                "project_path": null,
                "content": "observation from linux",
                "obs_type": "observation",
                "metadata": null,
                "derived_from": null,
                "derivation_chain": null
            }
        ]
    });
    let (_, push_resp2): (_, PushResponse) = json_request(
        &app,
        "POST",
        "/api/sync/push",
        push_body2,
        Some(&linux.machine_token),
    )
    .await;
    assert_eq!(push_resp2.accepted, 1);

    // Mac pull(应该看到 own_observations 包含 Linux 的那条)
    let pull_body = json!({"since_seq": 0});
    let (status, pull_resp): (_, PullResponse) = json_request(
        &app,
        "POST",
        "/api/sync/pull",
        pull_body.clone(),
        Some(&mac.machine_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pull_resp.own_observations.len(), 2);
    let ids: Vec<&str> = pull_resp
        .own_observations
        .iter()
        .map(|o| o.id.as_str())
        .collect();
    assert!(ids.contains(&"01900000-0000-7000-8000-000000000001"));
    assert!(ids.contains(&"01900000-0000-7000-8000-000000000002"));
    assert!(pull_resp.shared_observations.is_empty());
}

#[tokio::test]
async fn push_dedup_by_id() {
    let (app, _pool) = make_app().await;
    let _ = register_user(&app, "alice").await;
    let login = login_user(&app, "alice").await;
    let mac = register_machine(&app, &login.access_token, "alice-mac").await;

    let push_body = json!({
        "observations": [
            {
                "id": "01900000-0000-7000-8000-000000000abc",
                "timestamp": 1_700_000_000,
                "project_marker_id": null,
                "project_name": null,
                "project_path": null,
                "content": "first",
                "obs_type": null,
                "metadata": null,
                "derived_from": null,
                "derivation_chain": null
            }
        ]
    });
    let (_, p1): (_, PushResponse) = json_request(
        &app,
        "POST",
        "/api/sync/push",
        push_body.clone(),
        Some(&mac.machine_token),
    )
    .await;
    assert_eq!(p1.accepted, 1);
    assert_eq!(p1.duplicates, 0);

    // 重复 push 同 id
    let (_, p2): (_, PushResponse) = json_request(
        &app,
        "POST",
        "/api/sync/push",
        push_body,
        Some(&mac.machine_token),
    )
    .await;
    assert_eq!(p2.accepted, 0);
    assert_eq!(p2.duplicates, 1);
}

#[tokio::test]
async fn pull_filters_other_users() {
    let (app, _pool) = make_app().await;
    register_user(&app, "alice").await;
    register_user(&app, "bob").await;
    let alice = login_user(&app, "alice").await;
    let bob = login_user(&app, "bob").await;
    let alice_mac = register_machine(&app, &alice.access_token, "alice-mac").await;
    let bob_mac = register_machine(&app, &bob.access_token, "bob-mac").await;

    // Alice push
    let body = json!({
        "observations": [
            {
                "id": "01900000-0000-7000-8000-0000000000a1",
                "timestamp": 1,
                "project_marker_id": null, "project_name": null, "project_path": null,
                "content": "alice secret",
                "obs_type": null, "metadata": null, "derived_from": null, "derivation_chain": null
            }
        ]
    });
    json_request::<PushResponse>(
        &app,
        "POST",
        "/api/sync/push",
        body,
        Some(&alice_mac.machine_token),
    )
    .await;

    // Bob pull —— 不应该看到 Alice 的数据
    let (_, bob_pull): (_, PullResponse) = json_request(
        &app,
        "POST",
        "/api/sync/pull",
        json!({"since_seq": 0}),
        Some(&bob_mac.machine_token),
    )
    .await;
    assert_eq!(bob_pull.own_observations.len(), 0);
    assert_eq!(bob_pull.shared_observations.len(), 0);
}
