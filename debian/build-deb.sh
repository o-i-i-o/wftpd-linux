#!/bin/bash

set -e

if [ "$EUID" -ne 0 ]; then
    echo "请使用root权限运行此脚本"
    echo "Usage: sudo $0"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BUILD_DIR="${SCRIPT_DIR}/build"
PACKAGE_NAME="wftpg"
VERSION="2.0.0"
ARCH="arm64"

echo "========================================"
echo "WFTPG DEB包构建脚本"
echo "========================================"
echo "包名称: ${PACKAGE_NAME}"
echo "版本: ${VERSION}"
echo "架构: ${ARCH}"
echo "项目目录: ${PROJECT_DIR}"
echo "构建目录: ${BUILD_DIR}"
echo "========================================"

cd "${PROJECT_DIR}"

echo "[1/8] 清理旧的构建文件..."
rm -rf "${BUILD_DIR}"
mkdir -p "${BUILD_DIR}"

echo "[2/8] 检查编译文件..."
if [ ! -f "${PROJECT_DIR}/target/release/wftpg" ]; then
    echo "错误: 未找到编译后的可执行文件"
    echo "请先在用户环境下运行: cargo build --release"
    echo "然后再使用sudo运行此脚本"
    exit 1
fi

echo "找到已编译的可执行文件: target/release/wftpg"

echo "[3/8] 创建DEB包目录结构..."
DEB_DIR="${BUILD_DIR}/${PACKAGE_NAME}_${VERSION}_${ARCH}"
mkdir -p "${DEB_DIR}/DEBIAN"
mkdir -p "${DEB_DIR}/usr/bin"
mkdir -p "${DEB_DIR}/usr/share/applications"
mkdir -p "${DEB_DIR}/usr/share/icons/hicolor/256x256/apps"
mkdir -p "${DEB_DIR}/usr/share/icons/hicolor/scalable/apps"
mkdir -p "${DEB_DIR}/usr/share/polkit-1/actions"
mkdir -p "${DEB_DIR}/usr/share/dbus-1/system-services"
mkdir -p "${DEB_DIR}/usr/share/dbus-1/system.d"
mkdir -p "${DEB_DIR}/lib/systemd/system"
mkdir -p "${DEB_DIR}/usr/share/doc/${PACKAGE_NAME}"
mkdir -p "${DEB_DIR}/etc/wftpg"
mkdir -p "${DEB_DIR}/var/log/wftpg"

echo "[4/8] 复制可执行文件..."
cp "${PROJECT_DIR}/target/release/wftpg" "${DEB_DIR}/usr/bin/"
chmod 755 "${DEB_DIR}/usr/bin/wftpg"
chown root:root "${DEB_DIR}/usr/bin/wftpg"

echo "[5/8] 复制桌面文件..."
if [ -f "${PROJECT_DIR}/wftpg.desktop" ]; then
    cp "${PROJECT_DIR}/wftpg.desktop" "${DEB_DIR}/usr/share/applications/"
    chmod 644 "${DEB_DIR}/usr/share/applications/wftpg.desktop"
fi

echo "[6/8] 创建图标文件..."
if [ -d "${PROJECT_DIR}/ui" ]; then
    ICON_FOUND=0
    for icon_file in wftpg.png wftpg.svg icon.png icon.svg; do
        if [ -f "${PROJECT_DIR}/ui/${icon_file}" ]; then
            if [[ "${icon_file}" == *.png ]]; then
                cp "${PROJECT_DIR}/ui/${icon_file}" "${DEB_DIR}/usr/share/icons/hicolor/256x256/apps/wftpg.png"
                chmod 644 "${DEB_DIR}/usr/share/icons/hicolor/256x256/apps/wftpg.png"
            elif [[ "${icon_file}" == *.svg ]]; then
                mkdir -p "${DEB_DIR}/usr/share/icons/hicolor/scalable/apps"
                cp "${PROJECT_DIR}/ui/${icon_file}" "${DEB_DIR}/usr/share/icons/hicolor/scalable/apps/wftpg.svg"
                chmod 644 "${DEB_DIR}/usr/share/icons/hicolor/scalable/apps/wftpg.svg"
            fi
            ICON_FOUND=1
            echo "  使用图标: ${icon_file}"
            break
        fi
    done
    
    if [ ${ICON_FOUND} -eq 0 ]; then
        echo "警告: 未找到图标文件，创建默认SVG图标..."
        mkdir -p "${DEB_DIR}/usr/share/icons/hicolor/scalable/apps"
        cat > "${DEB_DIR}/usr/share/icons/hicolor/scalable/apps/wftpg.svg" << 'EOF'
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <defs>
    <linearGradient id="grad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" style="stop-color:#4A90E2;stop-opacity:1" />
      <stop offset="100%" style="stop-color:#357ABD;stop-opacity:1" />
    </linearGradient>
  </defs>
  <rect x="20" y="20" width="216" height="216" rx="20" ry="20" fill="url(#grad)"/>
  <text x="128" y="110" font-family="Arial, sans-serif" font-size="60" font-weight="bold" fill="white" text-anchor="middle">FTP</text>
  <text x="128" y="160" font-family="Arial, sans-serif" font-size="30" fill="white" text-anchor="middle">Server</text>
</svg>
EOF
        chmod 644 "${DEB_DIR}/usr/share/icons/hicolor/scalable/apps/wftpg.svg"
    fi
