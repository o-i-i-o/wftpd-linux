# -*- coding: utf-8 -*-
"""
WFTPG FTPS 完整功能测试脚本（支持显式和隐式模式）
测试用户：123 / 密码：123456
"""

import os
import sys
import time
import tempfile
import hashlib
import socket
import random
import string
from pathlib import Path
from datetime import datetime
import ssl
import json


# ==================== 集中配置区域 ====================
# 服务器配置
SERVER_HOST = '127.0.0.1'
EXPLICIT_FTPS_PORT = 2121      # 显式 FTPS 端口
IMPLICIT_FTPS_PORT = 990       # 隐式 FTPS 端口

# 用户认证配置
TEST_USERNAME = '123'
TEST_PASSWORD = '123456'

# SSL/TLS 配置
SSL_VERIFY_MODE = ssl.CERT_NONE
CONNECTION_TIMEOUT = 10

# 日志与输出
ENABLE_VERBOSE_LOG = False
OUTPUT_JSON_RESULT = True
# =================================================


def generate_unique_filename(prefix='', suffix='.txt'):
    """生成唯一的文件名"""
    timestamp = datetime.now().strftime('%Y%m%d_%H%M%S')
    random_str = ''.join(random.choices(string.ascii_lowercase + string.digits, k=8))
    return f"{prefix}{timestamp}_{random_str}{suffix}"


def calculate_md5(file_path):
    """计算文件的 MD5 值"""
    hash_md5 = hashlib.md5()
    with open(file_path, 'rb') as f:
        for chunk in iter(lambda: f.read(4096), b''):
            hash_md5.update(chunk)
    return hash_md5.hexdigest()


class TestResult:
    """测试结果统计"""
    def __init__(self):
        self.total = 0
        self.passed = 0
        self.failed = 0
        self.errors = []
        self.start_time = None
        self.end_time = None
    
    def add_pass(self, test_name):
        self.total += 1
        self.passed += 1
        print(f"  [PASS] {test_name}")
    
    def add_fail(self, test_name, reason):
        self.total += 1
        self.failed += 1
        self.errors.append({"test": test_name, "reason": reason})
        print(f"  [FAIL] {test_name}: {reason}")
    
    def get_summary(self):
        duration = (self.end_time - self.start_time) if self.end_time and self.start_time else None
        duration_seconds = duration.total_seconds() if duration else 0
        return {
            "total": self.total,
            "passed": self.passed,
            "failed": self.failed,
            "success_rate": f"{(self.passed / self.total * 100):.2f}%" if self.total > 0 else "N/A",
            "duration_seconds": round(duration_seconds, 2),
            "errors": self.errors
        }


