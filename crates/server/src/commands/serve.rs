//! `cmem-server serve`:启动 axum HTTP server。

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::net::TcpListener;

use crate::auth::JwtCodec;
use crate::config::AppConfig;
use crate::db;
use crate::server::{build_router, VERSION};
use crate::state::AppState;

/// 启动 server 主循环。
pub async fn run(config_path: Option<&Path>) -> Result<()> {
    let cfg = AppConfig::load_or_default(config_path).context("load config")?;
    let cfg = Arc::new(cfg);

    let pool = db::connect(&cfg.database.path)
        .await
        .with_context(|| format!("open db at {}", cfg.database.path.display()))?;
    db::migrate(&pool).await.context("apply migrations")?;

    let jwt = JwtCodec::new(&cfg.auth.jwt_secret).context("init jwt codec")?;

    let state = AppState {
        pool,
        jwt,
        config: cfg.clone(),
    };

    let router = build_router(state);

    let listener = TcpListener::bind(&cfg.server.bind)
        .await
        .with_context(|| format!("bind {}", cfg.server.bind))?;

    tracing::info!(addr = %cfg.server.bind, version = VERSION, "cmem-server listening");
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .context("axum serve loop")?;

    Ok(())
}
