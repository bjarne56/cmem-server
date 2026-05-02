//! axum router 装配。

use axum::{
    middleware::from_fn_with_state,
    routing::{get, post},
    Json, Router,
};
use cmem_shared::api::HealthResponse;

use crate::{auth::handlers as auth_handlers, middleware::require_auth, state::AppState};

/// 应用版本号(放进 healthz 响应)。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn build_router(state: AppState) -> Router {
    let public = Router::new()
        .route("/healthz", get(healthz))
        .route("/api/auth/register", post(auth_handlers::register))
        .route("/api/auth/login", post(auth_handlers::login))
        .route("/api/auth/refresh", post(auth_handlers::refresh));

    let protected = Router::new()
        .route("/api/auth/logout", post(auth_handlers::logout))
        .route(
            "/api/auth/change-password",
            post(auth_handlers::change_password),
        )
        .layer(from_fn_with_state(state.clone(), require_auth));

    public.merge(protected).with_state(state)
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: VERSION.to_string(),
    })
}
