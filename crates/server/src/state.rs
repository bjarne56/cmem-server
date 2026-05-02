//! 应用全局共享状态。

use std::sync::Arc;

use sqlx::SqlitePool;

use crate::auth::JwtCodec;
use crate::config::AppConfig;

/// HTTP handler 共享的应用状态。
#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub jwt: JwtCodec,
    pub config: Arc<AppConfig>,
}
