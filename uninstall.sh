#!/bin/bash
set -e

echo "=== WFTPG 卸载脚本 ==="

if [ "$EUID" -ne 0 ]; then
    echo "请使用 sudo 运行此脚本"
    exit 1
fi

echo "1. 停止服务..."
systemctl stop wftpg 2>/dev/null || true
systemctl disable wftpg 2>/dev/null || true

echo "2. 删除服务文件..."
rm -f /etc/systemd/system/wftpg.service
systemctl daemon-reload

echo "3. 删除可执行文件..."
rm -f /usr/bin/wftpg-service
rm -f /usr/bin/wftpg-gui
rm -f /usr/lib/libwftpg.so
rm -f /usr/lib/libwftpg.dylib

echo "4. 删除桌面快捷方式..."
rm -f /usr/share/applications/wftpg.desktop

echo "5. 是否删除配置和数据文件? [y/N]"
read -r answer
if [ "$answer" = "y" ] || [ "$answer" = "Y" ]; then
    rm -rf /etc/wftpg
    rm -rf /var/log/wftpg
    echo "配置和数据已删除"
else
    echo "保留配置和数据文件"
fi

echo ""
echo "=== 卸载完成 ==="
