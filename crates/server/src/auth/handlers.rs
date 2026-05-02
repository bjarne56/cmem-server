//! 认证 HTTP handlers:register / login / refresh / logout / change-password。

use axum::{extract::State, http::StatusCode, Extension, Json};
use chrono::{DateTime, TimeZone, Utc};
use cmem_shared::{
    api::{
        ChangePasswordRequest, LoginRequest, LoginResponse, LogoutRequest, RefreshRequest,
        RefreshResponse, RegisterRequest, RegisterResponse,
    },
    models::UserView,
};
use uuid::Uuid;

use crate::{
    auth::{
        jwt::{generate_refresh_token, sha256_hex},
        password::{hash_password, verify_password},
    },
    db::{audit, tokens as token_db, users},
    error::AppError,
    middleware::Principal,
    state::AppState,
};

const MIN_PASSWORD_LEN: usize = 8;
const MAX_PASSWORD_LEN: usize = 1024;
const MIN_USERNAME_LEN: usize = 3;
const MAX_USERNAME_LEN: usize = 64;

fn validate_username(username: &str) -> Result<(), AppError> {
    let len = username.chars().count();
    if !(MIN_USERNAME_LEN..=MAX_USERNAME_LEN).contains(&len) {
        return Err(AppError::Validation(format!(
            "username length must be {MIN_USERNAME_LEN}..={MAX_USERNAME_LEN}"
        )));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AppError::Validation(
            "username allows [a-zA-Z0-9_-] only".into(),
        ));
    }
    Ok(())
}

fn validate_password(password: &str) -> Result<(), AppError> {
    let len = password.chars().count();
    if !(MIN_PASSWORD_LEN..=MAX_PASSWORD_LEN).contains(&len) {
        return Err(AppError::Validation(format!(
            "password length must be {MIN_PASSWORD_LEN}..={MAX_PASSWORD_LEN}"
        )));
    }
    Ok(())
}

fn ts_to_dt(ts: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(ts, 0)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).unwrap())
}

fn user_view(row: &users::UserRow) -> UserView {
    UserView {
        id: row.id.clone(),
        username: row.username.clone(),
        email: row.email.clone(),
        created_at: ts_to_dt(row.created_at),
        last_login_at: row.last_login_at.map(ts_to_dt),
        is_admin: row.is_admin != 0,
    }
}

/// POST /api/auth/register
pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RegisterResponse>), AppError> {
    validate_username(&req.username)?;
    validate_password(&req.password)?;

    if let Some(email) = &req.email {
        if email.len() > 320 {
            return Err(AppError::Validation("email too long".into()));
        }
    }

    if users::find_by_username(&state.pool, &req.username)
        .await
        .map_err(AppError::Internal)?
        .is_some()
    {
        return Err(AppError::Conflict("username already taken".into()));
    }

    let id = Uuid::now_v7().to_string();
    let now = Utc::now().timestamp();
    let hash = hash_password(&req.password, &state.config.auth)
        .map_err(AppError::Internal)?;

    users::create_user(
        &state.pool,
        &id,
        &req.username,
        &hash,
        req.email.as_deref(),
        false,
        now,
    )
    .await
    .map_err(AppError::Internal)?;

    audit::record(
        &state.pool,
        Some(&id),
        None,
        "auth.register",
        Some("user"),
        Some(&id),
        None,
        None,
        None,
        now,
    )
    .await
    .map_err(AppError::Internal)?;

    let row = users::find_by_id(&state.pool, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("user disappeared after insert")))?;

    Ok((
        StatusCode::CREATED,
        Json(RegisterResponse {
            user: user_view(&row),
        }),
    ))
}

