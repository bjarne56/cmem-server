#!/usr/bin/env bash
# 简单 release 流程:bump 版本 → tag → 跨平台 build → 生成 SHA256SUMS。
#
# 用法:
#   ./scripts/release.sh 0.2.0
#
# 真正的二进制分发交给 GitHub Actions 的 release.yml 跑(待办)。
# 这个脚本主要用来本地验证 + 手动产物。

set -uo pipefail

if [[ -t 1 ]]; then
    GREEN=$'\033[32m'; RED=$'\033[31m'; BLUE=$'\033[34m'; YELLOW=$'\033[33m'; RESET=$'\033[0m'
else
    GREEN=""; RED=""; BLUE=""; YELLOW=""; RESET=""
fi
ok()   { echo -e "  ${GREEN}OK${RESET} $*"; }
fail() { echo -e "  ${RED}FAIL${RESET} $*"; exit 1; }
info() { echo -e "  ${BLUE}--${RESET} $*"; }
step() { echo; echo -e "${BLUE}>>> $*${RESET}"; }

VERSION="${1:-}"
[[ -z "$VERSION" ]] && fail "用法:$0 <NEW_VERSION>(例如 0.2.0)"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-z0-9.]+)?$ ]] \
    || fail "版本号要 semver:major.minor.patch[-pre]"

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

step "git 工作区干净检查"
[[ -z "$(git status --porcelain)" ]] || fail "工作区有改动,先 commit / stash"
ok "干净"

step "bump 版本到 $VERSION"
sed -i.bak "s/^version = .*/version = \"$VERSION\"/" Cargo.toml \
    crates/shared/Cargo.toml crates/server/Cargo.toml 2>/dev/null || true
rm -f Cargo.toml.bak crates/*/Cargo.toml.bak
cargo update --workspace || fail "cargo update 失败"
ok "Cargo.toml + Cargo.lock 已更新"

step "本机 release build"
cargo build --release --bin cmem-server || fail "build 失败"
ok "$(target/release/cmem-server --version)"

step "跨平台 build(可选,需要 cross)"
if command -v cross >/dev/null 2>&1; then
    for tgt in x86_64-unknown-linux-musl aarch64-unknown-linux-musl; do
        info "→ $tgt"
        cross build --release --bin cmem-server --target "$tgt" \
            && cp "target/$tgt/release/cmem-server" "target/cmem-server-$tgt"
    done
else
    info "未装 cross,跳过 Linux 跨平台 build(GitHub Actions 会做)"
fi

case "$(uname -s)-$(uname -m)" in
    Darwin-arm64)
        cp target/release/cmem-server target/cmem-server-aarch64-apple-darwin ;;
    Darwin-x86_64)
        cp target/release/cmem-server target/cmem-server-x86_64-apple-darwin ;;
    Linux-x86_64)
        cp target/release/cmem-server target/cmem-server-x86_64-unknown-linux-gnu ;;
esac

step "生成 SHA256SUMS"
( cd target && sha256sum cmem-server-* 2>/dev/null > SHA256SUMS \
    || shasum -a 256 cmem-server-* > SHA256SUMS )
cat target/SHA256SUMS
ok "校验和写入 target/SHA256SUMS"

step "git commit + tag"
git add Cargo.toml Cargo.lock crates/*/Cargo.toml
git commit -m "chore: release v$VERSION"
git tag -s "v$VERSION" -m "v$VERSION" 2>/dev/null \
    || git tag -a "v$VERSION" -m "v$VERSION"
ok "tagged v$VERSION(本地)"

echo
echo "${YELLOW}下一步:${RESET}"
echo "  git push origin main"
echo "  git push origin v$VERSION"
echo "  GitHub Actions 会自动触发 release workflow,把二进制 + SHA256SUMS 上传到 GitHub Release"
