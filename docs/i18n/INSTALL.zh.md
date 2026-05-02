# 安装 cmem-server

本文涵盖所有支持的安装路径。如果只想在干净服务器上一分钟跑起来,直接看
[一键安装](#一键安装)。

- [一键安装](#一键安装)
- [各 OS 注意事项](#各-os-注意事项)
  - [macOS(Intel + Apple Silicon)](#macos)
  - [Ubuntu / Debian](#ubuntu--debian)
  - [Rocky / CentOS / Fedora(RHEL 系)](#rhel-系)
  - [Arch Linux / Manjaro](#arch-linux--manjaro)
  - [Alpine Linux](#alpine-linux)
- [手动安装](#手动安装)
- [Docker](#docker)
- [升级](#升级)
- [卸载](#卸载)
- [验证安装](#验证安装)

---

## 一键安装

```bash
git clone https://github.com/bjarne/cmem-server
cd cmem-server
sudo ./scripts/install-server.sh
```

安装脚本自动检测 OS,装编译依赖,build release 二进制,装 `systemd`
unit(macOS 用 `launchd` plist),生成 `/etc/cmem-server.toml`(带新鲜
256-bit JWT secret),并创建默认管理员:**`admin` / `admin@123`** ——
*立即改密!*

```bash
/opt/cmem-server/cmem-server -c /etc/cmem-server.toml \
    admin user reset-password --username admin
```

### 常用参数

```bash
sudo ./scripts/install-server.sh \
    --bind 0.0.0.0:8080 \                 # 默认 127.0.0.1:8080
    --domain cmem.example.com \           # 同时配 Caddy + Let's Encrypt
    --user cmem \                         # 服务账号(默认 cmem)
    --bootstrap-password '换掉它!'        # 默认管理员密码
```

| 参数 | 默认 | 作用 |
|------|------|------|
| `--bind ADDR:PORT` | `127.0.0.1:8080` | 监听地址 |
| `--domain DOMAIN` | (无) | 装 + 配 Caddy 反代 |
| `--user NAME` | `cmem` | systemd `User=`(仅 Linux) |
| `--no-systemd` | off | 跳过 systemd;macOS 用 launchd |
| `--source-dir DIR` | `$repo` | 从指定 checkout build |
| `--upgrade` | off | 只重 build + 重启,保留 config + db |
| `--uninstall` | off | 委托 `uninstall-server.sh` |
| `--skip-bootstrap` | off | 不创建默认管理员 |
| `--bootstrap-password PW` | `admin@123` | 默认管理员密码 |
| `-y / --yes` | off | 非交互(用于 uninstall) |

### 退出码

| 退出码 | 含义 |
|--------|------|
| 0 | 成功 |
| 1 | 通用错误 |
| 2 | 参数错误 |
| 3 | 不支持的 OS |
| 4 | 权限不足 |
| 5 | 网络 / 包管理器失败 |

---

## 各 OS 注意事项

### macOS

在 Apple Silicon(M1/M2/M3)与 Intel macOS 13+ 上测试通过。

- 需要 Xcode Command Line Tools(`xcode-select --install`)。
- 强烈建议装 Homebrew(`pkg-config`、`openssl@3`)。
- `--no-systemd` 自动启用,装 launchd plist 到
  `/Library/LaunchDaemons/com.cmem.server.plist`。
- 默认路径:
  - 二进制:`/usr/local/share/cmem-server/cmem-server`
  - 配置:`/usr/local/etc/cmem-server.toml`
  - 数据:`/usr/local/var/cmem-server/`
  - 日志:`/usr/local/var/cmem-server/cmem-server.{log,err.log}`

```bash
sudo ./scripts/install-server.sh
sudo launchctl print system/com.cmem.server          # 状态
sudo launchctl kickstart -k system/com.cmem.server   # 重启
sudo launchctl bootout system /Library/LaunchDaemons/com.cmem.server.plist
```

### Ubuntu / Debian

在 Ubuntu 22.04 / 24.04 与 Debian 12 上测试通过。

```bash
sudo apt-get update
sudo ./scripts/install-server.sh    # apt-get install build-essential pkg-config libssl-dev curl
```

服务管理:

```bash
systemctl status cmem-server
sudo systemctl restart cmem-server
journalctl -u cmem-server -f -n 200
```

### RHEL 系

在 Rocky Linux 8 / 9、CentOS 7+、Fedora 40+ 上测试通过。

```bash
sudo ./scripts/install-server.sh   # dnf install gcc gcc-c++ pkgconf openssl-devel curl
```

> SELinux 提示:安装脚本不会主动加 SELinux 文件 context。如果你换了
> 端口或数据目录,记得 `sudo restorecon -Rv /opt/cmem-server
> /var/lib/cmem-server`,然后用 `journalctl -t setroubleshoot -f`
> 观察首次启动有没有报错。

### Arch Linux / Manjaro

```bash
sudo ./scripts/install-server.sh   # pacman -Sy base-devel openssl pkgconf curl
```

### Alpine Linux

```bash
sudo ./scripts/install-server.sh   # apk add build-base openssl-dev pkgconf curl bash
```

> Alpine 用 `busybox` 的 `useradd`,脚本里改用 `addgroup` + `adduser
> -S -D -H` 创建服务账号。Alpine 默认不带 systemd —— 加 `--no-systemd`
> 然后自己挑监管(`openrc-init` 或 `supervisord`)。

---

## 手动安装

不想让脚本动你的系统?

```bash
# 1. build
cargo build --release --bin cmem-server
sudo install -Dm755 target/release/cmem-server /opt/cmem-server/cmem-server

# 2. 服务账号 + 数据目录(Linux)
sudo useradd --system --no-create-home --shell /usr/sbin/nologin --user-group cmem
sudo install -d -m 0750 -o cmem -g cmem /var/lib/cmem-server

# 3. config
sudo install -d -m 0755 /etc
sudo cp deploy/config/server.toml.example /etc/cmem-server.toml
sudo chown root:cmem /etc/cmem-server.toml
sudo chmod 0640 /etc/cmem-server.toml
# 生成新鲜 256-bit JWT secret:
sudo sed -i "s/^jwt_secret = .*/jwt_secret = \"$(openssl rand -hex 32)\"/" /etc/cmem-server.toml

# 4. systemd
sudo install -m 0644 deploy/systemd/cmem-server.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now cmem-server

# 5. bootstrap admin
sudo -u cmem /opt/cmem-server/cmem-server -c /etc/cmem-server.toml \
    admin user create --username admin --password '换掉它!' --admin
```

macOS 等价用 `deploy/launchd/com.cmem.server.plist` —— 复制到
`/Library/LaunchDaemons/`,然后 `sudo launchctl bootstrap system
/Library/LaunchDaemons/com.cmem.server.plist`。

---

## Docker

```bash
# 单容器试一下(数据库不持久化)
docker build -t cmem-server -f Dockerfile .
docker run --rm -p 8080:8080 cmem-server

# 准生产(持久化卷 + 自定义 config)
docker run -d --name cmem-server \
    -p 127.0.0.1:8080:8080 \
    -v cmem-data:/var/lib/cmem-server \
    -v $(pwd)/cmem-server.toml:/etc/cmem-server.toml:ro \
    --restart unless-stopped \
    cmem-server
```

或用 `docker compose`(含 Caddy):

```bash
DOMAIN=cmem.example.com docker compose up -d
docker compose logs -f cmem-server
```

完整拓扑见 [DEPLOYMENT.md#docker](../DEPLOYMENT.md#docker)。

---

## 升级

```bash
cd cmem-server
git pull
sudo ./scripts/install-server.sh --upgrade
```

只重 build 二进制 + atomically 替换 + 重启服务。配置和数据库永远不动。
schema 迁移在新二进制首次启动时自动跑。

> 大版本升级前先备份:
> `sudo sqlite3 /var/lib/cmem-server/cmem-server.db "VACUUM INTO '/tmp/backup.db'" && sudo gzip /tmp/backup.db`

---

## 卸载

交互式(推荐 —— 每一步都问):

```bash
sudo ./scripts/uninstall-server.sh --backup
```

非交互(CI / 脚本化):

```bash
sudo ./scripts/uninstall-server.sh --yes --keep-data
```

参数:

- `--backup` —— 在任何破坏性步骤前先 `VACUUM INTO` + gzip 一份到
  `~/cmem-backup-*.db.gz`。
- `--keep-data` —— 保留 `/var/lib/cmem-server`(以及里面的 db)。
- `--yes` —— 跳过所有确认。**不带 `--keep-data` 会一并删 db。**

卸载脚本按顺序删:systemd unit / launchd plist、二进制目录、配置、
数据目录(除非 `--keep-data`)、Caddy 片段、`cmem` 系统账号。

---

## 验证安装

```bash
# 1. health
curl -s http://127.0.0.1:8080/healthz | jq
# → { "status": "ok", "version": "0.1.0" }

# 2. 版本
/opt/cmem-server/cmem-server --version

# 3. admin web
xdg-open http://127.0.0.1:8080/admin/login   # macOS 用 open ...
# 用 admin / admin@123 登录(然后改密!)

# 4. CLI smoke
sudo -u cmem /opt/cmem-server/cmem-server -c /etc/cmem-server.toml admin stats

# 5. 端到端鉴权
BASE=http://127.0.0.1:8080 ./scripts/smoke_auth.sh
```

任何一步失败,看 [TROUBLESHOOTING.md](../TROUBLESHOOTING.md)。
