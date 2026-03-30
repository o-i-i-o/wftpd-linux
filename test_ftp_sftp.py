# -*- coding: utf-8 -*-
"""
WFTPG FTP/SFTP 完整功能测试脚本
测试用户：123 / 密码：123456
"""

import os
import sys
import time
import subprocess
import tempfile
import hashlib
import socket
import random
import string
from pathlib import Path
from datetime import datetime
from ftplib import FTP, error_perm, error_temp
import paramiko
import stat
import json


# ==================== 集中配置区域 ====================
# 服务器配置
SERVER_HOST = '127.0.0.1'      # 服务器 IP 地址
FTP_PORT = 2121                   # FTP 端口
SFTP_PORT = 2222                # SFTP 端口

# 用户认证配置
TEST_USERNAME = '123'          # 测试用户名
TEST_PASSWORD = '123456'       # 测试密码

# 测试配置
CONNECTION_TIMEOUT = 10         # 连接超时时间（秒）
PORT_WAIT_TIMEOUT = 30          # 等待端口就绪超时时间（秒）
SERVICE_START_DELAY = 3         # 服务启动后等待时间（秒）
BETWEEN_TESTS_DELAY = 2         # FTP 和 SFTP 测试之间的延迟（秒）

# 日志与输出
ENABLE_VERBOSE_LOG = False      # 是否启用详细日志
OUTPUT_JSON_RESULT = True       # 是否输出 JSON 结果文件
# =================================================


def generate_unique_filename(prefix='', suffix='.txt'):
    """生成唯一的文件名（时间戳 + 随机字符串）"""
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


def is_port_in_use(host, port):
    """检测端口是否被占用"""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.settimeout(2)
        result = sock.connect_ex((host, port))
        return result == 0


def wait_for_port(host, port, timeout=30):
    """等待端口可用，返回 True 表示端口已开放，False 表示超时"""
    start_time = time.time()
    while time.time() - start_time < timeout:
        if is_port_in_use(host, port):
            return True
        time.sleep(0.5)
    return False


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


