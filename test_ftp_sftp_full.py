# -*- coding: utf-8 -*-
"""
WFTPG FTP/SFTP 完整功能测试脚本
跨平台版本 - 专注于协议功能测试

测试用户配置：
- 用户名：123
- 密码：123456
"""

import os
import sys
import time
import socket
import hashlib
import random
import string
import json
from pathlib import Path
from datetime import datetime
from ftplib import FTP, error_perm, error_temp
from typing import Optional, List, Dict, Any
import paramiko
import stat


# ==================== 集中配置区域 ====================
class Config:
    """测试配置"""
    # 服务器配置
    SERVER_HOST = '127.0.0.1'
    FTP_PORT = 2121
    SFTP_PORT = 2222
    
    # 用户认证
    USERNAME = '123'
    PASSWORD = '123456'
    
    # 超时配置
    CONNECTION_TIMEOUT = 10  # 连接超时（秒）
    PORT_WAIT_TIMEOUT = 30   # 等待端口就绪超时（秒）
    TEST_DELAY = 2          # 测试间延迟（秒）
    
    # 输出配置
    VERBOSE = False         # 详细日志
    OUTPUT_JSON = True      # 输出 JSON 结果


class TestResult:
    """测试结果统计"""
    
    def __init__(self, test_type: str):
        self.test_type = test_type
        self.total = 0
        self.passed = 0
        self.failed = 0
        self.errors: List[Dict[str, str]] = []
        self.start_time: Optional[datetime] = None
        self.end_time: Optional[datetime] = None
    
    def add_pass(self, test_name: str):
        """记录通过的测试"""
        self.total += 1
        self.passed += 1
        print(f"  ✓ {test_name}")
    
    def add_fail(self, test_name: str, reason: str):
        """记录失败的测试"""
        self.total += 1
        self.failed += 1
        self.errors.append({"test": test_name, "reason": reason})
        print(f"  ✗ {test_name}: {reason}")
    
    def get_summary(self) -> Dict[str, Any]:
        """获取测试摘要"""
        duration = (self.end_time - self.start_time) if self.end_time and self.start_time else None
        duration_seconds = duration.total_seconds() if duration else 0
        
        return {
            "type": self.test_type,
            "total": self.total,
            "passed": self.passed,
            "failed": self.failed,
            "success_rate": f"{(self.passed / self.total * 100):.2f}%" if self.total > 0 else "N/A",
            "duration_seconds": round(duration_seconds, 2),
            "errors": self.errors
        }


def is_port_open(host: str, port: int, timeout: int = 2) -> bool:
    """
    检查端口是否开放
    
    Args:
        host: 主机地址
        port: 端口号
        timeout: 超时时间（秒）
    
    Returns:
        bool: 端口是否开放
    """
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
            sock.settimeout(timeout)
            result = sock.connect_ex((host, port))
            return result == 0
    except Exception:
        return False


def wait_for_ports(host: str, ports: List[int], timeout: int = 30) -> bool:
    """
    等待多个端口就绪
    
    Args:
        host: 主机地址
        ports: 端口列表
        timeout: 超时时间（秒）
    
    Returns:
        bool: 所有端口是否都已就绪
    """
    print(f"等待端口 {', '.join(map(str, ports))} 就绪...")
    start_time = time.time()
    
    while time.time() - start_time < timeout:
        all_open = all(is_port_open(host, port) for port in ports)
        if all_open:
            print(f"✓ 所有端口已就绪: {', '.join(map(str, ports))}")
            return True
        time.sleep(0.5)
    
    # 报告哪些端口未就绪
    unavailable = [port for port in ports if not is_port_open(host, port)]
    if unavailable:
        print(f"✗ 以下端口未就绪：{', '.join(map(str, unavailable))}")
    
    return False


def calculate_md5(file_path: str) -> str:
    """计算文件 MD5 值"""
    hash_md5 = hashlib.md5()
    with open(file_path, 'rb') as f:
        for chunk in iter(lambda: f.read(8192), b''):
            hash_md5.update(chunk)
    return hash_md5.hexdigest()