class ImplicitFTPSClient:
    """隐式 FTPS 客户端实现"""
    
    def __init__(self, host, port, username, password, timeout=10):
        self.host = host
        self.port = port
        self.username = username
        self.password = password
        self.timeout = timeout
        self.sock = None
        self.ssl_sock = None
        self.file = None
        self.result = TestResult()
        self.test_dir = '/test_ftps_' + datetime.now().strftime('%Y%m%d_%H%M%S')
        self.temp_files = []
        self.unique_prefix = generate_unique_filename(prefix='ftps_', suffix='') + '_'
    
    def connect(self):
        """建立隐式 FTPS 连接"""
        try:
            # 创建 SSL 上下文
            ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
            ctx.check_hostname = False
            ctx.verify_mode = ssl.CERT_NONE
            ctx.minimum_version = ssl.TLSVersion.TLSv1_2
            
            # 建立 TCP 连接
            self.sock = socket.create_connection(
                (self.host, self.port),
                timeout=self.timeout
            )
            
            # 立即进行 SSL 握手（隐式模式特点）
            self.ssl_sock = ctx.wrap_socket(
                self.sock,
                server_hostname=self.host
            )
            
            # 创建文件对象用于通信
            self.file = self.ssl_sock.makefile('r', encoding='utf-8', newline='\r\n')
            
            # 读取欢迎消息
            welcome = self._read_response()
            print(f"  ✅ {welcome.strip()}")
            
            # 登录
            self._send_command(f'USER {self.username}')
            response = self._read_response()
            print(f"  ✅ {response.strip()}")
            
            if '331' not in response:
                raise Exception(f"用户名验证失败：{response}")
            
            self._send_command(f'PASS {self.password}')
            response = self._read_response()
            print(f"  ✅ {response.strip()}")
            
            if '230' not in response:
                raise Exception(f"密码验证失败：{response}")
            
            # 设置被动模式
            self._send_command('PASV')
            response = self._read_response()
            print(f"  ✅ 被动模式：{response.strip()}")
            
            print(f"  ✅ 隐式 FTPS 连接成功 - 协议版本：{self.ssl_sock.version()}")
            return True
            
        except Exception as e:
            print(f"  ❌ 连接失败：{e}")
            self.close()
            return False
    
    def _send_command(self, command):
        """发送 FTP 命令"""
        self.ssl_sock.sendall((command + '\r\n').encode('utf-8'))
    
    def _read_response(self):
        """读取服务器响应"""
        response = ''
        while True:
            line = self.file.readline()
            if not line:
                break
            response += line
            # 检查是否是最后一行（以数字开头和空格结尾）
            if len(line) >= 4 and line[0].isdigit() and line[3] == ' ':
                break
        return response
    
    def pwd(self):
        """获取当前目录"""
        test_name = "PWD 获取路径"
        try:
            self._send_command('PWD')
            response = self._read_response()
            if '257' in response:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"响应异常：{response}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def cwd(self, path):
        """切换目录"""
        test_name = "CWD 切换目录"
        try:
            self._send_command(f'CWD {path}')
            response = self._read_response()
            if '250' in response:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"响应异常：{response}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def mkd(self, path):
        """创建目录"""
        test_name = "MKD 创建目录"
        try:
            self._send_command(f'MKD {path}')
            response = self._read_response()
            if '257' in response:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"响应异常：{response}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def rmd(self, path):
        """删除目录"""
        test_name = "RMD 删除目录"
        try:
            self._send_command(f'RMD {path}')
            response = self._read_response()
            if '250' in response:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"响应异常：{response}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def list(self):
        """列出目录"""
        test_name = "LIST 列出目录"
        try:
            # 获取被动模式地址
            self._send_command('PASV')
            pasv_response = self._read_response()
            
            # 解析 PASV 响应
            import re
            match = re.search(r'\((.*?)\)', pasv_response)
            if match:
                parts = match.group(1).split(',')
                if len(parts) == 6:
                    ip = '.'.join(parts[:4])
                    port = int(parts[4]) * 256 + int(parts[5])
                    
                    # 建立数据连接
                    data_sock = socket.create_connection((ip, port), timeout=self.timeout)
                    data_sock.settimeout(self.timeout)
                    
                    # 发送 LIST 命令
                    self._send_command('LIST')
                    response = self._read_response()
                    
                    if '150' in response or '226' in response:
                        # 读取数据
                        data = data_sock.recv(4096)
                        data_sock.close()
                        
                        # 读取最终响应
                        final_response = self._read_response()
                        if '226' in final_response:
                            self.result.add_pass(test_name)
                        else:
                            self.result.add_fail(test_name, f"数据传输失败：{final_response}")
                    else:
                        data_sock.close()
                        self.result.add_fail(test_name, f"LIST 命令失败：{response}")
                else:
                    self.result.add_fail(test_name, "PASV 响应格式错误")
            else:
                self.result.add_fail(test_name, "无法解析 PASV 响应")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def stor(self, filename, content):
        """上传文件"""
        test_name = "STOR 上传文件"
        try:
            # 获取被动模式地址
            self._send_command('PASV')
            pasv_response = self._read_response()
            
            import re
            match = re.search(r'\((.*?)\)', pasv_response)
            if match:
                parts = match.group(1).split(',')
                if len(parts) == 6:
                    ip = '.'.join(parts[:4])
                    port = int(parts[4]) * 256 + int(parts[5])
                    
                    # 建立数据连接
                    data_sock = socket.create_connection((ip, port), timeout=self.timeout)
                    
                    # 发送 STOR 命令
                    self._send_command(f'STOR {filename}')
                    response = self._read_response()
                    
                    if '150' in response:
                        # 发送文件内容
                        data_sock.sendall(content.encode('utf-8'))
                        data_sock.shutdown(socket.SHUT_WR)
                        data_sock.close()
                        
                        # 读取最终响应
                        final_response = self._read_response()
                        if '226' in final_response:
                            self.result.add_pass(test_name)
                        else:
                            self.result.add_fail(test_name, f"传输失败：{final_response}")
                    else:
                        data_sock.close()
                        self.result.add_fail(test_name, f"STOR 命令失败：{response}")
            else:
                self.result.add_fail(test_name, "无法解析 PASV 响应")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def retr(self, filename):
        """下载文件"""
        test_name = "RETR 下载文件"
        try:
            # 获取被动模式地址
            self._send_command('PASV')
            pasv_response = self._read_response()
            
            import re
            match = re.search(r'\((.*?)\)', pasv_response)
            if match:
                parts = match.group(1).split(',')
                if len(parts) == 6:
                    ip = '.'.join(parts[:4])
                    port = int(parts[4]) * 256 + int(parts[5])
                    
                    # 建立数据连接
                    data_sock = socket.create_connection((ip, port), timeout=self.timeout)
                    
                    # 发送 RETR 命令
                    self._send_command(f'RETR {filename}')
                    response = self._read_response()
                    
                    if '150' in response:
                        # 接收文件内容
                        content = b''
                        while True:
                            chunk = data_sock.recv(4096)
                            if not chunk:
                                break
                            content += chunk
                        
                        data_sock.close()
                        
                        # 读取最终响应
                        final_response = self._read_response()
                        if '226' in final_response:
                            self.result.add_pass(test_name)
                            return content.decode('utf-8')
                        else:
                            self.result.add_fail(test_name, f"传输失败：{final_response}")
                    else:
                        data_sock.close()
                        self.result.add_fail(test_name, f"RETR 命令失败：{response}")
            else:
                self.result.add_fail(test_name, "无法解析 PASV 响应")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
        return ''
    
    def dele(self, filename):
        """删除文件"""
        test_name = "DELE 删除文件"
        try:
            self._send_command(f'DELE {filename}')
            response = self._read_response()
            if '250' in response:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"响应异常：{response}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def noop(self):
        """保持连接"""
        test_name = "NOOP 保持连接"
        try:
            self._send_command('NOOP')
            response = self._read_response()
            if '200' in response:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"响应异常：{response}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def quit(self):
        """退出连接"""
        test_name = "QUIT 退出连接"
        try:
            self._send_command('QUIT')
            response = self._read_response()
            if '221' in response:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"响应异常：{response}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def close(self):
        """关闭连接"""
        if self.file:
            self.file.close()
        if self.ssl_sock:
            self.ssl_sock.close()
        if self.sock:
            self.sock.close()
    
    def run_all_tests(self):
        """运行所有测试"""
        print("\n" + "="*60)
        print("开始隐式 FTPS 功能测试")
        print("="*60)
        
        self.result.start_time = datetime.now()
        
        if not self.connect():
            self.result.add_fail("FTPS 连接", "无法连接到 FTPS 服务器")
            self.result.end_time = datetime.now()
            return self.result
        
        try:
            # 基础测试
            self.pwd()
            self.mkd(self.test_dir)
            self.cwd(self.test_dir)
            
            # 文件操作测试
            test_content = f"FTPS 测试内容 - {datetime.now().isoformat()}"
            test_file = self.unique_prefix + 'test.txt'
            self.stor(test_file, test_content)
            downloaded = self.retr(test_file)
            
            # 验证 MD5
            md5_test_name = "MD5 校验"
            if downloaded == test_content:
                self.result.add_pass(md5_test_name)
            else:
                self.result.add_fail(md5_test_name, "内容不匹配")
            
            self.list()
            self.dele(test_file)
            self.rmd(self.test_dir)
            
            # 连接测试
            self.noop()
            self.quit()
            
        finally:
            self.close()
        
        self.result.end_time = datetime.now()
        return self.result