class FTPTester:
    """FTP 功能测试"""
    
    def __init__(self, host=SERVER_HOST, port=FTP_PORT, username=TEST_USERNAME, password=TEST_PASSWORD):
        self.host = host
        self.port = port
        self.username = username
        self.password = password
        self.ftp = None
        self.result = TestResult()
        self.test_dir = '/test_ftp_' + datetime.now().strftime('%Y%m%d_%H%M%S')
        self.temp_files = []
        # 使用唯一文件名
        self.unique_prefix = generate_unique_filename(prefix='ftp_', suffix='') + '_'
        self.encoding_exceptions_handled = 0
    
    def connect(self):
        """建立 FTP 连接"""
        try:
            self.ftp = FTP()
            self.ftp.connect(self.host, self.port, timeout=CONNECTION_TIMEOUT)
            self.ftp.login(self.username, self.password)
            self.ftp.set_pasv(True)
            print("FTP 连接成功")
            return True
        except Exception as e:
            print(f"FTP 连接失败：{e}")
            return False
    
    def disconnect(self):
        """断开 FTP 连接"""
        if self.ftp:
            try:
                self.ftp.quit()
            except:
                pass
    
    def cleanup(self):
        """清理测试文件"""
        if not self.ftp:
            return
        
        try:
            # 删除测试目录
            try:
                self.ftp.rmd(self.test_dir)
            except:
                pass
            
            # 删除本地临时文件
            for f in self.temp_files:
                try:
                    if os.path.exists(f):
                        os.remove(f)
                except:
                    pass
        except:
            pass
    
    def test_login(self):
        """测试登录"""
        test_name = "登录验证"
        try:
            # 已登录，发送 NOOP 命令测试
            response = self.ftp.voidcmd('NOOP')
            if '200' in response:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"NOOP 响应异常：{response}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_pwd(self):
        """测试获取当前目录"""
        test_name = "PWD 命令"
        try:
            pwd = self.ftp.pwd()
            if pwd:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "返回空路径")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
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
                self.result.add_fail(test_name, f"切换后路径不正确：{current}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_put_file(self):
        """测试上传文件（带 MD5 校验）"""
        test_name = "STOR 上传文件（MD5 校验）"
        try:
            # 创建测试文件（使用唯一文件名）
            unique_filename = self.unique_prefix + 'upload.txt'
            test_content = f"FTP 测试文件内容 - {datetime.now().isoformat()}\n随机数据：{''.join(random.choices(string.ascii_letters + string.digits, k=100))}"
            local_file = os.path.join(tempfile.gettempdir(), unique_filename)
            with open(local_file, 'w', encoding='utf-8') as f:
                f.write(test_content)
            
            self.temp_files.append(local_file)
            
            # 计算本地文件 MD5
            local_md5 = calculate_md5(local_file)
            
            # 上传文件
            remote_file = unique_filename
            with open(local_file, 'rb') as f:
                self.ftp.storbinary(f'STOR {remote_file}', f)
            
            # 验证文件存在
            files = self.ftp.nlst()
            if remote_file not in files:
                self.result.add_fail(test_name, "文件上传后不存在")
                return
            
            # 下载文件并校验 MD5
            download_file = os.path.join(tempfile.gettempdir(), unique_filename + '.download')
            self.temp_files.append(download_file)
            with open(download_file, 'wb') as f:
                self.ftp.retrbinary(f'RETR {remote_file}', f.write)
            
            download_md5 = calculate_md5(download_file)
            
            if local_md5 == download_md5:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"MD5 校验失败：本地={local_md5}, 远程={download_md5}")
        except UnicodeDecodeError as e:
            # Windows 编码兼容处理
            self.encoding_exceptions_handled += 1
            self.result.add_fail(test_name, f"编码异常（已记录）：{e}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_get_file(self):
        """测试下载文件（带 MD5 校验）"""
        test_name = "RETR 下载文件（MD5 校验）"
        try:
            # 使用之前上传的文件
            remote_file = self.unique_prefix + 'upload.txt'
            unique_filename = remote_file + '.download'
            local_file = os.path.join(tempfile.gettempdir(), unique_filename)
            self.temp_files.append(local_file)
            
            # 先获取远程文件 MD5（通过下载临时文件计算）
            temp_file = os.path.join(tempfile.gettempdir(), unique_filename + '.tmp')
            self.temp_files.append(temp_file)
            with open(temp_file, 'wb') as f:
                self.ftp.retrbinary(f'RETR {remote_file}', f.write)
            
            remote_md5 = calculate_md5(temp_file)
            
            # 重新下载用于 MD5 比对
            with open(local_file, 'wb') as f:
                self.ftp.retrbinary(f'RETR {remote_file}', f.write)
            
            # 验证文件大小和内容
            if os.path.exists(local_file) and os.path.getsize(local_file) > 0:
                local_md5 = calculate_md5(local_file)
                if local_md5 == remote_md5:
                    self.result.add_pass(test_name)
                else:
                    self.result.add_fail(test_name, f"MD5 校验失败：本地={local_md5}, 远程={remote_md5}")
            else:
                self.result.add_fail(test_name, "下载的文件为空或不存在")
        except UnicodeDecodeError as e:
            self.encoding_exceptions_handled += 1
            self.result.add_fail(test_name, f"编码异常（已记录）：{e}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_list(self):
        """测试列出目录"""
        test_name = "LIST 列出目录"
        try:
            result = []
            self.ftp.retrlines('LIST', result.append)
            if len(result) > 0:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "目录列表为空")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_nlst(self):
        """测试简单列出"""
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
        test_name = "SIZE 获取文件大小"
        try:
            remote_file = self.unique_prefix + 'upload.txt'
            size = self.ftp.size(remote_file)
            if size is not None and size > 0:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"文件大小异常：{size}")
        except UnicodeDecodeError as e:
            self.encoding_exceptions_handled += 1
            self.result.add_fail(test_name, f"编码异常（已记录）：{e}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_mdtm(self):
        """测试获取文件修改时间"""
        test_name = "MDTM 获取修改时间"
        try:
            remote_file = self.unique_prefix + 'upload.txt'
            mdtm = self.ftp.sendcmd(f'MDTM {remote_file}')
            if '213' in mdtm:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"MDTM 响应异常：{mdtm}")
        except UnicodeDecodeError as e:
            self.encoding_exceptions_handled += 1
            self.result.add_fail(test_name, f"编码异常（已记录）：{e}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_rename(self):
        """测试重命名文件"""
        test_name = "RNFR/RNTO 重命名"
        try:
            old_name = self.unique_prefix + 'upload.txt'
            new_name = self.unique_prefix + 'renamed.txt'
            
            # 检查文件是否存在，不存在则重新上传
            files = self.ftp.nlst()
            if old_name not in files:
                # 重新上传
                test_content = f'Rename test content - {datetime.now().isoformat()}'
                local_file = os.path.join(tempfile.gettempdir(), self.unique_prefix + 'rename_test.txt')
                with open(local_file, 'w', encoding='utf-8') as f:
                    f.write(test_content)
                self.temp_files.append(local_file)
                
                with open(local_file, 'rb') as f:
                    self.ftp.storbinary(f'STOR {old_name}', f)
            
            self.ftp.rename(old_name, new_name)
            
            # 验证新名称存在
            files = self.ftp.nlst()
            if new_name in files and old_name not in files:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "重命名后文件状态异常")
                return  # 文件不存在，跳过恢复原名
                
            # 恢复原名
            try:
                self.ftp.rename(new_name, old_name)
            except:
                pass
        except UnicodeDecodeError as e:
            self.encoding_exceptions_handled += 1
            self.result.add_fail(test_name, f"编码异常（已记录）：{e}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_delete(self):
        """测试删除文件"""
        test_name = "DELE 删除文件"
        try:
            # 尝试多个可能的文件名
            possible_files = [
                self.unique_prefix + 'upload.txt',
                self.unique_prefix + 'renamed.txt'
            ]
            remote_file = None
            
            files = self.ftp.nlst()
            for file in possible_files:
                if file in files:
                    remote_file = file
                    break
            
            if remote_file is None:
                # 如果没有找到文件，重新上传一个用于测试
                remote_file = self.unique_prefix + 'delete.txt'
                test_content = f'Delete test content - {datetime.now().isoformat()}'
                local_file = os.path.join(tempfile.gettempdir(), remote_file)
                with open(local_file, 'w', encoding='utf-8') as f:
                    f.write(test_content)
                self.temp_files.append(local_file)
                
                with open(local_file, 'rb') as f:
                    self.ftp.storbinary(f'STOR {remote_file}', f)
            
            self.ftp.delete(remote_file)
            
            # 验证文件已删除
            files = self.ftp.nlst()
            if remote_file not in files:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "文件删除后仍然存在")
        except UnicodeDecodeError as e:
            self.encoding_exceptions_handled += 1
            self.result.add_fail(test_name, f"编码异常（已记录）：{e}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_rmd(self):
        """测试删除目录"""
        test_name = "RMD 删除目录"
        try:
            # 先切换到父目录
            self.ftp.cwd('/')
            self.ftp.rmd(self.test_dir)
            self.result.add_pass(test_name)
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_syst(self):
        """测试系统类型查询"""
        test_name = "SYST 系统类型"
        try:
            syst = self.ftp.sendcmd('SYST')
            if '215' in syst:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"SYST 响应异常：{syst}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_feat(self):
        """测试功能查询"""
        test_name = "FEAT 功能列表"
        try:
            feat = self.ftp.sendcmd('FEAT')
            if '211' in feat:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"FEAT 响应异常：{feat}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_type(self):
        """测试传输类型设置"""
        test_name = "TYPE 设置类型"
        try:
            # 设置为 ASCII 模式
            resp = self.ftp.voidcmd('TYPE A')
            if '200' in resp:
                # 设置回 Binary 模式
                resp = self.ftp.voidcmd('TYPE I')
                if '200' in resp:
                    self.result.add_pass(test_name)
                else:
                    self.result.add_fail(test_name, f"Binary 模式设置失败：{resp}")
            else:
                self.result.add_fail(test_name, f"ASCII 模式设置失败：{resp}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_noop(self):
        """测试保持连接"""
        test_name = "NOOP 保持连接"
        try:
            resp = self.ftp.voidcmd('NOOP')
            if '200' in resp:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"NOOP 响应异常：{resp}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def run_all_tests(self):
        """运行所有 FTP 测试"""
        print("\n" + "="*60)
        print("开始 FTP 功能测试")
        print("="*60)
        
        self.result.start_time = datetime.now()
        
        if not self.connect():
            self.result.add_fail("FTP 连接", "无法连接到 FTP 服务器")
            self.result.end_time = datetime.now()
            return self.result
        
        try:
            # 基础命令测试
            self.test_login()
            self.test_pwd()
            self.test_syst()
            self.test_feat()
            self.test_type()
            self.test_noop()
            
            # 目录操作测试
            self.test_mkd()
            self.test_cwd()
            
            # 文件操作测试
            self.test_put_file()
            self.test_list()
            self.test_nlst()
            self.test_size()
            self.test_mdtm()
            self.test_get_file()
            self.test_rename()
            self.test_delete()
            
            # 清理测试
            self.test_rmd()
            
        finally:
            self.disconnect()
            self.cleanup()
        
        self.result.end_time = datetime.now()
        return self.result


class SFTPTester:
    """SFTP 功能测试"""
    
    def __init__(self, host=SERVER_HOST, port=SFTP_PORT, username=TEST_USERNAME, password=TEST_PASSWORD):
        self.host = host
        self.port = port
        self.username = username
        self.password = password
        self.sftp = None
        self.ssh = None
        self.result = TestResult()
        self.test_dir = 'test_sftp_' + datetime.now().strftime('%Y%m%d_%H%M%S')
        self.temp_files = []
        # 使用唯一文件名
        self.unique_prefix = generate_unique_filename(prefix='sftp_', suffix='') + '_'
    
    def connect(self):
        """建立 SFTP 连接"""
        try:
            self.ssh = paramiko.SSHClient()
            self.ssh.set_missing_host_key_policy(paramiko.AutoAddPolicy())
            self.ssh.connect(
                hostname=self.host,
                port=self.port,
                username=self.username,
                password=self.password,
                timeout=CONNECTION_TIMEOUT
            )
            self.sftp = self.ssh.open_sftp()
            print("SFTP 连接成功")
            return True
        except Exception as e:
            print(f"SFTP 连接失败：{e}")
            return False
    
    def disconnect(self):
        """断开 SFTP 连接"""
        if self.sftp:
            try:
                self.sftp.close()
            except:
                pass
        if self.ssh:
            try:
                self.ssh.close()
            except:
                pass
    
    def cleanup(self):
        """清理测试文件"""
        if not self.sftp:
            return
        
        try:
            # 删除测试目录
            try:
                self.sftp.rmdir(self.test_dir)
            except:
                pass
            
            # 删除本地临时文件
            for f in self.temp_files:
                try:
                    if os.path.exists(f):
                        os.remove(f)
                except:
                    pass
        except:
            pass
    
    def test_login(self):
        """测试登录"""
        test_name = "登录验证"
        try:
            # 尝试列出目录
            self.sftp.listdir('/')
            self.result.add_pass(test_name)
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_pwd(self):
        """测试获取当前目录"""
        test_name = "PWD 获取路径"
        try:
            pwd = self.sftp.getcwd()
            if pwd:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "返回空路径")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
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
                self.result.add_fail(test_name, "创建的目录不是目录类型")
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
                self.result.add_fail(test_name, f"切换后路径不正确：{current}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_put_file(self):
        """测试上传文件（带 MD5 校验）"""
        test_name = "PUT 上传文件（MD5 校验）"
        try:
            # 创建测试文件（使用唯一文件名）
            unique_filename = self.unique_prefix + 'upload.txt'
            test_content = f"SFTP 测试文件内容 - {datetime.now().isoformat()}\n随机数据：{''.join(random.choices(string.ascii_letters + string.digits, k=100))}"
            local_file = os.path.join(tempfile.gettempdir(), unique_filename)
            with open(local_file, 'w', encoding='utf-8') as f:
                f.write(test_content)
            
            self.temp_files.append(local_file)
            
            # 计算本地文件 MD5
            local_md5 = calculate_md5(local_file)
            
            # 上传文件 - 使用相对路径（因为可能在 test_dir 内）
            remote_file = unique_filename
            self.sftp.put(local_file, remote_file)
            
            # 验证文件存在
            stat_info = self.sftp.stat(remote_file)
            if not (stat_info and stat_info.st_size > 0):
                self.result.add_fail(test_name, "文件上传后不存在或为空")
                return
            
            # 下载文件并校验 MD5
            download_file = os.path.join(tempfile.gettempdir(), unique_filename + '.download')
            self.temp_files.append(download_file)
            self.sftp.get(remote_file, download_file)
            
            download_md5 = calculate_md5(download_file)
            
            if local_md5 == download_md5:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"MD5 校验失败：本地={local_md5}, 远程={download_md5}")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_get_file(self):
        """测试下载文件（带 MD5 校验）"""
        test_name = "GET 下载文件（MD5 校验）"
        try:
            # 使用之前上传的文件
            remote_file = self.unique_prefix + 'upload.txt'
            unique_filename = self.unique_prefix + 'upload.txt.download'
            local_file = os.path.join(tempfile.gettempdir(), unique_filename)
            self.temp_files.append(local_file)
            
            # 先获取远程文件 MD5（通过下载临时文件计算）
            temp_file = os.path.join(tempfile.gettempdir(), unique_filename + '.tmp')
            self.temp_files.append(temp_file)
            self.sftp.get(remote_file, temp_file)
            
            remote_md5 = calculate_md5(temp_file)
            
            # 重新下载用于 MD5 比对
            self.sftp.get(remote_file, local_file)
            
            # 验证文件大小和内容
            if os.path.exists(local_file) and os.path.getsize(local_file) > 0:
                local_md5 = calculate_md5(local_file)
                if local_md5 == remote_md5:
                    self.result.add_pass(test_name)
                else:
                    self.result.add_fail(test_name, f"MD5 校验失败：本地={local_md5}, 远程={remote_md5}")
            else:
                self.result.add_fail(test_name, "下载的文件为空或不存在")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_list(self):
        """测试列出目录"""
        test_name = "LIST 列出目录"
        try:
            files = self.sftp.listdir('.')
            if len(files) >= 0:  # 允许空目录
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "目录列表异常")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_stat(self):
        """测试获取文件属性"""
        test_name = "STAT 获取属性"
        try:
            remote_file = self.unique_prefix + 'upload.txt'
            stat_info = self.sftp.stat(remote_file)
            
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
            remote_file = self.unique_prefix + 'upload.txt'
            
            # 获取当前权限
            stat_info = self.sftp.stat(remote_file)
            old_mode = stat_info.st_mode
            
            # 修改权限
            self.sftp.chmod(remote_file, 0o644)
            
            # 验证权限已修改
            stat_info = self.sftp.stat(remote_file)
            if stat_info.st_mode != old_mode:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "权限未改变")
                
            # 恢复权限
            try:
                self.sftp.chmod(remote_file, old_mode)
            except:
                pass
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_rename(self):
        """测试重命名文件"""
        test_name = "RENAME 重命名"
        try:
            old_name = self.unique_prefix + 'upload.txt'
            new_name = self.unique_prefix + 'renamed.txt'
            
            # 检查文件是否存在，不存在则重新上传
            try:
                self.sftp.stat(old_name)
            except FileNotFoundError:
                # 文件不存在，重新上传
                local_file = os.path.join(tempfile.gettempdir(), 'test_rename_upload.txt')
                with open(local_file, 'w', encoding='utf-8') as f:
                    f.write('rename test content')
                self.temp_files.append(local_file)
                self.sftp.put(local_file, old_name)
            
            self.sftp.rename(old_name, new_name)
            
            # 验证新名称存在
            try:
                stat_info = self.sftp.stat(new_name)
                exists = stat_info is not None
            except:
                exists = False
            
            if exists:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "重命名后文件不存在")
                return  # 文件不存在，跳过恢复原名
                
            # 恢复原名
            try:
                self.sftp.rename(new_name, old_name)
            except:
                pass
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_remove(self):
        """测试删除文件"""
        test_name = "REMOVE 删除文件"
        try:
            # 尝试多个可能的文件名
            possible_files = [
                self.unique_prefix + 'upload.txt',
                self.unique_prefix + 'renamed.txt'
            ]
            
            remote_file = None
            for file in possible_files:
                try:
                    self.sftp.stat(file)
                    remote_file = file
                    break
                except FileNotFoundError:
                    continue
            
            if remote_file is None:
                # 如果没有找到文件，重新上传一个用于测试
                remote_file = 'test_delete.txt'
                local_file = os.path.join(tempfile.gettempdir(), 'test_remove_upload.txt')
                with open(local_file, 'w', encoding='utf-8') as f:
                    f.write('remove test content')
                self.temp_files.append(local_file)
                self.sftp.put(local_file, remote_file)
            
            self.sftp.remove(remote_file)
            
            # 验证文件已删除
            try:
                self.sftp.stat(remote_file)
                file_exists = True
            except FileNotFoundError:
                file_exists = False
            
            if not file_exists:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "文件删除后仍然存在")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_rmdir(self):
        """测试删除目录"""
        test_name = "RMDIR 删除目录"
        try:
            # 先切换到父目录
            self.sftp.chdir('/')
            self.sftp.rmdir(self.test_dir)
            self.result.add_pass(test_name)
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_lstat(self):
        """测试符号链接属性（如果支持）"""
        test_name = "LSTAT 链接属性"
        try:
            # 对于普通文件，lstat 应该返回与 stat 相同的结果
            remote_file = 'test_lstat.txt'
            
            # 重新创建文件（因为之前可能删除了）
            local_file = os.path.join(tempfile.gettempdir(), remote_file)
            with open(local_file, 'w', encoding='utf-8') as f:
                f.write('test')
            self.temp_files.append(local_file)
            self.sftp.put(local_file, remote_file)
            
            stat_info = self.sftp.lstat(remote_file)
            if stat_info:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "LSTAT 返回空")
        except Exception as e:
            self.result.add_fail(test_name, str(e))
    
    def test_symlink(self):
        """测试创建符号链接（如果支持）"""
        test_name = "SYMLINK 符号链接"
        try:
            target = 'test_symlink_target.txt'
            link = 'test_link'
            
            # 检查目标文件是否存在，不存在则重新上传
            try:
                self.sftp.stat(target)
            except FileNotFoundError:
                # 文件不存在，重新上传
                local_file = os.path.join(tempfile.gettempdir(), 'test_symlink_upload.txt')
                with open(local_file, 'w', encoding='utf-8') as f:
                    f.write('symlink test content')
                self.temp_files.append(local_file)
                self.sftp.put(local_file, target)
            
            self.sftp.symlink(target, link)
            
            # 验证链接存在
            try:
                stat_info = self.sftp.lstat(link)
                exists = stat_info is not None
            except:
                exists = False
            
            if exists:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, "符号链接创建失败")
            
            # 删除链接（不删除目标文件）
            try:
                self.sftp.remove(link)
            except:
                pass
        except Exception as e:
            self.result.add_fail(test_name, f"不支持符号链接：{e}")
    
    def test_readlink(self):
        """测试读取符号链接（如果支持）"""
        test_name = "READLINK 读取链接"
        try:
            target = 'test_readlink_target.txt'
            link = 'test_link2'
            
            # 检查目标文件是否存在，不存在则重新上传
            try:
                self.sftp.stat(target)
            except FileNotFoundError:
                # 文件不存在，重新上传
                local_file = os.path.join(tempfile.gettempdir(), 'test_readlink_upload.txt')
                with open(local_file, 'w', encoding='utf-8') as f:
                    f.write('readlink test content')
                self.temp_files.append(local_file)
                self.sftp.put(local_file, target)
            
            # 创建链接
            self.sftp.symlink(target, link)
            
            # 读取链接
            read_target = self.sftp.readlink(link)
            
            if read_target == target:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"读取的链接目标不匹配：{read_target}")
            
            # 删除链接（不删除目标文件）
            try:
                self.sftp.remove(link)
            except:
                pass
        except Exception as e:
            self.result.add_fail(test_name, f"不支持读取链接：{e}")
    
    def run_all_tests(self):
        """运行所有 SFTP 测试"""
        print("\n" + "="*60)
        print("开始 SFTP 功能测试")
        print("="*60)
        
        self.result.start_time = datetime.now()
        
        if not self.connect():
            self.result.add_fail("SFTP 连接", "无法连接到 SFTP 服务器")
            self.result.end_time = datetime.now()
            return self.result
        
        try:
            # 基础命令测试
            self.test_login()
            self.test_pwd()
            
            # 目录操作测试
            self.test_mkdir()
            self.test_chdir()
            
            # 文件操作测试
            self.test_put_file()
            self.test_list()
            self.test_stat()
            self.test_chmod()
            self.test_lstat()
            self.test_get_file()
            self.test_rename()
            
            # 高级功能测试
            self.test_symlink()
            self.test_readlink()
            
            # 清理测试
            self.test_remove()
            self.test_rmdir()
            
        finally:
            self.disconnect()
            self.cleanup()
        
        self.result.end_time = datetime.now()
        return self.result


