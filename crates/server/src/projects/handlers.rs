//! /api/projects 端点。

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::{DateTime, TimeZone, Utc};
use cmem_shared::{
    api::{
        CreateProjectRequest, ListProjectsResponse, PatchProjectRequest, ProjectResponse,
    },
    models::{ProjectDetail, ProjectPathView, ShareSummary, UserBrief},
};
use std::collections::HashMap;
use uuid::Uuid;

use crate::{
    db::{audit, projects, shares, users},
    error::AppError,
    middleware::Principal,
    projects::identification::normalize_project_name,
    state::AppState,
};

const MAX_PROJECT_NAME_LEN: usize = 128;
const MAX_DISPLAY_NAME_LEN: usize = 128;
const MAX_DESCRIPTION_LEN: usize = 1024;

fn ts_to_dt(ts: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(ts, 0)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap_or_default())
}

fn validate_name(name: &str) -> Result<String, AppError> {
    let normalized = normalize_project_name(name);
    if normalized.is_empty() {
        return Err(AppError::Validation(
            "project name normalizes to empty".into(),
        ));
    }
    if normalized.chars().count() > MAX_PROJECT_NAME_LEN {
        return Err(AppError::Validation("project name too long".into()));
    }
    Ok(normalized)
}

async fn build_project_detail(
    state: &AppState,
    row: &projects::ProjectRow,
    obs_count: i64,
) -> Result<ProjectDetail, AppError> {
    let path_rows = projects::list_paths(&state.pool, &row.id)
        .await
        .map_err(AppError::Internal)?;
    let paths = path_rows
        .into_iter()
        .map(|p| ProjectPathView {
            machine_id: p.machine_id,
            machine_name: p.machine_name,
            path: p.path,
            project_marker_id: p.project_marker_id,
        })
        .collect();

    let share_rows = shares::list_active_by_project(&state.pool, &row.id)
        .await
        .map_err(AppError::Internal)?;
    let mut share_summaries = Vec::with_capacity(share_rows.len());
    for s in &share_rows {
        let target_user = if let Some(uid) = &s.target_user_id {
            users::brief_by_id(&state.pool, uid)
                .await
                .map_err(AppError::Internal)?
                .map(|(id, username)| UserBrief { id, username })
        } else {
            None
        };
        share_summaries.push(ShareSummary {
            id: s.id.clone(),
            target_type: s.target_type.clone(),
            target_user,
            share_mode: s.share_mode.clone(),
            expires_at: s.expires_at.map(ts_to_dt),
            created_at: ts_to_dt(s.created_at),
        });
    }

    Ok(ProjectDetail {
        id: row.id.clone(),
        user_id: row.user_id.clone(),
        name: row.name.clone(),
        display_name: row.display_name.clone(),
        description: row.description.clone(),
        is_excluded: row.is_excluded != 0,
        forked_from_project: row.forked_from_project.clone(),
        forked_at: row.forked_at.map(ts_to_dt),
        created_at: ts_to_dt(row.created_at),
        observation_count: obs_count,
        paths,
        shares: share_summaries,
    })
}

/// GET /api/projects
pub async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<ListProjectsResponse>, AppError> {
    let rows = projects::list_by_user(&state.pool, principal.user_id())
        .await
        .map_err(AppError::Internal)?;
    let counts: HashMap<String, i64> =
        projects::observation_counts_for_user(&state.pool, principal.user_id())
            .await
            .map_err(AppError::Internal)?
            .into_iter()
            .collect();

    let mut details = Vec::with_capacity(rows.len());
    for row in &rows {
        let cnt = counts.get(&row.id).copied().unwrap_or(0);
        details.push(build_project_detail(&state, row, cnt).await?);
    }
    Ok(Json(ListProjectsResponse { projects: details }))
}

