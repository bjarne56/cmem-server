//! machines 表读写。

use anyhow::Result;
use sqlx::SqlitePool;

#[derive(Debug, Clone)]
pub struct MachineRow {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub description: Option<String>,
    pub last_seen_at: Option<i64>,
    pub created_at: i64,
    pub revoked: i64,
}

/// 创建一台机器(token 在调用前已经 hash)。
#[allow(clippy::too_many_arguments)]
pub async fn create_machine(
    pool: &SqlitePool,
    id: &str,
    user_id: &str,
    name: &str,
    description: Option<&str>,
    machine_token_hash: &str,
    created_at: i64,
) -> Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO machines
            (id, user_id, name, description, machine_token_hash, last_seen_at, created_at, revoked)
        VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, 0)
        "#,
        id,
        user_id,
        name,
        description,
        machine_token_hash,
        created_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// 根据 id 查询(任意状态,包括 revoked)。
pub async fn find_by_id(pool: &SqlitePool, id: &str) -> Result<Option<MachineRow>> {
    let row = sqlx::query_as!(
        MachineRow,
        r#"
        SELECT
            id           AS "id!: String",
            user_id      AS "user_id!: String",
            name         AS "name!: String",
            description  AS "description: String",
            last_seen_at AS "last_seen_at: i64",
            created_at   AS "created_at!: i64",
            revoked      AS "revoked!: i64"
        FROM machines
        WHERE id = ?1
        "#,
        id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 同一 user 下按 name 查询(用于注册时唯一性检查)。
pub async fn find_by_user_and_name(
    pool: &SqlitePool,
    user_id: &str,
    name: &str,
) -> Result<Option<MachineRow>> {
    let row = sqlx::query_as!(
        MachineRow,
        r#"
        SELECT
            id           AS "id!: String",
            user_id      AS "user_id!: String",
            name         AS "name!: String",
            description  AS "description: String",
            last_seen_at AS "last_seen_at: i64",
            created_at   AS "created_at!: i64",
            revoked      AS "revoked!: i64"
        FROM machines
        WHERE user_id = ?1 AND name = ?2
        "#,
        user_id,
        name,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 列出某用户的所有机器(包括已 revoked,留给前端展示历史)。
pub async fn list_by_user(pool: &SqlitePool, user_id: &str) -> Result<Vec<MachineRow>> {
    let rows = sqlx::query_as!(
        MachineRow,
        r#"
        SELECT
            id           AS "id!: String",
            user_id      AS "user_id!: String",
            name         AS "name!: String",
            description  AS "description: String",
            last_seen_at AS "last_seen_at: i64",
            created_at   AS "created_at!: i64",
            revoked      AS "revoked!: i64"
        FROM machines
        WHERE user_id = ?1
        ORDER BY created_at ASC
        "#,
        user_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 撤销机器(软撤销)。
pub async fn revoke(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query!(r#"UPDATE machines SET revoked = 1 WHERE id = ?1"#, id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 触发 last_seen_at 更新(push/pull 都调一次)。
pub async fn touch_last_seen(pool: &SqlitePool, id: &str, ts: i64) -> Result<()> {
    sqlx::query!(
        r#"UPDATE machines SET last_seen_at = ?2 WHERE id = ?1"#,
        id,
        ts
    )
    .execute(pool)
    .await?;
    Ok(())
}
