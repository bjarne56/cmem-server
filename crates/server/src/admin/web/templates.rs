//! Askama 模板 struct 定义。
//!
//! 编译时 inline 模板,运行时无 IO。模板文件在 `templates/` 目录。
//! 每个 page 内嵌 [`LangCtx`](super::i18n::LangCtx),模板里通过 `{{ ctx.t("key") }}`
//! 取本地化字符串、`{{ ctx.lang }}` / `{{ ctx.dir }}` 渲染 `<html lang>` / `<body dir>`。

use askama::Template;

use super::i18n::LangCtx;

#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginPage<'a> {
    pub ctx: LangCtx,
    pub error: Option<&'a str>,
    pub username: &'a str,
}

/// `/register` 公开注册页 — 给拿到邀请码的新用户用,不在 admin 子树下。
/// 注册成功后渲染同模板,把 `success=true`,把表单换成成功提示。
#[derive(Template)]
#[template(path = "register.html")]
pub struct RegisterPage<'a> {
    pub ctx: LangCtx,
    /// 已本地化的错误消息(直接显示);None = 无错
    pub error: Option<&'a str>,
    /// true = 注册成功,模板会切到 success 视图
    pub success: bool,
    /// server 是否强制邀请(影响 invite_code 必填 + 顶部提示文案)
    pub require_invite: bool,
    /// 表单回显:提交失败时保留输入,避免用户重输
    pub username: &'a str,
    pub email: &'a str,
    pub invite_code: &'a str,
}

#[derive(Template)]
#[template(path = "dashboard.html")]
pub struct DashboardPage<'a> {
    pub ctx: LangCtx,
    pub admin_username: &'a str,
    pub users: i64,
    pub machines: i64,
    pub projects: i64,
    pub observations: i64,
    pub active_shares: i64,
    pub invites: i64,
    pub recent_users: i64,
    pub recent_observations: i64,
    pub recent_audit: i64,
}

#[derive(Template)]
#[template(path = "users.html")]
pub struct UsersPage<'a> {
    pub ctx: LangCtx,
    pub admin_username: &'a str,
    pub query: &'a str,
    pub rows: Vec<UserRow>,
}

pub struct UserRow {
    pub id: String,
    pub username: String,
    pub email: String,
    pub admin_label: &'static str,
    pub active_label: &'static str,
    pub admin_class: &'static str,
    pub active_class: &'static str,
    pub is_admin: bool,
    pub is_active: bool,
    pub created_at: String,
    pub last_login_at: String,
    pub last_login_ip: String,
    pub registration_ip: String,
    pub machine_count: i64,
    pub project_count: i64,
    pub observation_count: i64,
}

#[derive(Template)]
#[template(path = "user_detail.html")]
pub struct UserDetailPage<'a> {
    pub ctx: LangCtx,
    pub admin_username: &'a str,
    pub user_id: &'a str,
    pub username: &'a str,
    pub email: &'a str,
    pub is_admin: bool,
    pub is_active: bool,
    pub created_at: String,
    pub registration_ip: String,
    pub last_login_at: String,
    pub last_login_ip: String,
    pub login_history: Vec<LoginHistoryRow>,
}

pub struct LoginHistoryRow {
    pub when: String,
    pub action: String,
    pub ip: String,
}

#[derive(Template)]
#[template(path = "invites.html")]
pub struct InvitesPage<'a> {
    pub ctx: LangCtx,
    pub admin_username: &'a str,
    pub rows: Vec<InviteRow>,
}

pub struct InviteRow {
    pub code: String,
    pub max_uses: i64,
    pub use_count: i64,
    /// 状态原始码:`active` / `expired` / `exhausted`(供模板查 i18n 标签)。
    pub status: &'static str,
    pub status_class: &'static str,
    pub created: String,
    pub expires: String,
    pub used_by: String,
}

impl InviteRow {
    /// 状态对应的 i18n key,模板 `{{ ctx.t(r.status_key()) }}`。
    pub fn status_key(&self) -> &'static str {
        match self.status {
            "active" => "invites.status.active",
            "expired" => "invites.status.expired",
            "exhausted" => "invites.status.exhausted",
            _ => "invites.status.active",
        }
    }
}

#[derive(Template)]
#[template(path = "projects.html")]
pub struct ProjectsPage<'a> {
    pub ctx: LangCtx,
    pub admin_username: &'a str,
    pub query: &'a str,
    pub user_filter: &'a str,
    pub rows: Vec<ProjectRow>,
}

pub struct ProjectRow {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub username: String,
    pub created: String,
    pub observation_count: i64,
    pub share_count: i64,
    pub is_excluded: bool,
}

#[derive(Template)]
#[template(path = "observations.html")]
pub struct ObservationsPage<'a> {
    pub ctx: LangCtx,
    pub admin_username: &'a str,
    pub query: &'a str,
    pub user_filter: &'a str,
    pub project_filter: &'a str,
    pub type_filter: &'a str,
    pub rows: Vec<ObservationRow>,
}

pub struct ObservationRow {
    pub id: String,
    pub username: String,
    pub project_name: String,
    pub project_path: String,
    pub timestamp: String,
    pub obs_type: String,
    pub content_preview: String,
    pub server_seq: i64,
    pub deleted: bool,
}

#[derive(Template)]
#[template(path = "shares.html")]
pub struct SharesPage<'a> {
    pub ctx: LangCtx,
    pub admin_username: &'a str,
    pub rows: Vec<ShareRow>,
}

pub struct ShareRow {
    pub id: String,
    pub project_name: String,
    pub sharer_username: String,
    pub target_type: String,
    pub target_username: String,
    pub share_mode: String,
    pub created: String,
    pub expires: String,
    pub revoked: bool,
}

#[derive(Template)]
#[template(path = "audit.html")]
pub struct AuditPage<'a> {
    pub ctx: LangCtx,
    pub admin_username: &'a str,
    pub user_filter: &'a str,
    pub action_filter: &'a str,
    pub rows: Vec<AuditRow>,
}

pub struct AuditRow {
    pub id: i64,
    pub when: String,
    pub username: String,
    pub action: String,
    pub target: String,
}

#[derive(Template)]
#[template(path = "export.html")]
pub struct ExportPage<'a> {
    pub ctx: LangCtx,
    pub admin_username: &'a str,
}
