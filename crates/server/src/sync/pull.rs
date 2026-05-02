//! POST /api/sync/pull 实现。

use chrono::{DateTime, TimeZone, Utc};
use cmem_shared::{
    api::{
        DowngradeNotice, PullRequest, PullResponse, RevokedShare, SharedObservation,
    },
    models::ObservationView,
};

use crate::{
    db::{audit, machines, observations, projects, shares, users},
    error::AppError,
    middleware::Principal,
    state::AppState,
};

const DEFAULT_LIMIT: i64 = 500;
const MAX_LIMIT: i64 = 5000;

fn ts_to_dt(ts: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(ts, 0)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap_or_default())
}

fn parse_json(s: Option<&str>) -> Option<serde_json::Value> {
    match s {
        Some(text) => serde_json::from_str(text).ok(),
        None => None,
    }
}

fn own_to_view(row: &observations::ObservationRow) -> ObservationView {
    ObservationView {
        id: row.id.clone(),
        user_id: row.user_id.clone(),
        machine_id: row.machine_id.clone(),
        project_id: row.project_id.clone(),
        timestamp: ts_to_dt(row.timestamp),
        project_path: row.project_path.clone(),
        content: row.content.clone(),
        obs_type: row.obs_type.clone(),
        metadata: parse_json(row.metadata.as_deref()),
        derived_from: row.derived_from.clone(),
        derivation_chain: parse_json(row.derivation_chain.as_deref()),
        server_seq: row.server_seq,
        server_received_at: ts_to_dt(row.server_received_at),
    }
}

fn shared_to_view(row: &observations::SharedObservationRow) -> ObservationView {
    ObservationView {
        id: row.id.clone(),
        user_id: row.user_id.clone(),
        machine_id: row.machine_id.clone(),
        project_id: Some(row.project_id.clone()),
        timestamp: ts_to_dt(row.timestamp),
        project_path: row.project_path.clone(),
        content: row.content.clone(),
        obs_type: row.obs_type.clone(),
        metadata: parse_json(row.metadata.as_deref()),
        derived_from: row.derived_from.clone(),
        derivation_chain: parse_json(row.derivation_chain.as_deref()),
        server_seq: row.server_seq,
        server_received_at: ts_to_dt(row.server_received_at),
    }
}