def generate_test_content(prefix: str, size: int = 1024) -> bytes:
    """
    生成测试文件内容
    
    Args:
        prefix: 前缀标识
        size: 内容大小（字节）
    
    Returns:
        bytes: 测试内容
    """
    timestamp = datetime.now().isoformat()
    base_content = f"{prefix} - Generated at {timestamp}\n".encode('utf-8')
    
    if len(base_content) >= size:
        return base_content[:size]
    
    # 填充随机数据
    random_data = ''.join(
        random.choices(string.ascii_letters + string.digits + '\n', k=size - len(base_content))
    ).encode('utf-8')
    
    return base_content + random_data


class FTPTester:
    """FTP 协议功能测试"""
    
    def __init__(self, host: str, port: int, username: str, password: str):
        self.host = host
        self.port = port
        self.username = username
        self.password = password
        self.ftp: Optional[FTP] = None
        self.result = TestResult("FTP")
        self.temp_files: List[str] = []
        self.test_dir = f"/test_ftp_{datetime.now().strftime('%Y%m%d_%H%M%S')}"
        self.current_file_prefix = f"ftp_{datetime.now().strftime('%Y%m%d_%H%M%S')}_"
    
    def connect(self) -> bool:
        """建立 FTP 连接"""
        try:
            self.ftp = FTP()
            self.ftp.connect(self.host, self.port, timeout=Config.CONNECTION_TIMEOUT)
            self.ftp.login(self.username, self.password)
            self.ftp.set_pasv(True)
            print("✓ FTP 连接成功")
            return True
        except Exception as e:
            print(f"✗ FTP 连接失败：{e}")
            return False
    
    def disconnect(self):
        """断开 FTP 连接"""
        if self.ftp:
            try:
                self.ftp.quit()
            except Exception:
                try:
                    self.ftp.close()
                except Exception:
                    pass
            self.ftp = None
    
    def create_temp_file(self, filename: str, content: Optional[bytes] = None) -> str:
        """创建临时测试文件"""
        import tempfile
        temp_path = os.path.join(tempfile.gettempdir(), filename)
        
        if content is None:
            content = generate_test_content(filename)
        
        with open(temp_path, 'wb') as f:
            f.write(content)
        
        self.temp_files.append(temp_path)
        return temp_path
    
    def cleanup(self):
        """清理测试文件"""
        # 清理远程文件
        if self.ftp:
            try:
                # 尝试删除测试目录
                try:
                    self.ftp.rmd(self.test_dir)
                except Exception:
                    pass
            except Exception:
                pass
        
        # 清理本地临时文件
        for file_path in self.temp_files:
            try:
                if os.path.exists(file_path):
                    os.remove(file_path)
            except Exception:
                pass
        
        self.temp_files.clear()
    
    # ========== 基础命令测试 ==========
    
    def test_login(self):
        """测试登录验证"""
        test_name = "登录验证"
        try:
            response = self.ftp.voidcmd('NOOP')
            if '200' in response:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"NOOP 响应异常：{response}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_pwd(self):
        """测试获取当前目录"""
        test_name = "PWD 获取路径"
        try:
            pwd = self.ftp.pwd()
            if pwd and isinstance(pwd, str):
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"返回空路径或类型错误：{pwd}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_syst(self):
        """测试系统类型查询"""
        test_name = "SYST 系统类型"
        try:
            response = self.ftp.sendcmd('SYST')
            if '215' in response:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"响应异常：{response}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_feat(self):
        """测试功能列表"""
        test_name = "FEAT 功能查询"
        try:
            response = self.ftp.sendcmd('FEAT')
            if '211' in response:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"响应异常：{response}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_type(self):
        """测试传输类型设置"""
        test_name = "TYPE 传输类型"
        try:
            # ASCII 模式
            resp_ascii = self.ftp.voidcmd('TYPE A')
            # Binary 模式
            resp_binary = self.ftp.voidcmd('TYPE I')
            
            if '200' in resp_ascii and '200' in resp_binary:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"ASCII:{resp_ascii}, Binary:{resp_binary}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_noop(self):
        """测试保持连接"""
        test_name = "NOOP 保持连接"
        try:
            response = self.ftp.voidcmd('NOOP')
            if '200' in response:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"响应异常：{response}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    # ========== 目录操作测试 ==========
    
    def test_mkd(self):
        """测试创建目录"""
        test_name = "MKD 创建目录"
        try:
            self.ftp.mkd(self.test_dir)
            self.result.add_pass(test_name)
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_cwd(self):
        """测试切换目录"""
        test_name = "CWD 切换目录"
        try:
            self.ftp.cwd(self.test_dir)
            current = self.ftp.pwd()
            
            if self.test_dir in current:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"路径不匹配：期望包含{self.test_dir}, 实际={current}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_rmd(self):
        """测试删除目录"""
        test_name = "RMD 删除目录"
        try:
            # 先回到根目录
            self.ftp.cwd('/')
            self.ftp.rmd(self.test_dir)
            self.result.add_pass(test_name)
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    # ========== 文件操作测试 ==========
    
    def test_put_file(self):
        """测试上传文件（带 MD5 校验）"""
        test_name = "STOR 上传文件"
        try:
            filename = f"{self.current_file_prefix}upload.txt"
            local_path = self.create_temp_file(filename)
            remote_filename = filename
            
            # 计算本地 MD5
            local_md5 = calculate_md5(local_path)
            
            # 上传文件
            with open(local_path, 'rb') as f:
                self.ftp.storbinary(f'STOR {remote_filename}', f)
            
            # 验证文件存在
            files = self.ftp.nlst()
            if remote_filename not in files:
                self.result.add_fail(test_name, "上传后文件不存在")
                return
            
            # 下载并校验 MD5
            import tempfile
            download_path = os.path.join(tempfile.gettempdir(), f"{filename}.verify")
            self.temp_files.append(download_path)
            
            with open(download_path, 'wb') as f:
                self.ftp.retrbinary(f'RETR {remote_filename}', f.write)
            
            download_md5 = calculate_md5(download_path)
            
            if local_md5 == download_md5:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"MD5 不匹配：本地={local_md5}, 远程={download_md5}")
        
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_get_file(self):
        """测试下载文件（带 MD5 校验）"""
        test_name = "RETR 下载文件"
        try:
            filename = f"{self.current_file_prefix}upload.txt"
            import tempfile
            local_path = os.path.join(tempfile.gettempdir(), f"{filename}.download")
            self.temp_files.append(local_path)
            
            # 首次下载用于获取 MD5
            with open(local_path, 'wb') as f:
                self.ftp.retrbinary(f'RETR {filename}', f.write)
            
            if not os.path.exists(local_path) or os.path.getsize(local_path) == 0:
                self.result.add_fail(test_name, "下载文件为空或不存在")
                return
            
            original_md5 = calculate_md5(local_path)
            
            # 再次下载进行比对
            verify_path = os.path.join(tempfile.gettempdir(), f"{filename}.verify")
            self.temp_files.append(verify_path)
            
            with open(verify_path, 'wb') as f:
                self.ftp.retrbinary(f'RETR {filename}', f.write)
            
            verify_md5 = calculate_md5(verify_path)
            
            if original_md5 == verify_md5:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"MD5 不匹配：{original_md5} ≠ {verify_md5}")
        
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_list(self):
        """测试列出目录（LIST 命令）"""
        test_name = "LIST 详细列表"
        try:
            result = []
            self.ftp.retrlines('LIST', result.append)
            
            if len(result) >= 0:  # 允许空目录
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "列表为空")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_nlst(self):
        """测试简单列出（NLST 命令）"""
        test_name = "NLST 简单列表"
        try:
            files = self.ftp.nlst()
            
            if isinstance(files, list):
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"返回类型错误：{type(files)}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_size(self):
        """测试获取文件大小"""
        test_name = "SIZE 文件大小"
        try:
            filename = f"{self.current_file_prefix}upload.txt"
            size = self.ftp.size(filename)
            
            if size is not None and size > 0:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"大小异常：{size}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_mdtm(self):
        """测试获取文件修改时间"""
        test_name = "MDTM 修改时间"
        try:
            filename = f"{self.current_file_prefix}upload.txt"
            response = self.ftp.sendcmd(f'MDTM {filename}')
            
            if '213' in response:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"响应异常：{response}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_rename(self):
        """测试重命名文件"""
        test_name = "RNFR/RNTO 重命名"
        try:
            old_name = f"{self.current_file_prefix}rename_old.txt"
            new_name = f"{self.current_file_prefix}rename_new.txt"
            
            # 上传测试文件
            content = generate_test_content("rename_test")
            local_path = self.create_temp_file(old_name, content)
            
            with open(local_path, 'rb') as f:
                self.ftp.storbinary(f'STOR {old_name}', f)
            
            # 重命名
            self.ftp.rename(old_name, new_name)
            
            # 验证
            files = self.ftp.nlst()
            if new_name in files and old_name not in files:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "重命名后文件状态异常")
        
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_delete(self):
        """测试删除文件"""
        test_name = "DELE 删除文件"
        try:
            filename = f"{self.current_file_prefix}delete.txt"
            
            # 上传测试文件
            content = generate_test_content("delete_test")
            local_path = self.create_temp_file(filename, content)
            
            with open(local_path, 'rb') as f:
                self.ftp.storbinary(f'STOR {filename}', f)
            
            # 删除
            self.ftp.delete(filename)
            
            # 验证已删除
            files = self.ftp.nlst()
            if filename not in files:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "删除后文件仍存在")
        
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def run_all_tests(self) -> TestResult:
        """运行所有 FTP 测试"""
        print("\n" + "="*70)
        print("FTP 协议功能测试")
        print("="*70)
        
        self.result.start_time = datetime.now()
        
        if not self.connect():
            self.result.add_fail("FTP 连接", "无法连接到服务器")
            self.result.end_time = datetime.now()
            return self.result
        
        try:
            # 基础命令
            print("\n[基础命令]")
            self.test_login()
            self.test_pwd()
            self.test_syst()
            self.test_feat()
            self.test_type()
            self.test_noop()
            
            # 目录操作
            print("\n[目录操作]")
            self.test_mkd()
            self.test_cwd()
            
            # 文件操作
            print("\n[文件操作]")
            self.test_put_file()
            self.test_list()
            self.test_nlst()
            self.test_size()
            self.test_mdtm()
            self.test_get_file()
            self.test_rename()
            self.test_delete()
            
            # 清理
            print("\n[清理]")
            self.test_rmd()
        
        finally:
            self.disconnect()
            self.cleanup()
        
        self.result.end_time = datetime.now()
        return self.result


