//! Admin 数据导出:CSV / SQLite dump (.db.gz) / per-user .zip。
//!
//! 设计要点:
//! - CSV 用 `csv` crate,流式写入内存缓冲后一次性返回(数据量可控)。
//! - 全库 `.db.gz`:`VACUUM INTO` 写到 tempfile,gzip 流式返回,响应结束后 tempfile 自动清理。
//! - per-user `.zip`:多张表 SELECT 后 JSON 序列化,丢进 zip 各自一个 entry。

use std::io::{Cursor, Write};

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension,
};
use chrono::Utc;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::Deserialize;
use serde_json::json;

use crate::{
    admin::middleware::AdminPrincipal,
    db::{audit, users},
    error::AppError,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct ExportFilter {
    pub user: Option<String>,
    pub project: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

fn csv_response(filename: &str, body: Vec<u8>) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "text/csv; charset=utf-8".parse().unwrap(),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{filename}\"").parse().unwrap(),
    );
    (StatusCode::OK, headers, body).into_response()
}

fn binary_response(filename: &str, content_type: &str, body: Vec<u8>) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, content_type.parse().unwrap());
    headers.insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{filename}\"").parse().unwrap(),
    );
    (StatusCode::OK, headers, body).into_response()
}

async fn audit_export_record(
    state: &AppState,
    admin: &AdminPrincipal,
    target_id: &str,
    note: &str,
) -> Result<(), AppError> {
    let now = Utc::now().timestamp();
    audit::record(
        &state.pool,
        Some(&admin.user_id),
        None,
        "admin.export",
        Some("export"),
        Some(target_id),
        Some(&json!({ "kind": note }).to_string()),
        None,
        None,
        now,
    )
    .await
    .map_err(AppError::Internal)
}

// ---------- users.csv ----------

pub async fn export_users_csv(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
) -> Result<Response, AppError> {
    let rows = users::list_all(&state.pool).await.map_err(AppError::Internal)?;
    let mut wtr = csv::Writer::from_writer(vec![]);
    wtr.write_record([
        "id",
        "username",
        "email",
        "is_admin",
        "is_active",
        "created_at",
        "last_login_at",
        "registration_ip",
        "last_login_ip",
    ])
    .map_err(|e| AppError::Internal(anyhow::anyhow!("csv: {e}")))?;
    for r in rows {
        wtr.write_record([
            r.id.as_str(),
            r.username.as_str(),
            r.email.as_deref().unwrap_or(""),
            if r.is_admin != 0 { "1" } else { "0" },
            if r.is_active != 0 { "1" } else { "0" },
            &r.created_at.to_string(),
            &r.last_login_at.map(|v| v.to_string()).unwrap_or_default(),
            r.registration_ip.as_deref().unwrap_or(""),
            r.last_login_ip.as_deref().unwrap_or(""),
        ])
        .map_err(|e| AppError::Internal(anyhow::anyhow!("csv: {e}")))?;
    }
    let body = wtr
        .into_inner()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("csv flush: {e}")))?;
    audit_export_record(&state, &admin, "users.csv", "users_csv").await?;
    Ok(csv_response("users.csv", body))
}

// ---------- audit.csv ----------

pub async fn export_audit_csv(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Query(f): Query<ExportFilter>,
) -> Result<Response, AppError> {
    let rows = audit::search(&state.pool, None, None, f.from, f.to, 100_000)
        .await
        .map_err(AppError::Internal)?;
    let mut wtr = csv::Writer::from_writer(vec![]);
    wtr.write_record([
        "id",
        "user_id",
        "machine_id",
        "action",
        "target_type",
        "target_id",
        "created_at",
    ])
    .map_err(|e| AppError::Internal(anyhow::anyhow!("csv: {e}")))?;
    for r in rows {
        wtr.write_record([
            &r.id.to_string(),
            r.user_id.as_deref().unwrap_or(""),
            r.machine_id.as_deref().unwrap_or(""),
            r.action.as_str(),
            r.target_type.as_deref().unwrap_or(""),
            r.target_id.as_deref().unwrap_or(""),
            &r.created_at.to_string(),
        ])
        .map_err(|e| AppError::Internal(anyhow::anyhow!("csv: {e}")))?;
    }
    let body = wtr
        .into_inner()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("csv flush: {e}")))?;
    audit_export_record(&state, &admin, "audit.csv", "audit_csv").await?;
    Ok(csv_response("audit.csv", body))
}

