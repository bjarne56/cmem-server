//! Admin web UI handlers — 渲染 askama 模板,挂在 `/admin/*`。

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    Extension, Form,
};
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;

use crate::{
    admin::middleware::{AdminPrincipal, ADMIN_COOKIE_NAME},
    admin::web::templates as t,
    auth::password::verify_password,
    db::{audit, invites, observations, projects, shares, stats, users},
    error::AppError,
    state::AppState,
};

/// 把 askama 渲染结果包成 HTML response。
fn render<T: Template>(tmpl: &T) -> Result<Response, AppError> {
    let body = tmpl
        .render()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("template render: {e}")))?;
    Ok(Html(body).into_response())
}

fn fmt_ts(ts: i64) -> String {
    let dt: DateTime<Utc> = Utc
        .timestamp_opt(ts, 0)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap_or_default());
    dt.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

fn fmt_opt_ts(ts: Option<i64>) -> String {
    ts.map(fmt_ts).unwrap_or_else(|| "-".into())
}

// ---------- /admin/login ----------

pub async fn login_page() -> Result<Response, AppError> {
    render(&t::LoginPage {
        error: None,
        username: "",
    })
}

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
}

pub async fn do_login(
    State(state): State<AppState>,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    let row = users::find_by_username(&state.pool, &form.username)
        .await
        .map_err(AppError::Internal)?;
    let user = match row {
        Some(u) if u.is_active != 0 && u.is_admin != 0 => u,
        Some(_) => {
            // username 存在但不是 admin / 已禁用 — 给同样的"无效凭据"提示,避免泄露 admin 身份信息
            let body = render(&t::LoginPage {
                error: Some("invalid credentials"),
                username: &form.username,
            })?;
            return Ok((StatusCode::UNAUTHORIZED, body).into_response());
        }
        None => {
            let body = render(&t::LoginPage {
                error: Some("invalid credentials"),
                username: &form.username,
            })?;
            return Ok((StatusCode::UNAUTHORIZED, body).into_response());
        }
    };
    let ok = verify_password(&form.password, &user.password_hash).map_err(AppError::Internal)?;
    if !ok {
        let body = render(&t::LoginPage {
            error: Some("invalid credentials"),
            username: &form.username,
        })?;
        return Ok((StatusCode::UNAUTHORIZED, body).into_response());
    }

    // 生成 access token,放进 HttpOnly cookie
    let (access, _exp) = state
        .jwt
        .encode_access(&user.id, None, state.config.auth.access_token_ttl_secs)
        .map_err(AppError::Internal)?;
    let now = Utc::now().timestamp();
    users::touch_last_login(&state.pool, &user.id, now)
        .await
        .map_err(AppError::Internal)?;
    audit::record(
        &state.pool,
        Some(&user.id),
        None,
        "admin.login",
        Some("user"),
        Some(&user.id),
        None,
        None,
        None,
        now,
    )
    .await
    .map_err(AppError::Internal)?;

    let cookie = format!(
        "{ADMIN_COOKIE_NAME}={access}; HttpOnly; Path=/; Max-Age={}; SameSite=Strict",
        state.config.auth.access_token_ttl_secs.max(60)
    );
    let mut resp = Redirect::to("/admin").into_response();
    resp.headers_mut()
        .append(header::SET_COOKIE, cookie.parse().expect("cookie value"));
    Ok(resp)
}

pub async fn do_logout() -> Response {
    let cookie =
        format!("{ADMIN_COOKIE_NAME}=; HttpOnly; Path=/; Max-Age=0; SameSite=Strict");
    let mut resp = Redirect::to("/admin/login").into_response();
    resp.headers_mut()
        .append(header::SET_COOKIE, cookie.parse().expect("cookie value"));
    resp
}

// ---------- /admin (dashboard) ----------

pub async fn dashboard(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
) -> Result<Response, AppError> {
    let g = stats::collect(&state.pool).await.map_err(AppError::Internal)?;
    let now = Utc::now().timestamp();
    let since = now - 86_400;
    let recent_users = users::count_recent_users(&state.pool, since)
        .await
        .map_err(AppError::Internal)?;
    let recent_obs = observations::count_recent(&state.pool, since)
        .await
        .map_err(AppError::Internal)?;
    let recent_audit = audit::count_recent(&state.pool, since)
        .await
        .map_err(AppError::Internal)?;
    render(&t::DashboardPage {
        admin_username: &admin.username,
        users: g.users,
        machines: g.machines,
        projects: g.projects,
        observations: g.observations,
        active_shares: g.shares,
        invites: g.invites,
        recent_users,
        recent_observations: recent_obs,
        recent_audit,
    })
}

// ---------- /admin/users ----------

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub q: Option<String>,
    pub user: Option<String>,
    pub project: Option<String>,
    #[serde(rename = "type")]
    pub obs_type: Option<String>,
    pub action: Option<String>,
}

