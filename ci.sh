#!/bin/bash
# WFTPD-Linux CI 脚本：格式检查、静态检查、测试与构建
#
# 用法:
#   ./ci.sh            完整流水线（含 release 构建）
#   ./ci.sh --fast     跳过 release 构建，仅做快速验证
#
# 系统依赖（Debian/Ubuntu）:
#   sudo apt install rustc cargo libgtk-3-dev pkg-config
#   rustup component add rustfmt clippy

set -euo pipefail

cd "$(dirname "$0")"

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info()    { echo -e "${BLUE}[CI][INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[CI][PASS]${NC} $1"; }
log_error()   { echo -e "${RED}[CI][FAIL]${NC} $1" >&2; }

FAST=0
if [[ "${1:-}" == "--fast" ]]; then
    FAST=1
elif [[ $# -gt 0 ]]; then
    echo "用法: $0 [--fast]" >&2
    exit 1
fi

step() { echo; log_info "$1"; }

# ---- 环境检查 ----
step "检查构建环境"

for cmd in cargo rustfmt cargo-clippy pkg-config; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        log_error "未找到 $cmd；请安装 Rust 工具链并执行 rustup component add rustfmt clippy"
        exit 1
    fi
done

if ! pkg-config --exists gtk+-3.0; then
    log_error "未找到 gtk+-3.0 开发库（wftpg 编译需要）；执行: sudo apt install libgtk-3-dev pkg-config"
    exit 1
fi
log_success "构建环境就绪"

# ---- 1. 格式检查 ----
step "cargo fmt 检查"
if cargo fmt --all -- --check; then
    log_success "代码格式检查通过"
else
    log_error "代码格式不符合 rustfmt 规范；执行 cargo fmt --all 后重试"
    exit 1
fi

# ---- 2. 静态检查（告警视为错误，项目规则禁止隐藏告警） ----
step "cargo clippy 检查（-D warnings）"
if cargo clippy --workspace --all-targets -- -D warnings; then
    log_success "clippy 检查通过，无任何告警"
else
    log_error "clippy 发现问题；修复所有警告后重试（禁止使用 #[allow] 隐藏）"
    exit 1
fi

# ---- 3. 测试 ----
step "cargo test 测试"
if cargo test --workspace; then
    log_success "全部测试通过"
else
    log_error "存在失败的测试"
    exit 1
fi

# ---- 4. 构建 ----
if [[ $FAST -eq 1 ]]; then
    step "cargo build 构建（debug，--fast 模式）"
    BUILD_ARGS=(--workspace)
else
    step "cargo build 构建（release）"
    BUILD_ARGS=(--workspace --release)
fi
if cargo build "${BUILD_ARGS[@]}"; then
    log_success "构建成功"
else
    log_error "构建失败"
    exit 1
fi

echo
log_success "CI 全部通过 ✓"