class SFTPTester:
    """SFTP 协议功能测试"""
    
    def __init__(self, host: str, port: int, username: str, password: str):
        self.host = host
        self.port = port
        self.username = username
        self.password = password
        self.ssh: Optional[paramiko.SSHClient] = None
        self.sftp: Optional[paramiko.SFTPClient] = None
        self.result = TestResult("SFTP")
        self.temp_files: List[str] = []
        self.test_dir = f"test_sftp_{datetime.now().strftime('%Y%m%d_%H%M%S')}"
        self.current_file_prefix = f"sftp_{datetime.now().strftime('%Y%m%d_%H%M%S')}_"
    
    def connect(self) -> bool:
        """建立 SFTP 连接"""
        try:
            self.ssh = paramiko.SSHClient()
            self.ssh.set_missing_host_key_policy(paramiko.AutoAddPolicy())
            self.ssh.connect(
                hostname=self.host,
                port=self.port,
                username=self.username,
                password=self.password,
                timeout=Config.CONNECTION_TIMEOUT,
                allow_agent=False,
                look_for_keys=False
            )
            self.sftp = self.ssh.open_sftp()
            print("✓ SFTP 连接成功")
            return True
        except Exception as e:
            print(f"✗ SFTP 连接失败：{e}")
            return False
    
    def disconnect(self):
        """断开 SFTP 连接"""
        if self.sftp:
            try:
                self.sftp.close()
            except Exception:
                pass
            self.sftp = None
        
        if self.ssh:
            try:
                self.ssh.close()
            except Exception:
                pass
            self.ssh = None
    
    def create_temp_file(self, filename: str, content: Optional[bytes] = None) -> str:
        """创建临时测试文件"""
        import tempfile
        temp_path = os.path.join(tempfile.gettempdir(), filename)
        
        if content is None:
            content = generate_test_content(filename)
        
        with open(temp_path, 'wb') as f:
            f.write(content)
        
        self.temp_files.append(temp_path)
        return temp_path
    
    def cleanup(self):
        """清理测试文件"""
        if self.sftp:
            try:
                # 尝试删除测试目录
                try:
                    self.sftp.rmdir(self.test_dir)
                except Exception:
                    pass
            except Exception:
                pass
        
        # 清理本地临时文件
        for file_path in self.temp_files:
            try:
                if os.path.exists(file_path):
                    os.remove(file_path)
            except Exception:
                pass
        
        self.temp_files.clear()
    
    # ========== 基础命令测试 ==========
    
    def test_login(self):
        """测试登录验证"""
        test_name = "登录验证"
        try:
            # 尝试列出根目录
            self.sftp.listdir('/')
            self.result.add_pass(test_name)
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_pwd(self):
        """测试获取当前目录"""
        test_name = "PWD 获取路径"
        try:
            pwd = self.sftp.getcwd()
            if pwd and isinstance(pwd, str):
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"返回空路径：{pwd}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    # ========== 目录操作测试 ==========
    
    def test_mkdir(self):
        """测试创建目录"""
        test_name = "MKDIR 创建目录"
        try:
            self.sftp.mkdir(self.test_dir)
            
            # 验证目录存在
            stat_info = self.sftp.stat(self.test_dir)
            if stat_info and stat.S_ISDIR(stat_info.st_mode):
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "创建的目录类型不正确")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_chdir(self):
        """测试切换目录"""
        test_name = "CHDIR 切换目录"
        try:
            self.sftp.chdir(self.test_dir)
            current = self.sftp.getcwd()
            
            if self.test_dir in current:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"路径不匹配：期望包含{self.test_dir}, 实际={current}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_rmdir(self):
        """测试删除目录"""
        test_name = "RMDIR 删除目录"
        try:
            # 先回到根目录
            self.sftp.chdir('/')
            self.sftp.rmdir(self.test_dir)
            self.result.add_pass(test_name)
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    # ========== 文件操作测试 ==========
    
    def test_put_file(self):
        """测试上传文件（带 MD5 校验）"""
        test_name = "PUT 上传文件"
        try:
            filename = f"{self.current_file_prefix}upload.txt"
            local_path = self.create_temp_file(filename)
            remote_filename = filename
            
            # 计算本地 MD5
            local_md5 = calculate_md5(local_path)
            
            # 上传文件
            self.sftp.put(local_path, remote_filename)
            
            # 验证文件存在
            try:
                stat_info = self.sftp.stat(remote_filename)
                if not stat_info or stat_info.st_size == 0:
                    raise Exception("文件为空")
            except FileNotFoundError:
                self.result.add_fail(test_name, "上传后文件不存在")
                return
            
            # 下载并校验 MD5
            import tempfile
            download_path = os.path.join(tempfile.gettempdir(), f"{filename}.verify")
            self.temp_files.append(download_path)
            
            self.sftp.get(remote_filename, download_path)
            download_md5 = calculate_md5(download_path)
            
            if local_md5 == download_md5:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"MD5 不匹配：本地={local_md5}, 远程={download_md5}")
        
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_get_file(self):
        """测试下载文件（带 MD5 校验）"""
        test_name = "GET 下载文件"
        try:
            filename = f"{self.current_file_prefix}upload.txt"
            import tempfile
            local_path = os.path.join(tempfile.gettempdir(), f"{filename}.download")
            self.temp_files.append(local_path)
            
            # 首次下载
            self.sftp.get(filename, local_path)
            
            if not os.path.exists(local_path) or os.path.getsize(local_path) == 0:
                self.result.add_fail(test_name, "下载文件为空或不存在")
                return
            
            original_md5 = calculate_md5(local_path)
            
            # 再次下载比对
            verify_path = os.path.join(tempfile.gettempdir(), f"{filename}.verify")
            self.temp_files.append(verify_path)
            
            self.sftp.get(filename, verify_path)
            verify_md5 = calculate_md5(verify_path)
            
            if original_md5 == verify_md5:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"MD5 不匹配：{original_md5} ≠ {verify_md5}")
        
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_list(self):
        """测试列出目录"""
        test_name = "LIST 列出目录"
        try:
            files = self.sftp.listdir('.')
            # 允许空目录
            self.result.add_pass(test_name)
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_stat(self):
        """测试获取文件属性"""
        test_name = "STAT 文件属性"
        try:
            filename = f"{self.current_file_prefix}upload.txt"
            stat_info = self.sftp.stat(filename)
            
            if stat_info and hasattr(stat_info, 'st_size'):
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "属性信息不完整")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_chmod(self):
        """测试修改文件权限"""
        test_name = "CHMOD 修改权限"
        try:
            filename = f"{self.current_file_prefix}upload.txt"
            
            # 获取当前权限
            old_stat = self.sftp.stat(filename)
            old_mode = old_stat.st_mode
            
            # 修改权限为 644
            self.sftp.chmod(filename, 0o644)
            
            # 验证权限已修改
            new_stat = self.sftp.stat(filename)
            if new_stat.st_mode != old_mode:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "权限未改变")
            
            # 恢复权限
            try:
                self.sftp.chmod(filename, old_mode)
            except Exception:
                pass
        
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_lstat(self):
        """测试符号链接属性"""
        test_name = "LSTAT 链接属性"
        try:
            filename = f"{self.current_file_prefix}lstat.txt"
            content = generate_test_content("lstat_test")
            local_path = self.create_temp_file(filename, content)
            
            # 上传文件
            self.sftp.put(local_path, filename)
            
            # 测试 lstat
            stat_info = self.sftp.lstat(filename)
            
            if stat_info:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "LSTAT 返回空")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_symlink(self):
        """测试创建符号链接"""
        test_name = "SYMLINK 符号链接"
        try:
            target = f"{self.current_file_prefix}symlink_target.txt"
            link = f"{self.current_file_prefix}symlink"
            
            # 创建目标文件
            content = generate_test_content("symlink_target")
            local_path = self.create_temp_file(target, content)
            self.sftp.put(local_path, target)
            
            # 创建符号链接
            self.sftp.symlink(target, link)
            
            # 验证链接存在
            try:
                stat_info = self.sftp.lstat(link)
                exists = stat_info is not None
            except Exception:
                exists = False
            
            if exists:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "符号链接创建失败")
            
            # 清理链接（保留目标文件）
            try:
                self.sftp.remove(link)
            except Exception:
                pass
        
        except Exception as e:
            self.result.add_fail(test_name, f"不支持符号链接：{e}")
    
    def test_readlink(self):
        """测试读取符号链接"""
        test_name = "READLINK 读取链接"
        try:
            target = f"{self.current_file_prefix}readlink_target.txt"
            link = f"{self.current_file_prefix}readlink"
            
            # 创建目标文件
            content = generate_test_content("readlink_target")
            local_path = self.create_temp_file(target, content)
            self.sftp.put(local_path, target)
            
            # 创建符号链接
            self.sftp.symlink(target, link)
            
            # 读取链接
            read_target = self.sftp.readlink(link)
            
            if read_target == target:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"链接目标不匹配：期望={target}, 实际={read_target}")
            
            # 清理链接
            try:
                self.sftp.remove(link)
            except Exception:
                pass
        
        except Exception as e:
            self.result.add_fail(test_name, f"不支持读取链接：{e}")
    
    def test_rename(self):
        """测试重命名文件"""
        test_name = "RENAME 重命名"
        try:
            old_name = f"{self.current_file_prefix}rename_old.txt"
            new_name = f"{self.current_file_prefix}rename_new.txt"
            
            # 上传测试文件
            content = generate_test_content("rename_test")
            local_path = self.create_temp_file(old_name, content)
            self.sftp.put(local_path, old_name)
            
            # 重命名
            self.sftp.rename(old_name, new_name)
            
            # 验证
            try:
                stat_info = self.sftp.stat(new_name)
                exists = stat_info is not None
                
                # 确认旧文件不存在
                try:
                    self.sftp.stat(old_name)
                    old_exists = True
                except FileNotFoundError:
                    old_exists = False
                
                if exists and not old_exists:
                    self.result.add_pass(test_name)
                else:
                    self.result.add_fail(test_name, "重命名后文件状态异常")
            
            except Exception:
                self.result.add_fail(test_name, "新文件不存在")
        
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_remove(self):
        """测试删除文件"""
        test_name = "REMOVE 删除文件"
        try:
            filename = f"{self.current_file_prefix}delete.txt"
            
            # 上传测试文件
            content = generate_test_content("delete_test")
            local_path = self.create_temp_file(filename, content)
            self.sftp.put(local_path, filename)
            
            # 删除
            self.sftp.remove(filename)
            
            # 验证已删除
            try:
                self.sftp.stat(filename)
                file_exists = True
            except FileNotFoundError:
                file_exists = False
            
            if not file_exists:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "删除后文件仍存在")
        
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def run_all_tests(self) -> TestResult:
        """运行所有 SFTP 测试"""
        print("\n" + "="*70)
        print("SFTP 协议功能测试")
        print("="*70)
        
        self.result.start_time = datetime.now()
        
        if not self.connect():
            self.result.add_fail("SFTP 连接", "无法连接到服务器")
            self.result.end_time = datetime.now()
            return self.result
        
        try:
            # 基础命令
            print("\n[基础命令]")
            self.test_login()
            self.test_pwd()
            
            # 目录操作
            print("\n[目录操作]")
            self.test_mkdir()
            self.test_chdir()
            
            # 文件操作
            print("\n[文件操作]")
            self.test_put_file()
            self.test_list()
            self.test_stat()
            self.test_chmod()
            self.test_lstat()
            self.test_get_file()
            self.test_rename()
            
            # 高级功能
            print("\n[高级功能]")
            self.test_symlink()
            self.test_readlink()
            
            # 清理
            print("\n[清理]")
            self.test_remove()
            self.test_rmdir()
        
        finally:
            self.disconnect()
            self.cleanup()
        
        self.result.end_time = datetime.now()
        return self.result


