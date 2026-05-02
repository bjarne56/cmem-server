//! 反代友好的真实 client IP 解析中间件。
//!
//! 算法:
//! 1. 取 axum [`ConnectInfo`](axum::extract::ConnectInfo) 中的对端 IP。
//! 2. 如果对端 IP 落在 `[security].trusted_proxies` CIDR 列表内,则把
//!    `X-Forwarded-For` 从右往左扫,首个 *不在* trusted_proxies 内的 IP
//!    即为真实 client。如果整条 chain 都在 trusted 内,退化为该 chain
//!    最左侧的 IP(理论上是发起请求的最远客户端)。
//! 3. 如果对端 IP 不在 trusted 内,无条件忽略 `X-Forwarded-For`(防伪造),
//!    用对端 IP 即可。
//!
//! 解析结果通过 [`ClientIp`] 注入 request extensions,handler 用
//! `Extension<ClientIp>` 提取。
//!
//! 这一层只做"提取并放进 extensions",不做策略;速率限制 / 审计日志 /
//! 用户 IP 记录这些下游模块各自从 extensions 拿即可。

use std::net::{IpAddr, SocketAddr};

use axum::{
    extract::{ConnectInfo, Request, State},
    http::header,
    middleware::Next,
    response::Response,
};
use ipnet::IpNet;

use crate::state::AppState;

/// 真实 client IP。`Option` 因为本地 unit/integration test 的 oneshot
/// 调用并不带 ConnectInfo;handler 应当容忍 None。
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub Option<IpAddr>);

impl ClientIp {
    pub fn as_string(&self) -> Option<String> {
        self.0.map(|ip| ip.to_string())
    }
}

/// 把 trusted_proxies 配置项 parse 成 [`IpNet`] 列表。失败的条目会被
/// `tracing::warn!` 提示后丢弃,不会让进程崩。
pub fn parse_trusted_cidrs(raw: &[String]) -> Vec<IpNet> {
    raw.iter()
        .filter_map(|s| match s.parse::<IpNet>() {
            Ok(n) => Some(n),
            Err(_) => match s.parse::<IpAddr>() {
                // 用户写裸 IP 也接受,等价于 /32 / /128。
                Ok(ip) => Some(IpNet::from(ip)),
                Err(e) => {
                    tracing::warn!(cidr = %s, error = %e, "ignored invalid trusted_proxy entry");
                    None
                }
            },
        })
        .collect()
}

fn ip_in(ip: IpAddr, cidrs: &[IpNet]) -> bool {
    cidrs.iter().any(|c| c.contains(&ip))
}

/// 从 `X-Forwarded-For` 头中按"逗号 / 空白"切片得到原始 IP 列表(可能含端口或非法值)。
fn parse_forwarded_for(header: &str) -> Vec<IpAddr> {
    header
        .split(',')
        .filter_map(|raw| {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            // 形如 [::1]:1234 / 1.2.3.4:5678,先去端口再 parse。
            if let Ok(sa) = raw.parse::<SocketAddr>() {
                return Some(sa.ip());
            }
            raw.parse::<IpAddr>().ok()
        })
        .collect()
}

/// 在 trusted proxy 已确认的前提下,从 X-Forwarded-For chain 取真实 client。
/// 最右非 trusted 的 IP = 直接发到最近一跳 trusted proxy 的客户端。
fn pick_client_from_chain(chain: &[IpAddr], trusted: &[IpNet]) -> Option<IpAddr> {
    for ip in chain.iter().rev() {
        if !ip_in(*ip, trusted) {
            return Some(*ip);
        }
    }
    // 整条 chain 都 trusted —— 退化:取最左(理论上是 origin client)。
    chain.first().copied()
}

/// 计算给定请求的真实 client IP。
///
/// 公开是为了测试,生产代码用中间件即可。
pub fn resolve_client_ip(
    peer_addr: Option<IpAddr>,
    forwarded_for: Option<&str>,
    trusted: &[IpNet],
) -> Option<IpAddr> {
    let peer = peer_addr?;
    if !ip_in(peer, trusted) {
        // 不信对端,直接用对端 IP,X-Forwarded-For 一律忽略
        return Some(peer);
    }
    if let Some(h) = forwarded_for {
        let chain = parse_forwarded_for(h);
        if !chain.is_empty() {
            return pick_client_from_chain(&chain, trusted).or(Some(peer));
        }
    }
    Some(peer)
}

