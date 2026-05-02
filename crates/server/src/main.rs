//! cmem-server 入口。

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use cmem_server::{
    auth::JwtCodec,
    config::AppConfig,
    db,
    server::{build_router, VERSION},
    state::AppState,
};
use tokio::net::TcpListener;
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Debug, Parser)]
#[command(version = VERSION, about = "cmem-sync server")]
struct Cli {
    /// 配置文件路径(可选;不存在时使用默认值并写回)。
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();

    let cfg = AppConfig::load_or_default(cli.config.as_deref()).context("load config")?;
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
    axum::serve(listener, router)
        .await
        .context("axum serve loop")?;

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).compact().init();
}