def print_summary(ftp_result: TestResult, sftp_result: TestResult):
    """打印测试摘要"""
    print("\n" + "="*70)
    print("测试结果汇总")
    print("="*70)
    
    total_tests = ftp_result.total + sftp_result.total
    total_passed = ftp_result.passed + sftp_result.passed
    total_failed = ftp_result.failed + sftp_result.failed
    success_rate = (total_passed / total_tests * 100) if total_tests > 0 else 0
    
    print(f"\n总体统计:")
    print(f"  总测试数：{total_tests}")
    print(f"  通过：{total_passed}")
    print(f"  失败：{total_failed}")
    print(f"  成功率：{success_rate:.2f}%")
    
    print(f"\nFTP 测试:")
    print(f"  测试数：{ftp_result.total}")
    print(f"  通过：{ftp_result.passed}")
    print(f"  失败：{ftp_result.failed}")
    print(f"  成功率：{(ftp_result.passed / ftp_result.total * 100) if ftp_result.total > 0 else 0:.2f}%")
    
    print(f"\nSFTP 测试:")
    print(f"  测试数：{sftp_result.total}")
    print(f"  通过：{sftp_result.passed}")
    print(f"  失败：{sftp_result.failed}")
    print(f"  成功率：{(sftp_result.passed / sftp_result.total * 100) if sftp_result.total > 0 else 0:.2f}%")
    
    # 显示错误详情
    all_errors = ftp_result.errors + sftp_result.errors
    if all_errors:
        print("\n错误详情:")
        for error in all_errors:
            print(f"  ✗ {error['test']}: {error['reason']}")