pub async fn users_page(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Query(q): Query<ListQuery>,
) -> Result<Response, AppError> {
    let query = q.q.as_deref().unwrap_or("");
    let rows = users::list_paged(&state.pool, q.q.as_deref(), 200, 0)
        .await
        .map_err(AppError::Internal)?;
    let view: Vec<t::UserRow> = rows
        .into_iter()
        .map(|r| t::UserRow {
            admin_label: if r.is_admin != 0 { "yes" } else { "no" },
            active_label: if r.is_active != 0 { "yes" } else { "no" },
            admin_class: if r.is_admin != 0 {
                "bg-yellow-100 text-yellow-800"
            } else {
                "bg-gray-100 text-gray-700"
            },
            active_class: if r.is_active != 0 {
                "bg-green-100 text-green-800"
            } else {
                "bg-red-100 text-red-800"
            },
            id: r.id,
            username: r.username,
            email: r.email.unwrap_or_default(),
            is_admin: r.is_admin != 0,
            is_active: r.is_active != 0,
            created_at: fmt_ts(r.created_at),
            last_login_at: fmt_opt_ts(r.last_login_at),
            last_login_ip: r.last_login_ip.unwrap_or_else(|| "-".into()),
            registration_ip: r.registration_ip.unwrap_or_else(|| "-".into()),
            machine_count: r.machine_count,
            project_count: r.project_count,
            observation_count: r.observation_count,
        })
        .collect();
    render(&t::UsersPage {
        admin_username: &admin.username,
        query,
        rows: view,
    })
}

pub async fn user_detail_page(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let user = users::find_by_id(&state.pool, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    // 抽 audit_log 中该用户的 auth.login / admin.login 记录
    let audit_rows = audit::search(
        &state.pool,
        Some(&user.id),
        Some("auth."),
        None,
        None,
        100,
    )
    .await
    .map_err(AppError::Internal)?;
    let history: Vec<t::LoginHistoryRow> = audit_rows
        .into_iter()
        .filter(|r| matches!(r.action.as_str(), "auth.login" | "auth.register" | "auth.logout" | "auth.refresh" | "auth.password_change"))
        .map(|r| t::LoginHistoryRow {
            when: fmt_ts(r.created_at),
            action: r.action,
            ip: "-".into(), // ip_address 现在没有出现在 AuditRow,简化展示
        })
        .collect();
    render(&t::UserDetailPage {
        admin_username: &admin.username,
        user_id: &user.id,
        username: &user.username,
        email: user.email.as_deref().unwrap_or("-"),
        is_admin: user.is_admin != 0,
        is_active: user.is_active != 0,
        created_at: fmt_ts(user.created_at),
        registration_ip: user.registration_ip.clone().unwrap_or_else(|| "-".into()),
        last_login_at: fmt_opt_ts(user.last_login_at),
        last_login_ip: user.last_login_ip.clone().unwrap_or_else(|| "-".into()),
        login_history: history,
    })
}

// ---------- /admin/invites ----------

pub async fn invites_page(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
) -> Result<Response, AppError> {
    let now = Utc::now().timestamp();
    let rows = invites::list_all(&state.pool).await.map_err(AppError::Internal)?;
    let view: Vec<t::InviteRow> = rows
        .into_iter()
        .map(|r| {
            let (status, cls) = if r.use_count >= r.max_uses {
                ("exhausted", "bg-gray-100 text-gray-700")
            } else if r.expires_at.map(|e| e <= now).unwrap_or(false) {
                ("expired", "bg-orange-100 text-orange-800")
            } else {
                ("active", "bg-green-100 text-green-800")
            };
            t::InviteRow {
                code: r.code,
                max_uses: r.max_uses,
                use_count: r.use_count,
                status,
                status_class: cls,
                created: fmt_ts(r.created_at),
                expires: r
                    .expires_at
                    .map(fmt_ts)
                    .unwrap_or_else(|| "-".into()),
                used_by: r.used_by.unwrap_or_default(),
            }
        })
        .collect();
    render(&t::InvitesPage {
        admin_username: &admin.username,
        rows: view,
    })
}

// ---------- /admin/projects ----------

pub async fn projects_page(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Query(q): Query<ListQuery>,
) -> Result<Response, AppError> {
    let query = q.q.as_deref().unwrap_or("");
    let user_filter = q.user.as_deref().unwrap_or("");
    let user_id = if !user_filter.is_empty() {
        users::brief_by_username(&state.pool, user_filter)
            .await
            .map_err(AppError::Internal)?
            .map(|(id, _)| id)
    } else {
        None
    };
    let rows = projects::admin_search(
        &state.pool,
        user_id.as_deref(),
        if query.is_empty() { None } else { Some(query) },
        200,
        0,
    )
    .await
    .map_err(AppError::Internal)?;
    let view: Vec<t::ProjectRow> = rows
        .into_iter()
        .map(|r| t::ProjectRow {
            id: r.id,
            name: r.name,
            display_name: r.display_name.unwrap_or_default(),
            username: r.username,
            created: fmt_ts(r.created_at),
            observation_count: r.observation_count,
            share_count: r.share_count,
            is_excluded: r.is_excluded != 0,
        })
        .collect();
    render(&t::ProjectsPage {
        admin_username: &admin.username,
        query,
        user_filter,
        rows: view,
    })
}

// ---------- /admin/observations ----------

pub async fn observations_page(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Query(q): Query<ListQuery>,
) -> Result<Response, AppError> {
    let text_query = q.q.as_deref().unwrap_or("");
    let user_filter = q.user.as_deref().unwrap_or("");
    let project_filter = q.project.as_deref().unwrap_or("");
    let type_filter = q.obs_type.as_deref().unwrap_or("");
    let user_id = if !user_filter.is_empty() {
        users::brief_by_username(&state.pool, user_filter)
            .await
            .map_err(AppError::Internal)?
            .map(|(id, _)| id)
    } else {
        None
    };
    let rows = observations::admin_search(
        &state.pool,
        if text_query.is_empty() {
            None
        } else {
            Some(text_query)
        },
        user_id.as_deref(),
        if project_filter.is_empty() {
            None
        } else {
            Some(project_filter)
        },
        if type_filter.is_empty() {
            None
        } else {
            Some(type_filter)
        },
        None,
        None,
        true, // include deleted - admin 视角想看到全部
        100,
        0,
    )
    .await
    .map_err(AppError::Internal)?;
    let view: Vec<t::ObservationRow> = rows
        .into_iter()
        .map(|r| t::ObservationRow {
            id: r.id,
            username: r.username,
            project_name: r.project_name.unwrap_or_else(|| "(none)".into()),
            project_path: r.project_path.unwrap_or_default(),
            timestamp: fmt_ts(r.timestamp),
            obs_type: r.obs_type.unwrap_or_default(),
            content_preview: preview(&r.content, 200),
            server_seq: r.server_seq,
            deleted: r.deleted_at.is_some(),
        })
        .collect();
    render(&t::ObservationsPage {
        admin_username: &admin.username,
        query: text_query,
        user_filter,
        project_filter,
        type_filter,
        rows: view,
    })
}

fn preview(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{truncated}…")
    }
}

