//! axum router 装配。

use axum::{
    middleware::from_fn_with_state,
    routing::{delete, get, patch, post},
    Json, Router,
};
use cmem_shared::api::HealthResponse;

use crate::{
    admin::handlers as admin_api,
    admin::require_admin,
    admin::web::{export as admin_export, handlers as admin_web},
    auth::handlers as auth_handlers,
    machines::handlers as machine_handlers,
    middleware::{
        api_governor_layer, csrf_protect, extract_client_ip, login_governor_layer, require_auth,
    },
    projects::fork as project_fork,
    projects::handlers as project_handlers,
    shares::handlers as share_handlers,
    state::AppState,
    sync::handlers as sync_handlers,
};

/// 应用版本号(放进 healthz 响应)。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn build_router(state: AppState) -> Router {
    // 速率限制 layer。配置错误时打 warn,fall back 成"不限速"以避免启动失败。
    let login_rate = state.config.security.login_rate_per_minute;
    let api_rate = state.config.security.api_rate_per_minute;
    let login_layer = match login_governor_layer(login_rate) {
        Ok(l) => Some(l),
        Err(e) => {
            tracing::warn!(error = %e, "login rate limiter disabled (invalid config)");
            None
        }
    };
    let api_layer = match api_governor_layer(api_rate) {
        Ok(l) => Some(l),
        Err(e) => {
            tracing::warn!(error = %e, "api rate limiter disabled (invalid config)");
            None
        }
    };

    // /api/auth/{login,register} —— 公开但要严格限速防 brute / spam。
    let mut public_login_router = Router::new()
        .route("/api/auth/register", post(auth_handlers::register))
        .route("/api/auth/login", post(auth_handlers::login));
    if let Some(layer) = login_layer.clone() {
        public_login_router = public_login_router.layer(layer);
    }

    let public = Router::new()
        .route("/healthz", get(healthz))
        .route("/api/auth/refresh", post(auth_handlers::refresh))
        .merge(public_login_router);

    let protected = Router::new()
        // auth
        .route("/api/auth/logout", post(auth_handlers::logout))
        .route(
            "/api/auth/change-password",
            post(auth_handlers::change_password),
        )
        // machines
        .route("/api/machines", post(machine_handlers::create))
        .route("/api/machines", get(machine_handlers::list))
        .route("/api/machines/:id", delete(machine_handlers::revoke))
        // projects
        .route("/api/projects", get(project_handlers::list))
        .route("/api/projects", post(project_handlers::create))
        .route("/api/projects/:id", get(project_handlers::get))
        .route("/api/projects/:id", patch(project_handlers::patch))
        .route("/api/projects/:id", delete(project_handlers::delete))
        // fork
        .route("/api/projects/:id/fork", post(project_fork::fork_project))
        .route(
            "/api/observations/:id/fork",
            post(project_fork::fork_observation),
        )
        // sync
        .route("/api/sync/push", post(sync_handlers::push_handler))
        .route("/api/sync/pull", post(sync_handlers::pull_handler))
        // shares
        .route("/api/shares", post(share_handlers::create))
        .route("/api/shares", get(share_handlers::list))
        .route("/api/shares/:id", patch(share_handlers::patch))
        .route("/api/shares/:id", delete(share_handlers::revoke))
        .route("/api/shared", get(share_handlers::list_received))
        .route(
            "/api/shared/notifications/ack",
            post(share_handlers::ack_downgrades),
        )
        .layer(from_fn_with_state(state.clone(), require_auth));

    let mut admin_api_router = Router::new()
        .route("/stats", get(admin_api::get_stats))
        .route("/users", get(admin_api::list_users).post(admin_api::create_user))
        .route(
            "/users/:id",
            patch(admin_api::patch_user).delete(admin_api::delete_user),
        )
        .route(
            "/users/:id/reset-password",
            post(admin_api::reset_user_password),
        )
        .route(
            "/invites",
            get(admin_api::list_invites).post(admin_api::create_invite),
        )
        .route("/invites/:code", delete(admin_api::revoke_invite))
        .route("/projects", get(admin_api::list_projects))
        .route("/observations", get(admin_api::list_observations))
        .route(
            "/observations/:id",
            delete(admin_api::delete_observation),
        )
        .route("/shares", get(admin_api::list_shares))
        .route("/shares/:id", delete(admin_api::revoke_share))
        .route("/audit", get(admin_api::list_audit))
        .route("/export/users.csv", get(admin_export::export_users_csv))
        .route("/export/audit.csv", get(admin_export::export_audit_csv))
        .route(
            "/export/observations.csv",
            get(admin_export::export_observations_csv),
        )
        .route("/export/full.db.gz", get(admin_export::export_full_db))
        .route("/export/user/:id.zip", get(admin_export::export_user_zip))
        .layer(from_fn_with_state(state.clone(), require_admin));
    if let Some(layer) = api_layer.clone() {
        admin_api_router = admin_api_router.layer(layer);
    }

    let admin_web_protected = Router::new()
        .route("/", get(admin_web::dashboard))
        .route("/users", get(admin_web::users_page).post(admin_web::users_create_form))
        .route("/users/:id", get(admin_web::user_detail_page))
        .route("/invites", get(admin_web::invites_page).post(admin_web::invites_create_form))
        .route("/projects", get(admin_web::projects_page))
        .route("/observations", get(admin_web::observations_page))
        .route("/shares", get(admin_web::shares_page).post(admin_web::shares_create_form))
        .route("/audit", get(admin_web::audit_page))
        .route("/export", get(admin_web::export_page))
        .layer(from_fn_with_state(state.clone(), require_admin));

    // /admin/login 单独 sub-router,套两层中间件:
    //   - 内侧: CSRF(GET 发 token,POST 校验 _csrf)
    //   - 外侧: 严格 login 限速(5/min/IP)
    // 严格限速放最外层,这样 brute force 在中间件链最早就被 429,
    // CSRF 校验失败 / 密码错误等都不会被算进 brute 计数(因为 brute 需要先通过 limiter)。
    let mut admin_login_router = Router::new()
        .route(
            "/login",
            get(admin_web::login_page).post(admin_web::do_login),
        )
        .layer(from_fn_with_state(state.clone(), csrf_protect));
    if let Some(layer) = login_layer.clone() {
        admin_login_router = admin_login_router.layer(layer);
    }

    // 其它 /admin 公开路由(logout / lang switch)只需 CSRF。
    let admin_other_public = Router::new()
        .route("/logout", post(admin_web::do_logout))
        // 切换语言:GET /admin/lang/:code?next=/admin/users → set cookie + 302
        // 公开路由,因为登录页也需要切换语言
        .route("/lang/:code", get(admin_web::switch_lang))
        .layer(from_fn_with_state(state.clone(), csrf_protect));

    // /register 公开注册页 — 跟 /admin/login 同等级:CSRF + 严格 login rate limit。
    // 不挂在 /admin 下,因为这不是 admin 操作而是面向新用户。
    let mut register_router = Router::new()
        .route(
            "/register",
            get(admin_web::register_page).post(admin_web::do_register),
        )
        .layer(from_fn_with_state(state.clone(), csrf_protect));
    if let Some(layer) = login_layer.clone() {
        register_router = register_router.layer(layer);
    }

    // 受保护的 admin web 套 CSRF。其余按需拼一起。
    let admin_protected_with_csrf = admin_web_protected
        .layer(from_fn_with_state(state.clone(), csrf_protect));

    let admin_subtree = admin_login_router
        .merge(admin_other_public)
        .merge(admin_protected_with_csrf);

    public
        .merge(protected)
        .merge(register_router)
        .nest("/api/admin", admin_api_router)
        .nest("/admin", admin_subtree)
        // 全局最外层:把真实 client IP 解析进 extensions,所有下游(限速 /
        // CSRF / handlers / audit)都从 extensions 拿。这一层必须是最外层,
        // 因为 ClientIp 是限速 KeyExtractor 的依赖。
        .layer(from_fn_with_state(state.clone(), extract_client_ip))
        .with_state(state)
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: VERSION.to_string(),
    })
}