def main():
    """主函数"""
    print("="*60)
    print("WFTPG FTPS (隐式 SSL/TLS) 完整功能测试")
    print("测试用户：123 / 密码：123456")
    print("="*60)
    
    print("\n[INFO] 测试模式：IMPLICIT (端口 990)\n")
    
    # 运行测试
    ftps_client = ImplicitFTPSClient(
        host=SERVER_HOST,
        port=IMPLICIT_FTPS_PORT,
        username=TEST_USERNAME,
        password=TEST_PASSWORD,
        timeout=CONNECTION_TIMEOUT
    )
    
    result = ftps_client.run_all_tests()
    
    # 汇总结果
    print("\n" + "="*60)
    print("FTPS 测试结果汇总")
    print("="*60)
    
    print(f"\n总测试数：{result.total}")
    print(f"通过：{result.passed}")
    print(f"失败：{result.failed}")
    print(f"成功率：{result.get_summary()['success_rate']}")
    print(f"耗时：{result.get_summary()['duration_seconds']:.2f} 秒")
    
    if result.errors:
        print("\n--- 错误详情 ---")
        for error in result.errors:
            print(f"  [FAIL] {error['test']}: {error['reason']}")
    
    # 保存测试结果到 JSON 文件
    if OUTPUT_JSON_RESULT:
        test_result = {
            "start_time": result.start_time.isoformat() if result.start_time else None,
            "end_time": result.end_time.isoformat() if result.end_time else None,
            "ftps_implicit": result.get_summary()
        }
        
        result_file = os.path.join(os.path.dirname(__file__), 'test_ftps_full_result.json')
        with open(result_file, 'w', encoding='utf-8') as f:
            json.dump(test_result, f, ensure_ascii=False, indent=2)
        
        print(f"\n测试结果已保存到：{result_file}")
    
    # 返回退出码
    if result.failed > 0:
        print(f"\n❌ 有 {result.failed} 项测试失败")
        sys.exit(1)
    else:
        print(f"\n✅ 所有 {result.total} 项测试全部通过！")
        sys.exit(0)


if __name__ == '__main__':
    main()