/// GET /api/projects/:id
pub async fn get(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> Result<Json<ProjectResponse>, AppError> {
    let row = projects::find_by_id(&state.pool, principal.user_id(), &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    let cnt = projects::observation_count(&state.pool, &row.id)
        .await
        .map_err(AppError::Internal)?;
    let detail = build_project_detail(&state, &row, cnt).await?;
    Ok(Json(ProjectResponse { project: detail }))
}

/// POST /api/projects
pub async fn create(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<ProjectResponse>), AppError> {
    let name = validate_name(&req.name)?;
    if let Some(d) = &req.description {
        if d.chars().count() > MAX_DESCRIPTION_LEN {
            return Err(AppError::Validation("description too long".into()));
        }
    }

    if projects::find_by_name(&state.pool, principal.user_id(), &name)
        .await
        .map_err(AppError::Internal)?
        .is_some()
    {
        return Err(AppError::Conflict(format!(
            "project '{name}' already exists"
        )));
    }

    let id = Uuid::now_v7().to_string();
    let now = Utc::now().timestamp();
    projects::create(
        &state.pool,
        &id,
        principal.user_id(),
        &name,
        req.description.as_deref(),
        now,
    )
    .await
    .map_err(AppError::Internal)?;

    audit::record(
        &state.pool,
        Some(principal.user_id()),
        None,
        "project.create",
        Some("project"),
        Some(&id),
        None,
        None,
        None,
        now,
    )
    .await
    .map_err(AppError::Internal)?;

    let row = projects::find_by_id(&state.pool, principal.user_id(), &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("project vanished after insert")))?;
    let detail = build_project_detail(&state, &row, 0).await?;
    Ok((StatusCode::CREATED, Json(ProjectResponse { project: detail })))
}

/// PATCH /api/projects/:id
pub async fn patch(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    Json(req): Json<PatchProjectRequest>,
) -> Result<Json<ProjectResponse>, AppError> {
    let existing = projects::find_by_id(&state.pool, principal.user_id(), &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;

    let mut new_name: Option<String> = None;
    if let Some(n) = &req.name {
        let normalized = validate_name(n)?;
        // 改名后检查同 user 是否已有别的项目占用此 name
        if normalized != existing.name {
            if projects::find_by_name(&state.pool, principal.user_id(), &normalized)
                .await
                .map_err(AppError::Internal)?
                .is_some()
            {
                return Err(AppError::Conflict(format!(
                    "project '{normalized}' already exists"
                )));
            }
            new_name = Some(normalized);
        }
    }

    if let Some(s) = &req.display_name {
        if s.chars().count() > MAX_DISPLAY_NAME_LEN {
            return Err(AppError::Validation("display_name too long".into()));
        }
    }
    if let Some(s) = &req.description {
        if s.chars().count() > MAX_DESCRIPTION_LEN {
            return Err(AppError::Validation("description too long".into()));
        }
    }
    let display_name_arg: Option<Option<&str>> = req
        .display_name
        .as_ref()
        .map(|s| Some(s.as_str()));
    let description_arg: Option<Option<&str>> =
        req.description.as_ref().map(|s| Some(s.as_str()));

    projects::patch(
        &state.pool,
        principal.user_id(),
        &id,
        new_name.as_deref(),
        display_name_arg,
        description_arg,
        req.is_excluded,
    )
    .await
    .map_err(AppError::Internal)?;

    let action = if req.is_excluded == Some(true) {
        "project.exclude"
    } else {
        "project.update"
    };
    audit::record(
        &state.pool,
        Some(principal.user_id()),
        None,
        action,
        Some("project"),
        Some(&id),
        None,
        None,
        None,
        Utc::now().timestamp(),
    )
    .await
    .map_err(AppError::Internal)?;

    let updated = projects::find_by_id(&state.pool, principal.user_id(), &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    let cnt = projects::observation_count(&state.pool, &id)
        .await
        .map_err(AppError::Internal)?;
    let detail = build_project_detail(&state, &updated, cnt).await?;
    Ok(Json(ProjectResponse { project: detail }))
}

/// DELETE /api/projects/:id
pub async fn delete(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let now = Utc::now().timestamp();
    let removed = projects::delete(&state.pool, principal.user_id(), &id, now)
        .await
        .map_err(AppError::Internal)?;
    if !removed {
        return Err(AppError::NotFound);
    }
    audit::record(
        &state.pool,
        Some(principal.user_id()),
        None,
        "project.delete",
        Some("project"),
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
