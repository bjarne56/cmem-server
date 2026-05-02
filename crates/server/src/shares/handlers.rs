//! /api/shares 端点。
//!
//! 不变量保证(对照 docs/PROJECT_SHARING.md 第 8 节):
//! - #1 owner 永远拥有完整权限:create 时校验 sharer 必须是 owner
//! - #4 撤销共享只影响 shared_view:revoke 仅 set revoked_at,不删 observations
//! - #5 mode 降级触发 share_mode_downgrades:patch_mode 时若新 mode 权限收紧则记一条

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::{DateTime, TimeZone, Utc};
use cmem_shared::{
    api::{
        AckDowngradesRequest, CreateShareRequest, ListSharedProjectsResponse,
        ListSharesResponse, PatchShareRequest, ShareResponse, ShareView, SharedProjectEntry,
    },
    models::UserBrief,
};
use uuid::Uuid;

use crate::{
    db::{audit, projects, shares, users},
    error::AppError,
    middleware::Principal,
    shares::permissions::{parse_db_mode, AccessLevel},
    state::AppState,
};

const ALLOWED_TARGETS: [&str; 3] = ["user", "public", "link"];
const ALLOWED_MODES: [&str; 3] = ["read-only", "fork-allowed", "auto-copy"];

fn ts_to_dt(ts: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(ts, 0)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap_or_default())
}

async fn share_to_view(
    state: &AppState,
    row: &shares::ShareRow,
) -> Result<ShareView, AppError> {
    let project = projects::find_any_by_id(&state.pool, &row.project_id)
        .await
        .map_err(AppError::Internal)?;
    let project_name = project.as_ref().map(|p| p.name.clone()).unwrap_or_default();
    let sharer_username = users::brief_by_id(&state.pool, &row.sharer_user_id)
        .await
        .map_err(AppError::Internal)?
        .map(|(_, n)| n)
        .unwrap_or_default();
    let target_user = if let Some(uid) = &row.target_user_id {
        users::brief_by_id(&state.pool, uid)
            .await
            .map_err(AppError::Internal)?
            .map(|(id, username)| UserBrief { id, username })
    } else {
        None
    };
    Ok(ShareView {
        id: row.id.clone(),
        project_id: row.project_id.clone(),
        project_name,
        sharer_user_id: row.sharer_user_id.clone(),
        sharer_username,
        target_type: row.target_type.clone(),
        target_user,
        share_token: row.share_token.clone(),
        share_mode: row.share_mode.clone(),
        expires_at: row.expires_at.map(ts_to_dt),
        created_at: ts_to_dt(row.created_at),
    })
}

