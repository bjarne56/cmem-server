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
