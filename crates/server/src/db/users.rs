//! users 表读写。

use anyhow::Result;
use sqlx::SqlitePool;

/// 数据库里的 user 行(原始字段)。
#[derive(Debug, Clone)]
pub struct UserRow {
    pub id: String,
    pub username: String,
    pub password_hash: String,
    pub email: Option<String>,
    pub is_admin: i64,
    pub is_active: i64,
    pub created_at: i64,
    pub last_login_at: Option<i64>,
}

/// 创建用户。返回新增行 id。
///
/// 调用者必须保证 password_hash 已经是 argon2id 形态。
pub async fn create_user(
    pool: &SqlitePool,
    id: &str,
    username: &str,
    password_hash: &str,
    email: Option<&str>,
    is_admin: bool,
    created_at: i64,
) -> Result<()> {
    let admin_flag: i64 = if is_admin { 1 } else { 0 };
    sqlx::query!(
        r#"
        INSERT INTO users (id, username, password_hash, email, is_admin, is_active, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)
        "#,
        id,
        username,
        password_hash,
        email,
        admin_flag,
        created_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// 按用户名查找(COLLATE NOCASE)。
pub async fn find_by_username(pool: &SqlitePool, username: &str) -> Result<Option<UserRow>> {
    let row = sqlx::query_as!(
        UserRow,
        r#"
        SELECT
            id            AS "id!: String",
            username      AS "username!: String",
            password_hash AS "password_hash!: String",
            email         AS "email: String",
            is_admin      AS "is_admin!: i64",
            is_active     AS "is_active!: i64",
            created_at    AS "created_at!: i64",
            last_login_at AS "last_login_at: i64"
        FROM users
        WHERE username = ?1 COLLATE NOCASE
        "#,
        username,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 按 id 查找。
pub async fn find_by_id(pool: &SqlitePool, id: &str) -> Result<Option<UserRow>> {
    let row = sqlx::query_as!(
        UserRow,
        r#"
        SELECT
            id            AS "id!: String",
            username      AS "username!: String",
            password_hash AS "password_hash!: String",
            email         AS "email: String",
            is_admin      AS "is_admin!: i64",
            is_active     AS "is_active!: i64",
            created_at    AS "created_at!: i64",
            last_login_at AS "last_login_at: i64"
        FROM users
        WHERE id = ?1
        "#,
        id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 更新登录时间。
pub async fn touch_last_login(pool: &SqlitePool, id: &str, ts: i64) -> Result<()> {
    sqlx::query!(
        r#"UPDATE users SET last_login_at = ?2 WHERE id = ?1"#,
        id,
        ts
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// 仅返回 (id, username),给 lookup / share 用。
pub async fn brief_by_id(pool: &SqlitePool, id: &str) -> Result<Option<(String, String)>> {
    let row = sqlx::query!(
        r#"SELECT id AS "id!: String", username AS "username!: String" FROM users WHERE id = ?1"#,
        id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.id, r.username)))
}

/// 仅返回 (id, username),按 username 查找(共享时用)。
pub async fn brief_by_username(
    pool: &SqlitePool,
    username: &str,
) -> Result<Option<(String, String)>> {
    let row = sqlx::query!(
        r#"
        SELECT id AS "id!: String", username AS "username!: String"
        FROM users
        WHERE username = ?1 COLLATE NOCASE
          AND is_active = 1
        "#,
        username,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.id, r.username)))
}

/// 修改密码哈希。
pub async fn update_password_hash(
    pool: &SqlitePool,
    id: &str,
    new_hash: &str,
) -> Result<()> {
    sqlx::query!(
        r#"UPDATE users SET password_hash = ?2 WHERE id = ?1"#,
        id,
        new_hash
    )
    .execute(pool)
    .await?;
    Ok(())
}
