//! Fork:复制别人共享的项目 / 单条 observation 到自己名下。
//!
//! 不变量(对应 docs/PROJECT_SHARING.md):
//! - #6 Bob fork 后归 Bob:新 project 的 user_id = forker
//! - #7 fork 不同步内容变化:derived_from 仅记录溯源,不订阅原 project 后续变化
//!   (复制完毕后,原项目新增 obs 不会自动出现在 fork 副本里)
//! - #4 撤销共享后已 fork 的副本仍在(因为副本的 user_id 已经是 forker,
//!   与原项目 share 状态无关)

use anyhow::Context;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::{DateTime, TimeZone, Utc};
use cmem_shared::{
    api::{
        ForkObservationRequest, ForkObservationResponse, ForkProjectRequest,
        ForkProjectResponse,
    },
    models::ObservationView,
};
use uuid::Uuid;

use crate::{
    db::{audit, machines, observations, projects, users},
    error::AppError,
    middleware::Principal,
    projects::identification::normalize_project_name,
    shares::permissions::{project_access_for, AccessLevel},
    state::AppState,
};

const MAX_PROJECT_NAME_LEN: usize = 128;

fn ts_to_dt(ts: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(ts, 0)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap_or_default())
}

fn parse_json(s: Option<&str>) -> Option<serde_json::Value> {
    s.and_then(|t| serde_json::from_str(t).ok())
}

/// 默认 fork 后的项目名:`<orig-name>-fork-of-<owner-username>`,经规范化。
fn default_fork_name(orig_name: &str, owner_username: &str) -> String {
    let raw = format!("{orig_name}-fork-of-{owner_username}");
    normalize_project_name(&raw)
}

/// POST /api/projects/:id/fork
///
/// 把别人共享给我的(且至少 fork-allowed 的)项目整体克隆一份到我名下。
pub async fn fork_project(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(source_project_id): Path<String>,
    Json(req): Json<ForkProjectRequest>,
) -> Result<(StatusCode, Json<ForkProjectResponse>), AppError> {
    let forker_user_id = principal.user_id().to_string();

    // 1. 权限校验:必须 fork-allowed / auto-copy / owner(owner fork 自己其实没意义,但允许)
    let access = project_access_for(&state.pool, &forker_user_id, &source_project_id)
        .await
        .map_err(AppError::Internal)?;
    if !access.can_fork() {
        // owner / fork-allowed / auto-copy 才可以;read-only 显式拒绝
        return Err(AppError::Forbidden);
    }

    // 2. 拿原 project + owner 信息(为默认命名)
    let source = projects::find_any_by_id(&state.pool, &source_project_id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;

    // 不允许 fork 自己的项目(没意义,且容易把唯一约束搞坏)
    if source.user_id == forker_user_id && access == AccessLevel::Owner {
        return Err(AppError::Validation(
            "cannot fork your own project".into(),
        ));
    }

    let owner_username = users::brief_by_id(&state.pool, &source.user_id)
        .await
        .map_err(AppError::Internal)?
        .map(|(_, u)| u)
        .unwrap_or_else(|| "unknown".to_string());

    // 3. 决定新项目名
    let raw_new_name = req
        .new_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default_fork_name(&source.name, &owner_username));
    let new_name = normalize_project_name(&raw_new_name);
    if new_name.is_empty() || new_name.chars().count() > MAX_PROJECT_NAME_LEN {
        return Err(AppError::Validation(
            "fork project name normalizes to invalid value".into(),
        ));
    }

    // 4. forker 同名冲突 → 加后缀直到不冲突
    let mut final_name = new_name.clone();
    for suffix in 1..=99 {
        let exists = projects::find_by_name(&state.pool, &forker_user_id, &final_name)
            .await
            .map_err(AppError::Internal)?
            .is_some();
        if !exists {
            break;
        }
        final_name = format!("{new_name}-{suffix}");
        if suffix == 99 {
            return Err(AppError::Conflict(
                "fork target name has too many duplicates".into(),
            ));
        }
    }

    // 5. forker 必须至少有一台 active 机器(observation 必须有 machine_id)
    let machine = machines::pick_active_machine(&state.pool, &forker_user_id)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| {
            AppError::Validation(
                "forker must have at least one active machine before forking".into(),
            )
        })?;

    // 6. 拉所有源 observation
    let source_obs = observations::list_by_project(&state.pool, &source_project_id)
        .await
        .map_err(AppError::Internal)?;

    let new_project_id = Uuid::now_v7().to_string();
    let now = Utc::now().timestamp();

    // 7. 一个事务创建 project + 复制 observation
    let mut tx = state.pool.begin().await.map_err(AppError::Db)?;

    projects::create_fork_in_tx(
        &mut tx,
        &new_project_id,
        &forker_user_id,
        &final_name,
        source.description.as_deref(),
        &source_project_id,
        now,
        now,
    )
    .await
    .map_err(AppError::Internal)?;

    let mut copied: i64 = 0;
    for src in &source_obs {
        // derivation_chain:在原链基础上 append 当前 obs id;原 chain 缺则起一条新链。
        let chain = match parse_json(src.derivation_chain.as_deref()) {
            Some(serde_json::Value::Array(mut arr)) => {
                arr.push(serde_json::Value::String(src.id.clone()));
                serde_json::Value::Array(arr)
            }
            _ => serde_json::Value::Array(vec![serde_json::Value::String(src.id.clone())]),
        };
        let chain_str = chain.to_string();
        let new_obs_id = Uuid::now_v7().to_string();
        let seq = observations::next_server_seq(&mut tx)
            .await
            .map_err(AppError::Internal)?;
        let inserted = observations::insert_in_tx(
            &mut tx,
            &new_obs_id,
            &forker_user_id,
            &machine.id,
            Some(&new_project_id),
            src.timestamp,
            src.project_path.as_deref(),
            &src.content,
            src.obs_type.as_deref(),
            src.metadata.as_deref(),
            Some(&src.id),
            Some(&chain_str),
            seq,
            now,
        )
        .await
        .map_err(AppError::Internal)?;
        if inserted {
            copied += 1;
        }
    }

    tx.commit().await.map_err(AppError::Db)?;

    // 8. audit
    let meta = serde_json::json!({
        "source_project_id": source_project_id,
        "new_project_id": new_project_id,
        "copied": copied,
    })
    .to_string();
    audit::record(
        &state.pool,
        Some(&forker_user_id),
        Some(&machine.id),
        "project.fork",
        Some("project"),
        Some(&new_project_id),
        Some(&meta),
        None,
        None,
        now,
    )
    .await
    .map_err(AppError::Internal)?;

    // 9. 返回 ProjectDetail
    let row = projects::find_by_id(&state.pool, &forker_user_id, &new_project_id)
        .await
        .map_err(AppError::Internal)?
        .context("fork project vanished after insert")
        .map_err(AppError::Internal)?;
    let cnt = projects::observation_count(&state.pool, &new_project_id)
        .await
        .map_err(AppError::Internal)?;

    use cmem_shared::models::{ProjectDetail, ProjectPathView, ShareSummary};
    let detail = ProjectDetail {
        id: row.id.clone(),
        user_id: row.user_id.clone(),
        name: row.name.clone(),
        display_name: row.display_name.clone(),
        description: row.description.clone(),
        is_excluded: row.is_excluded != 0,
        forked_from_project: row.forked_from_project.clone(),
        forked_at: row.forked_at.map(ts_to_dt),
        created_at: ts_to_dt(row.created_at),
        observation_count: cnt,
        paths: Vec::<ProjectPathView>::new(),
        shares: Vec::<ShareSummary>::new(),
    };

    Ok((
        StatusCode::CREATED,
        Json(ForkProjectResponse {
            project: detail,
            copied_observations: copied,
        }),
    ))
}

