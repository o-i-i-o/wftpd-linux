#!/bin/bash
# WFTPD 快速启动脚本

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="$SCRIPT_DIR/target/release/wftpd"

echo "======================================"
echo "WFTPD - FTP/SFTP 后端服务"
echo "======================================"

# 检查是否以 root 运行
if [ "$EUID" -ne 0 ]; then 
    echo "⚠️  请使用 sudo 运行此脚本"
    echo "用法：sudo $0 [start|stop|restart|status]"
    exit 1
fi

# 检查二进制文件
if [ ! -f "$BINARY" ]; then
    echo "❌ 二进制文件不存在: $BINARY"
    echo "请先运行：cargo build --release"
    exit 1
fi

case "${1:-start}" in
    start)
        echo "🚀 启动 WFTPD 服务..."
        cd "$SCRIPT_DIR"
        exec "$BINARY"
        ;;
    stop)
        echo "🛑 停止 WFTPD 服务..."
        pkill -f "wftpd$" || echo "服务未运行"
        ;;
    restart)
        echo "🔄 重启 WFTPD 服务..."
        pkill -f "wftpd$" || true
        sleep 1
        cd "$SCRIPT_DIR"
        exec "$BINARY"
        ;;
    status)
        if pgrep -f "wftpd$" > /dev/null; then
            echo "✅ WFTPD 服务正在运行"
            pgrep -a -f "wftpd$"
        else
            echo "❌ WFTPD 服务未运行"
        fi
        ;;
    *)
        echo "用法：$0 {start|stop|restart|status}"
        exit 1
        ;;
esac
