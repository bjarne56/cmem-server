//! M4 集成测试:项目识别四个 case + project CRUD。

mod common;

use axum::http::StatusCode;
use cmem_shared::api::{
    ListProjectsResponse, ProjectResponse, PullResponse, PushResponse,
};
use serde_json::json;

use common::*;

fn obs(id: &str, marker: Option<&str>, name: Option<&str>, path: Option<&str>) -> serde_json::Value {
    json!({
        "id": id,
        "timestamp": 1_700_000_000_i64,
        "project_marker_id": marker,
        "project_name": name,
        "project_path": path,
        "content": "test obs",
        "obs_type": null,
        "metadata": null,
        "derived_from": null,
        "derivation_chain": null
    })
}

/// case 1:Mac path 无 marker + Linux path 无 marker,name 一样 → 合并到同一项目。
#[tokio::test]
async fn case1_same_name_no_marker_merges_across_machines() {
    let (app, _pool) = make_app().await;
    register_user(&app, "alice").await;
    let alice = login_user(&app, "alice").await;
    let mac = register_machine(&app, &alice.access_token, "alice-mac").await;
    let linux = register_machine(&app, &alice.access_token, "alice-linux").await;

    // Mac push: name = "nginx-rce", path = /Users/alice/work/nginx-rce
    let body = json!({
        "observations": [
            obs(
                "01900000-0000-7000-8000-000000000001",
                None,
                Some("nginx-rce"),
                Some("/Users/alice/work/nginx-rce"),
            )
        ]
    });
    let (_, mac_push): (_, PushResponse) = json_request(
        &app,
        "POST",
        "/api/sync/push",
        body,
        Some(&mac.machine_token),
    )
    .await;
    assert_eq!(mac_push.accepted, 1);
    let mac_pid = mac_push.projects_resolved[0]
        .project_id
        .clone()
        .expect("mac assigns project_id");

    // Linux push: 同 name = "nginx-rce", 不同 path
    let body = json!({
        "observations": [
            obs(
                "01900000-0000-7000-8000-000000000002",
                None,
                Some("nginx-rce"),
                Some("/home/alice/projects/nginx-rce"),
            )
        ]
    });
    let (_, linux_push): (_, PushResponse) = json_request(
        &app,
        "POST",
        "/api/sync/push",
        body,
        Some(&linux.machine_token),
    )
    .await;
    let linux_pid = linux_push.projects_resolved[0]
        .project_id
        .clone()
        .expect("linux assigns project_id");
    assert_eq!(mac_pid, linux_pid, "case 1: same name should merge");

    // 验证 GET /api/projects 显示 1 个项目,2 paths
    let (_, list): (_, ListProjectsResponse) = json_request(
        &app,
        "GET",
        "/api/projects",
        json!({}),
        Some(&alice.access_token),
    )
    .await;
    assert_eq!(list.projects.len(), 1);
    assert_eq!(list.projects[0].id, mac_pid);
    assert_eq!(list.projects[0].name, "nginx-rce");
    assert_eq!(list.projects[0].observation_count, 2);
    assert_eq!(list.projects[0].paths.len(), 2);
}

/// case 2:Mac 有 marker (project_id=P2) + Linux 无 marker → 都合并到 P2。
#[tokio::test]
async fn case2_marker_then_no_marker_merges_to_marker_project() {
    let (app, _pool) = make_app().await;
    register_user(&app, "alice").await;
    let alice = login_user(&app, "alice").await;
    let mac = register_machine(&app, &alice.access_token, "alice-mac").await;
    let linux = register_machine(&app, &alice.access_token, "alice-linux").await;

    let p2 = "01900000-0000-7000-8000-0000000000aa";

    // Mac push: marker_id=P2, name=nginx-rce
    let body = json!({
        "observations": [
            obs(
                "01900000-0000-7000-8000-000000000010",
                Some(p2),
                Some("nginx-rce"),
                Some("/Users/alice/work/nginx-rce"),
            )
        ]
    });
    let (_, mac_push): (_, PushResponse) = json_request(
        &app,
        "POST",
        "/api/sync/push",
        body,
        Some(&mac.machine_token),
    )
    .await;
    assert_eq!(mac_push.projects_resolved[0].project_id.as_deref(), Some(p2));

    // Linux push: 无 marker,但 name = nginx-rce → 应该合并到 P2(同 name)
    let body = json!({
        "observations": [
            obs(
                "01900000-0000-7000-8000-000000000011",
                None,
                Some("nginx-rce"),
                Some("/home/alice/projects/nginx-rce"),
            )
        ]
    });
    let (_, linux_push): (_, PushResponse) = json_request(
        &app,
        "POST",
        "/api/sync/push",
        body,
        Some(&linux.machine_token),
    )
    .await;
    assert_eq!(
        linux_push.projects_resolved[0].project_id.as_deref(),
        Some(p2),
        "case 2: linux without marker should merge into P2"
    );
}

