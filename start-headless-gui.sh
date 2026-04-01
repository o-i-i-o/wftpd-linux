#!/bin/bash
# WFTPG GUI 无头模式启动脚本
# 使用 Xvfb 创建虚拟显示环境

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GUI_BINARY="${SCRIPT_DIR}/target/release/wftp-gui"
XVFB_DISPLAY=":99"
XVFB_SCREEN="1920x1080x24"
XVFB_PID_FILE="/tmp/xvfb-wftpg.pid"

echo "============================================================"
echo "WFTPG GUI 无头模式启动"
echo "============================================================"

# 检查 GUI 二进制文件
if [ ! -f "$GUI_BINARY" ]; then
    echo "❌ GUI 二进制文件不存在：$GUI_BINARY"
    echo "请先运行：cargo build --release"
    exit 1
fi

# 检查 wftpd 服务是否运行
if ! pgrep -f "wftpd" > /dev/null; then
    echo "⚠️  警告：wftpd 后台服务未运行"
    echo "建议先启动 wftpd 服务：./target/release/wftpd &"
fi

# 停止旧的 Xvfb 进程（如果有）
if [ -f "$XVFB_PID_FILE" ]; then
    OLD_PID=$(cat "$XVFB_PID_FILE")
    if kill -0 "$OLD_PID" 2>/dev/null; then
        echo "ℹ️  停止旧的 Xvfb 进程 (PID: $OLD_PID)"
        kill "$OLD_PID" || true
        sleep 1
    fi
    rm -f "$XVFB_PID_FILE"
fi

# 停止旧的 GUI 进程
pkill -f "wftp-gui" 2>/dev/null || true
sleep 1

# 启动 Xvfb
echo "🚀 启动 Xvfb 虚拟显示..."
Xvfb "$XVFB_DISPLAY" -screen 0 "$XVFB_SCREEN" &
XVFB_PID=$!
echo "$XVFB_PID" > "$XVFB_PID_FILE"
echo "✓ Xvfb 已启动 (PID: $XVFB_PID, Display: $XVFB_DISPLAY)"

# 等待 Xvfb 准备就绪
sleep 2

# 验证 Xvfb 是否运行
if ! kill -0 "$XVFB_PID" 2>/dev/null; then
    echo "❌ Xvfb 启动失败"
    exit 1
fi

# 设置 DISPLAY 环境变量
export DISPLAY="$XVFB_DISPLAY"

# 启动 wftp-gui
echo "🚀 启动 wftp-gui (无头模式)..."
echo "   DISPLAY: $DISPLAY"
echo "   二进制：$GUI_BINARY"
echo ""

# 在后台运行 GUI
"$GUI_BINARY" &
GUI_PID=$!

echo "✓ wftp-gui 已启动 (PID: $GUI_PID)"
echo ""
echo "============================================================"
echo "✅ GUI 已在无头模式下成功启动！"
echo "============================================================"
echo ""
echo "进程信息:"
echo "  - Xvfb: PID $XVFB_PID, Display $XVFB_DISPLAY"
echo "  - wftp-gui: PID $GUI_PID"
echo ""
echo "测试 IPC 连接:"
echo "  python3 test_socket.py"
echo ""
echo "查看进程:"
echo "  ps aux | grep -E 'Xvfb|wftp-gui'"
echo ""
echo "停止 GUI:"
echo "  $0 stop"
echo ""
echo "清理所有进程:"
echo "  $0 cleanup"
echo ""

# 保存 GUI PID
echo "$GUI_PID" > /tmp/wftp-gui.pid

# 等待进程结束
wait $GUI_PID
