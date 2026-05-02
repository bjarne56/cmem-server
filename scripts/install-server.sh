#!/usr/bin/env bash
# cmem-server 通用安装脚本
#
# 兼容 OS:macOS / Ubuntu / Debian / Rocky 8 / Rocky 9 / CentOS / Fedora / Arch / Alpine
#
# 用法:
#   ./install-server.sh                                # 默认本地装(127.0.0.1:8080)
#   ./install-server.sh --bind 0.0.0.0:8080            # 监听所有接口
#   ./install-server.sh --domain cmem.example.com      # 自动配 Caddy + Let's Encrypt
#   ./install-server.sh --no-systemd                   # 不装 systemd unit(macOS 自动用 launchd)
#   ./install-server.sh --user cmem                    # 创建 cmem user 跑 systemd(默认)
#   ./install-server.sh --upgrade                      # 升级现有安装(只重 build + 重启)
#   ./install-server.sh --uninstall                    # 完整卸载(委托 uninstall-server.sh)
#   ./install-server.sh --source-dir /path/to/repo     # 从指定本地源码装(否则从当前 git work tree)
#   ./install-server.sh --skip-bootstrap               # 不创建默认 admin/admin@123
#   ./install-server.sh --bootstrap-password <PW>      # 指定默认 admin 密码
#
# 退出码:
#   0  成功
#   1  通用错误
#   2  参数错误
#   3  环境检测失败(不支持的 OS)
#   4  权限不足
#   5  网络 / 包管理器失败

set -uo pipefail

# ─── 颜色 ──────────────────────────────────────────────
if [[ -t 1 ]]; then
    BOLD=$'\033[1m'; DIM=$'\033[2m'
    RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; BLUE=$'\033[34m'; CYAN=$'\033[36m'
    RESET=$'\033[0m'
else
    BOLD=""; DIM=""; RED=""; GREEN=""; YELLOW=""; BLUE=""; CYAN=""; RESET=""
fi
OK="${GREEN}OK${RESET}"; FAIL="${RED}FAIL${RESET}"; INFO="${BLUE}--${RESET}"; WARN="${YELLOW}!!${RESET}"

log()  { echo -e "$@" >&2; }
ok()   { log "  [${OK}]   $*"; }
fail() { log "  [${FAIL}] ${RED}$*${RESET}"; exit "${2:-1}"; }
warn() { log "  [${WARN}] ${YELLOW}$*${RESET}"; }
info() { log "  [${INFO}] $*"; }
step() { echo >&2; log "${BOLD}${BLUE}>>> $*${RESET}"; }

# ─── 默认值 ─────────────────────────────────────────────
BIND_ADDR="127.0.0.1:8080"
DOMAIN=""
SERVICE_USER="cmem"
SOURCE_DIR=""
SKIP_SYSTEMD=0
DO_UPGRADE=0
DO_UNINSTALL=0
SKIP_BOOTSTRAP=0
BOOTSTRAP_USER="admin"
BOOTSTRAP_PASSWORD="admin@123"
ASSUME_YES=0

# ─── 解析参数 ──────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --bind)                BIND_ADDR="$2"; shift 2 ;;
        --domain)              DOMAIN="$2"; shift 2 ;;
        --user)                SERVICE_USER="$2"; shift 2 ;;
        --source-dir)          SOURCE_DIR="$2"; shift 2 ;;
        --no-systemd)          SKIP_SYSTEMD=1; shift ;;
        --upgrade)             DO_UPGRADE=1; shift ;;
        --uninstall)           DO_UNINSTALL=1; shift ;;
        --skip-bootstrap)      SKIP_BOOTSTRAP=1; shift ;;
        --bootstrap-password)  BOOTSTRAP_PASSWORD="$2"; shift 2 ;;
        -y|--yes)              ASSUME_YES=1; shift ;;
        -h|--help)
            sed -n '2,30p' "$0"
            exit 0 ;;
        *) fail "未知参数:$1(用 --help 看用法)" 2 ;;
    esac
