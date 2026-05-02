//! refresh_tokens 表读写。
//!
//! 数据库存 token 的 SHA-256 hex,绝不存明文。

use anyhow::Result;
use sqlx::SqlitePool;

#[derive(Debug, Clone)]
pub struct RefreshTokenRow {
    pub token_hash: String,
    pub user_id: String,
    pub issued_at: i64,
    pub expires_at: i64,
    pub revoked: i64,
}

pub async fn insert_refresh(
    pool: &SqlitePool,
    token_hash: &str,
    user_id: &str,
    issued_at: i64,
    expires_at: i64,
    user_agent: Option<&str>,
    ip_address: Option<&str>,
) -> Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO refresh_tokens
            (token_hash, user_id, issued_at, expires_at, revoked, user_agent, ip_address)
        VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6)
        "#,
        token_hash,
        user_id,
        issued_at,
        expires_at,
        user_agent,
        ip_address,
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn find_active_refresh(
    pool: &SqlitePool,
    token_hash: &str,
    now: i64,
) -> Result<Option<RefreshTokenRow>> {
    let row = sqlx::query_as!(
        RefreshTokenRow,
        r#"
        SELECT
            token_hash AS "token_hash!: String",
            user_id    AS "user_id!: String",
            issued_at  AS "issued_at!: i64",
            expires_at AS "expires_at!: i64",
            revoked    AS "revoked!: i64"
        FROM refresh_tokens
        WHERE token_hash = ?1
          AND revoked = 0
          AND expires_at > ?2
        "#,
        token_hash,
        now,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 标记单条 refresh token 撤销。
pub async fn revoke_refresh(pool: &SqlitePool, token_hash: &str) -> Result<()> {
    sqlx::query!(
        r#"UPDATE refresh_tokens SET revoked = 1 WHERE token_hash = ?1"#,
        token_hash,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// 撤销该用户的所有 refresh token(改密时用)。
pub async fn revoke_all_for_user(pool: &SqlitePool, user_id: &str) -> Result<()> {
    sqlx::query!(
        r#"UPDATE refresh_tokens SET revoked = 1 WHERE user_id = ?1"#,
        user_id,
    )
    .execute(pool)
    .await?;
    Ok(())
}
