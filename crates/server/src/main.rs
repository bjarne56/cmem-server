//! cmem-server 入口:解析 CLI 后转交 commands 模块。

use anyhow::Result;
use clap::Parser;
use cmem_server::commands::{dispatch, Cli};
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    dispatch(cli).await
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).compact().init();
}
