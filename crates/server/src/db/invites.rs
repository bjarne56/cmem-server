//! invite_codes 表读写。

use anyhow::Result;
use sqlx::SqlitePool;

#[derive(Debug, Clone)]
pub struct InviteRow {
    pub code: String,
    pub created_by: Option<String>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub max_uses: i64,
    pub use_count: i64,
    pub used_by: Option<String>,
    pub used_at: Option<i64>,
}

/// 创建一条邀请码。
pub async fn create(
    pool: &SqlitePool,
    code: &str,
    created_by: Option<&str>,
    created_at: i64,
    expires_at: Option<i64>,
    max_uses: i64,
) -> Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO invite_codes
            (code, created_by, created_at, expires_at, max_uses, use_count)
        VALUES (?1, ?2, ?3, ?4, ?5, 0)
        "#,
        code,
        created_by,
        created_at,
        expires_at,
        max_uses,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// 列出所有邀请码,按 created_at 升序。
pub async fn list_all(pool: &SqlitePool) -> Result<Vec<InviteRow>> {
    let rows = sqlx::query_as!(
        InviteRow,
        r#"
        SELECT
            code        AS "code!: String",
            created_by  AS "created_by: String",
            created_at  AS "created_at!: i64",
            expires_at  AS "expires_at: i64",
            max_uses    AS "max_uses!: i64",
            use_count   AS "use_count!: i64",
            used_by     AS "used_by: String",
            used_at     AS "used_at: i64"
        FROM invite_codes
        ORDER BY created_at ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 按 code 查找。
pub async fn find_by_code(pool: &SqlitePool, code: &str) -> Result<Option<InviteRow>> {
    let row = sqlx::query_as!(
        InviteRow,
        r#"
        SELECT
            code        AS "code!: String",
            created_by  AS "created_by: String",
            created_at  AS "created_at!: i64",
            expires_at  AS "expires_at: i64",
            max_uses    AS "max_uses!: i64",
            use_count   AS "use_count!: i64",
            used_by     AS "used_by: String",
            used_at     AS "used_at: i64"
        FROM invite_codes
        WHERE code = ?1
        "#,
        code,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 撤销:直接删除。返回是否真的删了。
pub async fn revoke(pool: &SqlitePool, code: &str) -> Result<bool> {
    let res = sqlx::query!(r#"DELETE FROM invite_codes WHERE code = ?1"#, code)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// 消费一次邀请码:use_count += 1,首次使用记录 used_by/used_at。
///
/// 调用方负责事先校验 (expires_at, max_uses)。
pub async fn consume(
    pool: &SqlitePool,
    code: &str,
    user_id: &str,
    used_at: i64,
) -> Result<()> {
    sqlx::query!(
        r#"
        UPDATE invite_codes
        SET use_count = use_count + 1,
            used_by   = COALESCE(used_by, ?2),
            used_at   = COALESCE(used_at, ?3)
        WHERE code = ?1
        "#,
        code,
        user_id,
        used_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}
