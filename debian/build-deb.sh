#!/bin/bash

set -e

set -o pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

show_help() {
    cat << EOF
用法: $0 [选项]

选项:
    -a, --arch ARCH       目标架构 (arm64, amd64, armhf, i386) [默认: 自动检测]
    -c, --clean           构建前清理target目录
    -d, --debug           使用debug模式编译
    -h, --help            显示此帮助信息
    -n, --no-lintian      跳过lintian检查
    -o, --output DIR      指定输出目录 [默认: debian/]
    -r, --release         使用release模式编译 [默认]
    -t, --target TARGET   Rust编译目标三元组 (如: aarch64-unknown-linux-gnu)
    -v, --version VERSION 指定包版本 [默认: 从Cargo.toml读取]
    --cross               启用交叉编译模式

示例:
    $0                          # 默认构建当前架构
    $0 -a arm64                 # 构建arm64包
    $0 -a amd64 --cross         # 交叉编译amd64包
    $0 -c -n                    # 清理构建并跳过lintian检查
EOF
}

detect_arch() {
    local arch
    arch=$(dpkg --print-architecture 2>/dev/null || uname -m)
    case "$arch" in
        x86_64|amd64)
            echo "amd64"
            ;;
        aarch64|arm64)
            echo "arm64"
            ;;
        armv7l|armhf)
            echo "armhf"
            ;;
        i686|i386)
            echo "i386"
            ;;
        *)
            echo "$arch"
            ;;
    esac
}

get_rust_target() {
    local arch="$1"
    case "$arch" in
        arm64)
            echo "aarch64-unknown-linux-gnu"
            ;;
        amd64)
            echo "x86_64-unknown-linux-gnu"
            ;;
        armhf)
            echo "armv7-unknown-linux-gnueabihf"
            ;;
        i386)
            echo "i686-unknown-linux-pc-windows-gnu"
            ;;
        *)
            echo ""
            ;;
    esac
}

get_version_from_cargo() {
    local cargo_toml="$1"
    if [ -f "$cargo_toml" ]; then
        grep -m1 '^version = ' "$cargo_toml" | sed 's/version = "\(.*\)"/\1/' | tr -d '"'
    else
        echo "0.0.0"
    fi
}

check_dependencies() {
    local missing=()
    
    local deps=("cargo" "dpkg-deb" "dpkg")
    for dep in "${deps[@]}"; do
        if ! command -v "$dep" &> /dev/null; then
            missing+=("$dep")
        fi
    done
    
    if [ ${#missing[@]} -gt 0 ]; then
        log_error "缺少必要的依赖: ${missing[*]}"
        log_info "请安装缺少的依赖后重试"
        exit 1
    fi
    
    if [ "$ENABLE_CROSS" = "true" ] && ! command -v cross &> /dev/null; then
        log_warning "未找到cross命令，正在尝试安装..."
        cargo install cross || {
            log_error "安装cross失败，请手动安装: cargo install cross"
            exit 1
        }
    fi
}

CLEAN_BUILD=false
DEBUG_MODE=false
SKIP_LINTIAN=false
ENABLE_CROSS=false
TARGET_ARCH=""
RUST_TARGET=""
OUTPUT_DIR=""
CUSTOM_VERSION=""

while [[ $# -gt 0 ]]; do
    case $1 in
        -a|--arch)
            TARGET_ARCH="$2"
            shift 2
            ;;
        -c|--clean)
            CLEAN_BUILD=true
            shift
            ;;
        -d|--debug)
            DEBUG_MODE=true
            shift
            ;;
        -h|--help)
            show_help
            exit 0
            ;;
        -n|--no-lintian)
            SKIP_LINTIAN=true
            shift
            ;;
        -o|--output)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        -r|--release)
            DEBUG_MODE=false
            shift
            ;;
        -t|--target)
            RUST_TARGET="$2"
            shift 2
            ;;
        -v|--version)
            CUSTOM_VERSION="$2"
            shift 2
            ;;
        --cross)
            ENABLE_CROSS=true
            shift
            ;;
        *)
            log_error "未知选项: $1"
            show_help
            exit 1
            ;;
    esac
done

if [ "$EUID" -ne 0 ]; then
    log_error "请使用root权限运行此脚本"
    echo "Usage: sudo $0"
    exit 1
fi

