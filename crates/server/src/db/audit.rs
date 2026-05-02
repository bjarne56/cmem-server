//! audit_log 表写入。
//!
//! 严禁写入 password / token / observation content / API key 等敏感数据。

use anyhow::Result;
use sqlx::SqlitePool;

/// 记录一次审计事件。
///
/// `action` 必须用 CLAUDE.md 第「审计日志的 action 词汇表」中的固定词。
#[allow(clippy::too_many_arguments)]
pub async fn record(
    pool: &SqlitePool,
    user_id: Option<&str>,
    machine_id: Option<&str>,
    action: &str,
    target_type: Option<&str>,
    target_id: Option<&str>,
    metadata: Option<&str>,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
    created_at: i64,
) -> Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO audit_log
            (user_id, machine_id, action, target_type, target_id, metadata, ip_address, user_agent, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
        user_id,
        machine_id,
        action,
        target_type,
        target_id,
        metadata,
        ip_address,
        user_agent,
        created_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct AuditRow {
    pub id: i64,
    pub user_id: Option<String>,
    pub machine_id: Option<String>,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub created_at: i64,
}

/// 24h 内的事件总数。
pub async fn count_recent(pool: &SqlitePool, since: i64) -> Result<i64> {
    let row = sqlx::query!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM audit_log WHERE created_at >= ?1"#,
        since,
    )
    .fetch_one(pool)
    .await?;
    Ok(row.n)
}

/// admin 用过滤查询。所有过滤都是可选,limit 必填。
pub async fn search(
    pool: &SqlitePool,
    user_id: Option<&str>,
    action_prefix: Option<&str>,
    from: Option<i64>,
    to: Option<i64>,
    limit: i64,
) -> Result<Vec<AuditRow>> {
    let action_pat = action_prefix
        .map(|p| format!("{}%", p.replace('%', r"\%").replace('_', r"\_")))
        .unwrap_or_else(|| "%".to_string());
    let from = from.unwrap_or(0);
    let to = to.unwrap_or(i64::MAX);
    let user_filter_off: i64 = if user_id.is_some() { 0 } else { 1 };
    let user_id_arg = user_id.unwrap_or("");
    let rows = sqlx::query_as!(
        AuditRow,
        r#"
        SELECT
            id          AS "id!: i64",
            user_id     AS "user_id: String",
            machine_id  AS "machine_id: String",
            action      AS "action!: String",
            target_type AS "target_type: String",
            target_id   AS "target_id: String",
            created_at  AS "created_at!: i64"
        FROM audit_log
        WHERE (?1 = 1 OR user_id = ?2)
          AND action LIKE ?3 ESCAPE '\'
          AND created_at >= ?4
          AND created_at <= ?5
        ORDER BY created_at DESC
        LIMIT ?6
        "#,
        user_filter_off,
        user_id_arg,
        action_pat,
        from,
        to,
        limit,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 列出最近 N 条审计事件,可选按 user_id 过滤。
pub async fn list_recent(
    pool: &SqlitePool,
    user_id: Option<&str>,
    limit: i64,
) -> Result<Vec<AuditRow>> {
    let rows = match user_id {
        Some(uid) => {
            sqlx::query_as!(
                AuditRow,
                r#"
                SELECT
                    id          AS "id!: i64",
                    user_id     AS "user_id: String",
                    machine_id  AS "machine_id: String",
                    action      AS "action!: String",
                    target_type AS "target_type: String",
                    target_id   AS "target_id: String",
                    created_at  AS "created_at!: i64"
                FROM audit_log
                WHERE user_id = ?1
                ORDER BY created_at DESC
                LIMIT ?2
                "#,
                uid,
                limit,
            )
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as!(
                AuditRow,
                r#"
                SELECT
                    id          AS "id!: i64",
                    user_id     AS "user_id: String",
                    machine_id  AS "machine_id: String",
                    action      AS "action!: String",
                    target_type AS "target_type: String",
                    target_id   AS "target_id: String",
                    created_at  AS "created_at!: i64"
                FROM audit_log
                ORDER BY created_at DESC
                LIMIT ?1
                "#,
                limit,
            )
            .fetch_all(pool)
            .await?
        }
    };
    Ok(rows)
}
