//! 项目识别核心算法(REQUIREMENTS 4.2 + 9.4)。
//!
//! resolve_project 决定一条 push observation 关联到哪个 project_id;
//! - 若客户端提交了 `.cmem-project.toml` 里的 marker_id → 用 marker_id 查/建
//! - 否则用规范化的 name 查找(自动跨机器合并)
//! - 都没有 → project_id = NULL(孤立 observation)

use anyhow::Result;
use sqlx::{Sqlite, Transaction};
use uuid::Uuid;

use crate::db::projects;

/// 客户端 push 时附带的项目元信息。
#[derive(Debug, Clone, Default)]
pub struct SubmittedProjectInfo {
    pub marker_id: Option<String>,
    pub name: Option<String>,
    pub path: Option<String>,
}

/// 规范化项目名:小写 + 非字母数字 → `-` + 合并连续 `-` + 去首尾 `-`。
///
/// 例: `Nginx RCE` → `nginx-rce`,`55.ai/research` → `55-ai-research`。
pub fn normalize_project_name(input: &str) -> String {
    input
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// 在事务里解析项目 id;不存在则创建,并写 path 别名(IF NOT EXISTS)。
///
/// 返回值:Some(project_id) 或 None(无 marker 也无 name)。
pub async fn resolve_project<'c>(
    tx: &mut Transaction<'c, Sqlite>,
    user_id: &str,
    machine_id: &str,
    submitted: &SubmittedProjectInfo,
    now: i64,
) -> Result<Option<String>> {
    // 1. marker_id 优先
    if let Some(marker_id) = submitted.marker_id.as_deref().map(str::trim) {
        if !marker_id.is_empty() {
            // 找已有
            if let Some(existing) =
                projects::find_by_id_in_tx(tx, user_id, marker_id).await?
            {
                if let Some(path) = submitted.path.as_deref() {
                    projects::record_path_in_tx(
                        tx,
                        &existing.id,
                        machine_id,
                        path,
                        Some(marker_id),
                        now,
                    )
                    .await?;
                }
                return Ok(Some(existing.id));
            }

            // 没找到 marker_id → 客户端持有的 .cmem-project.toml 里的 id 服务器还没看过。
            // 决定:用 marker_id 作为新项目 id,name 用规范化后的 submitted.name(或 marker_id 自身)。
            let raw_name = submitted.name.as_deref().unwrap_or(marker_id);
            let mut name = normalize_project_name(raw_name);
            if name.is_empty() {
                name = format!("project-{}", &marker_id[..marker_id.len().min(8)]);
            }

            // name 冲突时:同 user 下规范化 name 已被占,合并到已有项目而不是用 marker_id 创建。
            if let Some(existing) = projects::find_by_name_in_tx(tx, user_id, &name).await? {
                if let Some(path) = submitted.path.as_deref() {
                    projects::record_path_in_tx(
                        tx,
                        &existing.id,
                        machine_id,
                        path,
                        Some(marker_id),
                        now,
                    )
                    .await?;
                }
                return Ok(Some(existing.id));
            }

            projects::create_in_tx(tx, marker_id, user_id, &name, None, now).await?;
            if let Some(path) = submitted.path.as_deref() {
                projects::record_path_in_tx(
                    tx,
                    marker_id,
                    machine_id,
                    path,
                    Some(marker_id),
                    now,
                )
                .await?;
            }
            return Ok(Some(marker_id.to_string()));
        }
    }

    // 2. 按规范化的 name 查找/创建
    let raw_name = match &submitted.name {
        Some(n) if !n.trim().is_empty() => n.clone(),
        _ => return Ok(None),
    };
    let normalized = normalize_project_name(&raw_name);
    if normalized.is_empty() {
        return Ok(None);
    }

    if let Some(existing) = projects::find_by_name_in_tx(tx, user_id, &normalized).await? {
        if let Some(path) = submitted.path.as_deref() {
            projects::record_path_in_tx(tx, &existing.id, machine_id, path, None, now).await?;
        }
        return Ok(Some(existing.id));
    }

    let new_id = Uuid::now_v7().to_string();
    projects::create_in_tx(tx, &new_id, user_id, &normalized, None, now).await?;
    if let Some(path) = submitted.path.as_deref() {
        projects::record_path_in_tx(tx, &new_id, machine_id, path, None, now).await?;
    }
    Ok(Some(new_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_basic() {
        assert_eq!(normalize_project_name("nginx-rce"), "nginx-rce");
        assert_eq!(normalize_project_name("Nginx RCE"), "nginx-rce");
        assert_eq!(normalize_project_name("55.ai/research"), "55-ai-research");
        assert_eq!(normalize_project_name("---foo___bar---"), "foo-bar");
        assert_eq!(normalize_project_name(""), "");
        assert_eq!(normalize_project_name("   "), "");
    }

    #[test]
    fn normalize_unicode_keeps_alnum() {
        // is_alphanumeric 在 Rust 里包括 Unicode 字母,这是有意为之(中文项目名也有效)。
        let n = normalize_project_name("研究 项目 nginx");
        assert!(n.contains("nginx"));
        assert!(n.contains("研究"));
    }
}