done

# ─── 平台路径(在 OS 检测后赋值) ────────────────────────
INSTALL_PREFIX=""        # 二进制安装目录(/opt/cmem-server 或 /usr/local/share/cmem-server)
DATA_DIR=""              # /var/lib/cmem-server 或 macOS 等价物
CONFIG_PATH=""           # /etc/cmem-server.toml 或 macOS 等价物
SYSTEMD_UNIT="/etc/systemd/system/cmem-server.service"
LAUNCHD_PLIST="/Library/LaunchDaemons/com.cmem.server.plist"
CADDY_SNIPPET=""         # /etc/caddy/Caddyfile.d/cmem.conf 或 /opt/homebrew/etc/Caddyfile.d/cmem.conf

OS=""           # macos / ubuntu / debian / rocky / centos / fedora / arch / alpine
OS_FAMILY=""    # mac / debian / rhel / arch / alpine
ARCH=""

# ─── OS 检测 ───────────────────────────────────────────
detect_os() {
    local kernel; kernel="$(uname -s)"
    ARCH="$(uname -m)"

    case "$kernel" in
        Darwin)
            OS="macos"; OS_FAMILY="mac"
            INSTALL_PREFIX="/usr/local/share/cmem-server"
            DATA_DIR="/usr/local/var/cmem-server"
            CONFIG_PATH="/usr/local/etc/cmem-server.toml"
            SKIP_SYSTEMD=1   # macOS 永远不用 systemd
            CADDY_SNIPPET="/opt/homebrew/etc/Caddyfile.d/cmem.conf"
            [[ -d /opt/homebrew ]] || CADDY_SNIPPET="/usr/local/etc/Caddyfile.d/cmem.conf"
            ;;
        Linux)
            INSTALL_PREFIX="/opt/cmem-server"
            DATA_DIR="/var/lib/cmem-server"
            CONFIG_PATH="/etc/cmem-server.toml"
            CADDY_SNIPPET="/etc/caddy/Caddyfile.d/cmem.conf"

            if [[ -f /etc/os-release ]]; then
                # shellcheck disable=SC1091
                . /etc/os-release
                case "${ID:-}" in
                    ubuntu)            OS="ubuntu"; OS_FAMILY="debian" ;;
                    debian)            OS="debian"; OS_FAMILY="debian" ;;
                    rocky|almalinux)   OS="rocky";  OS_FAMILY="rhel"   ;;
                    centos|rhel)       OS="centos"; OS_FAMILY="rhel"   ;;
                    fedora)            OS="fedora"; OS_FAMILY="rhel"   ;;
                    arch|manjaro)      OS="arch";   OS_FAMILY="arch"   ;;
                    alpine)            OS="alpine"; OS_FAMILY="alpine" ;;
                    *) fail "不支持的 Linux 发行版:${ID:-unknown}" 3 ;;
                esac
            else
                fail "/etc/os-release 不存在,无法识别发行版" 3
            fi
            ;;
        *) fail "不支持的内核:$kernel" 3 ;;
    esac

    info "OS=${OS} family=${OS_FAMILY} arch=${ARCH}"
}

# ─── 权限检查 ──────────────────────────────────────────
need_root() {
    if [[ "$OS_FAMILY" == "mac" ]]; then
        # macOS:有些操作需要 sudo,我们让脚本自己 sudo,而不是要求脚本以 root 启动
        SUDO="sudo"
        # 但如果 LaunchDaemons 写入或 brew 安装,后面再 sudo
    else
        if [[ "$EUID" -ne 0 ]]; then
            if command -v sudo >/dev/null 2>&1; then
                SUDO="sudo"
                info "未以 root 启动,后续命令将使用 sudo"
            else
                fail "Linux 安装需要 root 或 sudo" 4
            fi
        else
            SUDO=""
        fi
    fi
}

# ─── 检查依赖工具 ──────────────────────────────────────
ensure_tool() {
    local tool="$1"
    if ! command -v "$tool" >/dev/null 2>&1; then
        return 1
    fi
}