/// axum 中间件:解析并注入 [`ClientIp`]。
pub async fn extract_client_ip(
    State(state): State<AppState>,
    connect: Option<ConnectInfo<SocketAddr>>,
    mut req: Request,
    next: Next,
) -> Response {
    let trusted = parse_trusted_cidrs(&state.config.security.trusted_proxies);
    let peer_ip = connect.map(|ConnectInfo(sa)| sa.ip());
    let xff = req
        .headers()
        .get_all(header::FORWARDED.as_str()) // 占位,真正用的是 X-Forwarded-For,但 header 常量是 HeaderName
        .iter()
        .next();
    // 取 X-Forwarded-For 头(标准命名)
    let xff_str = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let resolved = resolve_client_ip(peer_ip, xff_str.as_deref(), &trusted);
    let _ = xff; // 抑制未使用的 future use 警告
    req.extensions_mut().insert(ClientIp(resolved));
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn nets(raw: &[&str]) -> Vec<IpNet> {
        parse_trusted_cidrs(&raw.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn parses_cidr_and_bare_ip() {
        let n = nets(&["127.0.0.1/32", "10.0.0.0/8", "192.168.1.1"]);
        assert_eq!(n.len(), 3);
    }

    #[test]
    fn invalid_cidr_dropped() {
        let n = nets(&["nonsense", "127.0.0.1/32"]);
        assert_eq!(n.len(), 1);
    }

    #[test]
    fn untrusted_peer_ignores_xff() {
        let trusted = nets(&["10.0.0.0/8"]);
        let peer = "203.0.113.5".parse().ok();
        let got = resolve_client_ip(peer, Some("198.51.100.7"), &trusted);
        assert_eq!(got, Some(IpAddr::from_str("203.0.113.5").unwrap()));
    }

    #[test]
    fn trusted_peer_uses_xff_rightmost_non_trusted() {
        let trusted = nets(&["10.0.0.0/8", "127.0.0.1/32"]);
        let peer = "127.0.0.1".parse().ok();
        // chain: external -> external -> trusted -> trusted-peer
        let xff = "203.0.113.5, 198.51.100.7, 10.0.0.99";
        let got = resolve_client_ip(peer, Some(xff), &trusted);
        assert_eq!(got, Some(IpAddr::from_str("198.51.100.7").unwrap()));
    }

    #[test]
    fn trusted_peer_single_xff() {
        let trusted = nets(&["127.0.0.1/32"]);
        let peer = "127.0.0.1".parse().ok();
        let got = resolve_client_ip(peer, Some("203.0.113.42"), &trusted);
        assert_eq!(got, Some(IpAddr::from_str("203.0.113.42").unwrap()));
    }

    #[test]
    fn trusted_peer_no_xff_falls_back_to_peer() {
        let trusted = nets(&["127.0.0.1/32"]);
        let peer = "127.0.0.1".parse().ok();
        let got = resolve_client_ip(peer, None, &trusted);
        assert_eq!(got, Some(IpAddr::from_str("127.0.0.1").unwrap()));
    }

    #[test]
    fn xff_with_port_is_accepted() {
        let trusted = nets(&["127.0.0.1/32"]);
        let peer = "127.0.0.1".parse().ok();
        let got = resolve_client_ip(peer, Some("203.0.113.5:65000"), &trusted);
        assert_eq!(got, Some(IpAddr::from_str("203.0.113.5").unwrap()));
    }

    #[test]
    fn ipv6_loopback_trusted() {
        let trusted = nets(&["::1/128"]);
        let peer = "::1".parse().ok();
        let got = resolve_client_ip(peer, Some("2001:db8::1"), &trusted);
        assert_eq!(got, Some(IpAddr::from_str("2001:db8::1").unwrap()));
    }

    #[test]
    fn entire_chain_trusted_returns_leftmost() {
        let trusted = nets(&["10.0.0.0/8", "127.0.0.1/32"]);
        let peer = "127.0.0.1".parse().ok();
        let xff = "10.0.0.5, 10.0.0.6";
        let got = resolve_client_ip(peer, Some(xff), &trusted);
        assert_eq!(got, Some(IpAddr::from_str("10.0.0.5").unwrap()));
    }
}