/// POST /api/shares
pub async fn create(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<CreateShareRequest>,
) -> Result<(StatusCode, Json<ShareResponse>), AppError> {
    if !ALLOWED_TARGETS.contains(&req.target_type.as_str()) {
        return Err(AppError::Validation(format!(
            "target_type must be one of {ALLOWED_TARGETS:?}"
        )));
    }
    let mut share_mode = req.share_mode.clone();
    // link share 强制 read-only(匿名访问无法 fork)。
    if req.target_type == "link" && share_mode != "read-only" {
        tracing::warn!(
            requested_mode = %share_mode,
            "link share forced to read-only (anonymous viewer cannot fork)"
        );
        share_mode = "read-only".to_string();
    }
    if !ALLOWED_MODES.contains(&share_mode.as_str()) {
        return Err(AppError::Validation(format!(
            "share_mode must be one of {ALLOWED_MODES:?}"
        )));
    }

    // 必须是 project owner(不变量 #1)。
    let project = projects::find_by_id(&state.pool, principal.user_id(), &req.project_id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;

    let mut target_user_id: Option<String> = None;
    let mut share_token: Option<String> = None;

    match req.target_type.as_str() {
        "user" => {
            let username = req
                .target_username
                .as_ref()
                .ok_or_else(|| AppError::Validation("target_username required".into()))?;
            let target = users::brief_by_username(&state.pool, username)
                .await
                .map_err(AppError::Internal)?
                .ok_or(AppError::NotFound)?;
            if target.0 == project.user_id {
                return Err(AppError::Validation(
                    "cannot share a project to yourself".into(),
                ));
            }
            target_user_id = Some(target.0);
        }
        "link" => {
            // 32-char nanoid
            share_token =
                Some(nanoid::nanoid!(32, &nanoid::alphabet::SAFE));
        }
        "public" => {}
        _ => unreachable!(),
    }

    let id = Uuid::now_v7().to_string();
    let now = Utc::now().timestamp();
    let expires_at = req.expires_in_secs.map(|s| now + s);

    let create_res = shares::create(
        &state.pool,
        &id,
        &req.project_id,
        principal.user_id(),
        &req.target_type,
        target_user_id.as_deref(),
        share_token.as_deref(),
        &share_mode,
        expires_at,
        now,
    )
    .await;

    if let Err(e) = create_res {
        // 可能命中 UNIQUE (project_id, target_type, target_user_id) 冲突
        if let Some(sqlx::Error::Database(db)) = e.downcast_ref::<sqlx::Error>() {
            if db.message().contains("UNIQUE") {
                return Err(AppError::Conflict(
                    "share already exists for this target".into(),
                ));
            }
        }
        return Err(AppError::Internal(e));
    }

    audit::record(
        &state.pool,
        Some(principal.user_id()),
        None,
        "share.create",
        Some("project"),
        Some(&req.project_id),
        Some(
            &serde_json::json!({
                "target_type": req.target_type,
                "share_mode": share_mode,
            })
            .to_string(),
        ),
        None,
        None,
        now,
    )
    .await
    .map_err(AppError::Internal)?;

    let row = shares::find_by_id(&state.pool, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("share vanished after insert")))?;
    let view = share_to_view(&state, &row).await?;
    let share_url = if row.target_type == "link" {
        row.share_token
            .as_ref()
            .map(|t| format!("/p/{t}"))
    } else {
        None
    };
    Ok((
        StatusCode::CREATED,
        Json(ShareResponse {
            share: view,
            share_url,
        }),
    ))
}

/// GET /api/shares
pub async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<ListSharesResponse>, AppError> {
    let user_id = principal.user_id().to_string();
    let now = Utc::now().timestamp();
    let owned_rows = shares::list_owned(&state.pool, &user_id)
        .await
        .map_err(AppError::Internal)?;
    let received_rows = shares::list_received_active(&state.pool, &user_id, now)
        .await
        .map_err(AppError::Internal)?;
    let mut owned = Vec::with_capacity(owned_rows.len());
    for r in &owned_rows {
        owned.push(share_to_view(&state, r).await?);
    }
    let mut received = Vec::with_capacity(received_rows.len());
    for r in &received_rows {
        received.push(share_to_view(&state, r).await?);
    }
    Ok(Json(ListSharesResponse { owned, received }))
}