/// POST /api/auth/login
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let row = users::find_by_username(&state.pool, &req.username)
        .await
        .map_err(AppError::Internal)?;

    let user = match row {
        Some(u) if u.is_active != 0 => u,
        _ => return Err(AppError::InvalidCredentials),
    };

    let ok = verify_password(&req.password, &user.password_hash)
        .map_err(AppError::Internal)?;
    if !ok {
        return Err(AppError::InvalidCredentials);
    }

    let (access, exp) = state
        .jwt
        .encode_access(&user.id, None, state.config.auth.access_token_ttl_secs)
        .map_err(AppError::Internal)?;

    let (refresh_plain, refresh_hash) = generate_refresh_token();
    let now = Utc::now().timestamp();
    let refresh_exp = now + state.config.auth.refresh_token_ttl_secs;

    token_db::insert_refresh(
        &state.pool,
        &refresh_hash,
        &user.id,
        now,
        refresh_exp,
        None,
        None,
    )
    .await
    .map_err(AppError::Internal)?;

    users::touch_last_login(&state.pool, &user.id, now)
        .await
        .map_err(AppError::Internal)?;

    audit::record(
        &state.pool,
        Some(&user.id),
        None,
        "auth.login",
        Some("user"),
        Some(&user.id),
        None,
        None,
        None,
        now,
    )
    .await
    .map_err(AppError::Internal)?;

    Ok(Json(LoginResponse {
        user: user_view(&user),
        access_token: access,
        access_token_expires_at: ts_to_dt(exp),
        refresh_token: refresh_plain,
    }))
}

/// POST /api/auth/refresh
///
/// 旋转策略:旧 refresh 立即撤销,签发新 refresh + 新 access。
pub async fn refresh(
    State(state): State<AppState>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<RefreshResponse>, AppError> {
    let now = Utc::now().timestamp();
    let hash = sha256_hex(&req.refresh_token);

    let row = token_db::find_active_refresh(&state.pool, &hash, now)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::Unauthorized)?;

    // 旋转:吊销旧的
    token_db::revoke_refresh(&state.pool, &hash)
        .await
        .map_err(AppError::Internal)?;

    let (access, exp) = state
        .jwt
        .encode_access(
            &row.user_id,
            None,
            state.config.auth.access_token_ttl_secs,
        )
        .map_err(AppError::Internal)?;

    let (new_plain, new_hash) = generate_refresh_token();
    let new_exp = now + state.config.auth.refresh_token_ttl_secs;
    token_db::insert_refresh(
        &state.pool,
        &new_hash,
        &row.user_id,
        now,
        new_exp,
        None,
        None,
    )
    .await
    .map_err(AppError::Internal)?;

    audit::record(
        &state.pool,
        Some(&row.user_id),
        None,
        "auth.refresh",
        Some("user"),
        Some(&row.user_id),
        None,
        None,
        None,
        now,
    )
    .await
    .map_err(AppError::Internal)?;

    Ok(Json(RefreshResponse {
        access_token: access,
        access_token_expires_at: ts_to_dt(exp),
        refresh_token: new_plain,
    }))
}

/// POST /api/auth/logout
///
/// 需要带 access_token(via Bearer)+ refresh_token in body。
/// 撤销该 refresh token。
pub async fn logout(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<LogoutRequest>,
) -> Result<StatusCode, AppError> {
    let hash = sha256_hex(&req.refresh_token);
    token_db::revoke_refresh(&state.pool, &hash)
        .await
        .map_err(AppError::Internal)?;

    audit::record(
        &state.pool,
        Some(principal.user_id()),
        None,
        "auth.logout",
        Some("user"),
        Some(principal.user_id()),
        None,
        None,
        None,
        Utc::now().timestamp(),
    )
    .await
    .map_err(AppError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/auth/change-password
///
/// 必须带 access_token。改密后吊销该用户所有 refresh token。
pub async fn change_password(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<StatusCode, AppError> {
    validate_password(&req.new_password)?;

    let user = users::find_by_id(&state.pool, principal.user_id())
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::Unauthorized)?;

    let ok = verify_password(&req.old_password, &user.password_hash)
        .map_err(AppError::Internal)?;
    if !ok {
        return Err(AppError::InvalidCredentials);
    }

    let new_hash =
        hash_password(&req.new_password, &state.config.auth).map_err(AppError::Internal)?;
    users::update_password_hash(&state.pool, &user.id, &new_hash)
        .await
        .map_err(AppError::Internal)?;
    token_db::revoke_all_for_user(&state.pool, &user.id)
        .await
        .map_err(AppError::Internal)?;

    audit::record(
        &state.pool,
        Some(&user.id),
        None,
        "auth.password_change",
        Some("user"),
        Some(&user.id),
        None,
        None,
        None,
        Utc::now().timestamp(),
    )
    .await
    .map_err(AppError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}