/// POST /api/observations/:id/fork
///
/// 单条 fork:仅 fork-allowed 模式有效;新 obs 归 forker 名下,挂到 forker 指定的项目。
pub async fn fork_observation(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(source_obs_id): Path<String>,
    Json(req): Json<ForkObservationRequest>,
) -> Result<(StatusCode, Json<ForkObservationResponse>), AppError> {
    let forker_user_id = principal.user_id().to_string();

    let src = observations::find_by_id(&state.pool, &source_obs_id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;

    // 必须有源 project,否则 share 关系无从谈起。
    let src_pid = src
        .project_id
        .as_deref()
        .ok_or_else(|| AppError::Validation("source observation has no project".into()))?;

    let access = project_access_for(&state.pool, &forker_user_id, src_pid)
        .await
        .map_err(AppError::Internal)?;
    if !access.can_fork() {
        return Err(AppError::Forbidden);
    }
    if src.user_id == forker_user_id {
        return Err(AppError::Validation(
            "cannot fork your own observation".into(),
        ));
    }

    // 目标 project 必须是 forker 自己的
    let target = projects::find_by_id(&state.pool, &forker_user_id, &req.to_project_id)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Validation("to_project_id must be one of your own projects".into()))?;

    let machine = machines::pick_active_machine(&state.pool, &forker_user_id)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| {
            AppError::Validation(
                "forker must have at least one active machine before forking".into(),
            )
        })?;

    let chain = match parse_json(src.derivation_chain.as_deref()) {
        Some(serde_json::Value::Array(mut arr)) => {
            arr.push(serde_json::Value::String(src.id.clone()));
            serde_json::Value::Array(arr)
        }
        _ => serde_json::Value::Array(vec![serde_json::Value::String(src.id.clone())]),
    };
    let chain_str = chain.to_string();

    let now = Utc::now().timestamp();
    let new_id = Uuid::now_v7().to_string();
    let mut tx = state.pool.begin().await.map_err(AppError::Db)?;
    let seq = observations::next_server_seq(&mut tx)
        .await
        .map_err(AppError::Internal)?;
    observations::insert_in_tx(
        &mut tx,
        &new_id,
        &forker_user_id,
        &machine.id,
        Some(&target.id),
        src.timestamp,
        src.project_path.as_deref(),
        &src.content,
        src.obs_type.as_deref(),
        src.metadata.as_deref(),
        Some(&src.id),
        Some(&chain_str),
        seq,
        now,
    )
    .await
    .map_err(AppError::Internal)?;
    tx.commit().await.map_err(AppError::Db)?;

    audit::record(
        &state.pool,
        Some(&forker_user_id),
        Some(&machine.id),
        "observation.fork",
        Some("observation"),
        Some(&new_id),
        Some(
            &serde_json::json!({
                "source_observation_id": source_obs_id,
                "to_project_id": target.id,
            })
            .to_string(),
        ),
        None,
        None,
        now,
    )
    .await
    .map_err(AppError::Internal)?;

    let view = ObservationView {
        id: new_id,
        user_id: forker_user_id.clone(),
        machine_id: machine.id.clone(),
        project_id: Some(target.id.clone()),
        timestamp: ts_to_dt(src.timestamp),
        project_path: src.project_path.clone(),
        content: src.content.clone(),
        obs_type: src.obs_type.clone(),
        metadata: parse_json(src.metadata.as_deref()),
        derived_from: Some(src.id.clone()),
        derivation_chain: Some(chain),
        server_seq: seq,
        server_received_at: ts_to_dt(now),
    };

    Ok((StatusCode::CREATED, Json(ForkObservationResponse { observation: view })))
}
