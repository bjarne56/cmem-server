//! Admin REST API handlers(JSON,挂在 `/api/admin/*`)。
//!
//! 所有 handler 都假设 `require_admin` middleware 已经注入 `AdminPrincipal`。

use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    admin::middleware::AdminPrincipal,
    auth::password::hash_password,
    db::{audit, invites, observations, projects, shares, stats, users},
    error::AppError,
    state::AppState,
};

// ---------- 通用 helper ----------

fn rand_password() -> String {
    use rand::RngCore;
    let mut buf = [0u8; 18];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    // base64-url 风格,无填充,直接给人看
    let mut s = String::with_capacity(24);
    const ALPHA: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    for &b in &buf {
        s.push(ALPHA[(b as usize) % ALPHA.len()] as char);
    }
    s
}

#[derive(Debug, Deserialize)]
pub struct PageParams {
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

impl PageParams {
    fn limit(&self) -> i64 {
        self.limit.unwrap_or(50).clamp(1, 500)
    }
    fn offset(&self) -> i64 {
        self.offset.unwrap_or(0).max(0)
    }
}

// ---------- /api/admin/stats ----------

#[derive(Debug, Serialize)]
pub struct AdminStatsResponse {
    pub users: i64,
    pub machines: i64,
    pub projects: i64,
    pub observations: i64,
    pub active_shares: i64,
    pub invites: i64,
    pub recent: Recent24h,
}

#[derive(Debug, Serialize)]
pub struct Recent24h {
    pub users: i64,
    pub projects: i64,
    pub observations: i64,
    pub audit_events: i64,
}

pub async fn get_stats(
    State(state): State<AppState>,
) -> Result<Json<AdminStatsResponse>, AppError> {
    let g = stats::collect(&state.pool).await.map_err(AppError::Internal)?;
    let now = Utc::now().timestamp();
    let since = now - 86_400;
    let r = Recent24h {
        users: users::count_recent_users(&state.pool, since).await.map_err(AppError::Internal)?,
        projects: projects::count_recent(&state.pool, since).await.map_err(AppError::Internal)?,
        observations: observations::count_recent(&state.pool, since)
            .await
            .map_err(AppError::Internal)?,
        audit_events: audit::count_recent(&state.pool, since)
            .await
            .map_err(AppError::Internal)?,
    };
    Ok(Json(AdminStatsResponse {
        users: g.users,
        machines: g.machines,
        projects: g.projects,
        observations: g.observations,
        active_shares: g.shares,
        invites: g.invites,
        recent: r,
    }))
}

// ---------- /api/admin/users ----------

#[derive(Debug, Serialize)]
pub struct AdminUserView {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub is_admin: bool,
    pub is_active: bool,
    pub created_at: i64,
    pub last_login_at: Option<i64>,
    pub registration_ip: Option<String>,
    pub last_login_ip: Option<String>,
    pub machine_count: i64,
    pub project_count: i64,
    pub observation_count: i64,
}

pub async fn list_users(
    State(state): State<AppState>,
    Query(p): Query<PageParams>,
) -> Result<Json<Vec<AdminUserView>>, AppError> {
    let rows = users::list_paged(&state.pool, p.q.as_deref(), p.limit(), p.offset())
        .await
        .map_err(AppError::Internal)?;
    let view = rows
        .into_iter()
        .map(|r| AdminUserView {
            id: r.id,
            username: r.username,
            email: r.email,
            is_admin: r.is_admin != 0,
            is_active: r.is_active != 0,
            created_at: r.created_at,
            last_login_at: r.last_login_at,
            registration_ip: r.registration_ip,
            last_login_ip: r.last_login_ip,
            machine_count: r.machine_count,
            project_count: r.project_count,
            observation_count: r.observation_count,
        })
        .collect();
    Ok(Json(view))
}

#[derive(Debug, Deserialize)]
pub struct CreateUserBody {
    pub username: String,
    pub password: String,
    pub email: Option<String>,
    #[serde(default)]
    pub is_admin: bool,
}

#[derive(Debug, Serialize)]
pub struct CreateUserResponse {
    pub id: String,
    pub username: String,
}

pub async fn create_user(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Json(body): Json<CreateUserBody>,
) -> Result<(StatusCode, Json<CreateUserResponse>), AppError> {
    if body.username.trim().is_empty() {
        return Err(AppError::Validation("username required".into()));
    }
    if body.password.chars().count() < 8 {
        return Err(AppError::Validation("password must be ≥8 chars".into()));
    }
    if users::find_by_username(&state.pool, &body.username)
        .await
        .map_err(AppError::Internal)?
        .is_some()
    {
        return Err(AppError::Conflict("username already taken".into()));
    }
    let id = Uuid::now_v7().to_string();
    let now = Utc::now().timestamp();
    let hash = hash_password(&body.password, &state.config.auth).map_err(AppError::Internal)?;
    users::create_user(
        &state.pool,
        &id,
        &body.username,
        &hash,
        body.email.as_deref(),
        body.is_admin,
        now,
    )
    .await
    .map_err(AppError::Internal)?;
    audit::record(
        &state.pool,
        Some(&admin.user_id),
        None,
        "admin.user_create",
        Some("user"),
        Some(&id),
        Some(&serde_json::json!({ "is_admin": body.is_admin }).to_string()),
        None,
        None,
        now,
    )
    .await
    .map_err(AppError::Internal)?;
    Ok((
        StatusCode::CREATED,
        Json(CreateUserResponse {
            id,
            username: body.username,
        }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct PatchUserBody {
    pub is_admin: Option<bool>,
    pub is_active: Option<bool>,
    pub password: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PatchUserResponse {
    pub ok: bool,
    /// 若调用方提供了 password=None,但传 reset=true(用 query),则在这里返回新随机密码。
    /// 当前 API 简化:只接受显式 password。
    pub new_password: Option<String>,
}

pub async fn patch_user(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Path(id): Path<String>,
    Json(body): Json<PatchUserBody>,
) -> Result<Json<PatchUserResponse>, AppError> {
    let target = users::find_by_id(&state.pool, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;

    let now = Utc::now().timestamp();

    // 不允许把最后一个活跃 admin 降级 / 禁用
    let demoting = body.is_admin == Some(false) && target.is_admin != 0;
    let disabling = body.is_active == Some(false) && target.is_active != 0;
    if demoting || disabling {
        let active_admins = users::count_active_admins(&state.pool)
            .await
            .map_err(AppError::Internal)?;
        if target.is_admin != 0 && target.is_active != 0 && active_admins <= 1 {
            return Err(AppError::Conflict(
                "cannot demote/disable the last active admin".into(),
            ));
        }
    }

    if let Some(v) = body.is_admin {
        users::set_admin(&state.pool, &id, v).await.map_err(AppError::Internal)?;
        audit::record(
            &state.pool,
            Some(&admin.user_id),
            None,
            if v { "admin.user_promote" } else { "admin.user_demote" },
            Some("user"),
            Some(&id),
            None,
            None,
            None,
            now,
        )
        .await
        .map_err(AppError::Internal)?;
    }
    if let Some(v) = body.is_active {
        users::set_active(&state.pool, &id, v).await.map_err(AppError::Internal)?;
        audit::record(
            &state.pool,
            Some(&admin.user_id),
            None,
            if v { "admin.user_enable" } else { "admin.user_disable" },
            Some("user"),
            Some(&id),
            None,
            None,
            None,
            now,
        )
        .await
        .map_err(AppError::Internal)?;
    }

    let mut new_password: Option<String> = None;
    if let Some(pw) = body.password.as_deref() {
        if pw.chars().count() < 8 {
            return Err(AppError::Validation("password must be ≥8 chars".into()));
        }
        let hash = hash_password(pw, &state.config.auth).map_err(AppError::Internal)?;
        users::update_password_hash(&state.pool, &id, &hash)
            .await
            .map_err(AppError::Internal)?;
        // 改密后吊销所有 refresh token
        crate::db::tokens::revoke_all_for_user(&state.pool, &id)
            .await
            .map_err(AppError::Internal)?;
        audit::record(
            &state.pool,
            Some(&admin.user_id),
            None,
            "admin.password_reset",
            Some("user"),
            Some(&id),
            None,
            None,
            None,
            now,
        )
        .await
        .map_err(AppError::Internal)?;
        new_password = Some(pw.to_string());
    }

    Ok(Json(PatchUserResponse {
        ok: true,
        new_password,
    }))
}

/// POST /api/admin/users/:id/reset-password — 生成随机新密码并返回。
pub async fn reset_user_password(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Path(id): Path<String>,
) -> Result<Json<PatchUserResponse>, AppError> {
    let user = users::find_by_id(&state.pool, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    let new_pw = rand_password();
    let hash = hash_password(&new_pw, &state.config.auth).map_err(AppError::Internal)?;
    users::update_password_hash(&state.pool, &user.id, &hash)
        .await
        .map_err(AppError::Internal)?;
    crate::db::tokens::revoke_all_for_user(&state.pool, &user.id)
        .await
        .map_err(AppError::Internal)?;
    audit::record(
        &state.pool,
        Some(&admin.user_id),
        None,
        "admin.password_reset",
        Some("user"),
        Some(&user.id),
        None,
        None,
        None,
        Utc::now().timestamp(),
    )
    .await
    .map_err(AppError::Internal)?;
    Ok(Json(PatchUserResponse {
        ok: true,
        new_password: Some(new_pw),
    }))
}

pub async fn delete_user(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let target = users::find_by_id(&state.pool, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;

    // 防止删最后一个活跃 admin
    if target.is_admin != 0 && target.is_active != 0 {
        let active_admins = users::count_active_admins(&state.pool)
            .await
            .map_err(AppError::Internal)?;
        if active_admins <= 1 {
            return Err(AppError::Conflict(
                "cannot delete the last active admin".into(),
            ));
        }
    }
    // 也禁止 admin 删自己(避免锁死)
    if target.id == admin.user_id {
        return Err(AppError::Conflict("admin cannot delete self".into()));
    }
    let removed = users::delete_by_id(&state.pool, &id)
        .await
        .map_err(AppError::Internal)?;
    if !removed {
        return Err(AppError::NotFound);
    }
    audit::record(
        &state.pool,
        Some(&admin.user_id),
        None,
        "admin.user_delete",
        Some("user"),
        Some(&id),
        None,
        None,
        None,
        Utc::now().timestamp(),
    )
    .await
    .map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- /api/admin/invites ----------

#[derive(Debug, Serialize)]
pub struct AdminInviteView {
    pub code: String,
    pub max_uses: i64,
    pub use_count: i64,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub used_by: Option<String>,
    pub status: &'static str,
}

pub async fn list_invites(
    State(state): State<AppState>,
) -> Result<Json<Vec<AdminInviteView>>, AppError> {
    let rows = invites::list_all(&state.pool).await.map_err(AppError::Internal)?;
    let now = Utc::now().timestamp();
    let view = rows
        .into_iter()
        .map(|r| {
            let status = if r.use_count >= r.max_uses {
                "exhausted"
            } else if r.expires_at.map(|e| e <= now).unwrap_or(false) {
                "expired"
            } else {
                "active"
            };
            AdminInviteView {
                code: r.code,
                max_uses: r.max_uses,
                use_count: r.use_count,
                created_at: r.created_at,
                expires_at: r.expires_at,
                used_by: r.used_by,
                status,
            }
        })
        .collect();
    Ok(Json(view))
}

#[derive(Debug, Deserialize)]
pub struct CreateInviteBody {
    #[serde(default = "default_one")]
    pub max_uses: i64,
    pub expires_days: Option<i64>,
}

fn default_one() -> i64 {
    1
}

#[derive(Debug, Serialize)]
pub struct CreateInviteResponse {
    pub code: String,
    pub expires_at: Option<i64>,
}

pub async fn create_invite(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Json(body): Json<CreateInviteBody>,
) -> Result<(StatusCode, Json<CreateInviteResponse>), AppError> {
    if body.max_uses < 1 {
        return Err(AppError::Validation("max_uses must be ≥1".into()));
    }
    let code = nanoid::nanoid!(32);
    let now = Utc::now().timestamp();
    let expires_at = body.expires_days.map(|d| now + d * 86_400);
    invites::create(
        &state.pool,
        &code,
        Some(&admin.user_id),
        now,
        expires_at,
        body.max_uses,
    )
    .await
    .map_err(AppError::Internal)?;
    audit::record(
        &state.pool,
        Some(&admin.user_id),
        None,
        "admin.invite_create",
        Some("invite"),
        Some(&code),
        None,
        None,
        None,
        now,
    )
    .await
    .map_err(AppError::Internal)?;
    Ok((
        StatusCode::CREATED,
        Json(CreateInviteResponse {
            code,
            expires_at,
        }),
    ))
}

pub async fn revoke_invite(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Path(code): Path<String>,
) -> Result<StatusCode, AppError> {
    let removed = invites::revoke(&state.pool, &code)
        .await
        .map_err(AppError::Internal)?;
    if !removed {
        return Err(AppError::NotFound);
    }
    audit::record(
        &state.pool,
        Some(&admin.user_id),
        None,
        "admin.invite_revoke",
        Some("invite"),
        Some(&code),
        None,
        None,
        None,
        Utc::now().timestamp(),
    )
    .await
    .map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- /api/admin/projects ----------

#[derive(Debug, Deserialize)]
pub struct ProjectsQuery {
    pub user: Option<String>,
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct AdminProjectView {
    pub id: String,
    pub user_id: String,
    pub username: String,
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub is_excluded: bool,
    pub created_at: i64,
    pub observation_count: i64,
    pub share_count: i64,
}

pub async fn list_projects(
    State(state): State<AppState>,
    Query(q): Query<ProjectsQuery>,
) -> Result<Json<Vec<AdminProjectView>>, AppError> {
    let user_id = resolve_user_id(&state, q.user.as_deref()).await?;
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    let offset = q.offset.unwrap_or(0).max(0);
    let rows = projects::admin_search(&state.pool, user_id.as_deref(), q.q.as_deref(), limit, offset)
        .await
        .map_err(AppError::Internal)?;
    let view = rows
        .into_iter()
        .map(|r| AdminProjectView {
            id: r.id,
            user_id: r.user_id,
            username: r.username,
            name: r.name,
            display_name: r.display_name,
            description: r.description,
            is_excluded: r.is_excluded != 0,
            created_at: r.created_at,
            observation_count: r.observation_count,
            share_count: r.share_count,
        })
        .collect();
    Ok(Json(view))
}

// ---------- /api/admin/observations ----------

#[derive(Debug, Deserialize)]
pub struct ObservationsQuery {
    pub q: Option<String>,
    pub user: Option<String>,
    pub project: Option<String>,
    #[serde(rename = "type")]
    pub obs_type: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    #[serde(default)]
    pub include_deleted: bool,
}

#[derive(Debug, Serialize)]
pub struct AdminObservationView {
    pub id: String,
    pub user_id: String,
    pub username: String,
    pub machine_id: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub timestamp: i64,
    pub project_path: Option<String>,
    pub content: String,
    pub obs_type: Option<String>,
    pub server_seq: i64,
    pub server_received_at: i64,
    pub deleted_at: Option<i64>,
}

pub async fn list_observations(
    State(state): State<AppState>,
    Query(q): Query<ObservationsQuery>,
) -> Result<Json<Vec<AdminObservationView>>, AppError> {
    let user_id = resolve_user_id(&state, q.user.as_deref()).await?;
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let offset = q.offset.unwrap_or(0).max(0);
    let rows = observations::admin_search(
        &state.pool,
        q.q.as_deref(),
        user_id.as_deref(),
        q.project.as_deref(),
        q.obs_type.as_deref(),
        q.from,
        q.to,
        q.include_deleted,
        limit,
        offset,
    )
    .await
    .map_err(AppError::Internal)?;
    let view = rows
        .into_iter()
        .map(|r| AdminObservationView {
            id: r.id,
            user_id: r.user_id,
            username: r.username,
            machine_id: r.machine_id,
            project_id: r.project_id,
            project_name: r.project_name,
            timestamp: r.timestamp,
            project_path: r.project_path,
            content: r.content,
            obs_type: r.obs_type,
            server_seq: r.server_seq,
            server_received_at: r.server_received_at,
            deleted_at: r.deleted_at,
        })
        .collect();
    Ok(Json(view))
}

pub async fn delete_observation(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let now = Utc::now().timestamp();
    let removed = observations::soft_delete(&state.pool, &id, now)
        .await
        .map_err(AppError::Internal)?;
    if !removed {
        return Err(AppError::NotFound);
    }
    audit::record(
        &state.pool,
        Some(&admin.user_id),
        None,
        "admin.observation_delete",
        Some("observation"),
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

// ---------- /api/admin/shares ----------

#[derive(Debug, Serialize)]
pub struct AdminShareView {
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    pub sharer_user_id: String,
    pub sharer_username: String,
    pub target_type: String,
    pub target_user_id: Option<String>,
    pub target_username: Option<String>,
    pub share_token: Option<String>,
    pub share_mode: String,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
}

pub async fn list_shares(
    State(state): State<AppState>,
    Query(p): Query<PageParams>,
) -> Result<Json<Vec<AdminShareView>>, AppError> {
    let rows = shares::admin_list(&state.pool, p.limit(), p.offset())
        .await
        .map_err(AppError::Internal)?;
    let view = rows
        .into_iter()
        .map(|r| AdminShareView {
            id: r.id,
            project_id: r.project_id,
            project_name: r.project_name,
            sharer_user_id: r.sharer_user_id,
            sharer_username: r.sharer_username,
            target_type: r.target_type,
            target_user_id: r.target_user_id,
            target_username: r.target_username,
            share_token: r.share_token,
            share_mode: r.share_mode,
            expires_at: r.expires_at,
            created_at: r.created_at,
            revoked_at: r.revoked_at,
        })
        .collect();
    Ok(Json(view))
}

pub async fn revoke_share(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let now = Utc::now().timestamp();
    let removed = shares::admin_revoke(&state.pool, &id, now)
        .await
        .map_err(AppError::Internal)?;
    if !removed {
        return Err(AppError::NotFound);
    }
    audit::record(
        &state.pool,
        Some(&admin.user_id),
        None,
        "admin.share_revoke",
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

// ---------- /api/admin/audit ----------

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    pub user: Option<String>,
    pub action: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct AdminAuditView {
    pub id: i64,
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub machine_id: Option<String>,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub created_at: i64,
}

pub async fn list_audit(
    State(state): State<AppState>,
    Query(q): Query<AuditQuery>,
) -> Result<Json<Vec<AdminAuditView>>, AppError> {
    let user_id = resolve_user_id(&state, q.user.as_deref()).await?;
    let limit = q.limit.unwrap_or(200).clamp(1, 2000);
    let rows = audit::search(
        &state.pool,
        user_id.as_deref(),
        q.action.as_deref(),
        q.from,
        q.to,
        limit,
    )
    .await
    .map_err(AppError::Internal)?;

    // 一次性 join username
    let mut user_cache: HashMap<String, String> = HashMap::new();
    for r in &rows {
        if let Some(uid) = &r.user_id {
            if !user_cache.contains_key(uid) {
                if let Some((id, name)) = users::brief_by_id(&state.pool, uid)
                    .await
                    .map_err(AppError::Internal)?
                {
                    user_cache.insert(id, name);
                }
            }
        }
    }
    let view = rows
        .into_iter()
        .map(|r| AdminAuditView {
            id: r.id,
            username: r.user_id.as_ref().and_then(|u| user_cache.get(u).cloned()),
            user_id: r.user_id,
            machine_id: r.machine_id,
            action: r.action,
            target_type: r.target_type,
            target_id: r.target_id,
            created_at: r.created_at,
        })
        .collect();
    Ok(Json(view))
}

// ---------- helper:把 username/user_id 都接受 ----------

/// 接受 username 或 user_id,返回 user_id。空字符串视为 None。
async fn resolve_user_id(
    state: &AppState,
    raw: Option<&str>,
) -> Result<Option<String>, AppError> {
    let Some(s) = raw else {
        return Ok(None);
    };
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    // 先按 id 找
    if let Some(u) = users::find_by_id(&state.pool, s)
        .await
        .map_err(AppError::Internal)?
    {
        return Ok(Some(u.id));
    }
    // 否则按 username
    if let Some((id, _)) = users::brief_by_username(&state.pool, s)
        .await
        .map_err(AppError::Internal)?
    {
        return Ok(Some(id));
    }
    Ok(None)
}
