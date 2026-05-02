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
    pub registration_ip: Option<String>,
    pub last_login_ip: Option<String>,
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
    create_user_with_ip(pool, id, username, password_hash, email, is_admin, created_at, None).await
}

/// 创建用户 + 记录注册 IP(register handler 调用)。
#[allow(clippy::too_many_arguments)]
pub async fn create_user_with_ip(
    pool: &SqlitePool,
    id: &str,
    username: &str,
    password_hash: &str,
    email: Option<&str>,
    is_admin: bool,
    created_at: i64,
    registration_ip: Option<&str>,
) -> Result<()> {
    let admin_flag: i64 = if is_admin { 1 } else { 0 };
    sqlx::query!(
        r#"
        INSERT INTO users (id, username, password_hash, email, is_admin, is_active, created_at, registration_ip)
        VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7)
        "#,
        id,
        username,
        password_hash,
        email,
        admin_flag,
        created_at,
        registration_ip,
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
            id              AS "id!: String",
            username        AS "username!: String",
            password_hash   AS "password_hash!: String",
            email           AS "email: String",
            is_admin        AS "is_admin!: i64",
            is_active       AS "is_active!: i64",
            created_at      AS "created_at!: i64",
            last_login_at   AS "last_login_at: i64",
            registration_ip AS "registration_ip: String",
            last_login_ip   AS "last_login_ip: String"
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
            id              AS "id!: String",
            username        AS "username!: String",
            password_hash   AS "password_hash!: String",
            email           AS "email: String",
            is_admin        AS "is_admin!: i64",
            is_active       AS "is_active!: i64",
            created_at      AS "created_at!: i64",
            last_login_at   AS "last_login_at: i64",
            registration_ip AS "registration_ip: String",
            last_login_ip   AS "last_login_ip: String"
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

/// 更新登录时间 + 客户端 IP。
pub async fn touch_last_login_with_ip(
    pool: &SqlitePool,
    id: &str,
    ts: i64,
    ip: Option<&str>,
) -> Result<()> {
    sqlx::query!(
        r#"UPDATE users SET last_login_at = ?2, last_login_ip = ?3 WHERE id = ?1"#,
        id,
        ts,
        ip,
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

// ---------- 管理员操作 ----------

/// 列出全部用户(admin 用)。
pub async fn list_all(pool: &SqlitePool) -> Result<Vec<UserRow>> {
    let rows = sqlx::query_as!(
        UserRow,
        r#"
        SELECT
            id              AS "id!: String",
            username        AS "username!: String",
            password_hash   AS "password_hash!: String",
            email           AS "email: String",
            is_admin        AS "is_admin!: i64",
            is_active       AS "is_active!: i64",
            created_at      AS "created_at!: i64",
            last_login_at   AS "last_login_at: i64",
            registration_ip AS "registration_ip: String",
            last_login_ip   AS "last_login_ip: String"
        FROM users
        ORDER BY created_at ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 按 username 删除(级联删除 machines / projects / observations 等)。
pub async fn delete_by_username(pool: &SqlitePool, username: &str) -> Result<bool> {
    let res = sqlx::query!(
        r#"DELETE FROM users WHERE username = ?1 COLLATE NOCASE"#,
        username,
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 设置 is_admin 标志。
pub async fn set_admin(pool: &SqlitePool, id: &str, is_admin: bool) -> Result<()> {
    let flag: i64 = if is_admin { 1 } else { 0 };
    sqlx::query!(
        r#"UPDATE users SET is_admin = ?2 WHERE id = ?1"#,
        id,
        flag,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// 设置 is_active 标志(disable / enable)。
pub async fn set_active(pool: &SqlitePool, id: &str, is_active: bool) -> Result<()> {
    let flag: i64 = if is_active { 1 } else { 0 };
    sqlx::query!(
        r#"UPDATE users SET is_active = ?2 WHERE id = ?1"#,
        id,
        flag,
    )
    .execute(pool)
    .await?;
    Ok(())
}

// ---------- admin web 用 ----------

/// 列出用户(带分页 + 模糊匹配 + 计数)。供 admin web 表格用。
#[derive(Debug, Clone)]
pub struct UserListRow {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub is_admin: i64,
    pub is_active: i64,
    pub created_at: i64,
    pub last_login_at: Option<i64>,
    pub registration_ip: Option<String>,
    pub last_login_ip: Option<String>,
    pub machine_count: i64,
    pub project_count: i64,
    pub observation_count: i64,
}

/// 带 LIKE/limit/offset 的分页查询;附带 machine/project/observation 计数。
pub async fn list_paged(
    pool: &SqlitePool,
    query: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<UserListRow>> {
    let like = query
        .map(|q| format!("%{}%", q.replace('%', r"\%").replace('_', r"\_")))
        .unwrap_or_else(|| "%".to_string());
    let rows = sqlx::query_as!(
        UserListRow,
        r#"
        SELECT
            u.id              AS "id!: String",
            u.username        AS "username!: String",
            u.email           AS "email: String",
            u.is_admin        AS "is_admin!: i64",
            u.is_active       AS "is_active!: i64",
            u.created_at      AS "created_at!: i64",
            u.last_login_at   AS "last_login_at: i64",
            u.registration_ip AS "registration_ip: String",
            u.last_login_ip   AS "last_login_ip: String",
            (SELECT COUNT(*) FROM machines     m WHERE m.user_id = u.id AND m.revoked = 0)        AS "machine_count!: i64",
            (SELECT COUNT(*) FROM projects     p WHERE p.user_id = u.id)                          AS "project_count!: i64",
            (SELECT COUNT(*) FROM observations o WHERE o.user_id = u.id AND o.deleted_at IS NULL) AS "observation_count!: i64"
        FROM users u
        WHERE u.username LIKE ?1 ESCAPE '\' OR (u.email IS NOT NULL AND u.email LIKE ?1 ESCAPE '\')
        ORDER BY u.created_at DESC
        LIMIT ?2 OFFSET ?3
        "#,
        like,
        limit,
        offset,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 当前活跃 admin 数量。用于"防止删/降级最后一个 admin"校验。
pub async fn count_active_admins(pool: &SqlitePool) -> Result<i64> {
    let row = sqlx::query!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM users WHERE is_admin = 1 AND is_active = 1"#
    )
    .fetch_one(pool)
    .await?;
    Ok(row.n)
}

/// 自某时间戳起新注册的用户数(用于 24h 趋势)。
pub async fn count_recent_users(pool: &SqlitePool, since: i64) -> Result<i64> {
    let row = sqlx::query!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM users WHERE created_at >= ?1"#,
        since,
    )
    .fetch_one(pool)
    .await?;
    Ok(row.n)
}

/// 按 id 删除(级联到 machines/projects/observations 等)。
pub async fn delete_by_id(pool: &SqlitePool, id: &str) -> Result<bool> {
    let res = sqlx::query!(r#"DELETE FROM users WHERE id = ?1"#, id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}
