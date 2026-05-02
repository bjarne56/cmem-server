//! 基于 [`tower_governor`] 的速率限制中间件。
//!
//! 关键差异:本项目把 client IP 解析逻辑前置到了
//! [`crate::middleware::ip::extract_client_ip`],已经做过 trusted_proxies +
//! X-Forwarded-For 校验,结果通过 [`crate::middleware::ip::ClientIp`] 注入
//! request extensions。这里的 [`ClientIpKeyExtractor`] 直接读 extensions,
//! 而不是 tower_governor 自带的 `PeerIpKeyExtractor`(后者只看 ConnectInfo,
//! 反代后会把所有请求归到 proxy IP 一个 key,导致整服务被限制)。
//!
//! 三档配额:
//! * `login_governor_layer`  —— 严格档:`/admin/login`、`/api/auth/login`、
//!   `/api/auth/register`,默认 5 req/min/IP。
//! * `api_governor_layer`    —— 中档:`/api/admin/*` 与 admin web GET,
//!   默认 60 req/min/IP。

use std::net::IpAddr;
use std::sync::Arc;

use axum::http::Request as HttpRequest;
use thiserror::Error;
use tower_governor::{
    governor::{GovernorConfig, GovernorConfigBuilder},
    key_extractor::KeyExtractor,
    GovernorError, GovernorLayer,
};

use crate::middleware::ip::ClientIp;

/// 自定义 key extractor:从 request extensions 取 [`ClientIp`]。
///
/// 找不到时(中间件没接上、测试 oneshot 等场景)用一个固定假 IP 0.0.0.0
/// 作为兜底 key —— 在测试里这等价于"全局共享一个 bucket",生产里因为
/// 真实部署都接了 [`crate::middleware::ip::extract_client_ip`],不会走到。
#[derive(Debug, Clone, Copy)]
pub struct ClientIpKeyExtractor;

impl KeyExtractor for ClientIpKeyExtractor {
    type Key = IpAddr;

    fn extract<T>(&self, req: &HttpRequest<T>) -> Result<Self::Key, GovernorError> {
        let ip = req
            .extensions()
            .get::<ClientIp>()
            .and_then(|c| c.0)
            .unwrap_or_else(|| IpAddr::from([0, 0, 0, 0]));
        Ok(ip)
    }
}

/// 构造 governor layer 时的错误。
#[derive(Debug, Error)]
pub enum GovernorBuildError {
    #[error("invalid rate limit: requests per minute must be > 0")]
    InvalidRate,
}

/// 通用构造:`per_minute` requests / minute / IP。
///
/// 实际策略:周期 = `60 / per_minute` 秒补充 1 token,burst = `per_minute`。
/// 例如 5 req/min → 12 秒一次补充,允许偶发 burst 5 个。
pub fn build_governor_layer(
    per_minute: u32,
) -> Result<GovernorLayer<ClientIpKeyExtractor, governor::middleware::NoOpMiddleware>, GovernorBuildError>
{
    if per_minute == 0 {
        return Err(GovernorBuildError::InvalidRate);
    }
    let interval_ms = (60_000_u64 / per_minute as u64).max(1);
    let mut builder: GovernorConfigBuilder<
        tower_governor::key_extractor::PeerIpKeyExtractor,
        governor::middleware::NoOpMiddleware,
    > = GovernorConfigBuilder::default();
    builder.per_millisecond(interval_ms).burst_size(per_minute);
    let mut builder = builder.key_extractor(ClientIpKeyExtractor);
    let cfg: GovernorConfig<ClientIpKeyExtractor, governor::middleware::NoOpMiddleware> = builder
        .finish()
        .ok_or(GovernorBuildError::InvalidRate)?;
    Ok(GovernorLayer {
        config: Arc::new(cfg),
    })
}

/// 严格档:用于 /login、/register。
pub fn login_governor_layer(
    per_minute: u32,
) -> Result<GovernorLayer<ClientIpKeyExtractor, governor::middleware::NoOpMiddleware>, GovernorBuildError>
{
    build_governor_layer(per_minute)
}

/// 中档:用于一般 API。
pub fn api_governor_layer(
    per_minute: u32,
) -> Result<GovernorLayer<ClientIpKeyExtractor, governor::middleware::NoOpMiddleware>, GovernorBuildError>
{
    build_governor_layer(per_minute)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_with_zero_fails() {
        assert!(matches!(
            build_governor_layer(0),
            Err(GovernorBuildError::InvalidRate)
        ));
    }

    #[test]
    fn build_with_positive_succeeds() {
        assert!(build_governor_layer(5).is_ok());
        assert!(build_governor_layer(60).is_ok());
    }
}
