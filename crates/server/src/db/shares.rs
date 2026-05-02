//! project_shares + share_mode_downgrades 表读写。

use anyhow::Result;
use sqlx::SqlitePool;

#[derive(Debug, Clone)]
pub struct ShareRow {
    pub id: String,
    pub project_id: String,
    pub sharer_user_id: String,
    pub target_type: String,
    pub target_user_id: Option<String>,
    pub share_token: Option<String>,
    pub share_mode: String,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
}

/// 同一项目对同一 user 的 active share(用于 INSERT 前查重 + permissions)。
pub async fn find_active_user_share(
    pool: &SqlitePool,
    project_id: &str,
    target_user_id: &str,
    now: i64,
) -> Result<Option<ShareRow>> {
    let row = sqlx::query_as!(
        ShareRow,
        r#"
        SELECT
            id              AS "id!: String",
            project_id      AS "project_id!: String",
            sharer_user_id  AS "sharer_user_id!: String",
            target_type     AS "target_type!: String",
            target_user_id  AS "target_user_id: String",
            share_token     AS "share_token: String",
            share_mode      AS "share_mode!: String",
            expires_at      AS "expires_at: i64",
            created_at      AS "created_at!: i64",
            revoked_at      AS "revoked_at: i64"
        FROM project_shares
        WHERE project_id = ?1
          AND target_type = 'user'
          AND target_user_id = ?2
          AND revoked_at IS NULL
          AND (expires_at IS NULL OR expires_at > ?3)
        "#,
        project_id,
        target_user_id,
        now,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// public share 检查(target_type='public')。
pub async fn find_active_public_share(
    pool: &SqlitePool,
    project_id: &str,
    now: i64,
) -> Result<Option<ShareRow>> {
    let row = sqlx::query_as!(
        ShareRow,
        r#"
        SELECT
            id              AS "id!: String",
            project_id      AS "project_id!: String",
            sharer_user_id  AS "sharer_user_id!: String",
            target_type     AS "target_type!: String",
            target_user_id  AS "target_user_id: String",
            share_token     AS "share_token: String",
            share_mode      AS "share_mode!: String",
            expires_at      AS "expires_at: i64",
            created_at      AS "created_at!: i64",
            revoked_at      AS "revoked_at: i64"
        FROM project_shares
        WHERE project_id = ?1
          AND target_type = 'public'
          AND revoked_at IS NULL
          AND (expires_at IS NULL OR expires_at > ?2)
        "#,
        project_id,
        now,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

#[allow(clippy::too_many_arguments)]
pub async fn create(
    pool: &SqlitePool,
    id: &str,
    project_id: &str,
    sharer_user_id: &str,
    target_type: &str,
    target_user_id: Option<&str>,
    share_token: Option<&str>,
    share_mode: &str,
    expires_at: Option<i64>,
    created_at: i64,
) -> Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO project_shares
            (id, project_id, sharer_user_id, target_type, target_user_id,
             share_token, share_mode, expires_at, created_at, revoked_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)
        "#,
        id,
        project_id,
        sharer_user_id,
        target_type,
        target_user_id,
        share_token,
        share_mode,
        expires_at,
        created_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn find_by_id(pool: &SqlitePool, id: &str) -> Result<Option<ShareRow>> {
    let row = sqlx::query_as!(
        ShareRow,
        r#"
        SELECT
            id              AS "id!: String",
            project_id      AS "project_id!: String",
            sharer_user_id  AS "sharer_user_id!: String",
            target_type     AS "target_type!: String",
            target_user_id  AS "target_user_id: String",
            share_token     AS "share_token: String",
            share_mode      AS "share_mode!: String",
            expires_at      AS "expires_at: i64",
            created_at      AS "created_at!: i64",
            revoked_at      AS "revoked_at: i64"
        FROM project_shares
        WHERE id = ?1
        "#,
        id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 修改 share_mode + expires_at(只动这两个字段;调用方判断是否需要 downgrade 记录)。
pub async fn update_mode_and_expiry(
    pool: &SqlitePool,
    id: &str,
    new_mode: Option<&str>,
    new_expires_at: Option<Option<i64>>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    if let Some(m) = new_mode {
        sqlx::query!(
            r#"UPDATE project_shares SET share_mode = ?2 WHERE id = ?1"#,
            id,
            m,
        )
        .execute(&mut *tx)
        .await?;
    }
    if let Some(e) = new_expires_at {
        sqlx::query!(
            r#"UPDATE project_shares SET expires_at = ?2 WHERE id = ?1"#,
            id,
            e,
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// admin 全局视角 share + project/sharer/target 元信息。
#[derive(Debug, Clone)]
pub struct AdminShareRow {
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    pub sharer_user_id: String,
    pub sharer_username: String,
    pub target_type: String,
    pub target_user_id: Option<String>,
    pub target_username: Option<String>,
    pub share_token: Option<String>,
    pub share_mode: String,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
}

/// admin 全局列出所有 share(含 revoked,按 created_at DESC)。
pub async fn admin_list(pool: &SqlitePool, limit: i64, offset: i64) -> Result<Vec<AdminShareRow>> {
    let rows = sqlx::query_as!(
        AdminShareRow,
        r#"
        SELECT
            s.id              AS "id!: String",
            s.project_id      AS "project_id!: String",
            p.name            AS "project_name!: String",
            s.sharer_user_id  AS "sharer_user_id!: String",
            su.username       AS "sharer_username!: String",
            s.target_type     AS "target_type!: String",
            s.target_user_id  AS "target_user_id: String",
            tu.username       AS "target_username: String",
            s.share_token     AS "share_token: String",
            s.share_mode      AS "share_mode!: String",
            s.expires_at      AS "expires_at: i64",
            s.created_at      AS "created_at!: i64",
            s.revoked_at      AS "revoked_at: i64"
        FROM project_shares s
        JOIN projects p   ON p.id  = s.project_id
        JOIN users    su  ON su.id = s.sharer_user_id
        LEFT JOIN users tu ON tu.id = s.target_user_id
        ORDER BY s.created_at DESC
        LIMIT ?1 OFFSET ?2
        "#,
        limit,
        offset,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 当前活跃 share 数(未 revoked 且未过期)。
pub async fn count_active(pool: &SqlitePool, now: i64) -> Result<i64> {
    let row = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "n!: i64"
        FROM project_shares
        WHERE revoked_at IS NULL
          AND (expires_at IS NULL OR expires_at > ?1)
        "#,
        now,
    )
    .fetch_one(pool)
    .await?;
    Ok(row.n)
}

/// admin 强制 revoke(不要求 sharer 一致)。
pub async fn admin_revoke(pool: &SqlitePool, id: &str, now: i64) -> Result<bool> {
    let res = sqlx::query!(
        r#"UPDATE project_shares SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL"#,
        id,
        now,
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 撤销(set revoked_at)。
pub async fn revoke(pool: &SqlitePool, id: &str, now: i64) -> Result<()> {
    sqlx::query!(
        r#"UPDATE project_shares SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL"#,
        id,
        now,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// 我作为 sharer 创建的 share(包括 revoked,便于历史追溯)。
pub async fn list_owned(pool: &SqlitePool, sharer_user_id: &str) -> Result<Vec<ShareRow>> {
    let rows = sqlx::query_as!(
        ShareRow,
        r#"
        SELECT
            id              AS "id!: String",
            project_id      AS "project_id!: String",
            sharer_user_id  AS "sharer_user_id!: String",
            target_type     AS "target_type!: String",
            target_user_id  AS "target_user_id: String",
            share_token     AS "share_token: String",
            share_mode      AS "share_mode!: String",
            expires_at      AS "expires_at: i64",
            created_at      AS "created_at!: i64",
            revoked_at      AS "revoked_at: i64"
        FROM project_shares
        WHERE sharer_user_id = ?1
        ORDER BY created_at DESC
        "#,
        sharer_user_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 列出某项目所有 active share(用于 ProjectDetail.shares 字段)。
pub async fn list_active_by_project(pool: &SqlitePool, project_id: &str) -> Result<Vec<ShareRow>> {
    let rows = sqlx::query_as!(
        ShareRow,
        r#"
        SELECT
            id              AS "id!: String",
            project_id      AS "project_id!: String",
            sharer_user_id  AS "sharer_user_id!: String",
            target_type     AS "target_type!: String",
            target_user_id  AS "target_user_id: String",
            share_token     AS "share_token: String",
            share_mode      AS "share_mode!: String",
            expires_at      AS "expires_at: i64",
            created_at      AS "created_at!: i64",
            revoked_at      AS "revoked_at: i64"
        FROM project_shares
        WHERE project_id = ?1 AND revoked_at IS NULL
        ORDER BY created_at ASC
        "#,
        project_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 别人共享给我的 active share(target_type='user' AND target_user_id=me)。
pub async fn list_received_active(
    pool: &SqlitePool,
    target_user_id: &str,
    now: i64,
) -> Result<Vec<ShareRow>> {
    let rows = sqlx::query_as!(
        ShareRow,
        r#"
        SELECT
            id              AS "id!: String",
            project_id      AS "project_id!: String",
            sharer_user_id  AS "sharer_user_id!: String",
            target_type     AS "target_type!: String",
            target_user_id  AS "target_user_id: String",
            share_token     AS "share_token: String",
            share_mode      AS "share_mode!: String",
            expires_at      AS "expires_at: i64",
            created_at      AS "created_at!: i64",
            revoked_at      AS "revoked_at: i64"
        FROM project_shares
        WHERE target_type = 'user'
          AND target_user_id = ?1
          AND revoked_at IS NULL
          AND (expires_at IS NULL OR expires_at > ?2)
        ORDER BY created_at DESC
        "#,
        target_user_id,
        now,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 最近撤销的 share(给 pull 提示)。
///
/// `since` = 客户端记录的 last pulled time(epoch seconds);
/// 返回所有 revoked_at > since 且 target_user_id = me 的 share。
pub async fn list_recent_revoked(
    pool: &SqlitePool,
    target_user_id: &str,
    since: i64,
) -> Result<Vec<ShareRow>> {
    let rows = sqlx::query_as!(
        ShareRow,
        r#"
        SELECT
            id              AS "id!: String",
            project_id      AS "project_id!: String",
            sharer_user_id  AS "sharer_user_id!: String",
            target_type     AS "target_type!: String",
            target_user_id  AS "target_user_id: String",
            share_token     AS "share_token: String",
            share_mode      AS "share_mode!: String",
            expires_at      AS "expires_at: i64",
            created_at      AS "created_at!: i64",
            revoked_at      AS "revoked_at: i64"
        FROM project_shares
        WHERE target_type = 'user'
          AND target_user_id = ?1
          AND revoked_at IS NOT NULL
          AND revoked_at > ?2
        ORDER BY revoked_at DESC
        "#,
        target_user_id,
        since,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

// ---------- mode 降级通知 ----------

#[derive(Debug, Clone)]
pub struct DowngradeRow {
    pub id: i64,
    pub project_id: String,
    pub target_user_id: String,
    pub old_mode: String,
    pub new_mode: String,
    pub created_at: i64,
}

pub async fn record_downgrade(
    pool: &SqlitePool,
    project_id: &str,
    target_user_id: &str,
    old_mode: &str,
    new_mode: &str,
    created_at: i64,
) -> Result<i64> {
    let res = sqlx::query!(
        r#"
        INSERT INTO share_mode_downgrades
            (project_id, target_user_id, old_mode, new_mode, notified_at, created_at)
        VALUES (?1, ?2, ?3, ?4, NULL, ?5)
        "#,
        project_id,
        target_user_id,
        old_mode,
        new_mode,
        created_at,
    )
    .execute(pool)
    .await?;
    Ok(res.last_insert_rowid())
}

/// 列出未通知的 downgrade(target_user_id = me)。
pub async fn pending_downgrades(
    pool: &SqlitePool,
    target_user_id: &str,
) -> Result<Vec<DowngradeRow>> {
    let rows = sqlx::query_as!(
        DowngradeRow,
        r#"
        SELECT
            id              AS "id!: i64",
            project_id      AS "project_id!: String",
            target_user_id  AS "target_user_id!: String",
            old_mode        AS "old_mode!: String",
            new_mode        AS "new_mode!: String",
            created_at      AS "created_at!: i64"
        FROM share_mode_downgrades
        WHERE target_user_id = ?1 AND notified_at IS NULL
        ORDER BY created_at ASC
        "#,
        target_user_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 客户端 ack 一组 downgrade,设 notified_at = now。只能 ack 自己的。
pub async fn ack_downgrades(
    pool: &SqlitePool,
    target_user_id: &str,
    ids: &[i64],
    now: i64,
) -> Result<u64> {
    if ids.is_empty() {
        return Ok(0);
    }
    // sqlx query! 不支持 IN (?, ?, ?) 动态展开;走逐条 update 在事务里。
    let mut tx = pool.begin().await?;
    let mut total: u64 = 0;
    for id in ids {
        let res = sqlx::query!(
            r#"
            UPDATE share_mode_downgrades
            SET notified_at = ?3
            WHERE id = ?1 AND target_user_id = ?2 AND notified_at IS NULL
            "#,
            id,
            target_user_id,
            now,
        )
        .execute(&mut *tx)
        .await?;
        total += res.rows_affected();
    }
    tx.commit().await?;
    Ok(total)
}