// ---------- observations.csv ----------

pub async fn export_observations_csv(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Query(f): Query<ExportFilter>,
) -> Result<Response, AppError> {
    let user_id = if let Some(s) = f.user.as_deref() {
        users::brief_by_username(&state.pool, s)
            .await
            .map_err(AppError::Internal)?
            .map(|(id, _)| id)
    } else {
        None
    };
    let rows = crate::db::observations::admin_search(
        &state.pool,
        None,
        user_id.as_deref(),
        f.project.as_deref(),
        None,
        f.from,
        f.to,
        true,
        100_000,
        0,
    )
    .await
    .map_err(AppError::Internal)?;
    let mut wtr = csv::Writer::from_writer(vec![]);
    wtr.write_record([
        "id",
        "user_id",
        "username",
        "machine_id",
        "project_id",
        "project_name",
        "timestamp",
        "project_path",
        "obs_type",
        "server_seq",
        "server_received_at",
        "deleted_at",
        "content",
    ])
    .map_err(|e| AppError::Internal(anyhow::anyhow!("csv: {e}")))?;
    for r in rows {
        wtr.write_record([
            r.id.as_str(),
            r.user_id.as_str(),
            r.username.as_str(),
            r.machine_id.as_str(),
            r.project_id.as_deref().unwrap_or(""),
            r.project_name.as_deref().unwrap_or(""),
            &r.timestamp.to_string(),
            r.project_path.as_deref().unwrap_or(""),
            r.obs_type.as_deref().unwrap_or(""),
            &r.server_seq.to_string(),
            &r.server_received_at.to_string(),
            &r.deleted_at.map(|v| v.to_string()).unwrap_or_default(),
            r.content.as_str(),
        ])
        .map_err(|e| AppError::Internal(anyhow::anyhow!("csv: {e}")))?;
    }
    let body = wtr
        .into_inner()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("csv flush: {e}")))?;
    audit_export_record(&state, &admin, "observations.csv", "observations_csv").await?;
    Ok(csv_response("observations.csv", body))
}

// ---------- full.db.gz ----------
//
// 走 `VACUUM INTO ?` 写到 tempfile,然后 gzip 整文件返回。
// 优点:得到的是完整的 SQLite 数据库,可直接 `sqlite3 file.db` 打开。
// 限制:tempfile 占用磁盘 ≈ 数据库当前大小。

pub async fn export_full_db(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
) -> Result<Response, AppError> {
    let tmp = tempfile::Builder::new()
        .prefix("cmem-export-")
        .suffix(".db")
        .tempfile()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("tempfile: {e}")))?;
    let path = tmp.path().to_path_buf();
    // VACUUM INTO 期望目标文件不存在 — 删 tempfile 占位
    let _ = std::fs::remove_file(&path);

    let path_str = path.to_string_lossy().to_string();
    let sql = format!("VACUUM INTO '{}'", path_str.replace('\'', "''"));
    sqlx::query(&sql)
        .execute(&state.pool)
        .await
        .map_err(AppError::Db)?;

    let raw = std::fs::read(&path)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("read tempfile: {e}")))?;
    let _ = std::fs::remove_file(&path);

    let mut enc = GzEncoder::new(Vec::with_capacity(raw.len() / 2), Compression::default());
    enc.write_all(&raw)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("gzip write: {e}")))?;
    let body = enc
        .finish()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("gzip finish: {e}")))?;

    audit_export_record(&state, &admin, "full.db.gz", "full_db_gz").await?;
    Ok(binary_response(
        "cmem-server.db.gz",
        "application/gzip",
        body,
    ))
}

// ---------- per-user .zip ----------