/// case 3:Mac "nginx-rce" + Linux "Nginx RCE" → 规范化后合并。
#[tokio::test]
async fn case3_normalization_merges_different_capitalization() {
    let (app, _pool) = make_app().await;
    register_user(&app, "alice").await;
    let alice = login_user(&app, "alice").await;
    let mac = register_machine(&app, &alice.access_token, "alice-mac").await;
    let linux = register_machine(&app, &alice.access_token, "alice-linux").await;

    let body_mac = json!({
        "observations": [
            obs(
                "01900000-0000-7000-8000-000000000020",
                None,
                Some("nginx-rce"),
                Some("/Users/alice/work/nginx-rce"),
            )
        ]
    });
    let (_, mac_push): (_, PushResponse) = json_request(
        &app,
        "POST",
        "/api/sync/push",
        body_mac,
        Some(&mac.machine_token),
    )
    .await;
    let mac_pid = mac_push.projects_resolved[0].project_id.clone().unwrap();

    let body_linux = json!({
        "observations": [
            obs(
                "01900000-0000-7000-8000-000000000021",
                None,
                Some("Nginx RCE"),
                Some("/home/alice/projects/nginx-rce"),
            )
        ]
    });
    let (_, linux_push): (_, PushResponse) = json_request(
        &app,
        "POST",
        "/api/sync/push",
        body_linux,
        Some(&linux.machine_token),
    )
    .await;
    let linux_pid = linux_push.projects_resolved[0].project_id.clone().unwrap();

    assert_eq!(mac_pid, linux_pid, "case 3: capitalization should normalize");

    // 名字按规范化保存
    let (_, list): (_, ListProjectsResponse) = json_request(
        &app,
        "GET",
        "/api/projects",
        json!({}),
        Some(&alice.access_token),
    )
    .await;
    assert_eq!(list.projects.len(), 1);
    assert_eq!(list.projects[0].name, "nginx-rce");
}

/// case 4:用户主动用不同 name 区分两个项目。
#[tokio::test]
async fn case4_explicit_separate_names_stay_separate() {
    let (app, _pool) = make_app().await;
    register_user(&app, "alice").await;
    let alice = login_user(&app, "alice").await;
    let mac = register_machine(&app, &alice.access_token, "alice-mac").await;

    // 项目 v1
    let body = json!({
        "observations": [
            obs(
                "01900000-0000-7000-8000-000000000030",
                None,
                Some("nginx-rce"),
                Some("/Users/alice/work/nginx-rce"),
            )
        ]
    });
    let (_, p1_push): (_, PushResponse) = json_request(
        &app,
        "POST",
        "/api/sync/push",
        body,
        Some(&mac.machine_token),
    )
    .await;

    // 项目 v2 —— 用户在 .cmem-project.toml 改 name = nginx-rce-v2
    let body = json!({
        "observations": [
            obs(
                "01900000-0000-7000-8000-000000000031",
                None,
                Some("nginx-rce-v2"),
                Some("/Users/alice/work/nginx-rce-2026"),
            )
        ]
    });
    let (_, p2_push): (_, PushResponse) = json_request(
        &app,
        "POST",
        "/api/sync/push",
        body,
        Some(&mac.machine_token),
    )
    .await;

    let p1 = p1_push.projects_resolved[0].project_id.clone().unwrap();
    let p2 = p2_push.projects_resolved[0].project_id.clone().unwrap();
    assert_ne!(p1, p2, "case 4: explicit separate names must NOT merge");

    // GET /api/projects 应该有 2 个项目
    let (_, list): (_, ListProjectsResponse) = json_request(
        &app,
        "GET",
        "/api/projects",
        json!({}),
        Some(&alice.access_token),
    )
    .await;
    assert_eq!(list.projects.len(), 2);
}

