//! HTTP API 请求 / 响应 DTO。
//!
//! 所有字段必须与 docs/API.md 保持一致。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::{MachineView, ObservationView, ProjectDetail, UserBrief, UserView};

// ---------- 认证 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
    pub email: Option<String>,
    pub invite_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub user: UserView,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResponse {
    pub user: UserView,
    pub access_token: String,
    pub access_token_expires_at: DateTime<Utc>,
    pub refresh_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshResponse {
    pub access_token: String,
    pub access_token_expires_at: DateTime<Utc>,
    pub refresh_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogoutRequest {
    pub refresh_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

// ---------- Health ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

// ---------- 机器 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMachineRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMachineResponse {
    pub machine: MachineView,
    /// 仅注册时返回一次,客户端必须妥善保存(类似 GitHub PAT)。
    pub machine_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListMachinesResponse {
    pub machines: Vec<MachineView>,
}

// ---------- 项目 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListProjectsResponse {
    pub projects: Vec<ProjectDetail>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectResponse {
    pub project: ProjectDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
    pub description: Option<String>,
}

/// PATCH /api/projects/:id 请求,字段全部可选。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PatchProjectRequest {
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub is_excluded: Option<bool>,
}

// ---------- 同步 ----------

/// 单条 push observation,客户端 JSON 提交。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushObservation {
    pub id: String,
    pub timestamp: i64,
    pub project_marker_id: Option<String>,
    pub project_name: Option<String>,
    pub project_path: Option<String>,
    pub content: String,
    pub obs_type: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub derived_from: Option<String>,
    pub derivation_chain: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushRequest {
    pub observations: Vec<PushObservation>,
}

/// 客户端提交的项目名 → 服务器分配的 project_id。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedProject {
    pub submitted_name: Option<String>,
    pub submitted_marker_id: Option<String>,
    pub submitted_path: Option<String>,
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushResponse {
    pub accepted: usize,
    pub duplicates: usize,
    pub errors: Vec<PushError>,
    pub server_seq_max: i64,
    pub projects_resolved: Vec<ResolvedProject>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushError {
    pub id: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PullRequest {
    #[serde(default)]
    pub since_seq: i64,
    pub limit: Option<i64>,
    #[serde(default = "default_true")]
    pub include_shared: bool,
    #[serde(default)]
    pub include_public: bool,
    #[serde(default)]
    pub exclude_machines: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedObservation {
    pub observation: ObservationView,
    pub share_mode: String,
    pub sharer_user_id: String,
    pub sharer_username: String,
    pub project_id: String,
    pub project_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DowngradeNotice {
    pub id: i64,
    pub project_id: String,
    pub project_name: String,
    pub owner_username: String,
    pub old_mode: String,
    pub new_mode: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevokedShare {
    pub project_id: String,
    pub project_name: String,
    pub owner_username: String,
    pub revoked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullResponse {
    pub own_observations: Vec<ObservationView>,
    pub shared_observations: Vec<SharedObservation>,
    pub pending_downgrades: Vec<DowngradeNotice>,
    pub revoked_shares: Vec<RevokedShare>,
    pub next_since_seq: i64,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AckDowngradesRequest {
    pub downgrade_ids: Vec<i64>,
}

// ---------- 共享 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateShareRequest {
    pub project_id: String,
    pub target_type: String, // "user" / "public" / "link"
    pub target_username: Option<String>,
    pub share_mode: String,
    pub expires_in_secs: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareResponse {
    pub share: ShareView,
    pub share_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareView {
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    pub sharer_user_id: String,
    pub sharer_username: String,
    pub target_type: String,
    pub target_user: Option<UserBrief>,
    pub share_token: Option<String>,
    pub share_mode: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PatchShareRequest {
    pub share_mode: Option<String>,
    pub expires_in_secs: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSharesResponse {
    /// 我作为 owner 创建的共享。
    pub owned: Vec<ShareView>,
    /// 别人共享给我的(target_type='user' 且 target_user_id = me)。
    pub received: Vec<ShareView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSharedProjectsResponse {
    pub shared_projects: Vec<SharedProjectEntry>,
    pub pending_downgrades: Vec<DowngradeNotice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedProjectEntry {
    pub project_id: String,
    pub project_name: String,
    pub owner_username: String,
    pub share_mode: String,
    pub observation_count: i64,
    pub shared_at: DateTime<Utc>,
}

// ---------- Fork ----------

/// POST /api/projects/:id/fork 请求体。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ForkProjectRequest {
    /// 可选:fork 后的项目名;不填则用 `<source-name>-fork-of-<owner-username>`。
    pub new_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkProjectResponse {
    pub project: ProjectDetail,
    /// 复制的 observation 数量。
    pub copied_observations: i64,
}

/// POST /api/observations/:id/fork 请求体。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkObservationRequest {
    /// 必填:fork 后归属的目标项目 id(必须是 forker 自己的项目)。
    pub to_project_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkObservationResponse {
    pub observation: ObservationView,
}
