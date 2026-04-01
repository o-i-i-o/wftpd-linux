#!/bin/bash

# Iced Demo 运行脚本

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "=================================================="
echo "Iced Demo - Rust GUI 示例程序"
echo "=================================================="
echo ""

# 检查是否已编译
if [ ! -f "./target/debug/iced-demo" ]; then
    echo "🔨 未找到编译文件，正在编译..."
    cargo build
    if [ $? -ne 0 ]; then
        echo "❌ 编译失败！"
        exit 1
    fi
    echo "✅ 编译完成！"
    echo ""
fi

echo "🚀 启动 Iced Demo..."
echo "   程序路径：$SCRIPT_DIR/target/debug/iced-demo"
echo ""

# 运行程序
./target/debug/iced-demo

# 捕获退出码
EXIT_CODE=$?

echo ""
echo "=================================================="
if [ $EXIT_CODE -eq 0 ]; then
    echo "✅ 程序正常退出"
else
    echo "⚠️  程序异常退出 (退出码：$EXIT_CODE)"
fi
echo "=================================================="

exit $EXIT_CODE
