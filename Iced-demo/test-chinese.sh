#!/bin/bash

# 测试 Iced Demo 中文显示

echo "=================================================="
echo "Iced Demo 中文显示测试"
echo "=================================================="
echo ""

cd /home/GGFWZX/Desktop/wftpg/Iced-demo

# 检查字体文件
if [ -f "./fonts/NotoSansSC-Regular.ttf" ]; then
    echo "✅ 中文字体文件存在"
else
    echo "❌ 中文字体文件不存在"
    exit 1
fi

# 检查程序是否已编译
if [ -f "./target/debug/iced-demo" ]; then
    echo "✅ 程序已编译"
else
    echo "❌ 程序未编译，正在编译..."
    cargo build
    if [ $? -ne 0 ]; then
        echo "❌ 编译失败"
        exit 1
    fi
fi

echo ""
echo "🚀 准备启动程序..."
echo "   请确保您在有图形界面的环境中运行此脚本"
echo "   或者使用 Xvfb 等虚拟显示服务器"
echo ""

# 设置环境变量以支持中文
export LANG=zh_CN.UTF-8
export LC_ALL=zh_CN.UTF-8

# 运行程序
./target/debug/iced-demo

EXIT_CODE=$?

echo ""
if [ $EXIT_CODE -eq 0 ]; then
    echo "✅ 程序运行正常"
else
    echo "⚠️  程序退出码：$EXIT_CODE"
fi

exit $EXIT_CODE
