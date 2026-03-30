#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""简单的 FTP 调试脚本"""

from ftplib import FTP
import os

HOST = '127.0.0.1'
PORT = 2121
USERNAME = '123'
PASSWORD = '123456'

print("连接到 FTP 服务器...")
ftp = FTP()
ftp.connect(HOST, PORT, timeout=10)
ftp.login(USERNAME, PASSWORD)
ftp.set_pasv(True)

print(f"当前目录：{ftp.pwd()}")

# 创建测试目录
test_dir = "/test_debug_001"
try:
    ftp.mkd(test_dir)
    print(f"✓ 创建目录：{test_dir}")
except Exception as e:
    print(f"✗ 创建目录失败：{e}")

# 切换到测试目录
try:
    ftp.cwd(test_dir)
    print(f"✓ 切换到目录：{ftp.pwd()}")
except Exception as e:
    print(f"✗ 切换目录失败：{e}")

# 上传文件
test_file = "test_upload.txt"
local_path = "/tmp/test_upload.txt"
with open(local_path, 'w') as f:
    f.write("这是测试内容\n")

try:
    with open(local_path, 'rb') as f:
        ftp.storbinary(f'STOR {test_file}', f)
    print(f"✓ 上传文件：{test_file}")
except Exception as e:
    print(f"✗ 上传文件失败：{e}")

# 列出目录内容
print("\n目录列表 (NLST):")
try:
    files = ftp.nlst()
    print(f"  文件列表：{files}")
    if test_file in files:
        print(f"  ✓ 文件 {test_file} 存在")
    else:
        print(f"  ✗ 文件 {test_file} 不存在")
except Exception as e:
    print(f"✗ NLST 失败：{e}")

print("\n详细列表 (LIST):")
try:
    ftp.retrlines('LIST')
except Exception as e:
    print(f"✗ LIST 失败：{e}")

# 清理
try:
    ftp.delete(test_file)
    print(f"✓ 删除文件：{test_file}")
except Exception as e:
    print(f"✗ 删除文件失败：{e}")

try:
    ftp.cwd('/')
    ftp.rmd(test_dir)
    print(f"✓ 删除目录：{test_dir}")
except Exception as e:
    print(f"✗ 删除目录失败：{e}")

# 断开连接
ftp.quit()
print("断开连接")

# 检查实际文件系统
print("\n检查实际文件系统:")
os.system("ls -la /home/wftpg/123/test_debug_001/ 2>&1 || echo '目录不存在'")