pub async fn pull(
    state: &AppState,
    principal: &Principal,
    req: PullRequest,
) -> Result<PullResponse, AppError> {
    let user_id = principal.user_id().to_string();
    let now = Utc::now().timestamp();

    let mut limit = req.limit.unwrap_or(DEFAULT_LIMIT);
    if limit <= 0 || limit > MAX_LIMIT {
        limit = DEFAULT_LIMIT;
    }
    let since = if req.since_seq < 0 { 0 } else { req.since_seq };

    // 1. own observations
    let own_rows = observations::list_own_since(
        &state.pool,
        &user_id,
        since,
        limit,
        &req.exclude_machines,
    )
    .await
    .map_err(AppError::Internal)?;

    // 2. shared observations(仅当 include_shared=true)
    let shared_rows = if req.include_shared {
        observations::list_shared_since(&state.pool, &user_id, since, limit, now)
            .await
            .map_err(AppError::Internal)?
    } else {
        Vec::new()
    };

    // 3. pending downgrades
    let downgrade_rows = shares::pending_downgrades(&state.pool, &user_id)
        .await
        .map_err(AppError::Internal)?;

    let mut downgrades = Vec::with_capacity(downgrade_rows.len());
    for d in &downgrade_rows {
        let project = projects::find_any_by_id(&state.pool, &d.project_id)
            .await
            .map_err(AppError::Internal)?;
        let (project_name, owner_username) = if let Some(p) = project {
            let owner = users::brief_by_id(&state.pool, &p.user_id)
                .await
                .map_err(AppError::Internal)?
                .map(|(_, u)| u)
                .unwrap_or_else(|| "unknown".to_string());
            (p.name, owner)
        } else {
            ("(deleted)".to_string(), "unknown".to_string())
        };
        downgrades.push(DowngradeNotice {
            id: d.id,
            project_id: d.project_id.clone(),
            project_name,
            owner_username,
            old_mode: d.old_mode.clone(),
            new_mode: d.new_mode.clone(),
            created_at: ts_to_dt(d.created_at),
        });
    }

    // 4. revoked shares(target=me,revoked_at > since 时间)
    //    用 since_seq 当 server_seq cursor,而 revoked 发生时间不是 server_seq;
    //    简化:返回最近 30 天内的 revoke,客户端按 project_id 去重。
    let revoke_lookback = now - 30 * 86400;
    let revoked_rows = shares::list_recent_revoked(&state.pool, &user_id, revoke_lookback)
        .await
        .map_err(AppError::Internal)?;
    let mut revoked = Vec::with_capacity(revoked_rows.len());
    for r in &revoked_rows {
        let project = projects::find_any_by_id(&state.pool, &r.project_id)
            .await
            .map_err(AppError::Internal)?;
        let (project_name, owner_username) = if let Some(p) = project {
            let owner = users::brief_by_id(&state.pool, &p.user_id)
                .await
                .map_err(AppError::Internal)?
                .map(|(_, u)| u)
                .unwrap_or_else(|| "unknown".to_string());
            (p.name, owner)
        } else {
            ("(deleted)".to_string(), "unknown".to_string())
        };
        revoked.push(RevokedShare {
            project_id: r.project_id.clone(),
            project_name,
            owner_username,
            revoked_at: ts_to_dt(r.revoked_at.unwrap_or(0)),
        });
    }

    // 计算 next_since_seq:取 own + shared 中最大的 server_seq。
    let max_own = own_rows.iter().map(|r| r.server_seq).max().unwrap_or(since);
    let max_shared = shared_rows
        .iter()
        .map(|r| r.server_seq)
        .max()
        .unwrap_or(since);
    let next_since_seq = max_own.max(max_shared).max(since);

    let has_more =
        own_rows.len() as i64 >= limit || shared_rows.len() as i64 >= limit;

    // 转视图
    let own_observations: Vec<ObservationView> = own_rows.iter().map(own_to_view).collect();
    let mut shared_observations = Vec::with_capacity(shared_rows.len());
    for sr in &shared_rows {
        shared_observations.push(SharedObservation {
            observation: shared_to_view(sr),
            share_mode: sr.share_mode.clone(),
            sharer_user_id: sr.sharer_user_id.clone(),
            sharer_username: sr.sharer_username.clone(),
            project_id: sr.project_id.clone(),
            project_name: sr.project_name.clone(),
        });
    }

    // 更新 machine.last_seen_at(若 principal 是 machine 或 user 带 mid)
    let machine_id_opt: Option<String> = match principal {
        Principal::Machine { machine_id, .. } => Some(machine_id.clone()),
        Principal::User { machine_id, .. } => machine_id.clone(),
    };
    if let Some(mid) = &machine_id_opt {
        let _ = machines::touch_last_seen(&state.pool, mid, now).await;
    }

    // audit
    let meta = serde_json::json!({
        "since_seq": since,
        "own": own_observations.len(),
        "shared": shared_observations.len(),
        "downgrades": downgrades.len(),
        "revoked": revoked.len(),
    })
    .to_string();
    audit::record(
        &state.pool,
        Some(&user_id),
        machine_id_opt.as_deref(),
        "sync.pull",
        Some("batch"),
        None,
        Some(&meta),
        None,
        None,
        now,
    )
    .await
    .map_err(AppError::Internal)?;

    Ok(PullResponse {
        own_observations,
        shared_observations,
        pending_downgrades: downgrades,
        revoked_shares: revoked,
        next_since_seq,
        has_more,
    })
}