# ─── 装系统包 ──────────────────────────────────────────
install_system_packages() {
    step "安装系统编译依赖"
    case "$OS_FAMILY" in
        mac)
            if ! command -v brew >/dev/null 2>&1; then
                warn "未检测到 Homebrew,跳过系统包安装(假设 Xcode CLT 已装)"
                xcode-select -p >/dev/null 2>&1 || \
                    fail "请先装 Xcode Command Line Tools:xcode-select --install" 5
            else
                brew list openssl@3 >/dev/null 2>&1 || brew install openssl@3 || \
                    warn "brew install openssl@3 失败,可能影响 build"
                brew list pkg-config >/dev/null 2>&1 || brew install pkg-config || true
            fi
            ;;
        debian)
            $SUDO apt-get update -y || fail "apt-get update 失败" 5
            $SUDO apt-get install -y --no-install-recommends \
                build-essential pkg-config libssl-dev ca-certificates curl \
                || fail "apt-get install 失败" 5
            ;;
        rhel)
            local pm; pm="dnf"; command -v dnf >/dev/null 2>&1 || pm="yum"
            $SUDO $pm install -y gcc gcc-c++ pkgconf-pkg-config openssl-devel ca-certificates curl \
                || fail "$pm install 失败" 5
            ;;
        arch)
            $SUDO pacman -Sy --noconfirm --needed base-devel openssl pkgconf curl ca-certificates \
                || fail "pacman 安装失败" 5
            ;;
        alpine)
            $SUDO apk add --no-cache build-base openssl-dev pkgconf curl ca-certificates bash \
                || fail "apk add 失败" 5
            ;;
    esac
    ok "系统编译依赖完成"
}

# ─── 装 Rust toolchain ────────────────────────────────
install_rust() {
    step "Rust toolchain"
    if command -v cargo >/dev/null 2>&1; then
        local v; v="$(cargo --version 2>/dev/null || true)"
        ok "已检测到:$v"
        return
    fi
    info "未检测到 cargo,通过 rustup 安装(stable)"
    if ! command -v curl >/dev/null 2>&1; then
        fail "未检测到 curl,无法装 rustup" 5
    fi
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal \
        || fail "rustup 安装失败" 5
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
    ok "rustup 安装完成"
}

# ─── 准备源码 ──────────────────────────────────────────
prepare_source() {
    step "准备源码"
    if [[ -n "$SOURCE_DIR" ]]; then
        [[ -d "$SOURCE_DIR" ]] || fail "源码目录不存在:$SOURCE_DIR" 1
        info "使用指定源码目录:$SOURCE_DIR"
    else
        # 默认使用脚本所在仓库根
        SOURCE_DIR="$(cd "$(dirname "$0")/.." && pwd)"
        info "使用脚本所在仓库:$SOURCE_DIR"
    fi
    [[ -f "$SOURCE_DIR/Cargo.toml" ]] || fail "源码目录缺 Cargo.toml:$SOURCE_DIR" 1
    ok "源码就绪"
}

# ─── 构建 ──────────────────────────────────────────────
build_release() {
    step "cargo build --release"
    (cd "$SOURCE_DIR" && cargo build --release --bin cmem-server) \
        || fail "cargo build 失败" 1
    [[ -f "$SOURCE_DIR/target/release/cmem-server" ]] || fail "build 后未生成二进制" 1
    ok "构建完成"
}

# ─── 创建 service user(Linux) ─────────────────────────
create_service_user() {
    [[ "$OS_FAMILY" == "mac" ]] && return
    [[ $SKIP_SYSTEMD -eq 1 ]] && return

    step "创建服务账号 $SERVICE_USER"
    if id "$SERVICE_USER" >/dev/null 2>&1; then
        ok "$SERVICE_USER 已存在"
        return
    fi

    case "$OS_FAMILY" in
        debian|rhel|arch)
            $SUDO useradd --system --no-create-home --shell /usr/sbin/nologin --user-group "$SERVICE_USER" \
                || fail "useradd 失败" 1
            ;;
        alpine)
            $SUDO addgroup -S "$SERVICE_USER" 2>/dev/null || true
            $SUDO adduser -S -D -H -G "$SERVICE_USER" -s /sbin/nologin "$SERVICE_USER" \
                || fail "adduser 失败" 1
            ;;
    esac
    ok "$SERVICE_USER 创建完成"
}

