#!/usr/bin/env bash
# cmem-server 卸载脚本
#
# 用法:
#   ./uninstall-server.sh                    # 交互式确认每一步
#   ./uninstall-server.sh --yes              # 全部确认(危险!)
#   ./uninstall-server.sh --keep-data        # 保留 /var/lib/cmem-server
#   ./uninstall-server.sh --backup           # 卸载前 dump db 到 ~/cmem-backup-*.db.gz

set -uo pipefail

if [[ -t 1 ]]; then
    BOLD=$'\033[1m'; RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; BLUE=$'\033[34m'; RESET=$'\033[0m'
else
    BOLD=""; RED=""; GREEN=""; YELLOW=""; BLUE=""; RESET=""
fi

log()  { echo -e "$@" >&2; }
ok()   { log "  [${GREEN}OK${RESET}]   $*"; }
fail() { log "  [${RED}FAIL${RESET}] $*"; exit 1; }
warn() { log "  [${YELLOW}!!${RESET}] $*"; }
info() { log "  [${BLUE}--${RESET}] $*"; }
step() { echo >&2; log "${BOLD}${BLUE}>>> $*${RESET}"; }

ASSUME_YES=0
KEEP_DATA=0
DO_BACKUP=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --yes|-y)     ASSUME_YES=1; shift ;;
        --keep-data)  KEEP_DATA=1; shift ;;
        --backup)     DO_BACKUP=1; shift ;;
        -h|--help)    sed -n '2,12p' "$0"; exit 0 ;;
        *) fail "未知参数:$1" ;;
    esac
done

confirm() {
    local prompt="$1"
    if [[ $ASSUME_YES -eq 1 ]]; then return 0; fi
    read -r -p "  ?? ${prompt} [y/N] " ans
    [[ "$ans" == "y" || "$ans" == "Y" ]]
}

OS=""; OS_FAMILY=""
case "$(uname -s)" in
    Darwin)
        OS="macos"; OS_FAMILY="mac"
        INSTALL_PREFIX="/usr/local/share/cmem-server"
        DATA_DIR="/usr/local/var/cmem-server"
        CONFIG_PATH="/usr/local/etc/cmem-server.toml"
        SUDO="sudo"
        ;;
    Linux)
        OS_FAMILY="linux"
        INSTALL_PREFIX="/opt/cmem-server"
        DATA_DIR="/var/lib/cmem-server"
        CONFIG_PATH="/etc/cmem-server.toml"
        if [[ $EUID -ne 0 ]]; then SUDO="sudo"; else SUDO=""; fi
        ;;
    *) fail "不支持的内核" ;;
esac

SYSTEMD_UNIT="/etc/systemd/system/cmem-server.service"
LAUNCHD_PLIST="/Library/LaunchDaemons/com.cmem.server.plist"
CADDY_SNIPPET_LINUX="/etc/caddy/Caddyfile.d/cmem.conf"
CADDY_SNIPPET_MAC1="/opt/homebrew/etc/Caddyfile.d/cmem.conf"
CADDY_SNIPPET_MAC2="/usr/local/etc/Caddyfile.d/cmem.conf"

step "cmem-server 卸载"

if [[ $DO_BACKUP -eq 1 ]] && [[ -f "$DATA_DIR/cmem-server.db" ]]; then
    local_backup="$HOME/cmem-backup-$(date +%Y%m%d-%H%M%S).db.gz"
    info "备份数据库 → $local_backup"
    $SUDO sqlite3 "$DATA_DIR/cmem-server.db" "VACUUM INTO '/tmp/cmem-uninstall.db'" 2>/dev/null \
        && $SUDO gzip -c /tmp/cmem-uninstall.db > "$local_backup" \
        && $SUDO rm -f /tmp/cmem-uninstall.db \
        && ok "备份完成"
fi

# ─── 停服务 ────────────────────────────────────────────
if [[ "$OS_FAMILY" == "mac" ]]; then
    if [[ -f "$LAUNCHD_PLIST" ]]; then
        confirm "停止并卸载 launchd 服务?" && {
            $SUDO launchctl bootout system "$LAUNCHD_PLIST" 2>/dev/null || \
                $SUDO launchctl unload "$LAUNCHD_PLIST" 2>/dev/null || true
            $SUDO rm -f "$LAUNCHD_PLIST" && ok "已删 $LAUNCHD_PLIST"
        }
    fi
else
    if [[ -f "$SYSTEMD_UNIT" ]]; then
        confirm "停止并禁用 systemd 服务?" && {
            $SUDO systemctl stop cmem-server.service 2>/dev/null || true
            $SUDO systemctl disable cmem-server.service 2>/dev/null || true
            $SUDO rm -f "$SYSTEMD_UNIT"
            $SUDO systemctl daemon-reload
            ok "systemd unit 已删除"
        }
    fi
fi

# ─── 二进制 ────────────────────────────────────────────
if [[ -d "$INSTALL_PREFIX" ]]; then
    confirm "删除二进制目录 $INSTALL_PREFIX?" && {
        $SUDO rm -rf "$INSTALL_PREFIX" && ok "已删 $INSTALL_PREFIX"
    }
fi

# ─── 配置 ──────────────────────────────────────────────
if [[ -f "$CONFIG_PATH" ]]; then
    confirm "删除配置 $CONFIG_PATH?" && {
        $SUDO rm -f "$CONFIG_PATH" && ok "已删配置"
    }
fi

# ─── 数据目录(保留与否) ─────────────────────────────
if [[ -d "$DATA_DIR" ]]; then
    if [[ $KEEP_DATA -eq 1 ]]; then
        warn "保留数据目录 $DATA_DIR(--keep-data)"
    else
        confirm "${RED}危险!${RESET} 删除数据目录 $DATA_DIR(包含数据库)?" && {
            $SUDO rm -rf "$DATA_DIR" && ok "已删 $DATA_DIR"
        }
    fi
fi

# ─── Caddy 片段 ────────────────────────────────────────
for s in "$CADDY_SNIPPET_LINUX" "$CADDY_SNIPPET_MAC1" "$CADDY_SNIPPET_MAC2"; do
    if [[ -f "$s" ]]; then
        confirm "删除 Caddy 片段 $s?" && {
            $SUDO rm -f "$s" && ok "已删 $s"
            if command -v caddy >/dev/null 2>&1; then
                $SUDO systemctl reload caddy 2>/dev/null || \
                    $SUDO brew services restart caddy 2>/dev/null || true
            fi
        }
    fi
done

# ─── service user(可选) ─────────────────────────────
if [[ "$OS_FAMILY" == "linux" ]] && id cmem >/dev/null 2>&1; then
    confirm "删除系统账号 cmem?" && {
        if command -v userdel >/dev/null 2>&1; then
            $SUDO userdel cmem 2>/dev/null && ok "用户 cmem 已删"
        elif command -v deluser >/dev/null 2>&1; then
            $SUDO deluser cmem 2>/dev/null && ok "用户 cmem 已删"
        fi
    }
fi

echo
log "${BOLD}${GREEN}卸载流程结束${RESET}"
log "如有备份,在 $HOME/cmem-backup-*.db.gz"