/// GET /api/shared:别人共享给我的项目 + pending downgrades。
pub async fn list_received(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<ListSharedProjectsResponse>, AppError> {
    let user_id = principal.user_id().to_string();
    let now = Utc::now().timestamp();

    let received = shares::list_received_active(&state.pool, &user_id, now)
        .await
        .map_err(AppError::Internal)?;
    let mut entries = Vec::with_capacity(received.len());
    for r in &received {
        let project = projects::find_any_by_id(&state.pool, &r.project_id)
            .await
            .map_err(AppError::Internal)?;
        let Some(project) = project else { continue };
        let owner_username = users::brief_by_id(&state.pool, &project.user_id)
            .await
            .map_err(AppError::Internal)?
            .map(|(_, u)| u)
            .unwrap_or_default();
        let cnt = projects::observation_count(&state.pool, &project.id)
            .await
            .map_err(AppError::Internal)?;
        entries.push(SharedProjectEntry {
            project_id: project.id,
            project_name: project.name,
            owner_username,
            share_mode: r.share_mode.clone(),
            observation_count: cnt,
            shared_at: ts_to_dt(r.created_at),
        });
    }

    let down_rows = shares::pending_downgrades(&state.pool, &user_id)
        .await
        .map_err(AppError::Internal)?;
    let mut downgrades = Vec::with_capacity(down_rows.len());
    for d in &down_rows {
        let project = projects::find_any_by_id(&state.pool, &d.project_id)
            .await
            .map_err(AppError::Internal)?;
        let (project_name, owner_username) = if let Some(p) = project {
            let owner = users::brief_by_id(&state.pool, &p.user_id)
                .await
                .map_err(AppError::Internal)?
                .map(|(_, u)| u)
                .unwrap_or_default();
            (p.name, owner)
        } else {
            ("(deleted)".to_string(), "unknown".to_string())
        };
        downgrades.push(cmem_shared::api::DowngradeNotice {
            id: d.id,
            project_id: d.project_id.clone(),
            project_name,
            owner_username,
            old_mode: d.old_mode.clone(),
            new_mode: d.new_mode.clone(),
            created_at: ts_to_dt(d.created_at),
        });
    }

    Ok(Json(ListSharedProjectsResponse {
        shared_projects: entries,
        pending_downgrades: downgrades,
    }))
}

/// PATCH /api/shares/:id
///
/// 改 share_mode 或 expires_at;若新 mode 权限收紧 → 写入 share_mode_downgrades(不变量 #5)。
pub async fn patch(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    Json(req): Json<PatchShareRequest>,
) -> Result<Json<ShareResponse>, AppError> {
    let row = shares::find_by_id(&state.pool, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;

    if row.sharer_user_id != principal.user_id() {
        return Err(AppError::Forbidden);
    }

    if let Some(m) = &req.share_mode {
        if !ALLOWED_MODES.contains(&m.as_str()) {
            return Err(AppError::Validation(format!(
                "share_mode must be one of {ALLOWED_MODES:?}"
            )));
        }
    }

    let now = Utc::now().timestamp();
    let new_expires = req.expires_in_secs.map(|s| Some(now + s));

    // 检测降级:仅 user share 才会写降级记录(public/link 没有具体 user 通知对象)
    let mut downgrade_to_record: Option<(String, String, String)> = None; // (old, new, target_user_id)
    if let (Some(new_mode), Some(target_uid)) = (&req.share_mode, &row.target_user_id) {
        let old_level =
            parse_db_mode(&row.share_mode).unwrap_or(AccessLevel::None);
        let new_level = parse_db_mode(new_mode).unwrap_or(AccessLevel::None);
        if new_level.rank() > old_level.rank() {
            downgrade_to_record =
                Some((row.share_mode.clone(), new_mode.clone(), target_uid.clone()));
        }
    }

    shares::update_mode_and_expiry(
        &state.pool,
        &id,
        req.share_mode.as_deref(),
        new_expires,
    )
    .await
    .map_err(AppError::Internal)?;

    if let Some((old, new, target_uid)) = downgrade_to_record {
        shares::record_downgrade(&state.pool, &row.project_id, &target_uid, &old, &new, now)
            .await
            .map_err(AppError::Internal)?;
        audit::record(
            &state.pool,
            Some(principal.user_id()),
            None,
            "share.mode_downgrade",
            Some("share"),
            Some(&id),
            Some(
                &serde_json::json!({
                    "old_mode": old,
                    "new_mode": new,
                })
                .to_string(),
            ),
            None,
            None,
            now,
        )
        .await
        .map_err(AppError::Internal)?;
    } else {
        audit::record(
            &state.pool,
            Some(principal.user_id()),
            None,
            "share.update",
            Some("share"),
            Some(&id),
            None,
            None,
            None,
            now,
        )
        .await
        .map_err(AppError::Internal)?;
    }

    let updated = shares::find_by_id(&state.pool, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    let view = share_to_view(&state, &updated).await?;
    Ok(Json(ShareResponse {
        share: view,
        share_url: None,
    }))
}

/// DELETE /api/shares/:id
pub async fn revoke(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let row = shares::find_by_id(&state.pool, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    if row.sharer_user_id != principal.user_id() {
        return Err(AppError::Forbidden);
    }
    let now = Utc::now().timestamp();
    shares::revoke(&state.pool, &id, now)
        .await
        .map_err(AppError::Internal)?;
    audit::record(
        &state.pool,
        Some(principal.user_id()),
        None,
        "share.revoke",
        Some("share"),
        Some(&id),
        None,
        None,
        None,
        now,
    )
    .await
    .map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/shared/notifications/ack
pub async fn ack_downgrades(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<AckDowngradesRequest>,
) -> Result<StatusCode, AppError> {
    let now = Utc::now().timestamp();
    shares::ack_downgrades(&state.pool, principal.user_id(), &req.downgrade_ids, now)
        .await
        .map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}
