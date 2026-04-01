#!/usr/bin/env python3
"""
WFTPG Socket 测试脚本
用于测试与 wftpd IPC socket 的连接
"""

import socket
import json
import os
import sys

SOCKET_PATH = "/run/wftpd/wftpg.sock"

def check_socket_exists():
    """检查 socket 文件是否存在"""
    if not os.path.exists(SOCKET_PATH):
        print(f"❌ Socket 文件不存在：{SOCKET_PATH}")
        print("请先运行 sudo ./target/release/wftpd 启动服务")
        return False
    print(f"✓ Socket 文件存在：{SOCKET_PATH}")
    return True

def test_socket_connection():
    """测试 socket 连接"""
    try:
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.connect(SOCKET_PATH)
        print(f"✓ Socket 连接成功")
        client.close()
        return True
    except PermissionError as e:
        print(f"❌ 权限错误：{e}")
        print("提示：您可能需要将用户添加到 wftpg 组:")
        print(f"  sudo usermod -aG wftpg $USER")
        print("然后重新登录或重启系统")
        return False
    except Exception as e:
        print(f"❌ 连接失败：{e}")
        return False

def send_ipc_request(command):
    """发送 IPC 请求并获取响应"""
    try:
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.connect(SOCKET_PATH)
        
        # 构建请求
        request = {
            "id": 1,
            "command": command
        }
        
        # 发送请求 (JSON 格式，以换行符结尾)
        request_json = json.dumps(request) + "\n"
        client.sendall(request_json.encode('utf-8'))
        
        # 接收响应
        response_data = b""
        while True:
            chunk = client.recv(4096)
            if not chunk:
                break
            response_data += chunk
            if b"\n" in chunk:
                break
        
        # 解析响应
        response_str = response_data.decode('utf-8').strip()
        response = json.loads(response_str)
        
        client.close()
        return response
        
    except Exception as e:
        return {"error": str(e)}

def test_get_status():
    """测试获取服务状态"""
    print("\n测试：获取服务状态")
    command = {"type": "get_status"}
    response = send_ipc_request(command)
    
    if "error" in response:
        print(f"❌ 请求失败：{response['error']}")
        return
    
    print(f"✓ 响应：{json.dumps(response, indent=2, ensure_ascii=False)}")
    
    if response.get("result", {}).get("type") == "status":
        status = response["result"]
        print(f"  FTP 运行中：{'是' if status.get('ftp_running') else '否'}")
        print(f"  SFTP 运行中：{'是' if status.get('sftp_running') else '否'}")

def test_get_config():
    """测试获取配置"""
    print("\n测试：获取配置文件")
    command = {"type": "get_config"}
    response = send_ipc_request(command)
    
    if "error" in response:
        print(f"❌ 请求失败：{response['error']}")
        return
    
    print(f"✓ 响应：{json.dumps(response, indent=2, ensure_ascii=False)}")

def test_get_initial_state():
    """测试获取初始状态"""
    print("\n测试：获取初始状态")
    command = {"type": "get_initial_state"}
    response = send_ipc_request(command)
    
    if "error" in response:
        print(f"❌ 请求失败：{response['error']}")
        return
    
    print(f"✓ 响应：{json.dumps(response, indent=2, ensure_ascii=False)}")

def main():
    print("=" * 60)
    print("WFTPG Socket 连接测试")
    print("=" * 60)
    
    # 检查 socket 是否存在
    if not check_socket_exists():
        sys.exit(1)
    
    # 测试连接
    if not test_socket_connection():
        sys.exit(1)
    
    print("\n" + "=" * 60)
    print("执行 IPC 命令测试")
    print("=" * 60)
    
    # 测试各种命令
    test_get_status()
    test_get_config()
    test_get_initial_state()
    
    print("\n" + "=" * 60)
    print("测试完成 ✓")
    print("=" * 60)

if __name__ == "__main__":
    main()
