//! 数据库连接 + migration runner + 仓储模块。

pub mod audit;
pub mod invites;
pub mod machines;
pub mod observations;
pub mod projects;
pub mod shares;
pub mod stats;
pub mod tokens;
pub mod users;

use anyhow::{Context, Result};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
};
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

/// 创建 SQLite 连接池(单连接 + WAL)。
pub async fn connect(db_path: &Path) -> Result<SqlitePool> {
    let url = format!("sqlite://{}", db_path.display());
    let opts = SqliteConnectOptions::from_str(&url)
        .with_context(|| format!("parse sqlite url {url}"))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .with_context(|| "connect sqlite")?;

    Ok(pool)
}

/// 启动时跑 migration。
pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    sqlx::migrate!("./src/db/migrations")
        .run(pool)
        .await
        .with_context(|| "run migrations")?;
    Ok(())
}
