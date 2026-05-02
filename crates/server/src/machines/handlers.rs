//! POST/GET/DELETE /api/machines

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::{DateTime, TimeZone, Utc};
use cmem_shared::{
    api::{CreateMachineRequest, CreateMachineResponse, ListMachinesResponse},
    models::MachineView,
};
use uuid::Uuid;

use crate::{
    auth::tokens::{generate_machine_token, hash_machine_token},
    db::{audit, machines},
    error::AppError,
    middleware::Principal,
    state::AppState,
};

const MAX_MACHINE_NAME_LEN: usize = 64;
const MAX_DESCRIPTION_LEN: usize = 256;

fn validate_machine_name(name: &str) -> Result<(), AppError> {
    let len = name.chars().count();
    if !(1..=MAX_MACHINE_NAME_LEN).contains(&len) {
        return Err(AppError::Validation(format!(
            "machine name length must be 1..={MAX_MACHINE_NAME_LEN}"
        )));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(AppError::Validation(
            "machine name allows [a-zA-Z0-9._-] only".into(),
        ));
    }
    Ok(())
}

fn ts_to_dt(ts: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(ts, 0)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap_or_default())
}

fn machine_view(row: &machines::MachineRow) -> MachineView {
    MachineView {
        id: row.id.clone(),
        user_id: row.user_id.clone(),
        name: row.name.clone(),
        description: row.description.clone(),
        last_seen_at: row.last_seen_at.map(ts_to_dt),
        created_at: ts_to_dt(row.created_at),
    }
}

/// POST /api/machines
pub async fn create(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<CreateMachineRequest>,
) -> Result<(StatusCode, Json<CreateMachineResponse>), AppError> {
    // 仅 user JWT 能注册新机器,machine token 不行(防止机器无限自繁殖)。
    let user_id = match &principal {
        Principal::User { user_id, .. } => user_id.clone(),
        Principal::Machine { .. } => return Err(AppError::Forbidden),
    };

    validate_machine_name(&req.name)?;
    if let Some(d) = &req.description {
        if d.chars().count() > MAX_DESCRIPTION_LEN {
            return Err(AppError::Validation("description too long".into()));
        }
    }

    if machines::find_by_user_and_name(&state.pool, &user_id, &req.name)
        .await
        .map_err(AppError::Internal)?
        .is_some()
    {
        return Err(AppError::Conflict("machine name already taken".into()));
    }

    let id = Uuid::now_v7().to_string();
    let token_plain = generate_machine_token();
    let token_hash = hash_machine_token(&token_plain);
    let now = Utc::now().timestamp();

    machines::create_machine(
        &state.pool,
        &id,
        &user_id,
        &req.name,
        req.description.as_deref(),
        &token_hash,
        now,
    )
    .await
    .map_err(AppError::Internal)?;

    audit::record(
        &state.pool,
        Some(&user_id),
        Some(&id),
        "machine.create",
        Some("machine"),
        Some(&id),
        None,
        None,
        None,
        now,
    )
    .await
    .map_err(AppError::Internal)?;

    let row = machines::find_by_id(&state.pool, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("machine vanished after insert")))?;

    Ok((
        StatusCode::CREATED,
        Json(CreateMachineResponse {
            machine: machine_view(&row),
            machine_token: token_plain,
        }),
    ))
}

/// GET /api/machines
pub async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<ListMachinesResponse>, AppError> {
    let rows = machines::list_by_user(&state.pool, principal.user_id())
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(ListMachinesResponse {
        machines: rows.iter().map(machine_view).collect(),
    }))
}

/// DELETE /api/machines/:id (撤销 machine token,该机器的 push/pull 立即失效)
pub async fn revoke(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(machine_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let row = machines::find_by_id(&state.pool, &machine_id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;

    if row.user_id != principal.user_id() {
        return Err(AppError::Forbidden);
    }

    machines::revoke(&state.pool, &machine_id)
        .await
        .map_err(AppError::Internal)?;

    audit::record(
        &state.pool,
        Some(principal.user_id()),
        Some(&machine_id),
        "machine.revoke",
        Some("machine"),
        Some(&machine_id),
        None,
        None,
        None,
        Utc::now().timestamp(),
    )
    .await
    .map_err(AppError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}
