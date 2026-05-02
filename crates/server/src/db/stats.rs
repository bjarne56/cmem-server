//! 全局统计(admin 用)。

use anyhow::Result;
use sqlx::SqlitePool;

#[derive(Debug, Clone, Copy)]
pub struct GlobalStats {
    pub users: i64,
    pub machines: i64,
    pub projects: i64,
    pub observations: i64,
    pub shares: i64,
    pub invites: i64,
}

/// 一次性聚合所有计数(单连接 + 多个 COUNT 查询)。
pub async fn collect(pool: &SqlitePool) -> Result<GlobalStats> {
    let users = sqlx::query!(r#"SELECT COUNT(*) AS "n!: i64" FROM users"#)
        .fetch_one(pool)
        .await?
        .n;
    let machines = sqlx::query!(r#"SELECT COUNT(*) AS "n!: i64" FROM machines"#)
        .fetch_one(pool)
        .await?
        .n;
    let projects = sqlx::query!(r#"SELECT COUNT(*) AS "n!: i64" FROM projects"#)
        .fetch_one(pool)
        .await?
        .n;
    let observations =
        sqlx::query!(r#"SELECT COUNT(*) AS "n!: i64" FROM observations WHERE deleted_at IS NULL"#)
            .fetch_one(pool)
            .await?
            .n;
    let shares = sqlx::query!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM project_shares WHERE revoked_at IS NULL"#
    )
    .fetch_one(pool)
    .await?
    .n;
    let invites = sqlx::query!(r#"SELECT COUNT(*) AS "n!: i64" FROM invite_codes"#)
        .fetch_one(pool)
        .await?
        .n;

    Ok(GlobalStats {
        users,
        machines,
        projects,
        observations,
        shares,
        invites,
    })
}
