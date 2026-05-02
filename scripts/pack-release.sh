#!/usr/bin/env bash
# pack-release.sh — 打 cmem-server release 包(开源 / 内部分发用)
#
# 跟 release.sh 区别:
#   release.sh  = 改 version + git tag + 触发 CI(发布流程)
#   pack-release.sh = 本地产出 distributable artifact(release 工件本身)
#
# 输出:
#   dist/
#     cmem-server-<version>-<platform>.tar.gz       含二进制 + 脚本 + 配置 + 文档
#     cmem-server-<version>-<platform>.tar.gz.sha256
#     install-server.sh                              便于和包一起分发
#     uninstall-server.sh
#     RELEASE_MANIFEST.txt                           内容清单 + 校验 + git hash + 时间
#
# 用法:
#   bash scripts/pack-release.sh                          默认 = build + pack
#   bash scripts/pack-release.sh --skip-build             跳过 cargo build(已 build)
#   bash scripts/pack-release.sh --target x86_64-unknown-linux-musl  cross build 产出 musl 静态二进制
#   bash scripts/pack-release.sh --tag v0.2.0-rc1         自定 release tag
#   bash scripts/pack-release.sh --dry-run                只打印不执行

set -euo pipefail

# ── 颜色 ───────────────────────────────────────
if [[ -t 1 ]]; then
    BOLD=$'\033[1m'; DIM=$'\033[2m'
    RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; BLUE=$'\033[34m'; RESET=$'\033[0m'
else
    BOLD=""; DIM=""; RED=""; GREEN=""; YELLOW=""; BLUE=""; RESET=""
fi
OK="${GREEN}OK${RESET}"; FAIL="${RED}FAIL${RESET}"; INFO="${BLUE}--${RESET}"
log()  { echo -e "$@"; }
ok()   { log "  [${OK}] $*"; }
fail() { log "  [${FAIL}] ${RED}$*${RESET}"; exit 1; }
info() { log "  [${INFO}] $*"; }
step() { echo; log "${BOLD}${BLUE}>>> $*${RESET}"; }

# ── 参数 ───────────────────────────────────────
SKIP_BUILD=0
DRY_RUN=0
RELEASE_TAG=""
TARGET=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --skip-build) SKIP_BUILD=1; shift ;;
        --dry-run)    DRY_RUN=1; shift ;;
        --tag)        RELEASE_TAG="$2"; shift 2 ;;
        --target)     TARGET="$2"; shift 2 ;;
        -h|--help)
            sed -n '1,/^set -euo/p' "$0" | sed '$d'
            exit 0 ;;
        *) fail "未知参数 $1" ;;
    esac
done

# ── 基本检查 ───────────────────────────────────
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT" || fail "找不到项目根目录"

[[ -f Cargo.toml ]] || fail "Cargo.toml 不存在,$ROOT 不是 cmem-server 仓库?"

# 从 server crate 的 Cargo.toml 拿版本(workspace root 没 [package])
PKG_VERSION=$(grep -E '^version\s*=' crates/server/Cargo.toml | head -1 | sed -E 's/.*"([^"]+)".*/\1/')
[[ -z "$PKG_VERSION" ]] && fail "无法从 crates/server/Cargo.toml 解析 version"
PKG_NAME="cmem-server"
[[ -z "$RELEASE_TAG" ]] && RELEASE_TAG="$PKG_VERSION"

# 平台:macOS = aarch64/x86_64-apple-darwin;Linux = x86_64-unknown-linux-gnu;cross 用 --target
if [[ -z "$TARGET" ]]; then
    case "$(uname -s)-$(uname -m)" in
        Darwin-arm64)   TARGET="aarch64-apple-darwin" ;;
        Darwin-x86_64)  TARGET="x86_64-apple-darwin" ;;
        Linux-x86_64)   TARGET="x86_64-unknown-linux-gnu" ;;
        Linux-aarch64)  TARGET="aarch64-unknown-linux-gnu" ;;
        *)              TARGET="$(uname -s)-$(uname -m)" ;;
    esac
fi

DIST="$ROOT/dist"
PACKAGE_NAME="${PKG_NAME}-${PKG_VERSION}-${TARGET}"
TARBALL="$PACKAGE_NAME.tar.gz"

run() {
    if [[ "$DRY_RUN" -eq 1 ]]; then
        info "[dry-run] $*"
    else
        eval "$@" || fail "命令失败:$*"
    fi
}

