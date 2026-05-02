//! JWT 编解码(HS256)。

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use jsonwebtoken::{
    decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation,
};
use serde::{Deserialize, Serialize};

/// Token 用途标记,放进 claims `kind` 字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenKind {
    Access,
    Refresh,
}

/// JWT claims。
///
/// `sub` 存 user_id;`mid` 存 machine_id(可选,M3 后端引入)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub kind: TokenKind,
    pub iat: i64,
    pub exp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mid: Option<String>,
}

/// JWT 编解码器。封装 secret 与算法。
#[derive(Clone)]
pub struct JwtCodec {
    encoding: EncodingKey,
    decoding: DecodingKey,
    algorithm: Algorithm,
}

impl JwtCodec {
    pub fn new(secret_hex: &str) -> Result<Self> {
        let secret_bytes =
            hex::decode(secret_hex).context("decode jwt_secret as hex (256-bit)")?;
        if secret_bytes.len() < 32 {
            return Err(anyhow!("jwt_secret must decode to >=32 bytes"));
        }
        Ok(Self {
            encoding: EncodingKey::from_secret(&secret_bytes),
            decoding: DecodingKey::from_secret(&secret_bytes),
            algorithm: Algorithm::HS256,
        })
    }

    /// 生成 access token。
    pub fn encode_access(&self, user_id: &str, machine_id: Option<&str>, ttl_secs: i64) -> Result<(String, i64)> {
        let now = Utc::now().timestamp();
        let exp = now + ttl_secs;
        let claims = Claims {
            sub: user_id.to_string(),
            kind: TokenKind::Access,
            iat: now,
            exp,
            mid: machine_id.map(str::to_string),
        };
        let token = encode(&Header::new(self.algorithm), &claims, &self.encoding)
            .context("encode access jwt")?;
        Ok((token, exp))
    }

    /// 解码并校验任意 JWT。返回 claims。
    pub fn decode(&self, token: &str) -> Result<Claims> {
        let mut validation = Validation::new(self.algorithm);
        validation.validate_exp = true;
        let data = decode::<Claims>(token, &self.decoding, &validation)
            .context("decode jwt")?;
        Ok(data.claims)
    }
}

/// 生成 32-byte 随机 refresh token,返回 (明文, sha256-hex)。
pub fn generate_refresh_token() -> (String, String) {
    use rand::RngCore;
    let mut buf = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    let plain = hex::encode(buf);
    let hash = sha256_hex(&plain);
    (plain, hash)
}

/// SHA-256(token) → 64 字符 hex。
pub fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(input.as_bytes());
    hex::encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_codec() -> JwtCodec {
        // 64 hex = 32 bytes
        let secret = "a".repeat(64);
        JwtCodec::new(&secret).unwrap()
    }

    #[test]
    fn access_token_roundtrip() {
        let codec = make_codec();
        let (tok, exp) = codec.encode_access("user-1", None, 60).unwrap();
        let claims = codec.decode(&tok).unwrap();
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.kind, TokenKind::Access);
        assert_eq!(claims.exp, exp);
    }

    #[test]
    fn rejects_short_secret() {
        // 短于 32 字节
        let res = JwtCodec::new("deadbeef");
        assert!(res.is_err());
    }

    #[test]
    fn refresh_token_hash_stable() {
        let (plain, hash) = generate_refresh_token();
        assert_eq!(plain.len(), 64);
        assert_eq!(hash.len(), 64);
        assert_eq!(sha256_hex(&plain), hash);
    }
}
