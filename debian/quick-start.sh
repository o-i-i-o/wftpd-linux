#!/bin/bash

echo "========================================"
echo "WFTPG 快速打包指南"
echo "========================================"
echo ""

echo "步骤1: 确保已编译程序"
echo "  cargo build --release"
echo ""

echo "步骤2: 运行打包脚本（需要root权限）"
echo "  cd debian"
echo "  sudo ./build-deb.sh"
echo ""

echo "步骤3: 安装生成的DEB包"
echo "  sudo dpkg -i build/wftpg_2.0.0_arm64.deb"
echo ""

echo "步骤4: 从应用菜单启动"
echo "  在应用启动器中搜索 'WFTPG'"
echo ""

echo "========================================"
echo "详细说明请查看: debian/README.md"
echo "========================================"