# ─── 安装二进制和配置 ──────────────────────────────────
install_files() {
    step "安装文件到 $INSTALL_PREFIX"

    $SUDO mkdir -p "$INSTALL_PREFIX" "$DATA_DIR" "$(dirname "$CONFIG_PATH")" \
        || fail "创建目录失败" 1

    $SUDO install -m 0755 "$SOURCE_DIR/target/release/cmem-server" "$INSTALL_PREFIX/cmem-server" \
        || fail "复制 cmem-server 失败" 1

    # 部署模板(可选,便于运维查阅)
    $SUDO mkdir -p "$INSTALL_PREFIX/deploy"
    if [[ -d "$SOURCE_DIR/deploy" ]]; then
        $SUDO cp -R "$SOURCE_DIR/deploy/." "$INSTALL_PREFIX/deploy/" || true
    fi

    if [[ "$OS_FAMILY" != "mac" ]] && [[ $SKIP_SYSTEMD -eq 0 ]]; then
        $SUDO chown -R "$SERVICE_USER:$SERVICE_USER" "$DATA_DIR"
    fi
    ok "文件安装完成"
}

# ─── 写配置 ────────────────────────────────────────────
write_config() {
    step "生成配置 $CONFIG_PATH"
    if $SUDO test -f "$CONFIG_PATH"; then
        warn "配置已存在,跳过(用 --upgrade 不会覆盖)"
        return
    fi

    local jwt_secret
    if command -v openssl >/dev/null 2>&1; then
        jwt_secret="$(openssl rand -hex 32)"
    else
        # fallback:让 server 启动时自己生成并写回
        jwt_secret=""
    fi

    local db_path
    if [[ "$OS_FAMILY" == "mac" ]]; then
        db_path="$DATA_DIR/cmem-server.db"
    else
        db_path="$DATA_DIR/cmem-server.db"
    fi

    local tmp; tmp="$(mktemp)"
    cat > "$tmp" <<EOF
# cmem-server 配置文件
# 由 install-server.sh 生成于 $(date -u +%Y-%m-%dT%H:%M:%SZ)
# 修改此文件后用 systemctl restart cmem-server(Linux)
# 或 sudo launchctl kickstart -k system/com.cmem.server(macOS)生效

[server]
bind = "${BIND_ADDR}"

[database]
path = "${db_path}"

[auth]
# 256-bit 随机 secret;留空时 server 启动会自动生成并写回
jwt_secret = "${jwt_secret}"
access_token_ttl_secs = 900
refresh_token_ttl_secs = 2592000
machine_token_ttl_secs = 15552000
# argon2id 推荐参数(RFC 9106)
argon2_memory_kib = 19456
argon2_iterations = 2
argon2_parallelism = 1
# 是否要求 register 必须带 invite_code
require_invite = false
EOF

    $SUDO install -m 0640 "$tmp" "$CONFIG_PATH" || fail "写配置失败" 1
    rm -f "$tmp"

    if [[ "$OS_FAMILY" != "mac" ]] && [[ $SKIP_SYSTEMD -eq 0 ]]; then
        $SUDO chown "root:$SERVICE_USER" "$CONFIG_PATH" 2>/dev/null || true
    fi
    ok "配置写入 $CONFIG_PATH"
}

