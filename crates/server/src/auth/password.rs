//! Argon2id 密码哈希。
//!
//! 参数从 `AuthConfig` 注入(默认 RFC 9106 推荐:19 MiB / 2 iter / 1 thread)。

use anyhow::{anyhow, Result};
use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Algorithm, Argon2, Params, Version,
};
use rand::rngs::OsRng;

use crate::config::AuthConfig;

/// 创建配置好的 argon2id 实例。
fn argon2_for(cfg: &AuthConfig) -> Result<Argon2<'static>> {
    let params = Params::new(
        cfg.argon2_memory_kib,
        cfg.argon2_iterations,
        cfg.argon2_parallelism,
        None,
    )
    .map_err(|e| anyhow!("argon2 params: {e}"))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

/// 计算密码哈希。
pub fn hash_password(password: &str, cfg: &AuthConfig) -> Result<String> {
    let argon2 = argon2_for(cfg)?;
    let salt = SaltString::generate(&mut OsRng);
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow!("argon2 hash: {e}"))?
        .to_string();
    Ok(hash)
}

/// 校验密码,返回是否匹配。
pub fn verify_password(password: &str, stored_hash: &str) -> Result<bool> {
    let parsed = PasswordHash::new(stored_hash)
        .map_err(|e| anyhow!("parse stored password hash: {e}"))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast_cfg() -> AuthConfig {
        AuthConfig {
            jwt_secret: "x".into(),
            access_token_ttl_secs: 60,
            refresh_token_ttl_secs: 600,
            machine_token_ttl_secs: 6000,
            // 测试参数:省内存 + 1 iter,加速测试
            argon2_memory_kib: 8,
            argon2_iterations: 1,
            argon2_parallelism: 1,
        }
    }

    #[test]
    fn hash_then_verify_succeeds() {
        let cfg = fast_cfg();
        let hash = hash_password("super-secret-pw", &cfg).unwrap();
        assert!(verify_password("super-secret-pw", &hash).unwrap());
        assert!(!verify_password("wrong-pw", &hash).unwrap());
    }

    #[test]
    fn hash_is_unique_due_to_salt() {
        let cfg = fast_cfg();
        let h1 = hash_password("same-pw", &cfg).unwrap();
        let h2 = hash_password("same-pw", &cfg).unwrap();
        assert_ne!(h1, h2);
    }
}
