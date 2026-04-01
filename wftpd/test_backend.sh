#!/bin/bash
# WFTPD 后端服务测试脚本

set -e

echo "======================================"
echo "WFTPD 后端服务测试"
echo "======================================"

# 检查二进制文件是否存在
if [ ! -f "./target/release/wftpd" ]; then
    echo "❌ wftpd 二进制文件不存在，请先编译"
    exit 1
fi

echo "✓ wftpd 二进制文件存在"

# 显示版本信息
echo ""
echo "程序信息:"
file ./target/release/wftpd

# 检查配置文件
echo ""
echo "检查配置文件..."
if [ -f "/etc/wftpg/config.toml" ]; then
    echo "✓ 配置文件已存在"
    echo "配置文件内容预览:"
    head -20 /etc/wftpg/config.toml
else
    echo "⚠ 配置文件不存在，首次运行将创建默认配置"
fi

# 检查用户配置
echo ""
echo "检查用户配置..."
if [ -f "/etc/wftpg/users.json" ]; then
    echo "✓ 用户配置已存在"
else
    echo "⚠ 用户配置不存在，首次运行将创建默认配置"
fi

# 提示运行命令
echo ""
echo "======================================"
echo "运行命令:"
echo "  sudo ./target/release/wftpd"
echo ""
echo "或者使用 systemd 服务:"
echo "  sudo systemctl start wftpd"
echo "======================================"

# 测试 FTP 连接（如果服务正在运行）
echo ""
echo "测试 FTP 连接 (需要服务已启动)..."
if command -v curl &> /dev/null; then
    timeout 2 curl -s ftp://localhost:2121/ 2>&1 | head -5 || echo "FTP 服务未运行或无法连接"
else
    echo "curl 未安装，跳过 FTP 测试"
fi

# 测试 SFTP 连接（如果服务正在运行）
echo ""
echo "测试 SFTP 连接 (需要服务已启动)..."
if command -v ssh &> /dev/null; then
    timeout 2 ssh -p 2222 -o ConnectTimeout=1 -o StrictHostKeyChecking=no localhost exit 2>&1 | head -3 || echo "SFTP 服务未运行或无法连接"
else
    echo "ssh 未安装，跳过 SFTP 测试"
fi

echo ""
echo "======================================"
echo "测试完成!"
echo "======================================"
