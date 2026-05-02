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
    admin::web::{
        i18n::{self, LangCtx, LANG_COOKIE_NAME, SUPPORTED_LANGS},
        templates as t,
    },
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

pub async fn login_page(ctx: LangCtx) -> Result<Response, AppError> {
    render(&t::LoginPage {
        ctx,
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
    ctx: LangCtx,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    // 表单层校验:用户名不能为空(避免传给 db 一个无效查询)。
    if form.username.trim().is_empty() {
        return Err(AppError::Validation("username required".into()));
    }
    let row = users::find_by_username(&state.pool, &form.username)
        .await
        .map_err(AppError::Internal)?;
    let user = match row {
        Some(u) if u.is_active != 0 && u.is_admin != 0 => u,
        Some(_) => {
            // username 存在但不是 admin / 已禁用 — 给同样的"无效凭据"提示,避免泄露 admin 身份信息
            let body = render(&t::LoginPage {
                ctx,
                // login.html 用 ctx.t(msg) 把 key 转成本地化文案
                error: Some("login.error.invalid"),
                username: &form.username,
            })?;
            return Ok((StatusCode::UNAUTHORIZED, body).into_response());
        }
        None => {
            let body = render(&t::LoginPage {
                ctx,
                error: Some("login.error.invalid"),
                username: &form.username,
            })?;
            return Ok((StatusCode::UNAUTHORIZED, body).into_response());
        }
    };
    let ok = verify_password(&form.password, &user.password_hash).map_err(AppError::Internal)?;
    if !ok {
        let body = render(&t::LoginPage {
            ctx,
            error: Some("login.error.invalid"),
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

// ---------- /register (公开注册页) ----------
//
// 跟 /admin/login 同等位置:公开访问,但套 CSRF + login rate limit。
// 注册成功 → 渲染同模板的成功视图(让用户看到去 viewer 登录的指引)。
// 失败 → 回模板 + 错误,表单字段回显避免用户重输。

#[derive(Debug, Deserialize)]
pub struct RegisterForm {
    pub username: String,
    pub password: String,
    pub password_confirm: String,
    pub email: Option<String>,
    pub invite_code: Option<String>,
}

pub async fn register_page(
    State(state): State<AppState>,
    ctx: LangCtx,
) -> Result<Response, AppError> {
    render(&t::RegisterPage {
        ctx,
        error: None,
        success: false,
        require_invite: state.config.auth.require_invite,
        username: "",
        email: "",
        invite_code: "",
    })
}

pub async fn do_register(
    State(state): State<AppState>,
    client_ip_ext: Option<axum::Extension<crate::middleware::ClientIp>>,
    connect: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    ctx: LangCtx,
    Form(form): Form<RegisterForm>,
) -> Result<Response, AppError> {
    let username = form.username.trim().to_string();
    let email_opt = form.email.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    let invite_opt = form.invite_code.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);

    // 表单层校验:password_confirm
    if form.password != form.password_confirm {
        return render(&t::RegisterPage {
            ctx: ctx.clone(),
            error: Some(&ctx.t("register.error.password_mismatch")),
            success: false,
            require_invite: state.config.auth.require_invite,
            username: &username,
            email: email_opt.as_deref().unwrap_or(""),
            invite_code: invite_opt.as_deref().unwrap_or(""),
        })
        .map(|r| (StatusCode::BAD_REQUEST, r).into_response());
    }

    // 复用 JSON API:构造 RegisterRequest 调内部 fn
    let req = cmem_shared::api::RegisterRequest {
        username: username.clone(),
        password: form.password,
        email: email_opt.clone(),
        invite_code: invite_opt.clone(),
    };

    match crate::auth::handlers::register(
        State(state.clone()),
        client_ip_ext,
        connect,
        axum::Json(req),
    )
    .await
    {
        Ok(_) => {
            // 成功 → success view
            render(&t::RegisterPage {
                ctx,
                error: None,
                success: true,
                require_invite: state.config.auth.require_invite,
                username: &username,
                email: email_opt.as_deref().unwrap_or(""),
                invite_code: invite_opt.as_deref().unwrap_or(""),
            })
        }
        Err(e) => {
            // 失败 → 同页面 + 错误。AppError 的 message 已经够直接(英文/中性)。
            // 表单字段回显,但密码不回显(不通过 form 字段保留密码本来就是对的)
            let msg = format!("{e}");
            render(&t::RegisterPage {
                ctx,
                error: Some(&msg),
                success: false,
                require_invite: state.config.auth.require_invite,
                username: &username,
                email: email_opt.as_deref().unwrap_or(""),
                invite_code: invite_opt.as_deref().unwrap_or(""),
            })
            .map(|r| (StatusCode::BAD_REQUEST, r).into_response())
        }
    }
}

pub async fn do_logout() -> Response {
    let cookie =
        format!("{ADMIN_COOKIE_NAME}=; HttpOnly; Path=/; Max-Age=0; SameSite=Strict");
    let mut resp = Redirect::to("/admin/login").into_response();
    resp.headers_mut()
        .append(header::SET_COOKIE, cookie.parse().expect("cookie value"));
    resp
}

// ---------- /admin/lang/:code ----------

#[derive(Debug, Deserialize)]
pub struct LangSwitchQuery {
    /// 切换语言后跳回的目标 URL。必须是同源相对路径,默认 `/admin`。
    pub next: Option<String>,
}

/// `GET /admin/lang/:code`:把 cookie `cmem_admin_lang` 设成指定语言,302 跳回 `next`(或 `/admin`)。
///
/// - 不在 SUPPORTED_LANGS 里的 code → 302 但不写 cookie(等价于无操作)。
/// - `next` 必须以 `/` 开头并且不含 `://`(防 open redirect)。
pub async fn switch_lang(
    Path(code): Path<String>,
    Query(q): Query<LangSwitchQuery>,
) -> Response {
    let next = q
        .next
        .as_deref()
        .filter(|n| n.starts_with('/') && !n.contains("://"))
        .unwrap_or("/admin");
    let mut resp = Redirect::to(next).into_response();
    if SUPPORTED_LANGS.contains(&code.as_str()) {
        // 一年期,Path=/,SameSite=Strict;明文存放即可(只是显示偏好)
        let cookie = format!(
            "{LANG_COOKIE_NAME}={code}; Path=/; Max-Age=31536000; SameSite=Strict"
        );
        resp.headers_mut()
            .append(header::SET_COOKIE, cookie.parse().expect("cookie value"));
    }
    resp
}

// ---------- /admin (dashboard) ----------

pub async fn dashboard(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    ctx: LangCtx,
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
        ctx,
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
    ctx: LangCtx,
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
        ctx,
        admin_username: &admin.username,
        query,
        rows: view,
    })
}

pub async fn user_detail_page(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    ctx: LangCtx,
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
        ctx,
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

// ---------- /admin/shares (form POST 创建共享) ----------

#[derive(Debug, Deserialize)]
pub struct ShareCreateForm {
    pub project_id: String,
    pub share_mode: String,
    /// 'user' / 'public' / 'link'(默认 user)
    #[serde(default)]
    pub target_type: String,
    /// target_type=user 时填
    #[serde(default)]
    pub target_username: String,
    /// 可选过期天数
    #[serde(default)]
    pub expires_days: String,
}

pub async fn shares_create_form(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Form(form): Form<ShareCreateForm>,
) -> Result<Redirect, AppError> {
    let project_id = form.project_id.trim();
    if project_id.is_empty() {
        return Err(AppError::Validation("project_id required".into()));
    }
    let share_mode = form.share_mode.trim();
    if !["read-only", "fork-allowed", "auto-copy"].contains(&share_mode) {
        return Err(AppError::Validation("share_mode must be read-only / fork-allowed / auto-copy".into()));
    }
    let target_type = if form.target_type.trim().is_empty() { "user" } else { form.target_type.trim() };
    if !["user", "public", "link"].contains(&target_type) {
        return Err(AppError::Validation("target_type must be user / public / link".into()));
    }
    let target_user_id: Option<String> = if target_type == "user" {
        let username = form.target_username.trim();
        if username.is_empty() {
            return Err(AppError::Validation("target_username required when target_type=user".into()));
        }
        let row = users::find_by_username(&state.pool, username).await.map_err(AppError::Internal)?;
        match row {
            Some(u) => Some(u.id),
            None => return Err(AppError::Validation(format!("user '{}' not found", username))),
        }
    } else {
        None
    };
    let expires_at: Option<i64> = form
        .expires_days
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|d| *d > 0)
        .map(|d| Utc::now().timestamp() + d * 86_400);
    let share_token: Option<String> = if target_type == "link" { Some(nanoid::nanoid!(32)) } else { None };

    let id = uuid::Uuid::now_v7().to_string();
    let now = Utc::now().timestamp();
    shares::create(
        &state.pool,
        &id,
        project_id,
        &admin.user_id,  // sharer_user_id = admin(代理 share,实际 owner 由 project 的 user_id 决定)
        target_type,
        target_user_id.as_deref(),
        share_token.as_deref(),
        share_mode,
        expires_at,
        now,
    )
    .await
    .map_err(AppError::Internal)?;
    audit::record(
        &state.pool,
        Some(&admin.user_id),
        None,
        "admin.share_create",
        Some("share"),
        Some(&id),
        Some(&serde_json::json!({"project_id":project_id,"target_type":target_type,"share_mode":share_mode}).to_string()),
        None,
        None,
        now,
    )
    .await
    .map_err(AppError::Internal)?;
    Ok(Redirect::to("/admin/shares"))
}

// ---------- /admin/users (form POST 创建用户) ----------

#[derive(Debug, Deserialize)]
pub struct UserCreateForm {
    pub username: String,
    #[serde(default)]
    pub email: String,
    pub password: String,
    /// HTML checkbox 不勾时 absent,勾时 value="true"
    pub is_admin: Option<String>,
}

pub async fn users_create_form(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Form(form): Form<UserCreateForm>,
) -> Result<Redirect, AppError> {
    use crate::auth::password::hash_password;
    use uuid::Uuid;

    let username = form.username.trim();
    if username.is_empty() {
        return Err(AppError::Validation("username required".into()));
    }
    if form.password.chars().count() < 8 {
        return Err(AppError::Validation("password must be ≥8 chars".into()));
    }
    if users::find_by_username(&state.pool, username)
        .await
        .map_err(AppError::Internal)?
        .is_some()
    {
        return Err(AppError::Conflict("username already taken".into()));
    }
    let id = Uuid::now_v7().to_string();
    let now = Utc::now().timestamp();
    let hash = hash_password(&form.password, &state.config.auth).map_err(AppError::Internal)?;
    let is_admin = matches!(form.is_admin.as_deref(), Some("true") | Some("on") | Some("1"));
    let email_opt = if form.email.trim().is_empty() { None } else { Some(form.email.trim()) };
    users::create_user(&state.pool, &id, username, &hash, email_opt, is_admin, now)
        .await
        .map_err(AppError::Internal)?;
    audit::record(
        &state.pool,
        Some(&admin.user_id),
        None,
        "admin.user_create",
        Some("user"),
        Some(&id),
        Some(&serde_json::json!({ "is_admin": is_admin }).to_string()),
        None,
        None,
        now,
    )
    .await
    .map_err(AppError::Internal)?;
    Ok(Redirect::to("/admin/users"))
}

// ---------- /admin/invites ----------

/// POST /admin/invites form-encoded(避免依赖 HTMX json-enc CDN)
/// 创建后 302 redirect 回 /admin/invites 让 list 刷新
#[derive(Debug, Deserialize)]
pub struct InviteCreateForm {
    pub max_uses: Option<String>,
    pub expires_days: Option<String>,
}

pub async fn invites_create_form(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    Form(form): Form<InviteCreateForm>,
) -> Result<Redirect, AppError> {
    let max_uses: i64 = form
        .max_uses
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<i64>().unwrap_or(1))
        .unwrap_or(1)
        .max(1);
    let expires_days: Option<i64> = form
        .expires_days
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|d| *d > 0);

    let code = nanoid::nanoid!(32);
    let now = Utc::now().timestamp();
    let expires_at = expires_days.map(|d| now + d * 86_400);
    invites::create(&state.pool, &code, Some(&admin.user_id), now, expires_at, max_uses)
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
    Ok(Redirect::to("/admin/invites"))
}

pub async fn invites_page(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    ctx: LangCtx,
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
        ctx,
        admin_username: &admin.username,
        rows: view,
    })
}

// ---------- /admin/projects ----------

pub async fn projects_page(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    ctx: LangCtx,
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
        ctx,
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
    ctx: LangCtx,
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
        ctx,
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
    ctx: LangCtx,
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
        ctx,
        admin_username: &admin.username,
        rows: view,
    })
}

// ---------- /admin/audit ----------

pub async fn audit_page(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminPrincipal>,
    ctx: LangCtx,
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
        ctx,
        admin_username: &admin.username,
        user_filter,
        action_filter,
        rows: view,
    })
}

// ---------- /admin/export ----------

pub async fn export_page(
    Extension(admin): Extension<AdminPrincipal>,
    ctx: LangCtx,
) -> Result<Response, AppError> {
    render(&t::ExportPage {
        ctx,
        admin_username: &admin.username,
    })
}

// ---------- HTMX 辅助:登录形式响应(仅 form,失败重渲染整页) ----------

#[allow(dead_code)]
pub fn unused_keep_helpers(_h: HeaderMap) {
    // 保留 i18n / SUPPORTED_LANGS 引用,避免未使用警告(实际两者都在 switch_lang 用到,不再需要)
    let _ = i18n::DEFAULT_LANG;
}