else
    echo "警告: ui目录不存在，创建默认SVG图标..."
    mkdir -p "${DEB_DIR}/usr/share/icons/hicolor/scalable/apps"
    cat > "${DEB_DIR}/usr/share/icons/hicolor/scalable/apps/wftpg.svg" << 'EOF'
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <defs>
    <linearGradient id="grad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" style="stop-color:#4A90E2;stop-opacity:1" />
      <stop offset="100%" style="stop-color:#357ABD;stop-opacity:1" />
    </linearGradient>
  </defs>
  <rect x="20" y="20" width="216" height="216" rx="20" ry="20" fill="url(#grad)"/>
  <text x="128" y="110" font-family="Arial, sans-serif" font-size="60" font-weight="bold" fill="white" text-anchor="middle">FTP</text>
  <text x="128" y="160" font-family="Arial, sans-serif" font-size="30" fill="white" text-anchor="middle">Server</text>
</svg>
EOF
    chmod 644 "${DEB_DIR}/usr/share/icons/hicolor/scalable/apps/wftpg.svg"
fi

echo "[7/8] 复制PolicyKit和DBus配置..."
if [ -f "${PROJECT_DIR}/debian/com.wftpg.pkexec.policy" ]; then
    cp "${PROJECT_DIR}/debian/com.wftpg.pkexec.policy" "${DEB_DIR}/usr/share/polkit-1/actions/"
    chmod 644 "${DEB_DIR}/usr/share/polkit-1/actions/com.wftpg.pkexec.policy"
fi

if [ -f "${PROJECT_DIR}/debian/com.wftpg.Server.service" ]; then
    cp "${PROJECT_DIR}/debian/com.wftpg.Server.service" "${DEB_DIR}/usr/share/dbus-1/system-services/"
    chmod 644 "${DEB_DIR}/usr/share/dbus-1/system-services/com.wftpg.Server.service"
fi

if [ -f "${PROJECT_DIR}/debian/wftpg.service" ]; then
    cp "${PROJECT_DIR}/debian/wftpg.service" "${DEB_DIR}/lib/systemd/system/"
    chmod 644 "${DEB_DIR}/lib/systemd/system/wftpg.service"
fi

if [ -f "${PROJECT_DIR}/debian/wftpg-launcher" ]; then
    cp "${PROJECT_DIR}/debian/wftpg-launcher" "${DEB_DIR}/usr/bin/"
    chmod 755 "${DEB_DIR}/usr/bin/wftpg-launcher"
fi

echo "[8/8] 创建DEBIAN控制文件..."
if [ -f "${PROJECT_DIR}/debian/control" ]; then
    cp "${PROJECT_DIR}/debian/control" "${DEB_DIR}/DEBIAN/"
fi

if [ -f "${PROJECT_DIR}/debian/postinst" ]; then
    cp "${PROJECT_DIR}/debian/postinst" "${DEB_DIR}/DEBIAN/"
    chmod 755 "${DEB_DIR}/DEBIAN/postinst"
fi

if [ -f "${PROJECT_DIR}/debian/prerm" ]; then
    cp "${PROJECT_DIR}/debian/prerm" "${DEB_DIR}/DEBIAN/"
    chmod 755 "${DEB_DIR}/DEBIAN/prerm"
fi

if [ -f "${PROJECT_DIR}/debian/postrm" ]; then
    cp "${PROJECT_DIR}/debian/postrm" "${DEB_DIR}/DEBIAN/"
    chmod 755 "${DEB_DIR}/DEBIAN/postrm"
fi

echo "创建changelog文件..."
cat > "${DEB_DIR}/usr/share/doc/${PACKAGE_NAME}/changelog" << 'EOF'
wftpg (2.0.0) stable; urgency=medium

  * Initial release for UOS/Deepin
  * Support SFTP and FTP server management
  * GTK3 GUI interface
  * PolicyKit integration for root privileges

 -- WFTPG Developer <developer@wftpg.com>  $(date -R)
EOF
gzip -9 "${DEB_DIR}/usr/share/doc/${PACKAGE_NAME}/changelog"

echo "创建copyright文件..."
cat > "${DEB_DIR}/usr/share/doc/${PACKAGE_NAME}/copyright" << 'EOF'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: wftpg
Source: https://github.com/wftpg/wftpg

Files: *
Copyright: 2024 WFTPG Developer <developer@wftpg.com>
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

echo "设置目录权限..."
chown -R root:root "${DEB_DIR}"
chmod -R 755 "${DEB_DIR}/usr/bin"
chmod -R 755 "${DEB_DIR}/etc/wftpg"
chmod -R 755 "${DEB_DIR}/var/log/wftpg"

echo "构建DEB包..."
cd "${BUILD_DIR}"
dpkg-deb --build "${PACKAGE_NAME}_${VERSION}_${ARCH}"

if [ -f "${BUILD_DIR}/${PACKAGE_NAME}_${VERSION}_${ARCH}.deb" ]; then
    echo ""
    echo "========================================"
    echo "构建成功!"
    echo "========================================"
    echo "DEB包位置: ${BUILD_DIR}/${PACKAGE_NAME}_${VERSION}_${ARCH}.deb"
    echo ""
    echo "安装方法:"
    echo "  sudo dpkg -i ${BUILD_DIR}/${PACKAGE_NAME}_${VERSION}_${ARCH}.deb"
    echo ""
    echo "或者:"
    echo "  sudo apt install ${BUILD_DIR}/${PACKAGE_NAME}_${VERSION}_${ARCH}.deb"
    echo ""
    echo "卸载方法:"
    echo "  sudo apt remove ${PACKAGE_NAME}"
    echo "========================================"
    rm -rf "${BUILD_DIR}/${PACKAGE_NAME}_${VERSION}_${ARCH}/"
    ls -lh "${BUILD_DIR}/${PACKAGE_NAME}_${VERSION}_${ARCH}.deb"
else
    echo "错误: DEB包构建失败"
    exit 1
fi