# ── pipeline ───────────────────────────────────
log "${BOLD}cmem-server release packer${RESET}"
info "package:    $PKG_NAME"
info "version:    $PKG_VERSION"
info "release:    $RELEASE_TAG"
info "target:     $TARGET"
info "output:     $DIST/"

step "1/7 准备 dist/"
run "rm -rf '$DIST'"
run "mkdir -p '$DIST'"

step "2/7 cargo build(可跳过)"
if [[ "$SKIP_BUILD" -eq 1 ]]; then
    info "跳过 build(--skip-build)"
else
    if [[ "$TARGET" != "$(uname -s)-$(uname -m)" && "$TARGET" =~ unknown-linux ]] && command -v cross >/dev/null 2>&1; then
        run "cross build --release --target '$TARGET' --bin cmem-server"
        BIN_PATH="target/$TARGET/release/cmem-server"
    elif [[ -n "$TARGET" && "$TARGET" != "$(uname -m)-$(uname -s | tr 'A-Z' 'a-z' | sed 's/darwin/apple-darwin/')"* ]]; then
        # 试 cargo native cross compile(可能需要 rustup target add)
        run "cargo build --release --target '$TARGET' --bin cmem-server"
        BIN_PATH="target/$TARGET/release/cmem-server"
    else
        run "cargo build --release --bin cmem-server"
        BIN_PATH="target/release/cmem-server"
    fi
fi

# 默认本机 build 就在 target/release
[[ -z "${BIN_PATH:-}" ]] && BIN_PATH="target/release/cmem-server"
[[ "$DRY_RUN" -eq 0 && ! -f "$BIN_PATH" ]] && fail "二进制不存在:$BIN_PATH"
[[ "$DRY_RUN" -eq 0 ]] && ok "binary: $(du -h "$BIN_PATH" | awk '{print $1}') $BIN_PATH"

step "3/7 准备打包内容到 $PACKAGE_NAME/"
local_pkg="$DIST/$PACKAGE_NAME"
run "mkdir -p '$local_pkg/bin' '$local_pkg/scripts' '$local_pkg/docs' '$local_pkg/config'"

# binary
run "cp '$BIN_PATH' '$local_pkg/bin/cmem-server'"
run "chmod +x '$local_pkg/bin/cmem-server'"

# scripts
run "cp scripts/install-server.sh scripts/uninstall-server.sh '$local_pkg/scripts/'"
run "chmod +x '$local_pkg/scripts/'install-server.sh '$local_pkg/scripts/'uninstall-server.sh"

# config example — 用 deploy/config/server.toml.example(干净 placeholder)
# 而不是 dev-server.toml(可能含 dev jwt_secret 等敏感字段)
if [[ -f deploy/config/server.toml.example ]]; then
    run "cp deploy/config/server.toml.example '$local_pkg/config/server.toml.example'"
elif [[ -f dev-server.toml ]]; then
    # fallback,但要 sanitize jwt_secret
    run "sed -E 's/^(jwt_secret\s*=\s*).*/\1\"\"/' dev-server.toml > '$local_pkg/config/server.toml.example'"
fi

# docs(全部)
if [[ -d docs ]]; then
    run "cp -r docs/* '$local_pkg/docs/'"
fi
run "cp README.md LICENSE '$local_pkg/' 2>/dev/null || true"

step "4/7 写 install + 启动 README"
if [[ "$DRY_RUN" -eq 0 ]]; then
    cat > "$local_pkg/QUICKSTART.txt" <<EOF
cmem-server $PKG_VERSION ($TARGET) — quickstart
═══════════════════════════════════════════════════════════════

包内容:
  bin/cmem-server                 主二进制(单文件)
  scripts/install-server.sh       一键装(systemd/launchd + Caddy + bootstrap admin)
  scripts/uninstall-server.sh     卸载
  config/server.toml.example      配置示例
  docs/                           完整文档(11 篇)
  README.md / LICENSE

5 分钟上手:

  # 一、用脚本装(推荐,自动 systemd + Caddy + admin)
  sudo ./scripts/install-server.sh --bind 127.0.0.1:8080
  # 加域名 + 自动 HTTPS:
  sudo ./scripts/install-server.sh --domain cmem.example.com

  # 二、不用脚本(手动)
  sudo cp bin/cmem-server /usr/local/bin/
  sudo cp config/server.toml.example /etc/cmem-server.toml
  sudo \$EDITOR /etc/cmem-server.toml             # 改 jwt_secret 等
  cmem-server -c /etc/cmem-server.toml admin user create --username admin --password '...' --admin
  cmem-server -c /etc/cmem-server.toml             # 前台启动测试

  # 检查健康度:
  ./scripts/install-server.sh --check

