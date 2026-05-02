//! 共享 mode 枚举与权限规则。

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

/// 项目共享 mode。
///
/// `Owner` 不存进数据库,仅用于权限判断时表示「viewer 是 owner」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShareMode {
    /// 项目所有者(权限最高)。
    #[serde(skip)]
    Owner,
    /// 仅可读,不进客户端 observations 表。
    ReadOnly,
    /// 可读 + 可主动 fork。
    ForkAllowed,
    /// pull 时自动复制为 viewer 名下的副本。
    AutoCopy,
}

impl ShareMode {
    pub fn as_db_str(self) -> &'static str {
        match self {
            ShareMode::Owner => "owner",
            ShareMode::ReadOnly => "read-only",
            ShareMode::ForkAllowed => "fork-allowed",
            ShareMode::AutoCopy => "auto-copy",
        }
    }

    pub fn can_fork(self) -> bool {
        matches!(self, Self::Owner | Self::ForkAllowed | Self::AutoCopy)
    }
}

impl fmt::Display for ShareMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_db_str())
    }
}

/// 解析共享 mode 字符串(数据库或 API)。
#[derive(Debug, Error)]
#[error("invalid share_mode: {0}")]
pub struct ParseShareModeError(String);

impl FromStr for ShareMode {
    type Err = ParseShareModeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "read-only" => Ok(Self::ReadOnly),
            "fork-allowed" => Ok(Self::ForkAllowed),
            "auto-copy" => Ok(Self::AutoCopy),
            "owner" => Ok(Self::Owner),
            other => Err(ParseShareModeError(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_roundtrip() {
        for m in [ShareMode::ReadOnly, ShareMode::ForkAllowed, ShareMode::AutoCopy] {
            let s = m.as_db_str();
            let parsed: ShareMode = s.parse().expect("parse known mode");
            assert_eq!(parsed, m);
        }
    }

    #[test]
    fn fork_permissions() {
        assert!(ShareMode::Owner.can_fork());
        assert!(ShareMode::ForkAllowed.can_fork());
        assert!(ShareMode::AutoCopy.can_fork());
        assert!(!ShareMode::ReadOnly.can_fork());
    }

    #[test]
    fn invalid_mode_rejected() {
        let err = "garbage".parse::<ShareMode>().unwrap_err();
        assert!(format!("{err}").contains("garbage"));
    }
}
