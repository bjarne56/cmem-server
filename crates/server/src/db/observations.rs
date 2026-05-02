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