# ─── systemd unit ──────────────────────────────────────
install_systemd_unit() {
    [[ "$OS_FAMILY" == "mac" ]] && return
    [[ $SKIP_SYSTEMD -eq 1 ]] && { warn "跳过 systemd"; return; }

    step "写入 systemd unit"
    local tmp; tmp="$(mktemp)"
    cat > "$tmp" <<EOF
[Unit]
Description=cmem-sync server
After=network.target
Documentation=https://github.com/bjarne/cmem-server

[Service]
Type=simple
User=${SERVICE_USER}
Group=${SERVICE_USER}
WorkingDirectory=${DATA_DIR}
ExecStart=${INSTALL_PREFIX}/cmem-server -c ${CONFIG_PATH}
Restart=on-failure
RestartSec=5

# 安全收紧
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=${DATA_DIR}
PrivateTmp=true
PrivateDevices=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
RestrictRealtime=true
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
EOF
    $SUDO install -m 0644 "$tmp" "$SYSTEMD_UNIT" || fail "写 systemd unit 失败" 1
    rm -f "$tmp"

    $SUDO systemctl daemon-reload
    $SUDO systemctl enable cmem-server.service >/dev/null 2>&1 || true
    if [[ $DO_UPGRADE -eq 1 ]]; then
        $SUDO systemctl restart cmem-server.service || fail "systemctl restart 失败" 1
    else
        $SUDO systemctl start cmem-server.service || fail "systemctl start 失败" 1
    fi
    ok "systemd unit 装好,服务已启动"
}

# ─── launchd plist(macOS) ─────────────────────────────
install_launchd_plist() {
    [[ "$OS_FAMILY" != "mac" ]] && return

    step "写入 launchd plist"
    local tmp; tmp="$(mktemp)"
    cat > "$tmp" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.cmem.server</string>
    <key>ProgramArguments</key>
    <array>
        <string>${INSTALL_PREFIX}/cmem-server</string>
        <string>-c</string>
        <string>${CONFIG_PATH}</string>
    </array>
    <key>WorkingDirectory</key>
    <string>${DATA_DIR}</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>${DATA_DIR}/cmem-server.log</string>
    <key>StandardErrorPath</key>
    <string>${DATA_DIR}/cmem-server.err.log</string>
</dict>
</plist>
EOF
    $SUDO install -m 0644 "$tmp" "$LAUNCHD_PLIST" || fail "写 plist 失败" 1
    rm -f "$tmp"

    # 卸载旧的(如果存在),再装新的
    $SUDO launchctl bootout system "$LAUNCHD_PLIST" 2>/dev/null || true
    $SUDO launchctl bootstrap system "$LAUNCHD_PLIST" || \
        $SUDO launchctl load -w "$LAUNCHD_PLIST" || \
        fail "launchctl 启动失败" 1
    ok "launchd 服务已启动"
}

# ─── 等 healthz ────────────────────────────────────────
wait_healthz() {
    step "等待 /healthz"
    local target="${BIND_ADDR}"
    # 如果 bind 是 0.0.0.0,本地探测要用 127.0.0.1
    target="${target/0.0.0.0/127.0.0.1}"
    local url="http://${target}/healthz"

    local i=0
    while [[ $i -lt 30 ]]; do
        if curl -fsS "$url" >/dev/null 2>&1; then
            ok "healthz 返回 200"
            return
        fi
        sleep 1
        i=$((i + 1))
    done
    warn "等了 30s 仍无 healthz 响应,请手动排查:journalctl -u cmem-server -n 50"
}