def save_json_result(ftp_result: TestResult, sftp_result: TestResult):
    """保存 JSON 结果"""
    result_data = {
        "timestamp": datetime.now().isoformat(),
        "config": {
            "host": Config.SERVER_HOST,
            "ftp_port": Config.FTP_PORT,
            "sftp_port": Config.SFTP_PORT,
            "username": Config.USERNAME
        },
        "ftp": ftp_result.get_summary(),
        "sftp": sftp_result.get_summary(),
        "overall": {
            "total_tests": ftp_result.total + sftp_result.total,
            "total_passed": ftp_result.passed + sftp_result.passed,
            "total_failed": ftp_result.failed + sftp_result.failed,
            "success_rate": f"{((ftp_result.passed + sftp_result.passed) / (ftp_result.total + sftp_result.total) * 100) if (ftp_result.total + sftp_result.total) > 0 else 0:.2f}%"
        }
    }
    
    output_path = os.path.join(os.path.dirname(__file__), 'test_result.json')
    with open(output_path, 'w', encoding='utf-8') as f:
        json.dump(result_data, f, ensure_ascii=False, indent=2)
    
    print(f"\n测试结果已保存到：{output_path}")


def main():
    """主函数"""
    print("="*70)
    print("WFTPG FTP/SFTP 完整功能测试")
    print("="*70)
    print(f"\n服务器配置:")
    print(f"  主机：{Config.SERVER_HOST}")
    print(f"  FTP 端口：{Config.FTP_PORT}")
    print(f"  SFTP 端口：{Config.SFTP_PORT}")
    print(f"  用户名：{Config.USERNAME}")
    print(f"  密码：{'*' * len(Config.PASSWORD)}")
    print("="*70)
    
    # 检查 Python 依赖
    print("\n检查环境依赖...")
    try:
        import paramiko
        print("  ✓ paramiko 已安装")
    except ImportError:
        print("  ✗ paramiko 未安装，请运行：pip install paramiko")
        sys.exit(1)
    
    # 检查端口是否就绪
    print("\n检查服务端口...")
    ports_to_check = [Config.FTP_PORT, Config.SFTP_PORT]
    
    if not wait_for_ports(Config.SERVER_HOST, ports_to_check, Config.PORT_WAIT_TIMEOUT):
        print("\n✗ 服务未就绪，请先启动 WFTPD 服务")
        print(f"  需要开放的端口：{', '.join(map(str, ports_to_check))}")
        sys.exit(1)
    
    try:
        # FTP 测试
        ftp_tester = FTPTester(
            host=Config.SERVER_HOST,
            port=Config.FTP_PORT,
            username=Config.USERNAME,
            password=Config.PASSWORD
        )
        ftp_result = ftp_tester.run_all_tests()
        
        # 测试间隔
        time.sleep(Config.TEST_DELAY)
        
        # SFTP 测试
        sftp_tester = SFTPTester(
            host=Config.SERVER_HOST,
            port=Config.SFTP_PORT,
            username=Config.USERNAME,
            password=Config.PASSWORD
        )
        sftp_result = sftp_tester.run_all_tests()
        
        # 打印摘要
        print_summary(ftp_result, sftp_result)
        
        # 保存 JSON 结果
        if Config.OUTPUT_JSON:
            save_json_result(ftp_result, sftp_result)
    
    except KeyboardInterrupt:
        print("\n\n✗ 测试被用户中断")
        sys.exit(1)
    except Exception as e:
        print(f"\n✗ 测试异常：{e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)
    
    print("\n" + "="*70)
    print("测试完成！")
    print("="*70)


if __name__ == '__main__':
    main()
