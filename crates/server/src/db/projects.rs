//! projects + project_paths 表读写。

use anyhow::Result;
use sqlx::{Sqlite, SqlitePool, Transaction};

#[derive(Debug, Clone)]
pub struct ProjectRow {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub is_excluded: i64,
    pub forked_from_project: Option<String>,
    pub forked_at: Option<i64>,
    pub created_at: i64,
    /// 软删时间戳(unix sec)。NULL = 正常项目。回收站页面查 IS NOT NULL。
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ProjectPathRow {
    pub project_id: String,
    pub machine_id: String,
    pub machine_name: String,
    pub path: String,
    pub project_marker_id: Option<String>,
    pub created_at: i64,
    pub last_seen_at: i64,
}

/// 在事务里创建项目。
pub async fn create_in_tx<'c>(
    tx: &mut Transaction<'c, Sqlite>,
    id: &str,
    user_id: &str,
    name: &str,
    description: Option<&str>,
    created_at: i64,
) -> Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO projects
            (id, user_id, name, display_name, description, is_excluded, created_at)
        VALUES (?1, ?2, ?3, NULL, ?4, 0, ?5)
        "#,
        id,
        user_id,
        name,
        description,
        created_at,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// 池上创建(显式 POST /api/projects 用)。
pub async fn create(
    pool: &SqlitePool,
    id: &str,
    user_id: &str,
    name: &str,
    description: Option<&str>,
    created_at: i64,
) -> Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO projects
            (id, user_id, name, display_name, description, is_excluded, created_at)
        VALUES (?1, ?2, ?3, NULL, ?4, 0, ?5)
        "#,
        id,
        user_id,
        name,
        description,
        created_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// 在事务里按 (user_id, id) 查找。
pub async fn find_by_id_in_tx<'c>(
    tx: &mut Transaction<'c, Sqlite>,
    user_id: &str,
    id: &str,
) -> Result<Option<ProjectRow>> {
    let row = sqlx::query_as!(
        ProjectRow,
        r#"
        SELECT
            id                  AS "id!: String",
            user_id             AS "user_id!: String",
            name                AS "name!: String",
            display_name        AS "display_name: String",
            description         AS "description: String",
            is_excluded         AS "is_excluded!: i64",
            forked_from_project AS "forked_from_project: String",
            forked_at           AS "forked_at: i64",
            created_at          AS "created_at!: i64",
            deleted_at          AS "deleted_at: i64"
        FROM projects
        WHERE user_id = ?1 AND id = ?2
        "#,
        user_id,
        id,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row)
}

/// 在事务里按 (user_id, name) 查找(规范化后的 name)。
pub async fn find_by_name_in_tx<'c>(
    tx: &mut Transaction<'c, Sqlite>,
    user_id: &str,
    name: &str,
) -> Result<Option<ProjectRow>> {
    let row = sqlx::query_as!(
        ProjectRow,
        r#"
        SELECT
            id                  AS "id!: String",
            user_id             AS "user_id!: String",
            name                AS "name!: String",
            display_name        AS "display_name: String",
            description         AS "description: String",
            is_excluded         AS "is_excluded!: i64",
            forked_from_project AS "forked_from_project: String",
            forked_at           AS "forked_at: i64",
            created_at          AS "created_at!: i64",
            deleted_at          AS "deleted_at: i64"
        FROM projects
        WHERE user_id = ?1 AND name = ?2
        "#,
        user_id,
        name,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row)
}