if [ -n "$SUDO_USER" ]; then
    USER_HOME=$(getent passwd "$SUDO_USER" | cut -d: -f6)
    USER_CARGO_BIN="$USER_HOME/.cargo/bin"
    if [ -d "$USER_CARGO_BIN" ]; then
        export PATH="$USER_CARGO_BIN:$PATH"
        export CARGO_HOME="$USER_HOME/.cargo"
        export RUSTUP_HOME="$USER_HOME/.rustup"
        log_info "已添加用户cargo路径到PATH: $USER_CARGO_BIN"
    fi
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BUILD_DIR="${SCRIPT_DIR}/build"
PACKAGE_NAME="wftpg"

if [ -n "$CUSTOM_VERSION" ]; then
    VERSION="$CUSTOM_VERSION"
else
    VERSION=$(get_version_from_cargo "${PROJECT_DIR}/Cargo.toml")
fi

if [ -z "$TARGET_ARCH" ]; then
    TARGET_ARCH=$(detect_arch)
fi

if [ -z "$RUST_TARGET" ]; then
    RUST_TARGET=$(get_rust_target "$TARGET_ARCH")
fi

if [ -z "$OUTPUT_DIR" ]; then
    OUTPUT_DIR="$SCRIPT_DIR"
fi

check_dependencies

echo "========================================"
echo "  WFTPG DEB包构建脚本 v2.0"
echo "========================================"
echo "包名称: ${PACKAGE_NAME}"
echo "版本: ${VERSION}"
echo "架构: ${TARGET_ARCH}"
echo "Rust目标: ${RUST_TARGET:-默认}"
echo "编译模式: $([ "$DEBUG_MODE" = true ] && echo "debug" || echo "release")"
echo "交叉编译: $([ "$ENABLE_CROSS" = true ] && echo "是" || echo "否")"
echo "项目目录: ${PROJECT_DIR}"
echo "构建目录: ${BUILD_DIR}"
echo "输出目录: ${OUTPUT_DIR}"
echo "========================================"

cd "${PROJECT_DIR}"

if [ "$CLEAN_BUILD" = true ]; then
    log_info "[1/9] 清理旧的构建文件..."
    rm -rf "${BUILD_DIR}"
    rm -rf "${PROJECT_DIR}/target"
fi

mkdir -p "${BUILD_DIR}"

log_info "[2/9] 编译项目..."
BUILD_MODE=$([ "$DEBUG_MODE" = true ] && echo "" || echo "--release")

if [ "$ENABLE_CROSS" = true ] && [ -n "$RUST_TARGET" ]; then
    log_info "使用cross进行交叉编译..."
    cross build $BUILD_MODE --target "$RUST_TARGET"
    TARGET_DIR="${PROJECT_DIR}/target/${RUST_TARGET}/$([ "$DEBUG_MODE" = true ] && echo "debug" || echo "release")"
else
    if [ -n "$RUST_TARGET" ]; then
        log_info "使用cargo编译目标: $RUST_TARGET"
        cargo build $BUILD_MODE --target "$RUST_TARGET"
        TARGET_DIR="${PROJECT_DIR}/target/${RUST_TARGET}/$([ "$DEBUG_MODE" = true ] && echo "debug" || echo "release")"
    else
        cargo build $BUILD_MODE
        TARGET_DIR="${PROJECT_DIR}/target/$([ "$DEBUG_MODE" = true ] && echo "debug" || echo "release")"
    fi
fi

if [ ! -f "${TARGET_DIR}/wftpg" ]; then
    log_error "编译失败: 未找到可执行文件 ${TARGET_DIR}/wftpg"
    exit 1
fi

log_success "找到已编译的可执行文件: ${TARGET_DIR}/wftpg"

log_info "[3/9] 创建DEB包目录结构..."
DEB_DIR="${BUILD_DIR}/${PACKAGE_NAME}_${VERSION}_${TARGET_ARCH}"
mkdir -p "${DEB_DIR}/DEBIAN"
mkdir -p "${DEB_DIR}/usr/bin"
mkdir -p "${DEB_DIR}/usr/share/applications"
mkdir -p "${DEB_DIR}/usr/share/icons/hicolor/256x256/apps"
mkdir -p "${DEB_DIR}/usr/share/icons/hicolor/scalable/apps"
mkdir -p "${DEB_DIR}/usr/share/icons/hicolor/48x48/apps"
mkdir -p "${DEB_DIR}/usr/share/polkit-1/actions"
mkdir -p "${DEB_DIR}/lib/systemd/system"
mkdir -p "${DEB_DIR}/usr/share/doc/${PACKAGE_NAME}"
mkdir -p "${DEB_DIR}/etc/wftpg"
mkdir -p "${DEB_DIR}/var/log/wftpg"
mkdir -p "${DEB_DIR}/usr/share/${PACKAGE_NAME}"

