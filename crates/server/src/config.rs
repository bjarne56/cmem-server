//! 配置加载与默认值。
//!
//! 从 `server.toml` 读取(若不存在则使用默认值并就地生成 jwt_secret)。

use anyhow::{Context, Result};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// 总配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub auth: AuthConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// 监听地址(默认 0.0.0.0:8080,M2 阶段先开放绑定)。
    pub bind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// HMAC SHA-256 secret(base64 hex 字符串)。
    pub jwt_secret: String,
    pub access_token_ttl_secs: i64,
    pub refresh_token_ttl_secs: i64,
    pub machine_token_ttl_secs: i64,
    pub argon2_memory_kib: u32,
    pub argon2_iterations: u32,
    pub argon2_parallelism: u32,
    /// 是否要求注册时必须带有效 invite_code。默认 false 以兼容现有部署。
    #[serde(default)]
    pub require_invite: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                bind: "0.0.0.0:8080".to_string(),
            },
            database: DatabaseConfig {
                path: PathBuf::from("./cmem-server.db"),
            },
            auth: AuthConfig {
                jwt_secret: String::new(),
                access_token_ttl_secs: 900,
                refresh_token_ttl_secs: 2_592_000,
                machine_token_ttl_secs: 15_552_000,
                argon2_memory_kib: 19_456,
                argon2_iterations: 2,
                argon2_parallelism: 1,
                require_invite: false,
            },
        }
    }
}

impl AppConfig {
    /// 从指定路径加载,缺失时使用默认值。空 jwt_secret 自动生成并写回。
    pub fn load_or_default(path: Option<&Path>) -> Result<Self> {
        let mut cfg = if let Some(p) = path {
            if p.exists() {
                let body = fs::read_to_string(p)
                    .with_context(|| format!("read config {}", p.display()))?;
                toml::from_str(&body).with_context(|| "parse server.toml")?
            } else {
                Self::default()
            }
        } else {
            Self::default()
        };

        if cfg.auth.jwt_secret.trim().is_empty() {
            cfg.auth.jwt_secret = generate_jwt_secret();
            if let Some(p) = path {
                let body = toml::to_string_pretty(&cfg).context("serialize default config")?;
                fs::write(p, body).with_context(|| format!("write config {}", p.display()))?;
                tracing::info!(
                    path = %p.display(),
                    "generated new jwt_secret and wrote config"
                );
            } else {
                tracing::warn!("running with ephemeral jwt_secret; restart will invalidate tokens");
            }
        }

        Ok(cfg)
    }
}

/// 生成 256-bit 随机 secret,转 hex 字符串。
fn generate_jwt_secret() -> String {
    let mut buf = [0u8; 32];
    OsRng.fill_bytes(&mut buf);
    hex::encode(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_jwt_secret_is_filled() {
        let cfg = AppConfig::load_or_default(None).expect("load default");
        assert_eq!(cfg.auth.jwt_secret.len(), 64); // 32 bytes hex
        assert_ne!(cfg.auth.jwt_secret, "");
    }
}
