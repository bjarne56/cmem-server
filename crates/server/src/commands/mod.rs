//! CLI 子命令分发。
//!
//! 顶层结构:
//!
//! ```text
//! cmem-server [serve]                        # 默认:启动 HTTP server
//! cmem-server admin user list|create|...      # 直接读写 SQLite
//! cmem-server admin invite create|list|...
//! cmem-server admin stats
//! cmem-server admin audit
//! ```
//!
//! 所有 admin 命令绕过 HTTP,直接打开同一个 sqlite 文件。

pub mod admin;
pub mod serve;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::server::VERSION;

#[derive(Debug, Parser)]
#[command(name = "cmem-server", version = VERSION, about = "cmem-sync server + admin CLI")]
pub struct Cli {
    /// 配置文件路径(可选;不存在时使用默认值并写回)。
    #[arg(short, long, value_name = "FILE", global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// 启动 HTTP server(默认子命令,可省略)。
    Serve,
    /// 管理操作:直接读写 SQLite,不需要 server 在跑。
    Admin {
        #[command(subcommand)]
        op: admin::AdminOp,
    },
}

/// 顶层入口:由 `main.rs` 调用,根据子命令分发。
pub async fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        None | Some(Commands::Serve) => serve::run(cli.config.as_deref()).await,
        Some(Commands::Admin { op }) => admin::run(cli.config.as_deref(), op).await,
    }
}
