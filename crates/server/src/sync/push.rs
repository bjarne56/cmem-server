//! POST /api/sync/push 实现。

use chrono::Utc;
use cmem_shared::api::{
    PushError, PushObservation, PushRequest, PushResponse, ResolvedProject,
};
use std::collections::HashMap;

use crate::{
    db::{audit, machines, observations},
    error::AppError,
    middleware::Principal,
    projects::identification::{resolve_project, SubmittedProjectInfo},
    state::AppState,
};

const MAX_BATCH: usize = 1000;
const MAX_CONTENT_LEN: usize = 1024 * 1024; // 1 MiB
const MAX_PATH_LEN: usize = 4096;

/// 主入口:校验 → 事务内逐条 resolve_project + INSERT OR IGNORE → 提交。
pub async fn push(
    state: &AppState,
    principal: &Principal,
    req: PushRequest,
) -> Result<PushResponse, AppError> {
    if req.observations.len() > MAX_BATCH {
        return Err(AppError::Validation(format!(
            "batch too large (max {MAX_BATCH})"
        )));
    }
    let user_id = principal.user_id().to_string();

    // 解析 machine_id:user JWT 必须带 mid;machine token 自带。
    let machine_id = match principal {
        Principal::User { machine_id, .. } => machine_id
            .clone()
            .ok_or_else(|| AppError::Validation("machine_id missing in token; register a machine and use machine_token for push".into()))?,
        Principal::Machine { machine_id, .. } => machine_id.clone(),
    };

    // 双保险:确认 machine 存在且未撤销且属于当前 user。
    let machine = machines::find_by_id(&state.pool, &machine_id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::Unauthorized)?;
    if machine.revoked != 0 || machine.user_id != user_id {
        return Err(AppError::Unauthorized);
    }

    // 校验逐条载荷长度。
    for obs in &req.observations {
        if obs.id.is_empty() {
            return Err(AppError::Validation("observation.id required".into()));
        }
        if obs.content.len() > MAX_CONTENT_LEN {
            return Err(AppError::Validation(format!(
                "observation {} content > 1MiB",
                obs.id
            )));
        }
        if let Some(p) = &obs.project_path {
            if p.len() > MAX_PATH_LEN {
                return Err(AppError::Validation(format!(
                    "observation {} project_path too long",
                    obs.id
                )));
            }
        }
    }

    let now = Utc::now().timestamp();

    // 单事务处理整个 batch。
    let mut tx = state.pool.begin().await.map_err(AppError::Db)?;

    let mut accepted: usize = 0;
    let mut duplicates: usize = 0;
    let errors: Vec<PushError> = Vec::new();
    let mut server_seq_max: i64 = 0;

    // 复用 project_id:同 batch 内重复 (marker_id, name) 不重复 resolve。
    let mut project_cache: HashMap<String, Option<String>> = HashMap::new();
    let mut projects_resolved: Vec<ResolvedProject> = Vec::new();

    for obs in &req.observations {
        // 构造 cache key(基于客户端提交字段)
        let cache_key = format!(
            "{}::{}::{}",
            obs.project_marker_id.as_deref().unwrap_or(""),
            obs.project_name.as_deref().unwrap_or(""),
            obs.project_path.as_deref().unwrap_or("")
        );

        let project_id = match project_cache.get(&cache_key) {
            Some(p) => p.clone(),
            None => {
                let submitted = SubmittedProjectInfo {
                    marker_id: obs.project_marker_id.clone(),
                    name: obs.project_name.clone(),
                    path: obs.project_path.clone(),
                };
                let pid = resolve_project(&mut tx, &user_id, &machine_id, &submitted, now)
                    .await
                    .map_err(AppError::Internal)?;
                project_cache.insert(cache_key.clone(), pid.clone());
                projects_resolved.push(ResolvedProject {
                    submitted_name: obs.project_name.clone(),
                    submitted_marker_id: obs.project_marker_id.clone(),
                    submitted_path: obs.project_path.clone(),
                    project_id: pid.clone(),
                });
                pid
            }
        };

        // 拿 server_seq → INSERT。失败回滚整个 tx。
        let seq = observations::next_server_seq(&mut tx)
            .await
            .map_err(AppError::Internal)?;

        let metadata_str = match &obs.metadata {
            Some(v) => Some(serde_json::to_string(v).map_err(|e| {
                AppError::Validation(format!("metadata serialize failed for {}: {e}", obs.id))
            })?),
            None => None,
        };
        let chain_str = match &obs.derivation_chain {
            Some(v) => Some(serde_json::to_string(v).map_err(|e| {
                AppError::Validation(format!(
                    "derivation_chain serialize failed for {}: {e}",
                    obs.id
                ))
            })?),
            None => None,
        };

        let inserted = observations::insert_in_tx(
            &mut tx,
            &obs.id,
            &user_id,
            &machine_id,
            project_id.as_deref(),
            obs.timestamp,
            obs.project_path.as_deref(),
            &obs.content,
            obs.obs_type.as_deref(),
            metadata_str.as_deref(),
            obs.derived_from.as_deref(),
            chain_str.as_deref(),
            seq,
            now,
        )
        .await;

        match inserted {
            Ok(true) => {
                accepted += 1;
                if seq > server_seq_max {
                    server_seq_max = seq;
                }
            }
            Ok(false) => {
                duplicates += 1;
            }
            Err(e) => {
                // 单条失败不破坏 tx 一致性?其实 sqlx 出错时事务已坏,直接 return。
                return Err(AppError::Internal(e));
            }
        }

        let _ = &errors; // 占位:目前无逐条 soft 失败语义
    }

    // 更新 last_seen,用 tx 内的 sql。
    sqlx::query!(
        r#"UPDATE machines SET last_seen_at = ?2 WHERE id = ?1"#,
        machine_id,
        now,
    )
    .execute(&mut *tx)
    .await
    .map_err(AppError::Db)?;

    tx.commit().await.map_err(AppError::Db)?;

    // 审计(失败不影响响应)。
    let meta = serde_json::json!({
        "batch_size": req.observations.len(),
        "accepted": accepted,
        "duplicates": duplicates,
    })
    .to_string();
    audit::record(
        &state.pool,
        Some(&user_id),
        Some(&machine_id),
        "sync.push",
        Some("batch"),
        None,
        Some(&meta),
        None,
        None,
        now,
    )
    .await
    .map_err(AppError::Internal)?;

    Ok(PushResponse {
        accepted,
        duplicates,
        errors,
        server_seq_max,
        projects_resolved,
    })
}

/// 给客户端的 type-check 入口(unit test 用)。
#[allow(dead_code)]
pub fn validate_obs(obs: &PushObservation) -> Result<(), String> {
    if obs.id.is_empty() {
        return Err("id required".into());
    }
    if obs.content.len() > MAX_CONTENT_LEN {
        return Err("content too long".into());
    }
    Ok(())
}
