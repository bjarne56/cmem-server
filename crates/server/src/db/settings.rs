//! server_settings 读写 — 全局热更新配置。
//!
//! 当前 key:
//!   - `registration_mode`:`open` | `invite_only` | `closed`
//!
//! 设计:lazy initialization — 第一次读不到 key 时,用 config.toml 的
//! `require_invite` 推算默认值并写回(向后兼容,无需 bootstrap 步骤)。

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// 注册策略 — 三档枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationMode {
    /// 任何人都能注册;邀请码可填可不填(填了便于追溯来源)
    Open,
    /// 必须有邀请码才能注册
    InviteOnly,
    /// 完全关闭注册;/register POST / /api/auth/register 都 reject
    Closed,
}

impl RegistrationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InviteOnly => "invite_only",
            Self::Closed => "closed",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Self::Open),
            "invite_only" => Some(Self::InviteOnly),
            "closed" => Some(Self::Closed),
            _ => None,
        }
    }
}

const KEY_REGISTRATION_MODE: &str = "registration_mode";

/// 读取注册模式;db 没有该 key 时按 config 的 require_invite 推算并 lazy 写入。
pub async fn get_registration_mode(pool: &SqlitePool, fallback_require_invite: bool) -> Result<RegistrationMode> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT value FROM server_settings WHERE key = ?",
    )
    .bind(KEY_REGISTRATION_MODE)
    .fetch_optional(pool)
    .await?;

    if let Some((v,)) = row {
        if let Some(m) = RegistrationMode::from_str(&v) {
            return Ok(m);
        }
        // db 里有但值非法,按默认走(不抛错,避免管理界面失败时整站挂)
        tracing::warn!(value = %v, "registration_mode in db is invalid, using fallback");
    }

    let default = if fallback_require_invite {
        RegistrationMode::InviteOnly
    } else {
        RegistrationMode::Open
    };
    // lazy 写入
    set_registration_mode(pool, default, None).await?;
    Ok(default)
}

/// 写入注册模式。`updated_by` = admin user_id(可选,审计用)。
pub async fn set_registration_mode(
    pool: &SqlitePool,
    mode: RegistrationMode,
    updated_by: Option<&str>,
) -> Result<()> {
    let now = Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO server_settings (key, value, updated_at, updated_by)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET
           value = excluded.value,
           updated_at = excluded.updated_at,
           updated_by = excluded.updated_by",
    )
    .bind(KEY_REGISTRATION_MODE)
    .bind(mode.as_str())
    .bind(now)
    .bind(updated_by)
    .execute(pool)
    .await?;
    Ok(())
}
