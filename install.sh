#!/bin/bash
set -e

echo "=== WFTPG 安装脚本 ==="

if [ "$EUID" -ne 0 ]; then
    echo "请使用 sudo 运行此脚本"
    exit 1
fi

cd "$(dirname "$0")"

echo "1. 创建目录..."
mkdir -p /etc/wftpg
mkdir -p /var/log/wftpg
mkdir -p /usr/share/wftpg

echo "2. 安装可执行文件..."
cp target/release/wftpg-service /usr/bin/wftpg-service
cp gui/wftpg-gui /usr/bin/wftpg-gui
cp target/release/libwftpg.so /usr/lib/ 2>/dev/null || cp target/release/libwftpg.dylib /usr/lib/ 2>/dev/null || true

echo "3. 安装桌面快捷方式..."
cat > /usr/share/applications/wftpg.desktop << 'EOF'
[Desktop Entry]
Version=1.0
Name=WFTPG
Name[zh_CN]=WFTPG 文件传输服务管理
Comment=SFTP/FTP Server Management Tool
Comment[zh_CN]=SFTP/FTP 服务器管理工具
Exec=/usr/bin/wftpg-gui
Icon=folder-remote
Terminal=false
Type=Application
Categories=Network;FileTransfer;System;
EOF

echo "4. 设置权限..."
chmod +x /usr/bin/wftpg-service
chmod +x /usr/bin/wftpg-gui
chmod 755 /etc/wftpg
chmod 755 /var/log/wftpg

echo ""
echo "=== 安装完成 ==="
echo ""
echo "使用方法:"
echo "  1. 运行GUI: wftpg-gui"
echo "  2. 在GUI中可以安装和管理系统服务"
echo "  3. 配置文件: /etc/wftpg/config.toml"
echo "  4. 用户数据: /etc/wftpg/users.json"
echo "  5. 日志目录: /var/log/wftpg/"
