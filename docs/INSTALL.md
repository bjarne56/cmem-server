# Installing cmem-server

This guide covers every supported install path. If you just want to get
running on a fresh server in one minute, jump to
[One-shot install](#one-shot-install).

- [One-shot install](#one-shot-install)
- [Per-OS notes](#per-os-notes)
  - [macOS (Intel + Apple Silicon)](#macos)
  - [Ubuntu / Debian](#ubuntu--debian)
  - [Rocky / CentOS / Fedora (RHEL family)](#rhel-family)
  - [Arch Linux / Manjaro](#arch-linux--manjaro)
  - [Alpine Linux](#alpine-linux)
- [Manual install](#manual-install)
- [Docker](#docker)
- [Upgrading](#upgrading)
- [Uninstalling](#uninstalling)
- [Verifying the install](#verifying-the-install)

---

## One-shot install

```bash
git clone https://github.com/bjarne/cmem-server
cd cmem-server
sudo ./scripts/install-server.sh
```

The installer detects your OS, installs build dependencies, builds the
release binary, drops a `systemd` unit (or `launchd` plist on macOS),
generates `/etc/cmem-server.toml` with a fresh 256-bit JWT secret, and
bootstraps a default admin: **`admin` / `admin@123`** — *change it
immediately* with:

```bash
/opt/cmem-server/cmem-server -c /etc/cmem-server.toml \
    admin user reset-password --username admin
```

### Common flags

```bash
sudo ./scripts/install-server.sh \
    --bind 0.0.0.0:8080 \                 # default 127.0.0.1:8080
    --domain cmem.example.com \           # also configures Caddy + Let's Encrypt
    --user cmem \                         # service account (default cmem)
    --bootstrap-password 'replace_me!'    # default admin password
```

| Flag | Default | Effect |
|------|---------|--------|
| `--bind ADDR:PORT` | `127.0.0.1:8080` | listening address |
| `--domain DOMAIN` | (none) | install + configure Caddy reverse proxy |
| `--user NAME` | `cmem` | systemd `User=` (Linux only) |
| `--no-systemd` | off | skip systemd; use `launchd` on macOS |
| `--source-dir DIR` | `$repo` | build from a different checkout |
| `--upgrade` | off | rebuild + restart only; keep config + db |
| `--uninstall` | off | delegate to `uninstall-server.sh` |
| `--skip-bootstrap` | off | don't create default admin |
| `--bootstrap-password PW` | `admin@123` | default admin password |
| `-y / --yes` | off | non-interactive (used by uninstall) |

### Exit codes

| Code | Meaning |
|------|---------|
| 0 | success |
| 1 | generic failure |
| 2 | bad arguments |
| 3 | unsupported OS |
| 4 | insufficient permissions |
| 5 | network or package-manager failure |

---

## Per-OS notes

### macOS

Tested on Apple Silicon (M1/M2/M3) and Intel macOS 13+.

- Requires Xcode Command Line Tools (`xcode-select --install`).
- Homebrew strongly recommended (`pkg-config`, `openssl@3`).
- `--no-systemd` is implied; the installer drops a launchd plist at
  `/Library/LaunchDaemons/com.cmem.server.plist`.
- Default paths:
  - binary: `/usr/local/share/cmem-server/cmem-server`
  - config: `/usr/local/etc/cmem-server.toml`
  - data:   `/usr/local/var/cmem-server/`
  - logs:   `/usr/local/var/cmem-server/cmem-server.{log,err.log}`

```bash
sudo ./scripts/install-server.sh
sudo launchctl print system/com.cmem.server          # status
sudo launchctl kickstart -k system/com.cmem.server   # restart
sudo launchctl bootout system /Library/LaunchDaemons/com.cmem.server.plist
```

### Ubuntu / Debian

Tested on Ubuntu 22.04 / 24.04 and Debian 12.

```bash
sudo apt-get update
sudo ./scripts/install-server.sh    # apt-get install build-essential pkg-config libssl-dev curl
```

Service:

```bash
systemctl status cmem-server
sudo systemctl restart cmem-server
journalctl -u cmem-server -f -n 200
```

### RHEL family

Tested on Rocky Linux 8 / 9, CentOS 7+, Fedora 40+.

```bash
# RHEL family auto-uses dnf if available, otherwise yum
sudo ./scripts/install-server.sh   # dnf install gcc gcc-c++ pkgconf openssl-devel curl
```

> SELinux note: the installer does **not** add SELinux file contexts.
> If you bind to a non-default port or move the data directory, run
> `sudo restorecon -Rv /opt/cmem-server /var/lib/cmem-server` and audit
> with `journalctl -t setroubleshoot -f` after first start.

### Arch Linux / Manjaro

```bash
sudo ./scripts/install-server.sh   # pacman -Sy base-devel openssl pkgconf curl
```

### Alpine Linux

```bash
sudo ./scripts/install-server.sh   # apk add build-base openssl-dev pkgconf curl bash
```

> Alpine ships with `busybox` `useradd`-style differences; the installer
> uses `addgroup` + `adduser -S -D -H` for service accounts. systemd is
> not standard on Alpine — pass `--no-systemd` and supervise manually
> (e.g. `openrc-init` or `supervisord`).

---

## Manual install

If you do not want the installer to touch your system:

```bash
# 1. Build
cargo build --release --bin cmem-server
sudo install -Dm755 target/release/cmem-server /opt/cmem-server/cmem-server

# 2. Service user + data dir (Linux)
sudo useradd --system --no-create-home --shell /usr/sbin/nologin --user-group cmem
sudo install -d -m 0750 -o cmem -g cmem /var/lib/cmem-server

# 3. Config
sudo install -d -m 0755 /etc
sudo cp deploy/config/server.toml.example /etc/cmem-server.toml
sudo chown root:cmem /etc/cmem-server.toml
sudo chmod 0640 /etc/cmem-server.toml
# Generate a fresh 256-bit JWT secret:
sudo sed -i "s/^jwt_secret = .*/jwt_secret = \"$(openssl rand -hex 32)\"/" /etc/cmem-server.toml

# 4. systemd
sudo install -m 0644 deploy/systemd/cmem-server.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now cmem-server

# 5. Bootstrap admin
sudo -u cmem /opt/cmem-server/cmem-server -c /etc/cmem-server.toml \
    admin user create --username admin --password 'change_me!' --admin
```

The macOS equivalent uses `deploy/launchd/com.cmem.server.plist` —
copy it to `/Library/LaunchDaemons/` and `sudo launchctl bootstrap
system /Library/LaunchDaemons/com.cmem.server.plist`.

---

## Docker

```bash
# Quick try (single container, ephemeral DB)
docker build -t cmem-server -f Dockerfile .
docker run --rm -p 8080:8080 cmem-server

# Production-ish (persistent volume, custom config)
docker run -d --name cmem-server \
    -p 127.0.0.1:8080:8080 \
    -v cmem-data:/var/lib/cmem-server \
    -v $(pwd)/cmem-server.toml:/etc/cmem-server.toml:ro \
    --restart unless-stopped \
    cmem-server
```

Or with `docker compose` (includes Caddy):

```bash
DOMAIN=cmem.example.com docker compose up -d
docker compose logs -f cmem-server
```

Full topology in [DEPLOYMENT.md#docker](DEPLOYMENT.md#docker).

---

## Upgrading

```bash
cd cmem-server
git pull
sudo ./scripts/install-server.sh --upgrade
```

This rebuilds the binary, atomically replaces it under `INSTALL_PREFIX`,
and restarts the service. Config (`/etc/cmem-server.toml`) and database
(`/var/lib/cmem-server/cmem-server.db`) are **never touched** by
upgrade. Schema migrations run automatically on first start of the new
binary.

> Always back up the SQLite file before a major-version upgrade:
> `sudo sqlite3 /var/lib/cmem-server/cmem-server.db "VACUUM INTO '/tmp/backup.db'" && sudo gzip /tmp/backup.db`.

---

## Uninstalling

Interactive (recommended — confirms each delete):

```bash
sudo ./scripts/uninstall-server.sh --backup
```

Non-interactive (CI / scripted teardown):

```bash
sudo ./scripts/uninstall-server.sh --yes --keep-data
```

Flags:

- `--backup` — `VACUUM INTO` + gzip the DB to `~/cmem-backup-*.db.gz`
  before any destructive step.
- `--keep-data` — leave `/var/lib/cmem-server` (and its DB) intact.
- `--yes` — skip every confirmation. **Will delete the DB unless
  combined with `--keep-data`.**

The uninstaller removes (in order): the systemd unit / launchd plist,
the binary directory, the config, the data directory (unless
`--keep-data`), the Caddy snippet, and the `cmem` system user.

---

## Verifying the install

**Recommended one-liner** — runs an 8-point health check:

```bash
sudo ./scripts/install-server.sh --check
```

Sample output (everything OK):

```
>>> 1/8 系统环境       OS: linux (Linux 6.x ...)
>>> 2/8 Rust toolchain cargo 1.83.0
>>> 3/8 cmem-server 二进制 /opt/cmem-server/cmem-server (v0.1.0)
>>> 4/8 配置文件        /etc/cmem-server.toml (bind = 127.0.0.1:8080)
>>> 5/8 数据目录        /var/lib/cmem-server (28M) + cmem-server.db (12M)
>>> 6/8 服务状态        systemd: cmem-server.service active + enabled
>>> 7/8 /healthz       {"status":"ok","version":"0.1.0"}
>>> 8/8 admin 用户      1 个 admin 账号
━━━ 全部通过 ━━━
```

Non-zero exit + `[!!]` markers point to which item failed. Suitable for cron
monitoring (`*/5 * * * * /opt/.../install-server.sh --check >>/var/log/cmem-check.log`).

Manual checks (still useful):

```bash
# health endpoint
curl -s http://127.0.0.1:8080/healthz | jq
# → { "status": "ok", "version": "0.1.0" }

# version
/opt/cmem-server/cmem-server --version

# admin web (default admin / admin@123 — change immediately!)
xdg-open http://127.0.0.1:8080/admin/login

# new-user registration (only if admin set registration_mode != closed)
xdg-open http://127.0.0.1:8080/register

# CLI smoke
sudo -u cmem /opt/cmem-server/cmem-server -c /etc/cmem-server.toml admin stats

# End-to-end auth
BASE=http://127.0.0.1:8080 ./scripts/smoke_auth.sh
```

If any check fails, see [TROUBLESHOOTING.md](TROUBLESHOOTING.md).

---

## Packaging a release tarball

For internal distribution, GitHub Release artifacts, or air-gapped installs:

```bash
# Native (current host arch)
bash scripts/pack-release.sh

# Cross-build a Linux musl static binary (needs `cross` from cargo)
bash scripts/pack-release.sh --target x86_64-unknown-linux-musl
```

Produces `dist/`:

```
cmem-server-0.1.0-<target>.tar.gz       binary + scripts + docs + config example
cmem-server-0.1.0-<target>.tar.gz.sha256
install-server.sh                       (top-level copy, easy to curl-pipe)
uninstall-server.sh
RELEASE_MANIFEST.txt                    git hash + dirty flag + build time + verify cmds
```

End-user install:

```bash
curl -fsSLO https://your-host/cmem-server-0.1.0-x86_64-unknown-linux-musl.tar.gz
curl -fsSLO https://your-host/cmem-server-0.1.0-x86_64-unknown-linux-musl.tar.gz.sha256
shasum -a 256 -c cmem-server-0.1.0-x86_64-unknown-linux-musl.tar.gz.sha256   # MUST verify
tar -xzf cmem-server-0.1.0-x86_64-unknown-linux-musl.tar.gz
cd cmem-server-0.1.0-x86_64-unknown-linux-musl
sudo ./scripts/install-server.sh --bind 127.0.0.1:8080
```

See `RELEASE_MANIFEST.txt` for the exact verify commands.
