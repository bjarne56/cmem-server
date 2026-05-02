//! 项目级权限检查。
//!
//! 见 docs/PROJECT_SHARING.md 第七节:`project_access_for` 是所有读取 endpoint 的入口。
//! 不变量 #1:owner 永远拥有完整权限。

use anyhow::Result;
use chrono::Utc;
use cmem_shared::ShareMode;
use sqlx::SqlitePool;

use crate::db::{projects, shares};

/// 三类访问权限(+ owner / none)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessLevel {
    Owner,
    AutoCopy,
    ForkAllowed,
    ReadOnly,
    None,
}

impl AccessLevel {
    pub fn can_read(self) -> bool {
        !matches!(self, Self::None)
    }

    pub fn can_fork(self) -> bool {
        matches!(self, Self::Owner | Self::ForkAllowed | Self::AutoCopy)
    }

    pub fn can_share(self) -> bool {
        matches!(self, Self::Owner)
    }

    pub fn from_mode(mode: ShareMode) -> Self {
        match mode {
            ShareMode::Owner => Self::Owner,
            ShareMode::ReadOnly => Self::ReadOnly,
            ShareMode::ForkAllowed => Self::ForkAllowed,
            ShareMode::AutoCopy => Self::AutoCopy,
        }
    }

    /// 用于「降级」判断:数值越小权限越大,fork-allowed → read-only 是降级(2 → 3),
    /// auto-copy → read-only 也是降级(1 → 3),read-only → fork-allowed 不是降级。
    pub fn rank(self) -> u8 {
        match self {
            Self::Owner => 0,
            Self::AutoCopy => 1,
            Self::ForkAllowed => 2,
            Self::ReadOnly => 3,
            Self::None => 4,
        }
    }
}

/// 解析数据库 share_mode 字符串(`'read-only'` / `'fork-allowed'` / `'auto-copy'`)。
pub fn parse_db_mode(s: &str) -> Option<AccessLevel> {
    match s {
        "read-only" => Some(AccessLevel::ReadOnly),
        "fork-allowed" => Some(AccessLevel::ForkAllowed),
        "auto-copy" => Some(AccessLevel::AutoCopy),
        _ => None,
    }
}

/// 主入口:某用户对某项目的访问权限。
pub async fn project_access_for(
    pool: &SqlitePool,
    user_id: &str,
    project_id: &str,
) -> Result<AccessLevel> {
    // 1. owner
    let project = projects::find_any_by_id(pool, project_id).await?;
    let Some(project) = project else {
        return Ok(AccessLevel::None);
    };
    if project.user_id == user_id {
        return Ok(AccessLevel::Owner);
    }

    let now = Utc::now().timestamp();

    // 2. 直接 user share
    if let Some(share) = shares::find_active_user_share(pool, project_id, user_id, now).await? {
        if let Some(level) = parse_db_mode(&share.share_mode) {
            return Ok(level);
        }
    }

    // 3. public share
    if let Some(share) = shares::find_active_public_share(pool, project_id, now).await? {
        if let Some(level) = parse_db_mode(&share.share_mode) {
            return Ok(level);
        }
    }

    Ok(AccessLevel::None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_ordering_matches_intuition() {
        assert!(AccessLevel::Owner.rank() < AccessLevel::AutoCopy.rank());
        assert!(AccessLevel::AutoCopy.rank() < AccessLevel::ForkAllowed.rank());
        assert!(AccessLevel::ForkAllowed.rank() < AccessLevel::ReadOnly.rank());
        assert!(AccessLevel::ReadOnly.rank() < AccessLevel::None.rank());
    }

    #[test]
    fn capabilities_per_level() {
        assert!(AccessLevel::Owner.can_share());
        assert!(!AccessLevel::ReadOnly.can_share());
        assert!(!AccessLevel::ReadOnly.can_fork());
        assert!(AccessLevel::ForkAllowed.can_fork());
        assert!(AccessLevel::AutoCopy.can_fork());
        assert!(AccessLevel::ReadOnly.can_read());
        assert!(!AccessLevel::None.can_read());
    }

    #[test]
    fn parse_db_mode_known_values() {
        assert_eq!(parse_db_mode("read-only"), Some(AccessLevel::ReadOnly));
        assert_eq!(parse_db_mode("fork-allowed"), Some(AccessLevel::ForkAllowed));
        assert_eq!(parse_db_mode("auto-copy"), Some(AccessLevel::AutoCopy));
        assert_eq!(parse_db_mode("garbage"), None);
    }
}