# ─── bootstrap admin ──────────────────────────────────
bootstrap_admin() {
    [[ $SKIP_BOOTSTRAP -eq 1 ]] && { info "跳过默认 admin 创建"; return; }

    step "bootstrap 默认管理员 ${BOOTSTRAP_USER}"
    # 已存在?跳过
    if $SUDO -u "${SERVICE_USER:-root}" "$INSTALL_PREFIX/cmem-server" -c "$CONFIG_PATH" admin user list 2>/dev/null \
        | grep -q "^[[:space:]]*[0-9a-f-]\+[[:space:]]\+${BOOTSTRAP_USER}[[:space:]]"; then
        warn "${BOOTSTRAP_USER} 已存在,跳过"
        return
    fi

    if [[ "$OS_FAMILY" == "mac" ]] || [[ $SKIP_SYSTEMD -eq 1 ]]; then
        $SUDO "$INSTALL_PREFIX/cmem-server" -c "$CONFIG_PATH" \
            admin user create --username "$BOOTSTRAP_USER" --password "$BOOTSTRAP_PASSWORD" --admin \
            || warn "bootstrap admin 失败(可能账号已存在)"
    else
        $SUDO -u "$SERVICE_USER" "$INSTALL_PREFIX/cmem-server" -c "$CONFIG_PATH" \
            admin user create --username "$BOOTSTRAP_USER" --password "$BOOTSTRAP_PASSWORD" --admin \
            || warn "bootstrap admin 失败(可能账号已存在)"
    fi
    ok "${BOOTSTRAP_USER} 已创建,密码:${BOOTSTRAP_PASSWORD}"
}

# ─── Caddy 反代 ────────────────────────────────────────
install_caddy() {
    [[ -z "$DOMAIN" ]] && return

    step "配置 Caddy 反代 → $DOMAIN"
    if ! command -v caddy >/dev/null 2>&1; then
        info "未检测到 caddy,自动安装"
        case "$OS_FAMILY" in
            mac)
                command -v brew >/dev/null 2>&1 || fail "需要 brew 装 caddy" 5
                brew install caddy
                ;;
            debian)
                $SUDO apt-get install -y debian-keyring debian-archive-keyring apt-transport-https || true
                curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' \
                    | $SUDO gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
                curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' \
                    | $SUDO tee /etc/apt/sources.list.d/caddy-stable.list >/dev/null
                $SUDO apt-get update -y && $SUDO apt-get install -y caddy
                ;;
            rhel)
                local pm; pm="dnf"; command -v dnf >/dev/null 2>&1 || pm="yum"
                $SUDO $pm install -y 'dnf-command(copr)' || true
                $SUDO $pm copr enable -y @caddy/caddy || true
                $SUDO $pm install -y caddy
                ;;
            arch)
                $SUDO pacman -S --noconfirm caddy
                ;;
            alpine)
                $SUDO apk add --no-cache caddy
                ;;
        esac
    fi

    $SUDO mkdir -p "$(dirname "$CADDY_SNIPPET")"
    local backend="${BIND_ADDR/0.0.0.0/127.0.0.1}"
    local tmp; tmp="$(mktemp)"
    cat > "$tmp" <<EOF
# cmem-server reverse proxy(由 install-server.sh 生成)
${DOMAIN} {
    reverse_proxy ${backend} {
        header_up X-Real-IP {remote_host}
        header_up X-Forwarded-For {remote_host}
        header_up X-Forwarded-Proto {scheme}
    }
    encode gzip zstd
    log {
        output file /var/log/caddy/cmem-server.log
        format json
    }
}
EOF
    $SUDO install -m 0644 "$tmp" "$CADDY_SNIPPET"
    rm -f "$tmp"

    # 主 Caddyfile 必须 import 这个目录
    local main="/etc/caddy/Caddyfile"
    [[ "$OS_FAMILY" == "mac" ]] && main="$(dirname "$CADDY_SNIPPET")/../Caddyfile"
    if [[ -f "$main" ]] && ! grep -q "Caddyfile.d" "$main" 2>/dev/null; then
        info "向主 Caddyfile 追加 import $(dirname "$CADDY_SNIPPET")/*.conf"
        echo "" | $SUDO tee -a "$main" >/dev/null
        echo "import $(dirname "$CADDY_SNIPPET")/*.conf" | $SUDO tee -a "$main" >/dev/null
    fi

    if [[ "$OS_FAMILY" == "mac" ]]; then
        $SUDO brew services restart caddy 2>/dev/null || warn "请手动 brew services restart caddy"
    else
        $SUDO systemctl enable --now caddy 2>/dev/null || true
        $SUDO systemctl reload caddy 2>/dev/null || $SUDO systemctl restart caddy
    fi
    ok "Caddy 已配置,自动签 Let's Encrypt(等几秒)"
}

