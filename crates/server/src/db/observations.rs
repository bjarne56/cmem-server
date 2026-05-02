//! observations 表读写,以及全局 server_seq 计数器。

use anyhow::Result;
use sqlx::{Sqlite, SqlitePool, Transaction};

#[derive(Debug, Clone)]
pub struct ObservationRow {
    pub id: String,
    pub user_id: String,
    pub machine_id: String,
    pub project_id: Option<String>,
    pub timestamp: i64,
    pub project_path: Option<String>,
    pub content: String,
    pub obs_type: Option<String>,
    pub metadata: Option<String>,
    pub derived_from: Option<String>,
    pub derivation_chain: Option<String>,
    pub server_seq: i64,
    pub server_received_at: i64,
}

/// 在事务里取下一个 server_seq。
pub async fn next_server_seq<'c>(tx: &mut Transaction<'c, Sqlite>) -> Result<i64> {
    let row = sqlx::query!(
        r#"
        UPDATE server_seq_counter
        SET value = value + 1
        WHERE id = 1
        RETURNING value AS "value!: i64"
        "#
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.value)
}

/// 插入一条 observation(INSERT OR IGNORE)。
///
/// 返回是否实际插入(false = 因 id 冲突被忽略)。
#[allow(clippy::too_many_arguments)]
pub async fn insert_in_tx<'c>(
    tx: &mut Transaction<'c, Sqlite>,
    id: &str,
    user_id: &str,
    machine_id: &str,
    project_id: Option<&str>,
    timestamp: i64,
    project_path: Option<&str>,
    content: &str,
    obs_type: Option<&str>,
    metadata: Option<&str>,
    derived_from: Option<&str>,
    derivation_chain: Option<&str>,
    server_seq: i64,
    server_received_at: i64,
) -> Result<bool> {
    let res = sqlx::query!(
        r#"
        INSERT OR IGNORE INTO observations
            (id, user_id, machine_id, project_id, timestamp, project_path,
             content, obs_type, metadata, derived_from, derivation_chain,
             server_seq, server_received_at, deleted_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, NULL)
        "#,
        id,
        user_id,
        machine_id,
        project_id,
        timestamp,
        project_path,
        content,
        obs_type,
        metadata,
        derived_from,
        derivation_chain,
        server_seq,
        server_received_at,
    )
    .execute(&mut **tx)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 查询某用户在 since_seq 之后的 observation(单调 server_seq 顺序)。
pub async fn list_own_since(
    pool: &SqlitePool,
    user_id: &str,
    since_seq: i64,
    limit: i64,
    exclude_machines: &[String],
) -> Result<Vec<ObservationRow>> {
    // exclude_machines 走应用层过滤即可,反正 SQLite 本地很快;
    // 这里直接 SQL 拉所有,然后 in-process 过滤,简化 SQL 拼接。
    let rows = sqlx::query_as!(
        ObservationRow,
        r#"
        SELECT
            id                  AS "id!: String",
            user_id             AS "user_id!: String",
            machine_id          AS "machine_id!: String",
            project_id          AS "project_id: String",
            timestamp           AS "timestamp!: i64",
            project_path        AS "project_path: String",
            content             AS "content!: String",
            obs_type            AS "obs_type: String",
            metadata            AS "metadata: String",
            derived_from        AS "derived_from: String",
            derivation_chain    AS "derivation_chain: String",
            server_seq          AS "server_seq!: i64",
            server_received_at  AS "server_received_at!: i64"
        FROM observations
        WHERE user_id = ?1
          AND server_seq > ?2
          AND deleted_at IS NULL
        ORDER BY server_seq ASC
        LIMIT ?3
        "#,
        user_id,
        since_seq,
        limit,
    )
    .fetch_all(pool)
    .await?;

    if exclude_machines.is_empty() {
        return Ok(rows);
    }
    Ok(rows
        .into_iter()
        .filter(|r| !exclude_machines.iter().any(|m| m == &r.machine_id))
        .collect())
}

/// admin 视角下的 observation 行(带 user/project 名)。
#[derive(Debug, Clone)]
pub struct AdminObservationRow {
    pub id: String,
    pub user_id: String,
    pub username: String,
    pub machine_id: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub timestamp: i64,
    pub project_path: Option<String>,
    pub content: String,
    pub obs_type: Option<String>,
    pub server_seq: i64,
    pub server_received_at: i64,
    pub deleted_at: Option<i64>,
}

/// admin 用 — 跨用户的 observation 搜索(可选 FTS 文本 + user/project/type/时间过滤)。
#[allow(clippy::too_many_arguments)]
pub async fn admin_search(
    pool: &SqlitePool,
    text_query: Option<&str>,
    user_id: Option<&str>,
    project_id: Option<&str>,
    obs_type: Option<&str>,
    from: Option<i64>,
    to: Option<i64>,
    include_deleted: bool,
    limit: i64,
    offset: i64,
) -> Result<Vec<AdminObservationRow>> {
    // 把所有 nullable 过滤都用 sentinel + 条件 SQL 表达,避免动态拼 SQL。
    let user_off: i64 = if user_id.is_some() { 0 } else { 1 };
    let user_arg = user_id.unwrap_or("");
    let project_off: i64 = if project_id.is_some() { 0 } else { 1 };
    let project_arg = project_id.unwrap_or("");
    let type_off: i64 = if obs_type.is_some() { 0 } else { 1 };
    let type_arg = obs_type.unwrap_or("");
    let from = from.unwrap_or(0);
    let to = to.unwrap_or(i64::MAX);
    let deleted_off: i64 = if include_deleted { 1 } else { 0 };

    if let Some(q) = text_query {
        if !q.trim().is_empty() {
            let rows = sqlx::query_as!(
                AdminObservationRow,
                r#"
                SELECT
                    o.id                  AS "id!: String",
                    o.user_id             AS "user_id!: String",
                    u.username            AS "username!: String",
                    o.machine_id          AS "machine_id!: String",
                    o.project_id          AS "project_id: String",
                    p.name                AS "project_name: String",
                    o.timestamp           AS "timestamp!: i64",
                    o.project_path        AS "project_path: String",
                    o.content             AS "content!: String",
                    o.obs_type            AS "obs_type: String",
                    o.server_seq          AS "server_seq!: i64",
                    o.server_received_at  AS "server_received_at!: i64",
                    o.deleted_at          AS "deleted_at: i64"
                FROM observations_fts
                JOIN observations o ON o.id = observations_fts.id
                JOIN users        u ON u.id = o.user_id
                LEFT JOIN projects p ON p.id = o.project_id
                WHERE observations_fts MATCH ?1
                  AND (?2 = 1 OR o.user_id    = ?3)
                  AND (?4 = 1 OR o.project_id = ?5)
                  AND (?6 = 1 OR o.obs_type   = ?7)
                  AND o.timestamp >= ?8 AND o.timestamp <= ?9
                  AND (?10 = 1 OR o.deleted_at IS NULL)
                ORDER BY o.server_seq DESC
                LIMIT ?11 OFFSET ?12
                "#,
                q,
                user_off, user_arg,
                project_off, project_arg,
                type_off, type_arg,
                from, to,
                deleted_off,
                limit, offset,
            )
            .fetch_all(pool)
            .await?;
            return Ok(rows);
        }
    }

    let rows = sqlx::query_as!(
        AdminObservationRow,
        r#"
        SELECT
            o.id                  AS "id!: String",
            o.user_id             AS "user_id!: String",
            u.username            AS "username!: String",
            o.machine_id          AS "machine_id!: String",
            o.project_id          AS "project_id: String",
            p.name                AS "project_name: String",
            o.timestamp           AS "timestamp!: i64",
            o.project_path        AS "project_path: String",
            o.content             AS "content!: String",
            o.obs_type            AS "obs_type: String",
            o.server_seq          AS "server_seq!: i64",
            o.server_received_at  AS "server_received_at!: i64",
            o.deleted_at          AS "deleted_at: i64"
        FROM observations o
        JOIN users        u ON u.id = o.user_id
        LEFT JOIN projects p ON p.id = o.project_id
        WHERE (?1 = 1 OR o.user_id    = ?2)
          AND (?3 = 1 OR o.project_id = ?4)
          AND (?5 = 1 OR o.obs_type   = ?6)
          AND o.timestamp >= ?7 AND o.timestamp <= ?8
          AND (?9 = 1 OR o.deleted_at IS NULL)
        ORDER BY o.server_seq DESC
        LIMIT ?10 OFFSET ?11
        "#,
        user_off, user_arg,
        project_off, project_arg,
        type_off, type_arg,
        from, to,
        deleted_off,
        limit, offset,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// admin 软删:set deleted_at = now(若已经 deleted 就不动)。返回是否影响。
pub async fn soft_delete(pool: &SqlitePool, id: &str, now: i64) -> Result<bool> {
    let res = sqlx::query!(
        r#"UPDATE observations SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL"#,
        id,
        now,
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 24h 内新增 observation 数量。
pub async fn count_recent(pool: &SqlitePool, since: i64) -> Result<i64> {
    let row = sqlx::query!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM observations WHERE server_received_at >= ?1 AND deleted_at IS NULL"#,
        since,
    )
    .fetch_one(pool)
    .await?;
    Ok(row.n)
}

/// 当前全局最大 server_seq(用于 push 响应)。
pub async fn max_server_seq(pool: &SqlitePool) -> Result<i64> {
    let row = sqlx::query!(
        r#"SELECT value AS "value!: i64" FROM server_seq_counter WHERE id = 1"#
    )
    .fetch_one(pool)
    .await?;
    Ok(row.value)
}

/// 共享给 viewer 的 observation(JOIN project_shares + projects + users)。
///
/// 返回类型嵌入 share_mode / sharer / project 元信息,供 pull handler 直接转 API 类型。
#[derive(Debug, Clone)]
pub struct SharedObservationRow {
    pub id: String,
    pub user_id: String,
    pub machine_id: String,
    pub project_id: String,
    pub timestamp: i64,
    pub project_path: Option<String>,
    pub content: String,
    pub obs_type: Option<String>,
    pub metadata: Option<String>,
    pub derived_from: Option<String>,
    pub derivation_chain: Option<String>,
    pub server_seq: i64,
    pub server_received_at: i64,
    pub share_mode: String,
    pub sharer_user_id: String,
    pub sharer_username: String,
    pub project_name: String,
}

/// 拉取共享给 viewer 的 observation(target_type='user' AND target_user_id = viewer)。
///
/// 注意:auto-copy mode 在 client 处理(生成本地 derived_from 副本);
/// 这里 server 只做 JOIN 返回,不区分 mode。
pub async fn list_shared_since(
    pool: &SqlitePool,
    viewer_user_id: &str,
    since_seq: i64,
    limit: i64,
    now: i64,
) -> Result<Vec<SharedObservationRow>> {
    let rows = sqlx::query_as!(
        SharedObservationRow,
        r#"
        SELECT
            o.id                  AS "id!: String",
            o.user_id             AS "user_id!: String",
            o.machine_id          AS "machine_id!: String",
            o.project_id          AS "project_id!: String",
            o.timestamp           AS "timestamp!: i64",
            o.project_path        AS "project_path: String",
            o.content             AS "content!: String",
            o.obs_type            AS "obs_type: String",
            o.metadata            AS "metadata: String",
            o.derived_from        AS "derived_from: String",
            o.derivation_chain    AS "derivation_chain: String",
            o.server_seq          AS "server_seq!: i64",
            o.server_received_at  AS "server_received_at!: i64",
            ps.share_mode         AS "share_mode!: String",
            ps.sharer_user_id     AS "sharer_user_id!: String",
            su.username           AS "sharer_username!: String",
            p.name                AS "project_name!: String"
        FROM observations o
        JOIN project_shares ps
          ON ps.project_id = o.project_id
         AND ps.target_type = 'user'
         AND ps.target_user_id = ?1
         AND ps.revoked_at IS NULL
         AND (ps.expires_at IS NULL OR ps.expires_at > ?4)
        JOIN projects p ON p.id = o.project_id
        JOIN users   su ON su.id = ps.sharer_user_id
        WHERE o.server_seq > ?2
          AND o.deleted_at IS NULL
          AND o.project_id IS NOT NULL
        ORDER BY o.server_seq ASC
        LIMIT ?3
        "#,
        viewer_user_id,
        since_seq,
        limit,
        now,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
