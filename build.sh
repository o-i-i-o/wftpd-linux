#!/bin/bash

set -e

echo "=== WFTPG 构建脚本 ==="

cd "$(dirname "$0")"

echo "[1/3] 构建 Rust 核心库..."
source "$HOME/.cargo/env"
cargo build --release

echo "[2/3] 构建 Qt5 GUI..."
cd gui
qmake
make -j$(nproc)

echo "[3/3] 安装..."
sudo cp ../target/release/wftpg /usr/local/bin/
sudo cp ../target/release/wftpg-service /usr/local/bin/
sudo cp wftpg-gui /usr/local/bin/
sudo mkdir -p /etc/wftpg
sudo mkdir -p /var/log/wftpg

echo "=== 构建完成 ==="
echo "运行: wftpg-gui"