/// 项目无 name 也无 marker → project_id 为 null(孤立 observation)。
#[tokio::test]
async fn obs_without_project_info_yields_null_project_id() {
    let (app, _pool) = make_app().await;
    register_user(&app, "alice").await;
    let alice = login_user(&app, "alice").await;
    let mac = register_machine(&app, &alice.access_token, "alice-mac").await;

    let body = json!({
        "observations": [
            obs(
                "01900000-0000-7000-8000-000000000040",
                None,
                None,
                None,
            )
        ]
    });
    let (_, push): (_, PushResponse) = json_request(
        &app,
        "POST",
        "/api/sync/push",
        body,
        Some(&mac.machine_token),
    )
    .await;
    assert_eq!(push.accepted, 1);
    assert!(push.projects_resolved[0].project_id.is_none());

    // pull 回来,project_id 应该是 None
    let (_, pull): (_, PullResponse) = json_request(
        &app,
        "POST",
        "/api/sync/pull",
        json!({"since_seq": 0}),
        Some(&mac.machine_token),
    )
    .await;
    assert_eq!(pull.own_observations.len(), 1);
    assert!(pull.own_observations[0].project_id.is_none());
}

/// PATCH /api/projects/:id rename + exclude
#[tokio::test]
async fn project_patch_rename_and_exclude() {
    let (app, _pool) = make_app().await;
    register_user(&app, "alice").await;
    let alice = login_user(&app, "alice").await;
    let mac = register_machine(&app, &alice.access_token, "alice-mac").await;

    // push 一条创建项目
    let body = json!({
        "observations": [
            obs(
                "01900000-0000-7000-8000-000000000050",
                None,
                Some("research"),
                Some("/home/alice/research"),
            )
        ]
    });
    let (_, push): (_, PushResponse) = json_request(
        &app,
        "POST",
        "/api/sync/push",
        body,
        Some(&mac.machine_token),
    )
    .await;
    let pid = push.projects_resolved[0].project_id.clone().unwrap();

    // rename
    let (status, patched): (_, ProjectResponse) = json_request(
        &app,
        "PATCH",
        &format!("/api/projects/{pid}"),
        json!({"name": "Research 2026"}),
        Some(&alice.access_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(patched.project.name, "research-2026");

    // exclude
    let (_, patched2): (_, ProjectResponse) = json_request(
        &app,
        "PATCH",
        &format!("/api/projects/{pid}"),
        json!({"is_excluded": true}),
        Some(&alice.access_token),
    )
    .await;
    assert!(patched2.project.is_excluded);
}

/// DELETE /api/projects/:id 级联删除 observation
#[tokio::test]
async fn project_delete_cascades_observations() {
    let (app, _pool) = make_app().await;
    register_user(&app, "alice").await;
    let alice = login_user(&app, "alice").await;
    let mac = register_machine(&app, &alice.access_token, "alice-mac").await;

    let body = json!({
        "observations": [
            obs(
                "01900000-0000-7000-8000-000000000060",
                None,
                Some("doomed"),
                Some("/tmp/doomed"),
            )
        ]
    });
    let (_, push): (_, PushResponse) = json_request(
        &app,
        "POST",
        "/api/sync/push",
        body,
        Some(&mac.machine_token),
    )
    .await;
    let pid = push.projects_resolved[0].project_id.clone().unwrap();

    let status = empty_request(
        &app,
        "DELETE",
        &format!("/api/projects/{pid}"),
        None,
        Some(&alice.access_token),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, pull): (_, PullResponse) = json_request(
        &app,
        "POST",
        "/api/sync/pull",
        json!({"since_seq": 0}),
        Some(&mac.machine_token),
    )
    .await;
    // observations 也被级联删
    assert_eq!(pull.own_observations.len(), 0);
}
