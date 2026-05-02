//! 跨端共享的领域模型(传输形态)。
//!
//! 时间字段对外用 RFC3339 字符串(`chrono::DateTime<Utc>`),内部存储用 Unix epoch seconds。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 用户公开视图。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserView {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub is_admin: bool,
}

/// 机器视图。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineView {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub description: Option<String>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// 项目视图(基础)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectView {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub is_excluded: bool,
    pub forked_from_project: Option<String>,
    pub forked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// observation 视图(完整)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationView {
    pub id: String,
    pub user_id: String,
    pub machine_id: String,
    pub project_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub project_path: Option<String>,
    pub content: String,
    pub obs_type: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub derived_from: Option<String>,
    pub derivation_chain: Option<serde_json::Value>,
    pub server_seq: i64,
    pub server_received_at: DateTime<Utc>,
}

/// 项目内每台机器上的路径。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectPathView {
    pub machine_id: String,
    pub machine_name: String,
    pub path: String,
    pub project_marker_id: Option<String>,
}

/// 项目详细视图(列表/详情共用,paths/shares 可选填充)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectDetail {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub is_excluded: bool,
    pub forked_from_project: Option<String>,
    pub forked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub observation_count: i64,
    pub paths: Vec<ProjectPathView>,
    pub shares: Vec<ShareSummary>,
}

/// 共享摘要(嵌入项目详情时使用)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareSummary {
    pub id: String,
    pub target_type: String,
    pub target_user: Option<UserBrief>,
    pub share_mode: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// 简化用户视图(仅 id + username)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserBrief {
    pub id: String,
    pub username: String,
}
