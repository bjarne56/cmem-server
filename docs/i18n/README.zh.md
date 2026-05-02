# cmem-server

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](../../rust-toolchain.toml)
[![axum](https://img.shields.io/badge/axum-0.7-brightgreen.svg)](https://github.com/tokio-rs/axum)
[![SQLite](https://img.shields.io/badge/sqlite-3-003B57.svg)](https://www.sqlite.org/)

> **[claude-mem](https://github.com/thedotmack/claude-mem) 的多机同步
> + 项目共享自托管服务器。** 一个 Rust 二进制。一个 SQLite 文件。
> argon2id + JWT。systemd / launchd / Docker。内置 admin web 后台。
> 二进制约 10 MB,空闲常驻约 5 MB RSS。

如果你在多台机器上使用 [claude-mem](https://github.com/thedotmack/claude-mem),
或者想把项目级记忆共享给团队成员,**cmem-server** 就是缺失的那一环:
单二进制服务器,跨机同步 observation,支持整项目共享(read-only /
fork-allowed / auto-copy),不需要把任何文件目录开放给别人。

[English README](../../README.md) · [安装](INSTALL.zh.md) ·
[管理后台](ADMIN.zh.md) · [API](../API.md) · [架构](../ARCHITECTURE.md) ·
[共享语义](../PROJECT_SHARING.md) · [安全](../SECURITY.md) ·
[故障排查](../TROUBLESHOOTING.md) · [贡献指南](../CONTRIBUTING.md)

---

## 为什么需要它?

`claude-mem` 在每台机器本地维护一份 SQLite 数据库,记录 Claude Code
学到的所有 observation —— 但它只到一台机器。一旦你 `ssh` 到 VPS、
从 MacBook 切到 Linux 桌面、想把项目上下文给同事看,本地这份 DB 就
不再是「正确的那一份」。

`cmem-server` 是一个微型自托管 daemon:

- 接收任意数量 `claude-mem` 客户端 push 上来的 JSONL observation
  (machine token 鉴权)
- 在 `pull` 时回放 —— 跨机同步自己的 observation,以及别人共享给你的
  observation
- 实现一层 **项目身份层**:Mac 上的 `~/work/nginx-rce` 和 Linux 上的
  `~/projects/nginx-rce` 是同一个 project,不是两个
- 实现三种共享模式(read-only / fork-allowed / auto-copy),带 mode
  降级通知机制
- 内置 `/admin` web 后台(askama + HTMX,无 SPA 维护负担)
- 所有状态在 **一个 SQLite 文件** 里 —— `cp` 就能备份

### 不做的事

- 取代 claude-mem 本身
- 百万用户级 SaaS
- 实时协同编辑
- 任何需要 Postgres / Redis / Elasticsearch 的功能

这是个人 / 小团队工具。**简单永远优于「工程上正确但臃肿」**。

---

## 5 分钟上手

```bash
# 在服务器上(macOS / Ubuntu / Debian / Rocky / Fedora / Arch / Alpine)
git clone https://github.com/bjarne/cmem-server
cd cmem-server
sudo ./scripts/install-server.sh --bind 127.0.0.1:8080
```

完事。安装脚本会:

1. 检测 OS
2. 装编译依赖(`build-essential` / `gcc` / `base-devel` ...)
3. 通过 `rustup` 装 Rust(已装跳过)
4. `cargo build --release`
5. 创建 `cmem` 系统账号 + `/var/lib/cmem-server` 数据目录
6. 生成 `/etc/cmem-server.toml`,带新鲜的 256-bit JWT secret
7. 装并启用一个收紧过的 `systemd` unit(macOS 改 `launchd`)
8. 创建默认管理员:**`admin` / `admin@123`**(立即改密!)
9. 等 `/healthz` 返回 200

打开 `http://127.0.0.1:8080/admin/login` 登录。

### 配域名 + 自动 HTTPS(Caddy)

```bash
sudo ./scripts/install-server.sh --domain cmem.example.com
```

安装脚本会顺手装 Caddy,反代 `127.0.0.1:8080` → 你的域名,Let's
Encrypt 证书自动签发 / 续签。

---

## 客户端(claude-mem)

```bash
curl -sSL https://raw.githubusercontent.com/<your>/claude-mem/main/install-client.sh \
    | bash -s -- --server https://cmem.example.com
claude-mem sync login --server https://cmem.example.com
claude-mem sync push
```

---

## 文档导航

| 文档 | 受众 | 主题 |
|------|------|------|
| [INSTALL.zh.md](INSTALL.zh.md) | 运维 | 各 OS 安装 |
| [ADMIN.zh.md](ADMIN.zh.md) | 管理员 | admin web 操作 |
| [DEPLOYMENT.md](../DEPLOYMENT.md) | 运维 | VPS / Docker / k8s,备份,监控 |
| [USAGE.md](../USAGE.md) | 终端用户 | claude-mem CLI + viewer 流程 |
| [API.md](../API.md) | 客户端开发 | REST 请求 / 响应 |
| [ARCHITECTURE.md](../ARCHITECTURE.md) | 贡献者 | crate / module 布局,数据模型 |
| [PROJECT_SHARING.md](../PROJECT_SHARING.md) | 贡献者 | 不变量与状态机 |
| [SECURITY.md](../SECURITY.md) | 运维 | 威胁模型,加固清单 |
| [TROUBLESHOOTING.md](../TROUBLESHOOTING.md) | 运维 | 常见故障 + 恢复 |
| [CONTRIBUTING.md](../CONTRIBUTING.md) | 贡献者 | 开发环境,提交规范 |

---

## 状态

| Milestone | 范围 | 进度 |
|-----------|------|------|
| M1 | Workspace + DB schema + `/healthz` | 已完成 |
| M2 | 认证(argon2id + JWT + refresh) | 已完成 |
| M3 | 机器 + machine token | 已完成 |
| M4 | 项目识别与合并 | 已完成 |
| M5 | sync push / pull(+ shares 半成品) | 部分 |
| M6 | Fork(项目 + 单条 observation) | 进行中 |
| M7 | claude-mem 客户端集成 | 已完成(TS 端 fork) |
| M8 | 部署 + 文档 + admin web | 已完成 |

---

## License

MIT。详见 [LICENSE](../../LICENSE)。

## 联系

Issues / PRs 欢迎到 <https://github.com/bjarne/cmem-server>。
安全漏洞披露见 [SECURITY.md](../SECURITY.md)。
