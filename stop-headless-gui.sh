#!/bin/bash
# 停止 WFTPG GUI 无头模式

echo "============================================================"
echo "停止 WFTPG GUI"
echo "============================================================"

# 停止 GUI 进程
if [ -f /tmp/wftp-gui.pid ]; then
    GUI_PID=$(cat /tmp/wftp-gui.pid)
    if kill -0 "$GUI_PID" 2>/dev/null; then
        echo "ℹ️  停止 wftp-gui (PID: $GUI_PID)"
        kill "$GUI_PID" || true
        sleep 1
    fi
    rm -f /tmp/wftp-gui.pid
else
    echo "ℹ️  未找到 wftp-gui PID 文件，尝试通过进程名停止..."
    pkill -f "wftp-gui" 2>/dev/null || true
fi

# 停止 Xvfb 进程
if [ -f /tmp/xvfb-wftpg.pid ]; then
    XVFB_PID=$(cat /tmp/xvfb-wftpg.pid)
    if kill -0 "$XVFB_PID" 2>/dev/null; then
        echo "ℹ️  停止 Xvfb (PID: $XVFB_PID)"
        kill "$XVFB_PID" || true
        sleep 1
    fi
    rm -f /tmp/xvfb-wftpg.pid
else
    echo "ℹ️  未找到 Xvfb PID 文件，尝试通过进程名停止..."
    pkill -f "Xvfb.*:99" 2>/dev/null || true
fi

echo "✅ 已停止所有 GUI 相关进程"
echo "============================================================"