/// 池上按 (user_id, id) 查询。
pub async fn find_by_id(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
) -> Result<Option<ProjectRow>> {
    let row = sqlx::query_as!(
        ProjectRow,
        r#"
        SELECT
            id                  AS "id!: String",
            user_id             AS "user_id!: String",
            name                AS "name!: String",
            display_name        AS "display_name: String",
            description         AS "description: String",
            is_excluded         AS "is_excluded!: i64",
            forked_from_project AS "forked_from_project: String",
            forked_at           AS "forked_at: i64",
            created_at          AS "created_at!: i64",
            deleted_at          AS "deleted_at: i64"
        FROM projects
        WHERE user_id = ?1 AND id = ?2 AND deleted_at IS NULL
        "#,
        user_id,
        id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 池上按 id 查询(无 user_id 过滤,共享场景查 owner 时使用)。
pub async fn find_any_by_id(pool: &SqlitePool, id: &str) -> Result<Option<ProjectRow>> {
    let row = sqlx::query_as!(
        ProjectRow,
        r#"
        SELECT
            id                  AS "id!: String",
            user_id             AS "user_id!: String",
            name                AS "name!: String",
            display_name        AS "display_name: String",
            description         AS "description: String",
            is_excluded         AS "is_excluded!: i64",
            forked_from_project AS "forked_from_project: String",
            forked_at           AS "forked_at: i64",
            created_at          AS "created_at!: i64",
            deleted_at          AS "deleted_at: i64"
        FROM projects
        WHERE id = ?1
        "#,
        id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 池上按 (user_id, name) 查询。
pub async fn find_by_name(
    pool: &SqlitePool,
    user_id: &str,
    name: &str,
) -> Result<Option<ProjectRow>> {
    let row = sqlx::query_as!(
        ProjectRow,
        r#"
        SELECT
            id                  AS "id!: String",
            user_id             AS "user_id!: String",
            name                AS "name!: String",
            display_name        AS "display_name: String",
            description         AS "description: String",
            is_excluded         AS "is_excluded!: i64",
            forked_from_project AS "forked_from_project: String",
            forked_at           AS "forked_at: i64",
            created_at          AS "created_at!: i64",
            deleted_at          AS "deleted_at: i64"
        FROM projects
        WHERE user_id = ?1 AND name = ?2 AND deleted_at IS NULL
        "#,
        user_id,
        name,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 列出该用户的所有项目。
pub async fn list_by_user(pool: &SqlitePool, user_id: &str) -> Result<Vec<ProjectRow>> {
    let rows = sqlx::query_as!(
        ProjectRow,
        r#"
        SELECT
            id                  AS "id!: String",
            user_id             AS "user_id!: String",
            name                AS "name!: String",
            display_name        AS "display_name: String",
            description         AS "description: String",
            is_excluded         AS "is_excluded!: i64",
            forked_from_project AS "forked_from_project: String",
            forked_at           AS "forked_at: i64",
            created_at          AS "created_at!: i64",
            deleted_at          AS "deleted_at: i64"
        FROM projects
        WHERE user_id = ?1 AND deleted_at IS NULL
        ORDER BY created_at ASC
        "#,
        user_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 列出某项目所有 path 别名(联机器名),按 last_seen_at DESC 排。
pub async fn list_paths(pool: &SqlitePool, project_id: &str) -> Result<Vec<ProjectPathRow>> {
    let rows = sqlx::query_as!(
        ProjectPathRow,
        r#"
        SELECT
            pp.project_id        AS "project_id!: String",
            pp.machine_id        AS "machine_id!: String",
            m.name               AS "machine_name!: String",
            pp.path              AS "path!: String",
            pp.project_marker_id AS "project_marker_id: String",
            pp.created_at        AS "created_at!: i64",
            pp.last_seen_at      AS "last_seen_at!: i64"
        FROM project_paths pp
        JOIN machines m ON m.id = pp.machine_id
        WHERE pp.project_id = ?1
        ORDER BY pp.last_seen_at DESC, pp.created_at ASC
        "#,
        project_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 在事务里写 path 别名(已存在则忽略)。
pub async fn record_path_in_tx<'c>(
    tx: &mut Transaction<'c, Sqlite>,
    project_id: &str,
    machine_id: &str,
    path: &str,
    project_marker_id: Option<&str>,
    created_at: i64,
) -> Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO project_paths
            (project_id, machine_id, path, project_marker_id, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(machine_id, path) DO NOTHING
        "#,
        project_id,
        machine_id,
        path,
        project_marker_id,
        created_at,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// 统计某用户每个项目的 observation 数。返回 HashMap-like Vec。
pub async fn observation_counts_for_user(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<(String, i64)>> {
    let rows = sqlx::query!(
        r#"
        SELECT project_id AS "project_id!: String",
               COUNT(*)   AS "count!: i64"
        FROM observations
        WHERE user_id = ?1 AND project_id IS NOT NULL AND deleted_at IS NULL
        GROUP BY project_id
        "#,
        user_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| (r.project_id, r.count)).collect())
}

/// admin 全局视角的项目行(带 owner username + obs 数 + path 数)。
#[derive(Debug, Clone)]
pub struct AdminProjectRow {
    pub id: String,
    pub user_id: String,
    pub username: String,
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub is_excluded: i64,
    pub created_at: i64,
    pub deleted_at: Option<i64>,
    pub observation_count: i64,
    pub share_count: i64,
    /// fork v12.7.2-plus.1 同步上来的 path 总数(跨所有机器)
    pub path_count: i64,
}

/// admin 全局列表 + 模糊搜 name + 过滤 user。
pub async fn admin_search(
    pool: &SqlitePool,
    user_id: Option<&str>,
    text_query: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<AdminProjectRow>> {
    let user_off: i64 = if user_id.is_some() { 0 } else { 1 };
    let user_arg = user_id.unwrap_or("");
    let like = text_query
        .map(|q| format!("%{}%", q.replace('%', r"\%").replace('_', r"\_")))
        .unwrap_or_else(|| "%".to_string());
    let rows = sqlx::query_as!(
        AdminProjectRow,
        r#"
        SELECT
            p.id           AS "id!: String",
            p.user_id      AS "user_id!: String",
            u.username     AS "username!: String",
            p.name         AS "name!: String",
            p.display_name AS "display_name: String",
            p.description  AS "description: String",
            p.is_excluded  AS "is_excluded!: i64",
            p.created_at   AS "created_at!: i64",
            p.deleted_at   AS "deleted_at: i64",
            (SELECT COUNT(*) FROM observations    o WHERE o.project_id = p.id AND o.deleted_at IS NULL) AS "observation_count!: i64",
            (SELECT COUNT(*) FROM project_shares  s WHERE s.project_id = p.id AND s.revoked_at IS NULL) AS "share_count!: i64",
            (SELECT COUNT(*) FROM project_paths   pp WHERE pp.project_id = p.id) AS "path_count!: i64"
        FROM projects p
        JOIN users u ON u.id = p.user_id
        WHERE (?1 = 1 OR p.user_id = ?2)
          AND p.deleted_at IS NULL
          AND (p.name LIKE ?3 ESCAPE '\' OR (p.display_name IS NOT NULL AND p.display_name LIKE ?3 ESCAPE '\'))
        ORDER BY p.created_at DESC
        LIMIT ?4 OFFSET ?5
        "#,
        user_off, user_arg,
        like,
        limit, offset,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 24h 内新增项目数。
pub async fn count_recent(pool: &SqlitePool, since: i64) -> Result<i64> {
    let row = sqlx::query!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM projects WHERE created_at >= ?1"#,
        since,
    )
    .fetch_one(pool)
    .await?;
    Ok(row.n)
}

/// 单项目 observation 数。
pub async fn observation_count(pool: &SqlitePool, project_id: &str) -> Result<i64> {
    let row = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "count!: i64"
        FROM observations
        WHERE project_id = ?1 AND deleted_at IS NULL
        "#,
        project_id,
    )
    .fetch_one(pool)
    .await?;
    Ok(row.count)
}

/// PATCH 字段(任意子集),返回是否更新成功。
pub async fn patch(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
    name: Option<&str>,
    display_name: Option<Option<&str>>,
    description: Option<Option<&str>>,
    is_excluded: Option<bool>,
) -> Result<bool> {
    // 构造原子 update;字段全可选,使用 COALESCE + sentinel 复杂,直接动态拼 4 个独立 query。
    // 为保持 sqlx 编译时检查,这里走「逐字段 update」策略,所有都在事务里。
    let mut tx = pool.begin().await?;
    if let Some(n) = name {
        sqlx::query!(
            r#"UPDATE projects SET name = ?3 WHERE user_id = ?1 AND id = ?2"#,
            user_id,
            id,
            n,
        )
        .execute(&mut *tx)
        .await?;
    }
    if let Some(dn) = display_name {
        sqlx::query!(
            r#"UPDATE projects SET display_name = ?3 WHERE user_id = ?1 AND id = ?2"#,
            user_id,
            id,
            dn,
        )
        .execute(&mut *tx)
        .await?;
    }
    if let Some(d) = description {
        sqlx::query!(
            r#"UPDATE projects SET description = ?3 WHERE user_id = ?1 AND id = ?2"#,
            user_id,
            id,
            d,
        )
        .execute(&mut *tx)
        .await?;
    }
    if let Some(ex) = is_excluded {
        let flag: i64 = if ex { 1 } else { 0 };
        sqlx::query!(
            r#"UPDATE projects SET is_excluded = ?3 WHERE user_id = ?1 AND id = ?2"#,
            user_id,
            id,
            flag,
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// 在事务里创建一个 fork 项目(forked_from_project + forked_at 不为 NULL)。
#[allow(clippy::too_many_arguments)]
pub async fn create_fork_in_tx<'c>(
    tx: &mut Transaction<'c, Sqlite>,
    id: &str,
    user_id: &str,
    name: &str,
    description: Option<&str>,
    forked_from_project: &str,
    forked_at: i64,
    created_at: i64,
) -> Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO projects
            (id, user_id, name, display_name, description, is_excluded,
             forked_from_project, forked_at, created_at)
        VALUES (?1, ?2, ?3, NULL, ?4, 0, ?5, ?6, ?7)
        "#,
        id,
        user_id,
        name,
        description,
        forked_from_project,
        forked_at,
        created_at,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// 删除项目:
/// 1. 先软删该项目下所有未删除的 observation(set deleted_at = now)
/// 2. 再硬删 project 行(level paths / shares 走 FK CASCADE)
///
/// 软删 observation 的好处:server_seq 仍单调,客户端按 cursor pull 不会拿到“已不存在的项目”。
///
/// 软删 project 本身(不再硬删):SET deleted_at = now。这样回收站可显示已删项目,
/// 用户可通过 restore() 恢复。彻底删除走单独的 hard_delete()(待实现)。
pub async fn delete(pool: &SqlitePool, user_id: &str, id: &str, now: i64) -> Result<bool> {
    let mut tx = pool.begin().await?;
    sqlx::query!(
        r#"
        UPDATE observations
        SET deleted_at = ?3
        WHERE project_id = ?1 AND user_id = ?2 AND deleted_at IS NULL
        "#,
        id,
        user_id,
        now,
    )
    .execute(&mut *tx)
    .await?;

    let res = sqlx::query!(
        r#"
        UPDATE projects
        SET deleted_at = ?3
        WHERE user_id = ?1 AND id = ?2 AND deleted_at IS NULL
        "#,
        user_id,
        id,
        now,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// 恢复软删的项目:project 的 deleted_at 清空 + 同步恢复其下所有 observations。
/// 与 delete() 对称,用于回收站"恢复"按钮。
pub async fn restore(pool: &SqlitePool, user_id: &str, id: &str) -> Result<bool> {
    let mut tx = pool.begin().await?;
    let res = sqlx::query!(
        r#"
        UPDATE projects
        SET deleted_at = NULL
        WHERE user_id = ?1 AND id = ?2 AND deleted_at IS NOT NULL
        "#,
        user_id,
        id,
    )
    .execute(&mut *tx)
    .await?;

    if res.rows_affected() > 0 {
        sqlx::query!(
            r#"
            UPDATE observations
            SET deleted_at = NULL
            WHERE project_id = ?1 AND user_id = ?2 AND deleted_at IS NOT NULL
            "#,
            id,
            user_id,
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// 彻底删除项目(物理删 projects + 级联删 obs/paths/shares 等)。
/// 与 delete()(软删进回收站) 区分,这是"回收站永久删除"按钮用的不可逆操作。
pub async fn hard_delete(pool: &SqlitePool, id: &str) -> Result<bool> {
    let mut tx = pool.begin().await?;
    // 先硬删该项目下所有 observations(级联清干净,不留孤儿)
    sqlx::query!("DELETE FROM observations WHERE project_id = ?1", id)
        .execute(&mut *tx).await?;
    // project_paths / project_shares 通过 ON DELETE CASCADE 自动清掉
    let res = sqlx::query!("DELETE FROM projects WHERE id = ?1", id)
        .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// 管理后台:列出所有已软删的项目(admin trash 页面用)。
pub async fn list_trashed_admin(
    pool: &SqlitePool,
    limit: i64,
    offset: i64,
) -> Result<Vec<AdminProjectRow>> {
    let rows = sqlx::query_as!(
        AdminProjectRow,
        r#"
        SELECT
            p.id           AS "id!: String",
            p.user_id      AS "user_id!: String",
            u.username     AS "username!: String",
            p.name         AS "name!: String",
            p.display_name AS "display_name: String",
            p.description  AS "description: String",
            p.is_excluded  AS "is_excluded!: i64",
            p.created_at   AS "created_at!: i64",
            p.deleted_at   AS "deleted_at: i64",
            (SELECT COUNT(*) FROM observations    o WHERE o.project_id = p.id) AS "observation_count!: i64",
            (SELECT COUNT(*) FROM project_shares  s WHERE s.project_id = p.id AND s.revoked_at IS NULL) AS "share_count!: i64",
            (SELECT COUNT(*) FROM project_paths   pp WHERE pp.project_id = p.id) AS "path_count!: i64"
        FROM projects p
        JOIN users u ON u.id = p.user_id
        WHERE p.deleted_at IS NOT NULL
        ORDER BY p.deleted_at DESC
        LIMIT ?1 OFFSET ?2
        "#,
        limit, offset,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
