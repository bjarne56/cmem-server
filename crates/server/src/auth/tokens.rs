//! Machine token 生成与哈希。
//!
//! 格式:`cmt_<32 字符 nanoid>`。数据库永远只存 SHA-256 hex。

use crate::auth::jwt::sha256_hex;

const TOKEN_PREFIX: &str = "cmt_";

/// 生成新的 machine token,返回明文。
pub fn generate_machine_token() -> String {
    let body = nanoid::nanoid!(32, &nanoid::alphabet::SAFE);
    format!("{TOKEN_PREFIX}{body}")
}

/// 计算 machine token hash(用于数据库查询/写入)。
pub fn hash_machine_token(token: &str) -> String {
    sha256_hex(token)
}

/// 判断是否是 machine token 形态。
pub fn is_machine_token(token: &str) -> bool {
    token.starts_with(TOKEN_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_token_format() {
        let t = generate_machine_token();
        assert!(is_machine_token(&t));
        assert_eq!(t.len(), 4 + 32);
    }

    #[test]
    fn token_hash_stable() {
        let h1 = hash_machine_token("cmt_abcdef");
        let h2 = hash_machine_token("cmt_abcdef");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }
}
