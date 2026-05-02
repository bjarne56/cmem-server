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
