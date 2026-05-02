//! 真实 client IP 解析行为测试。
//!
//! 用 `resolve_client_ip` 直接做断言(unit-style 集成测试),覆盖三类:
//!
//! 1. ConnectInfo 在 trusted_proxies 内 → 信任 X-Forwarded-For
//!    最右非 trusted 的 IP 作为 client IP。
//! 2. ConnectInfo 不在 trusted_proxies 内 → 一律忽略 X-Forwarded-For,
//!    用对端 IP(防伪造)。
//! 3. X-Forwarded-For 含多跳 chain → 取最右非 trusted IP。
//!
//! 算法实现在 `crates/server/src/middleware/ip.rs`。

use std::net::IpAddr;
use std::str::FromStr;

use cmem_server::middleware::ip::{parse_trusted_cidrs, resolve_client_ip};

fn ip(s: &str) -> IpAddr {
    IpAddr::from_str(s).expect("parse ip")
}

fn trusted(items: &[&str]) -> Vec<ipnet::IpNet> {
    parse_trusted_cidrs(&items.iter().map(|s| s.to_string()).collect::<Vec<_>>())
}

#[test]
fn untrusted_peer_ignores_xff() {
    // 直连(对端就是 client),X-Forwarded-For 是攻击者自己写的,不能信。
    let trusted = trusted(&["10.0.0.0/8", "127.0.0.1/32"]);
    let peer = Some(ip("203.0.113.5"));
    let got = resolve_client_ip(peer, Some("198.51.100.99"), &trusted);
    assert_eq!(got, Some(ip("203.0.113.5")));
}

#[test]
fn untrusted_peer_with_long_xff_still_uses_peer() {
    // 攻击者构造一长串 XFF 也无效。
    let trusted = trusted(&["10.0.0.0/8"]);
    let peer = Some(ip("203.0.113.5"));
    let got = resolve_client_ip(
        peer,
        Some("1.1.1.1, 2.2.2.2, 10.0.0.99, 198.51.100.7"),
        &trusted,
    );
    assert_eq!(got, Some(ip("203.0.113.5")));
}

#[test]
fn trusted_peer_uses_xff_single_hop() {
    // 经典反代:Caddy/nginx 在 127.0.0.1,XFF 只一项 = 真实 client。
    let trusted = trusted(&["127.0.0.1/32"]);
    let peer = Some(ip("127.0.0.1"));
    let got = resolve_client_ip(peer, Some("203.0.113.42"), &trusted);
    assert_eq!(got, Some(ip("203.0.113.42")));
}

#[test]
fn trusted_peer_uses_xff_chain_rightmost_non_trusted() {
    // CDN(Cloudflare 198.51.100.x) → LB 内网 10.0.0.5 → app 127.0.0.1
    // 真实 client = chain 中最右非 trusted = 198.51.100.7
    // 不能取最左,否则 client 可以伪造前缀。
    let trusted = trusted(&["10.0.0.0/8", "127.0.0.1/32"]);
    let peer = Some(ip("127.0.0.1"));
    let xff = "203.0.113.5, 198.51.100.7, 10.0.0.5";
    let got = resolve_client_ip(peer, Some(xff), &trusted);
    assert_eq!(got, Some(ip("198.51.100.7")));
}

#[test]
fn trusted_peer_no_xff_falls_back_to_peer() {
    let trusted = trusted(&["127.0.0.1/32"]);
    let peer = Some(ip("127.0.0.1"));
    let got = resolve_client_ip(peer, None, &trusted);
    assert_eq!(got, Some(ip("127.0.0.1")));
}

#[test]
fn trusted_peer_empty_xff_falls_back_to_peer() {
    let trusted = trusted(&["127.0.0.1/32"]);
    let peer = Some(ip("127.0.0.1"));
    let got = resolve_client_ip(peer, Some(""), &trusted);
    assert_eq!(got, Some(ip("127.0.0.1")));
}

#[test]
fn entire_chain_trusted_falls_back_to_leftmost() {
    // 内网调用,整条 chain 都在 trusted 范围;退化拿最左项,
    // 至少不返回 None,业务可以记录这是个内部请求。
    let trusted = trusted(&["10.0.0.0/8"]);
    let peer = Some(ip("10.0.0.1"));
    let got = resolve_client_ip(peer, Some("10.0.0.5, 10.0.0.6"), &trusted);
    assert_eq!(got, Some(ip("10.0.0.5")));
}

#[test]
fn ipv6_localhost_trusted() {
    let trusted = trusted(&["::1/128"]);
    let peer = Some(ip("::1"));
    let got = resolve_client_ip(peer, Some("2001:db8::dead"), &trusted);
    assert_eq!(got, Some(ip("2001:db8::dead")));
}

#[test]
fn xff_with_socket_addr_port_is_handled() {
    // 一些反代会写 1.2.3.4:5678 进 XFF,要能抽出 IP。
    let trusted = trusted(&["127.0.0.1/32"]);
    let peer = Some(ip("127.0.0.1"));
    let got = resolve_client_ip(peer, Some("203.0.113.5:65000"), &trusted);
    assert_eq!(got, Some(ip("203.0.113.5")));
}

#[test]
fn invalid_cidr_entries_are_dropped_silently() {
    // 配置文件写错 CIDR 不应该让进程崩 —— ip.rs 应该 warn + skip。
    let t = trusted(&["nonsense", "10.0.0.0/99", "127.0.0.1/32"]);
    assert_eq!(t.len(), 1);
}

#[test]
fn no_peer_returns_none() {
    // 罕见路径:测试 oneshot 调用没有 ConnectInfo,返回 None,handler 容忍。
    let trusted = trusted(&["127.0.0.1/32"]);
    let got = resolve_client_ip(None, Some("203.0.113.5"), &trusted);
    assert_eq!(got, None);
}
