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
    middleware::require_auth,
    projects::handlers as project_handlers,
    shares::handlers as share_handlers,
    state::AppState,
    sync::handlers as sync_handlers,
};

/// 应用版本号(放进 healthz 响应)。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn build_router(state: AppState) -> Router {
    let public = Router::new()
        .route("/healthz", get(healthz))
        .route("/api/auth/register", post(auth_handlers::register))
        .route("/api/auth/login", post(auth_handlers::login))
        .route("/api/auth/refresh", post(auth_handlers::refresh));

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

    let admin_api_router = Router::new()
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

    let admin_web_protected = Router::new()
        .route("/", get(admin_web::dashboard))
        .route("/users", get(admin_web::users_page))
        .route("/users/:id", get(admin_web::user_detail_page))
        .route("/invites", get(admin_web::invites_page))
        .route("/projects", get(admin_web::projects_page))
        .route("/observations", get(admin_web::observations_page))
        .route("/shares", get(admin_web::shares_page))
        .route("/audit", get(admin_web::audit_page))
        .route("/export", get(admin_web::export_page))
        .layer(from_fn_with_state(state.clone(), require_admin));

    let admin_web_public = Router::new()
        .route(
            "/login",
            get(admin_web::login_page).post(admin_web::do_login),
        )
        .route("/logout", post(admin_web::do_logout))
        // 切换语言:GET /admin/lang/:code?next=/admin/users → set cookie + 302
        // 公开路由,因为登录页也需要切换语言
        .route("/lang/:code", get(admin_web::switch_lang));

    public
        .merge(protected)
        .nest("/api/admin", admin_api_router)
        .nest("/admin", admin_web_protected.merge(admin_web_public))
        .with_state(state)
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: VERSION.to_string(),
    })
}