class WFTPDManager:
    """WFTPD 服务管理器（仅用于检测端口）"""
    
    def __init__(self):
        self.process = None
        self.wftpd_path = None
    
    def find_wftpd(self):
        """查找 wftpd.exe 路径"""
        # 在构建目录中查找
        possible_paths = [
            Path(__file__).parent / 'target' / 'release' / 'wftpd.exe',
            Path(__file__).parent / 'target' / 'debug' / 'wftpd.exe',
            Path(__file__).parent / 'wftpd.exe',
            Path('C:\\ProgramData\\wftpg\\wftpd.exe'),
        ]
        
        for path in possible_paths:
            if path.exists():
                self.wftpd_path = str(path)
                return True
        
        return False
    
    def check_service_running(self):
        """检查服务是否已在运行"""
        ftp_port_in_use = is_port_in_use(SERVER_HOST, FTP_PORT)
        sftp_port_in_use = is_port_in_use(SERVER_HOST, SFTP_PORT)
        
        if ftp_port_in_use and sftp_port_in_use:
            print(f"[PASS] FTP ({FTP_PORT}) 和 SFTP ({SFTP_PORT}) 端口均已就绪")
            return True
        elif ftp_port_in_use or sftp_port_in_use:
            print(f"[警告] 部分端口被占用 - FTP: {'是' if ftp_port_in_use else '否'}, SFTP: {'是' if sftp_port_in_use else '否'}")
            print("[提示] 可能存在端口冲突，请检查是否有其他实例在运行")
            return False
        else:
            print(f"[错误] FTP ({FTP_PORT}) 和 SFTP ({SFTP_PORT}) 端口均未就绪")
            print("\n请先手动启动 WFTPD 服务：")
            print("1. 以管理员身份运行：target/release/wftpd.exe")
            print("2. 或者运行 GUI 程序启动服务")
            print("3. 启动完成后重新运行此测试脚本")
            return False
    
    def stop_service(self):
        """停止 wftpd 服务（如果是由本脚本启动的）"""
        if self.process:
            try:
                self.process.terminate()
                self.process.wait(timeout=5)
                print("wftpd 服务已停止")
            except Exception as e:
                print(f"停止 wftpd 失败：{e}")
                try:
                    self.process.kill()
                except:
                    pass