pub async fn export_user_zip(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Path(user_id): Path<String>,
) -> Result<Response, AppError> {
    let user = users::find_by_id(&state.pool, &user_id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;

    let buf = Cursor::new(Vec::<u8>::with_capacity(64 * 1024));
    let mut zip = zip::ZipWriter::new(buf);
    let opts: zip::write::SimpleFileOptions =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // user.json
    let user_json = json!({
        "id": user.id,
        "username": user.username,
        "email": user.email,
        "is_admin": user.is_admin != 0,
        "is_active": user.is_active != 0,
        "created_at": user.created_at,
        "last_login_at": user.last_login_at,
        "registration_ip": user.registration_ip,
        "last_login_ip": user.last_login_ip,
    });
    write_zip_json(&mut zip, opts, "user.json", &user_json)?;

    // machines.json
    let machines = crate::db::machines::list_by_user(&state.pool, &user.id)
        .await
        .map_err(AppError::Internal)?;
    let machines_json: Vec<serde_json::Value> = machines
        .into_iter()
        .map(|m| {
            json!({
                "id": m.id,
                "name": m.name,
                "description": m.description,
                "last_seen_at": m.last_seen_at,
                "created_at": m.created_at,
                "revoked": m.revoked != 0,
            })
        })
        .collect();
    write_zip_json(&mut zip, opts, "machines.json", &machines_json)?;

    // projects.json
    let projects_rows = crate::db::projects::list_by_user(&state.pool, &user.id)
        .await
        .map_err(AppError::Internal)?;
    let projects_json: Vec<serde_json::Value> = projects_rows
        .into_iter()
        .map(|p| {
            json!({
                "id": p.id,
                "name": p.name,
                "display_name": p.display_name,
                "description": p.description,
                "is_excluded": p.is_excluded != 0,
                "created_at": p.created_at,
                "forked_from_project": p.forked_from_project,
                "forked_at": p.forked_at,
            })
        })
        .collect();
    write_zip_json(&mut zip, opts, "projects.json", &projects_json)?;

    // observations.json
    let obs = crate::db::observations::admin_search(
        &state.pool,
        None,
        Some(&user.id),
        None,
        None,
        None,
        None,
        true,
        1_000_000,
        0,
    )
    .await
    .map_err(AppError::Internal)?;
    let obs_json: Vec<serde_json::Value> = obs
        .into_iter()
        .map(|o| {
            json!({
                "id": o.id,
                "machine_id": o.machine_id,
                "project_id": o.project_id,
                "project_name": o.project_name,
                "timestamp": o.timestamp,
                "project_path": o.project_path,
                "content": o.content,
                "obs_type": o.obs_type,
                "server_seq": o.server_seq,
                "server_received_at": o.server_received_at,
                "deleted_at": o.deleted_at,
            })
        })
        .collect();
    write_zip_json(&mut zip, opts, "observations.json", &obs_json)?;

    // shares.json — sharer_user_id = user.id 的所有分享(含 revoked)
    let shares_rows = crate::db::shares::list_owned(&state.pool, &user.id)
        .await
        .map_err(AppError::Internal)?;
    let shares_json: Vec<serde_json::Value> = shares_rows
        .into_iter()
        .map(|s| {
            json!({
                "id": s.id,
                "project_id": s.project_id,
                "target_type": s.target_type,
                "target_user_id": s.target_user_id,
                "share_token": s.share_token,
                "share_mode": s.share_mode,
                "expires_at": s.expires_at,
                "created_at": s.created_at,
                "revoked_at": s.revoked_at,
            })
        })
        .collect();
    write_zip_json(&mut zip, opts, "shares.json", &shares_json)?;

    // audit.json — 该用户的事件
    let audit_rows = audit::search(&state.pool, Some(&user.id), None, None, None, 100_000)
        .await
        .map_err(AppError::Internal)?;
    let audit_json: Vec<serde_json::Value> = audit_rows
        .into_iter()
        .map(|a| {
            json!({
                "id": a.id,
                "machine_id": a.machine_id,
                "action": a.action,
                "target_type": a.target_type,
                "target_id": a.target_id,
                "created_at": a.created_at,
            })
        })
        .collect();
    write_zip_json(&mut zip, opts, "audit.json", &audit_json)?;

    let cursor = zip
        .finish()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("zip finish: {e}")))?;
    let body = cursor.into_inner();

    audit_export_record(&state, &admin, &format!("user/{}.zip", user.id), "user_zip").await?;
    Ok(binary_response(
        &format!("user-{}-{}.zip", user.username, user.id),
        "application/zip",
        body,
    ))
}

fn write_zip_json<W, V>(
    zip: &mut zip::ZipWriter<W>,
    opts: zip::write::SimpleFileOptions,
    name: &str,
    value: &V,
) -> Result<(), AppError>
where
    W: std::io::Write + std::io::Seek,
    V: serde::Serialize + ?Sized,
{
    zip.start_file(name, opts)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("zip start_file: {e}")))?;
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("json: {e}")))?;
    zip.write_all(&bytes)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("zip write: {e}")))?;
    Ok(())
}