log_info "[4/9] 复制可执行文件..."

if [ -f "${TARGET_DIR}/wftp-gui" ]; then
    cp "${TARGET_DIR}/wftp-gui" "${DEB_DIR}/usr/bin/"
    chmod 755 "${DEB_DIR}/usr/bin/wftp-gui"
    chown root:root "${DEB_DIR}/usr/bin/wftp-gui"
    log_info "  已复制: wftp-gui (GUI管理程序)"
fi

if [ -f "${TARGET_DIR}/wftpd" ]; then
    cp "${TARGET_DIR}/wftpd" "${DEB_DIR}/usr/bin/"
    chmod 755 "${DEB_DIR}/usr/bin/wftpd"
    chown root:root "${DEB_DIR}/usr/bin/wftpd"
    log_info "  已复制: wftpd (后台服务程序)"
fi

log_info "[5/9] 复制桌面文件..."
if [ -f "${SCRIPT_DIR}/wftpg.desktop" ]; then
    cp "${SCRIPT_DIR}/wftpg.desktop" "${DEB_DIR}/usr/share/applications/"
    chmod 644 "${DEB_DIR}/usr/share/applications/wftpg.desktop"
    log_info "  已复制: wftpg.desktop"
fi

log_info "[6/9] 创建图标文件..."
mkdir -p "${DEB_DIR}/usr/share/pixmaps"
mkdir -p "${DEB_DIR}/usr/share/icons/hicolor/scalable/apps"

log_info "  生成SVG图标..."
cat > "${DEB_DIR}/usr/share/icons/hicolor/scalable/apps/wftpg.svg" << 'EOF'
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <defs>
    <linearGradient id="grad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" style="stop-color:#4A90E2;stop-opacity:1" />
      <stop offset="100%" style="stop-color:#357ABD;stop-opacity:1" />
    </linearGradient>
  </defs>
  <rect x="20" y="20" width="216" height="216" rx="20" ry="20" fill="url(#grad)"/>
  <text x="128" y="110" font-family="Arial, sans-serif" font-size="50" font-weight="bold" fill="white" text-anchor="middle">WFTPG</text>
  <text x="128" y="160" font-family="Arial, sans-serif" font-size="30" fill="white" text-anchor="middle">Server</text>
</svg>
EOF
chmod 644 "${DEB_DIR}/usr/share/icons/hicolor/scalable/apps/wftpg.svg"
cp "${DEB_DIR}/usr/share/icons/hicolor/scalable/apps/wftpg.svg" "${DEB_DIR}/usr/share/pixmaps/wftpg.svg"
chmod 644 "${DEB_DIR}/usr/share/pixmaps/wftpg.svg"

if command -v convert &> /dev/null; then
    log_info "  生成PNG图标 (使用ImageMagick)..."
    convert -background none -size 256x256 "${DEB_DIR}/usr/share/icons/hicolor/scalable/apps/wftpg.svg" \
        "${DEB_DIR}/usr/share/icons/hicolor/256x256/apps/wftpg.png" 2>/dev/null || \
        log_warning "    PNG 256x256生成失败"
    
    if [ -f "${DEB_DIR}/usr/share/icons/hicolor/256x256/apps/wftpg.png" ]; then
        chmod 644 "${DEB_DIR}/usr/share/icons/hicolor/256x256/apps/wftpg.png"
        convert -background none -size 48x48 "${DEB_DIR}/usr/share/icons/hicolor/scalable/apps/wftpg.svg" \
            "${DEB_DIR}/usr/share/icons/hicolor/48x48/apps/wftpg.png" 2>/dev/null || \
            log_warning "    PNG 48x48生成失败"
        
        if [ -f "${DEB_DIR}/usr/share/icons/hicolor/48x48/apps/wftpg.png" ]; then
            chmod 644 "${DEB_DIR}/usr/share/icons/hicolor/48x48/apps/wftpg.png"
        fi
        
        cp "${DEB_DIR}/usr/share/icons/hicolor/256x256/apps/wftpg.png" "${DEB_DIR}/usr/share/pixmaps/wftpg.png"
        chmod 644 "${DEB_DIR}/usr/share/pixmaps/wftpg.png"
    fi