# ─── 输出收尾 ──────────────────────────────────────────
print_summary() {
    local backend="${BIND_ADDR/0.0.0.0/127.0.0.1}"
    local public_url="http://${backend}"
    [[ -n "$DOMAIN" ]] && public_url="https://${DOMAIN}"

    echo
    log "${BOLD}${GREEN}================ 安装完成 ================${RESET}"
    log "${BOLD}服务 URL${RESET}      ${public_url}"
    log "${BOLD}admin web${RESET}     ${public_url}/admin/login"
    log "${BOLD}healthz${RESET}       curl ${public_url}/healthz"
    log "${BOLD}config${RESET}        ${CONFIG_PATH}"
    log "${BOLD}data dir${RESET}      ${DATA_DIR}"
    log "${BOLD}binary${RESET}        ${INSTALL_PREFIX}/cmem-server"
    if [[ $SKIP_BOOTSTRAP -eq 0 ]]; then
        log ""
        log "${BOLD}${YELLOW}默认管理员${RESET}    ${BOOTSTRAP_USER} / ${BOOTSTRAP_PASSWORD}"
        log "${YELLOW}强烈建议立刻改密:${RESET}"
        log "  ${INSTALL_PREFIX}/cmem-server -c ${CONFIG_PATH} admin user reset-password --username ${BOOTSTRAP_USER}"
    fi
    log ""
    if [[ "$OS_FAMILY" != "mac" ]] && [[ $SKIP_SYSTEMD -eq 0 ]]; then
        log "${BOLD}操作${RESET}"
        log "  systemctl status cmem-server"
        log "  systemctl restart cmem-server"
        log "  journalctl -u cmem-server -f"
    elif [[ "$OS_FAMILY" == "mac" ]]; then
        log "${BOLD}操作${RESET}"
        log "  sudo launchctl print system/com.cmem.server"
        log "  sudo launchctl kickstart -k system/com.cmem.server"
        log "  tail -f ${DATA_DIR}/cmem-server.log"
    fi
    log ""
    log "${BOLD}卸载${RESET}          $0 --uninstall"
    log "${BOLD}升级${RESET}          $0 --upgrade --source-dir <repo>"
    echo
}

# ─── upgrade 流程 ──────────────────────────────────────
do_upgrade() {
    info "升级模式:只重 build 二进制 + 重启服务,不动 config / db"
    detect_os
    need_root
    prepare_source
    build_release
    $SUDO install -m 0755 "$SOURCE_DIR/target/release/cmem-server" "$INSTALL_PREFIX/cmem-server"
    if [[ "$OS_FAMILY" == "mac" ]]; then
        $SUDO launchctl kickstart -k system/com.cmem.server || warn "launchctl kickstart 失败"
    elif [[ $SKIP_SYSTEMD -eq 0 ]]; then
        $SUDO systemctl restart cmem-server.service
    fi
    wait_healthz
    log "${GREEN}升级完成${RESET}"
}

# ─── uninstall 流程(委托独立脚本) ───────────────────
do_uninstall() {
    local us="$(dirname "$0")/uninstall-server.sh"
    if [[ -x "$us" ]]; then
        exec "$us" "$@"
    else
        fail "未找到 uninstall-server.sh,请手动卸载" 1
    fi
}

# ─── main ──────────────────────────────────────────────
main() {
    if [[ $DO_UNINSTALL -eq 1 ]]; then do_uninstall; fi

    if [[ $DO_UPGRADE -eq 1 ]]; then
        do_upgrade
        exit 0
    fi

    detect_os
    need_root
    install_system_packages
    install_rust
    prepare_source
    build_release
    create_service_user
    install_files
    write_config
    install_systemd_unit
    install_launchd_plist
    wait_healthz
    bootstrap_admin
    install_caddy
    print_summary
}

main "$@"