详细文档:docs/INSTALL.md / DEPLOYMENT.md / ADMIN.md / SECURITY.md
EOF
    ok "QUICKSTART.txt 已生成"
fi

step "5/7 tar.gz"
run "tar -czf '$DIST/$TARBALL' -C '$DIST' '$PACKAGE_NAME'"
run "rm -rf '$local_pkg'"
[[ "$DRY_RUN" -eq 0 ]] && ok "$(du -h "$DIST/$TARBALL" | awk '{print $1}')  $DIST/$TARBALL"

step "6/7 SHA256 + 拷脚本"
if [[ "$DRY_RUN" -eq 0 ]]; then
    (cd "$DIST" && shasum -a 256 "$TARBALL" > "$TARBALL.sha256")
    ok "$(cat "$DIST/$TARBALL.sha256")"
fi
run "cp '$ROOT/scripts/install-server.sh' '$DIST/install-server.sh'"
run "cp '$ROOT/scripts/uninstall-server.sh' '$DIST/uninstall-server.sh'"
run "chmod +x '$DIST/install-server.sh' '$DIST/uninstall-server.sh'"

step "7/7 RELEASE_MANIFEST.txt"
if [[ "$DRY_RUN" -eq 0 ]]; then
    git_hash=$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo "?")
    git_branch=$(git -C "$ROOT" rev-parse --abbrev-ref HEAD 2>/dev/null || echo "?")
    git_dirty=""
    [[ -n "$(git -C "$ROOT" status --porcelain 2>/dev/null)" ]] && git_dirty=" (DIRTY)"

    cat > "$DIST/RELEASE_MANIFEST.txt" <<EOF
cmem-server release manifest
═══════════════════════════════════════════════════════════════
package:       $PKG_NAME
version:       $PKG_VERSION
release tag:   $RELEASE_TAG
target:        $TARGET
build host:    $(uname -srm)
build time:    $(date -u +"%Y-%m-%dT%H:%M:%SZ")
git commit:    $git_hash on $git_branch$git_dirty
rust version:  $(rustc --version 2>/dev/null || echo "?")
═══════════════════════════════════════════════════════════════

文件:
  $TARBALL                    $(du -h "$DIST/$TARBALL" | awk '{print $1}')
  $TARBALL.sha256             $(cat "$DIST/$TARBALL.sha256" | awk '{print $1}')
  install-server.sh           $(du -h "$DIST/install-server.sh" | awk '{print $1}')
  uninstall-server.sh         $(du -h "$DIST/uninstall-server.sh" | awk '{print $1}')

校验:
  shasum -a 256 -c $TARBALL.sha256

部署(推荐):
  # 1. 在目标机器
  curl -fsSLO https://your-host/$TARBALL
  curl -fsSLO https://your-host/$TARBALL.sha256
  shasum -a 256 -c $TARBALL.sha256          # 一定要校验!
  tar -xzf $TARBALL
  cd $PACKAGE_NAME

  # 2. 用包内 install 脚本一键装
  sudo ./scripts/install-server.sh --bind 127.0.0.1:8080

  # 3. 健康检查
  sudo ./scripts/install-server.sh --check

或一行 curl 安装(install 脚本独立托管):
  curl -sSL https://your-host/install-server.sh \\
    | sudo bash -s -- --domain cmem.example.com
EOF
    ok "RELEASE_MANIFEST.txt"
fi

# ── 总结 ───────────────────────────────────────
echo
log "${BOLD}${GREEN}━━━ 打包完成 ━━━${RESET}"
log "  ${BOLD}dist/${RESET}"
if [[ "$DRY_RUN" -eq 0 ]]; then
    ls -lh "$DIST" | tail -n +2 | awk '{printf "    %s  %s\n", $5, $NF}'
    echo
    log "  下一步:"
    log "    cat $DIST/RELEASE_MANIFEST.txt"
    log "    上传 $TARBALL + sha256 到 GitHub release / 自己的 server"
    log
    log "  如要 cross-build Linux musl 静态二进制:"
    log "    bash $0 --target x86_64-unknown-linux-musl"
fi