else
    log_warning "  未安装ImageMagick，跳过PNG图标生成"
fi

log_info "[7/9] 复制PolicyKit和Systemd配置..."
if [ -f "${SCRIPT_DIR}/com.wftpg.pkexec.policy" ]; then
    cp "${SCRIPT_DIR}/com.wftpg.pkexec.policy" "${DEB_DIR}/usr/share/polkit-1/actions/"
    chmod 644 "${DEB_DIR}/usr/share/polkit-1/actions/com.wftpg.pkexec.policy"
    log_info "  已复制: com.wftpg.pkexec.policy"
fi

if [ -f "${SCRIPT_DIR}/wftpd.service" ]; then
    cp "${SCRIPT_DIR}/wftpd.service" "${DEB_DIR}/lib/systemd/system/"
    chmod 644 "${DEB_DIR}/lib/systemd/system/wftpd.service"
    log_info "  已复制: wftpd.service"
fi

log_info "[8/9] 创建配置文件模板..."
cat > "${DEB_DIR}/etc/wftpg/config.toml.example" << 'EOF'
# WFTPG 配置文件
# 复制此文件到 /etc/wftpg/config.toml 进行自定义配置

[server]
bind_ip = "0.0.0.0"
ftp_port = 21
sftp_port = 22
max_connections = 100
connection_timeout = 300
idle_timeout = 600

[ftp]
enabled = true
default_home = "/home/user/Desktop/共享"
passive_ports = [50000, 51000]
welcome_message = "Welcome to WFTPG FTP Server"
allow_anonymous = false
max_speed_kbps = 0
encoding = "UTF-8"

[sftp]
enabled = true
default_home = "/home/user/Desktop/共享"
host_key_path = "/var/lib/wftpg/ssh/ssh_host_rsa_key"
max_auth_attempts = 3
auth_timeout = 60
log_level = "info"

[security]
allowed_ips = ["0.0.0.0/0"]
denied_ips = []
max_login_attempts = 5
ban_duration = 300
require_ssl = false

[logging]
log_dir = "/var/log/wftpg"
log_level = "info"
max_log_size = 10485760
max_log_files = 10
log_to_file = true
log_to_gui = true
EOF
chmod 644 "${DEB_DIR}/etc/wftpg/config.toml.example"

log_info "[9/9] 创建DEBIAN控制文件..."

cat > "${DEB_DIR}/DEBIAN/control" << EOF
Package: ${PACKAGE_NAME}
Version: ${VERSION}
Section: net
Priority: optional
Architecture: ${TARGET_ARCH}
Maintainer: WFTPG Developer <developer@wftpg.com>
Description: SFTP/FTP GUI Management Tool
 SFTP+FTP GUI Management Tool for Linux.
 A pure Rust + GTK implementation for managing
 SFTP and FTP servers with a user-friendly interface.
Description-zh_CN: SFTP/FTP图形化管理工具
 适用于Linux的SFTP+FTP图形化管理工具。
 使用纯Rust + GTK实现，提供友好的用户界面来管理
 SFTP和FTP服务器。
Homepage: https://github.com/wftpg/wftpg
Depends: libgtk-3-0, libc6, policykit-1
Recommends: openssh-server
Suggests: proftpd-basic
Installed-Size: $(du -sk "${DEB_DIR}" | cut -f1)
EOF

if [ -f "${SCRIPT_DIR}/preinst" ]; then
    cp "${SCRIPT_DIR}/preinst" "${DEB_DIR}/DEBIAN/"
    chmod 755 "${DEB_DIR}/DEBIAN/preinst"
fi

if [ -f "${SCRIPT_DIR}/postinst" ]; then
    cp "${SCRIPT_DIR}/postinst" "${DEB_DIR}/DEBIAN/"
    chmod 755 "${DEB_DIR}/DEBIAN/postinst"
fi