// ---------- /admin/shares ----------

pub async fn shares_page(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
) -> Result<Response, AppError> {
    let rows = shares::admin_list(&state.pool, 200, 0)
        .await
        .map_err(AppError::Internal)?;
    let view: Vec<t::ShareRow> = rows
        .into_iter()
        .map(|r| t::ShareRow {
            id: r.id,
            project_name: r.project_name,
            sharer_username: r.sharer_username,
            target_type: r.target_type,
            target_username: r.target_username.unwrap_or_default(),
            share_mode: r.share_mode,
            created: fmt_ts(r.created_at),
            expires: r.expires_at.map(fmt_ts).unwrap_or_else(|| "-".into()),
            revoked: r.revoked_at.is_some(),
        })
        .collect();
    render(&t::SharesPage {
        admin_username: &admin.username,
        rows: view,
    })
}

// ---------- /admin/audit ----------

pub async fn audit_page(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Query(q): Query<ListQuery>,
) -> Result<Response, AppError> {
    let user_filter = q.user.as_deref().unwrap_or("");
    let action_filter = q.action.as_deref().unwrap_or("");
    let user_id = if !user_filter.is_empty() {
        users::brief_by_username(&state.pool, user_filter)
            .await
            .map_err(AppError::Internal)?
            .map(|(id, _)| id)
    } else {
        None
    };
    let rows = audit::search(
        &state.pool,
        user_id.as_deref(),
        if action_filter.is_empty() {
            None
        } else {
            Some(action_filter)
        },
        None,
        None,
        500,
    )
    .await
    .map_err(AppError::Internal)?;
    let mut view: Vec<t::AuditRow> = Vec::with_capacity(rows.len());
    for r in rows {
        let username = if let Some(uid) = &r.user_id {
            users::brief_by_id(&state.pool, uid)
                .await
                .map_err(AppError::Internal)?
                .map(|(_, n)| n)
                .unwrap_or_else(|| uid.clone())
        } else {
            "-".into()
        };
        view.push(t::AuditRow {
            id: r.id,
            when: fmt_ts(r.created_at),
            username,
            action: r.action,
            target: format!(
                "{}:{}",
                r.target_type.unwrap_or_default(),
                r.target_id.unwrap_or_default()
            ),
        });
    }
    render(&t::AuditPage {
        admin_username: &admin.username,
        user_filter,
        action_filter,
        rows: view,
    })
}

// ---------- /admin/export ----------

pub async fn export_page(
    Extension(admin): Extension<AdminPrincipal>,
) -> Result<Response, AppError> {
    render(&t::ExportPage {
        admin_username: &admin.username,
    })
}

// ---------- HTMX 辅助:登录形式响应(仅 form,失败重渲染整页) ----------

#[allow(dead_code)]
pub fn unused_keep_helpers(_h: HeaderMap) {}