def check_prerequisites():
    """检查前置条件"""
    print("检查测试环境...")
    
    # 检查 Python 依赖
    required_packages = ['paramiko']
    missing_packages = []
    
    try:
        import paramiko
    except ImportError:
        missing_packages.append('paramiko')
    
    if missing_packages:
        print(f"错误：缺少必要的 Python 包：{', '.join(missing_packages)}")
        print("请运行：pip install " + " ".join(missing_packages))
        return False
    
    print("  [PASS] Python 依赖检查通过")
    return True


def main():
    """主函数"""
    print("="*60)
    print("WFTPG FTP/SFTP 完整功能测试")
    print("测试用户：123 / 密码：123456")
    print("="*60)
    
    # 检查前置条件
    if not check_prerequisites():
        sys.exit(1)
    
    # 初始化结果统计
    total_result = {
        "start_time": datetime.now().isoformat(),
        "ftp": None,
        "sftp": None,
        "summary": {}
    }
    
    wftpd_manager = WFTPDManager()
    
    try:
        # 检查服务是否已在运行
        print("\n" + "="*60)
        print("步骤 1: 检查 WFTPD 服务状态")
        print("="*60)
        
        service_running = wftpd_manager.check_service_running()
        
        if not service_running:
            print("\n测试无法继续，请先启动 WFTPD 服务")
            sys.exit(1)
        
        # FTP 测试
        print("\n" + "="*60)
        print("步骤 2: FTP 功能测试")
        print("="*60)
        
        ftp_tester = FTPTester(host=SERVER_HOST, port=FTP_PORT, username=TEST_USERNAME, password=TEST_PASSWORD)
        ftp_result = ftp_tester.run_all_tests()
        total_result["ftp"] = ftp_result.get_summary()
        
        # 等待一下再进行 SFTP 测试
        time.sleep(BETWEEN_TESTS_DELAY)
        
        # SFTP 测试
        print("\n" + "="*60)
        print("步骤 3: SFTP 功能测试")
        print("="*60)
        
        sftp_tester = SFTPTester(host=SERVER_HOST, port=SFTP_PORT, username=TEST_USERNAME, password=TEST_PASSWORD)
        sftp_result = sftp_tester.run_all_tests()
        total_result["sftp"] = sftp_result.get_summary()
        
        # 汇总结果
        print("\n" + "="*60)
        print("测试结果汇总")
        print("="*60)
        
        total_passed = ftp_result.passed + sftp_result.passed
        total_failed = ftp_result.failed + sftp_result.failed
        total_tests = ftp_result.total + sftp_result.total
        success_rate = (total_passed / total_tests * 100) if total_tests > 0 else 0
        
        print(f"\n总测试数：{total_tests}")
        print(f"通过：{total_passed}")
        print(f"失败：{total_failed}")
        print(f"成功率：{success_rate:.2f}%")
        
        print("\n--- FTP 测试结果 ---")
        print(f"测试数：{ftp_result.total}")
        print(f"通过：{ftp_result.passed}")
        print(f"失败：{ftp_result.failed}")
        print(f"成功率：{(ftp_result.passed / ftp_result.total * 100) if ftp_result.total > 0 else 0:.2f}%")
        
        print("\n--- SFTP 测试结果 ---")
        print(f"测试数：{sftp_result.total}")
        print(f"通过：{sftp_result.passed}")
        print(f"失败：{sftp_result.failed}")
        print(f"成功率：{(sftp_result.passed / sftp_result.total * 100) if sftp_result.total > 0 else 0:.2f}%")
        
        if ftp_result.errors or sftp_result.errors:
            print("\n--- 错误详情 ---")
            for error in ftp_result.errors + sftp_result.errors:
                print(f"  [FAIL] {error['test']}: {error['reason']}")
        
        # 保存测试结果到 JSON 文件
        total_result["end_time"] = datetime.now().isoformat()
        total_result["summary"] = {
            "total_tests": total_tests,
            "total_passed": total_passed,
            "total_failed": total_failed,
            "success_rate": f"{success_rate:.2f}%"
        }
        
        if OUTPUT_JSON_RESULT:
            result_file = os.path.join(os.path.dirname(__file__), 'test_result.json')
            with open(result_file, 'w', encoding='utf-8') as f:
                json.dump(total_result, f, ensure_ascii=False, indent=2)
            
            print(f"\n测试结果已保存到：{result_file}")
        
    except KeyboardInterrupt:
        print("\n\n测试被用户中断")
    except Exception as e:
        print(f"\n测试过程中发生异常：{e}")
        import traceback
        traceback.print_exc()
    finally:
        # 停止服务
        print("\n" + "="*60)
        print("清理环境")
        print("="*60)
        wftpd_manager.stop_service()
        print("测试完成！")


if __name__ == '__main__':
    main()
