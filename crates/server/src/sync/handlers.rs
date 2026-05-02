//! /api/sync/* HTTP handler 入口。

use axum::{extract::State, Extension, Json};
use cmem_shared::api::{PullRequest, PullResponse, PushRequest, PushResponse};

use crate::{
    error::AppError,
    middleware::Principal,
    state::AppState,
    sync::{pull, push},
};

pub async fn push_handler(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<PushRequest>,
) -> Result<Json<PushResponse>, AppError> {
    let resp = push::push(&state, &principal, req).await?;
    Ok(Json(resp))
}

pub async fn pull_handler(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<PullRequest>,
) -> Result<Json<PullResponse>, AppError> {
    let resp = pull::pull(&state, &principal, req).await?;
    Ok(Json(resp))
}