if [ -f "${SCRIPT_DIR}/prerm" ]; then
    cp "${SCRIPT_DIR}/prerm" "${DEB_DIR}/DEBIAN/"
    chmod 755 "${DEB_DIR}/DEBIAN/prerm"
fi

if [ -f "${SCRIPT_DIR}/postrm" ]; then
    cp "${SCRIPT_DIR}/postrm" "${DEB_DIR}/DEBIAN/"
    chmod 755 "${DEB_DIR}/DEBIAN/postrm"
fi

log_info "创建changelog文件..."
CHANGELOG_DATE=$(date -R)
cat > "${DEB_DIR}/usr/share/doc/${PACKAGE_NAME}/changelog" << EOF
${PACKAGE_NAME} (${VERSION}) stable; urgency=medium

  * Release version ${VERSION}
  * Support SFTP and FTP server management
  * GTK3 GUI interface
  * PolicyKit integration for root privileges
  * Multi-architecture support

 -- WFTPG Developer <developer@wftpg.com>  ${CHANGELOG_DATE}
EOF
gzip -9 "${DEB_DIR}/usr/share/doc/${PACKAGE_NAME}/changelog"

log_info "创建copyright文件..."
cat > "${DEB_DIR}/usr/share/doc/${PACKAGE_NAME}/copyright" << 'EOF'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: wftpg
Source: https://github.com/wftpg/wftpg

Files: *
Copyright: 2024-2025 WFTPG Developer <developer@wftpg.com>
License: MIT

License: MIT
 Permission is hereby granted, free of charge, to any person obtaining a copy
 of this software and associated documentation files (the "Software"), to deal
 in the Software without restriction, including without limitation the rights
 to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 copies of the Software, and to permit persons to whom the Software is
 furnished to do so, subject to the following conditions:
 .
 The above copyright notice and this permission notice shall be included in all
 copies or substantial portions of the Software.
 .
 THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 SOFTWARE.
EOF

log_info "创建conffiles文件..."
cat > "${DEB_DIR}/DEBIAN/conffiles" << EOF
/etc/wftpg/config.toml.example
EOF

log_info "设置目录权限..."
chown -R root:root "${DEB_DIR}"
chmod -R 755 "${DEB_DIR}/usr/bin"
chmod -R 755 "${DEB_DIR}/etc/wftpg"
chmod -R 755 "${DEB_DIR}/var/log/wftpg"
chmod 644 "${DEB_DIR}/etc/wftpg/config.toml.example"

log_info "构建DEB包..."
mkdir -p "${OUTPUT_DIR}"
cd "${BUILD_DIR}"
dpkg-deb --build "${PACKAGE_NAME}_${VERSION}_${TARGET_ARCH}" "${OUTPUT_DIR}/${PACKAGE_NAME}_${VERSION}_${TARGET_ARCH}.deb"

DEB_FILE="${OUTPUT_DIR}/${PACKAGE_NAME}_${VERSION}_${TARGET_ARCH}.deb"

if [ -f "$DEB_FILE" ]; then
    if [ "$SKIP_LINTIAN" = false ] && command -v lintian &> /dev/null; then
        log_info "运行lintian检查..."
        lintian "$DEB_FILE" || log_warning "lintian检查发现问题，请检查上述输出"
    fi
    
    log_success "构建成功!"
    echo ""
    echo "========================================"
    echo "  构建完成"
    echo "========================================"
    echo "DEB包位置: ${DEB_FILE}"
    echo "包大小: $(du -h "$DEB_FILE" | cut -f1)"
    echo ""
    echo "安装方法:"
    echo "  sudo dpkg -i ${DEB_FILE}"
    echo ""
    echo "或者:"
    echo "  sudo apt install ${DEB_FILE}"
    echo ""
    echo "卸载方法:"
    echo "  sudo apt remove ${PACKAGE_NAME}"
    echo ""
    echo "查看包信息:"
    echo "  dpkg -I ${DEB_FILE}"
    echo ""
    echo "查看包内容:"
    echo "  dpkg -c ${DEB_FILE}"
    echo "========================================"
    
    rm -rf "${BUILD_DIR}/${PACKAGE_NAME}_${VERSION}_${TARGET_ARCH}/"
    
    exit 0
else
    log_error "DEB包构建失败"
    exit 1
fi
