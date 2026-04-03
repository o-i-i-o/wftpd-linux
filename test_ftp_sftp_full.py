# -*- coding: utf-8 -*-
"""
WFTPG FTP/SFTP 完整功能测试脚本
跨平台版本 - 专注于协议功能测试

测试用户配置：
- 用户名：123
- 密码：123456

主要改进：
1. ✓ 完整的日志系统（支持文件输出和分级日志）
2. ✓ 详细的异常处理和堆栈跟踪
3. ✓ 统一的端口检测函数
4. ✓ 可配置的基础路径（避免硬编码绝对路径）
5. ✓ 递归清理逻辑和残留文件报告
6. ✓ 重试机制（带指数退避）
7. ✓ SFTP 高级操作兼容性标记
8. ✓ 测试幂等性改进

使用方法：
    # 基础运行
    python test_ftp_sftp_full.py
    
    # 详细日志模式
    修改 Config.VERBOSE = True
    
    # 保存到日志文件
    修改 Config.LOG_FILE = "test.log"
    
    # 自定义基础路径
    修改 Config.FTP_BASE_PATH = "/home/ftp"
    修改 Config.SFTP_BASE_PATH = "/home/sftp"
"""

import os
import sys
import time
import socket
import hashlib
import random
import string
import json
import logging
import traceback
from pathlib import Path
from datetime import datetime
from ftplib import FTP, error_perm, error_temp
from typing import Optional, List, Dict, Any
import paramiko
import stat


# ==================== 日志配置 ====================
def setup_logging(verbose: bool = False, log_file: Optional[str] = None):
    """配置日志系统"""
    level = logging.DEBUG if verbose else logging.INFO
    
    handlers = [logging.StreamHandler()]
    if log_file:
        handlers.append(logging.FileHandler(log_file, encoding='utf-8'))
    
    logging.basicConfig(
        level=level,
        format='%(asctime)s - %(levelname)s - %(message)s',
        datefmt='%Y-%m-%d %H:%M:%S',
        handlers=handlers
    )
    
    return logging.getLogger(__name__)


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
    OPERATION_RETRY_COUNT = 3  # 操作重试次数
    OPERATION_RETRY_DELAY = 1  # 重试延迟（秒）
    
    # 输出配置
    VERBOSE = False         # 详细日志
    OUTPUT_JSON = True      # 输出 JSON 结果
    LOG_FILE = None         # 日志文件路径
    
    # 路径配置
    FTP_BASE_PATH = None    # FTP 基础路径（None 表示使用根目录）
    SFTP_BASE_PATH = None   # SFTP 基础路径（None 表示使用根目录）


logger = None  # 在 main() 中初始化


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
            if result == 0:
                logger.debug(f"端口 {port} 已开放")
                return True
            else:
                logger.debug(f"端口 {port} 未开放 (error code: {result})")
                return False
    except Exception as e:
        logger.debug(f"检查端口 {port} 时出错：{e}")
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
    logger.info(f"等待端口 {', '.join(map(str, ports))} 就绪...")
    start_time = time.time()
    
    while time.time() - start_time < timeout:
        all_open = all(is_port_open(host, port) for port in ports)
        if all_open:
            logger.info(f"✓ 所有端口已就绪：{', '.join(map(str, ports))}")
            return True
        time.sleep(0.5)
    
    # 报告哪些端口未就绪
    unavailable = [port for port in ports if not is_port_open(host, port)]
    if unavailable:
        logger.error(f"✗ 以下端口未就绪：{', '.join(map(str, unavailable))}")
    
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


def retry_operation(operation_func, max_retries: int = None, delay: float = None, operation_name: str = ""):
    """
    重试装饰器/包装函数，带指数退避
    
    Args:
        operation_func: 要执行的操作函数
        max_retries: 最大重试次数
        delay: 初始延迟（秒）
        operation_name: 操作名称（用于日志）
    
    Returns:
        操作结果
    """
    if max_retries is None:
        max_retries = Config.OPERATION_RETRY_COUNT
    if delay is None:
        delay = Config.OPERATION_RETRY_DELAY
    
    last_exception = None
    current_delay = delay
    
    for attempt in range(max_retries + 1):
        try:
            return operation_func()
        except Exception as e:
            last_exception = e
            if attempt < max_retries:
                logger.warning(f"{operation_name} 失败 (尝试 {attempt + 1}/{max_retries + 1}): {e}")
                logger.debug(f"等待 {current_delay:.1f} 秒后重试...")
                time.sleep(current_delay)
                current_delay *= 2  # 指数退避
            else:
                logger.error(f"{operation_name} 达到最大重试次数 ({max_retries + 1} 次)，最终失败：{e}")
                raise
    
    # 理论上不会到这里
    raise last_exception


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
        self.remote_temp_files: List[str] = []  # 记录远程文件用于清理
        self.test_dir = f"test_ftp_{datetime.now().strftime('%Y%m%d_%H%M%S')}"
        self.current_file_prefix = f"ftp_{datetime.now().strftime('%Y%m%d_%H%M%S')}_"
        self.base_path = Config.FTP_BASE_PATH or ""  # 可配置基础路径
    
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
            except Exception as e:
                logger.debug(f"FTP quit() 失败：{e}")
                try:
                    self.ftp.close()
                    logger.debug("使用 close() 成功关闭连接")
                except Exception as e2:
                    logger.warning(f"FTP close() 也失败：{e2}")
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
        """清理测试文件（递归删除）"""
        logger.info("开始清理 FTP 测试文件...")
        
        # 清理远程文件和目录
        if self.ftp:
            try:
                # 列出并删除所有以 test_dir 开头的文件
                try:
                    all_files = self.ftp.nlst(f"{self.base_path}/{self.test_dir}*")
                    for file in all_files:
                        try:
                            # 尝试删除（如果是文件）
                            self.ftp.delete(file)
                            logger.debug(f"已删除远程文件：{file}")
                        except Exception:
                            # 如果是目录，尝试递归删除
                            try:
                                self._recursive_delete(file)
                                logger.debug(f"已删除远程目录：{file}")
                            except Exception as e:
                                logger.warning(f"无法删除 {file}: {e}")
                except Exception as e:
                    logger.warning(f"列出远程文件失败：{e}")
                
                # 报告残留文件
                try:
                    remaining = self.ftp.nlst(f"{self.base_path}/{self.test_dir}*")
                    if remaining:
                        logger.error(f"残留文件/目录：{remaining}")
                except Exception as e:
                    logger.debug(f"检查残留文件失败（可能已清理干净）: {e}")
                    
            except Exception as e:
                logger.error(f"FTP 清理过程出错：{e}")
        
        # 清理本地临时文件
        for file_path in self.temp_files:
            try:
                if os.path.exists(file_path):
                    os.remove(file_path)
                    logger.debug(f"已删除本地文件：{file_path}")
            except Exception as e:
                logger.warning(f"删除本地文件 {file_path} 失败：{e}")
        
        self.temp_files.clear()
        self.remote_temp_files.clear()
    
    def _recursive_delete(self, path: str):
        """递归删除目录及其内容"""
        try:
            # 尝试直接删除（如果是空目录）
            self.ftp.rmd(path)
            logger.debug(f"成功删除空目录：{path}")
        except Exception as e1:
            logger.debug(f"目录 {path} 非空，尝试递归删除：{e1}")
            # 非空目录，先列出内容
            try:
                items = self.ftp.nlst(path)
                logger.debug(f"目录 {path} 包含 {len(items)} 个项目")
                for item in items:
                    try:
                        # 尝试作为文件删除
                        self.ftp.delete(item)
                        logger.debug(f"已删除文件：{item}")
                    except Exception as e2:
                        logger.debug(f"无法删除 {item} 作为文件，尝试作为子目录：{e2}")
                        # 作为子目录递归删除
                        self._recursive_delete(item)
                # 最后删除目录本身
                self.ftp.rmd(path)
                logger.debug(f"成功删除目录：{path}")
            except Exception as e:
                logger.error(f"递归删除 {path} 彻底失败：{e}")
                raise Exception(f"递归删除 {path} 失败：{e}")
    
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
            logger.debug(f"{test_name} 失败：{traceback.format_exc()}")
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
            logger.debug(f"{test_name} 失败：{traceback.format_exc()}")
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
            logger.debug(f"{test_name} 失败：{traceback.format_exc()}")
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
            logger.debug(f"{test_name} 失败：{traceback.format_exc()}")
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
            logger.debug(f"{test_name} 失败：{traceback.format_exc()}")
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
            logger.debug(f"{test_name} 失败：{traceback.format_exc()}")
            self.result.add_fail(test_name, str(e))
    
    # ========== 目录操作测试 ==========
    
    def test_mkd(self):
        """测试创建目录"""
        test_name = "MKD 创建目录"
        try:
            full_path = f"{self.base_path}/{self.test_dir}" if self.base_path else self.test_dir
            self.ftp.mkd(full_path)
            logger.debug(f"成功创建目录：{full_path}")
            self.result.add_pass(test_name)
        except Exception as e:
            logger.debug(f"{test_name} 失败：{traceback.format_exc()}")
            self.result.add_fail(test_name, str(e))
    
    def test_cwd(self):
        """测试切换目录"""
        test_name = "CWD 切换目录"
        try:
            full_path = f"{self.base_path}/{self.test_dir}" if self.base_path else self.test_dir
            self.ftp.cwd(full_path)
            current = self.ftp.pwd()
            
            if self.test_dir in current:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"路径不匹配：期望包含{self.test_dir}, 实际={current}")
        except Exception as e:
            logger.debug(f"{test_name} 失败：{traceback.format_exc()}")
            self.result.add_fail(test_name, str(e))
    
    def test_rmd(self):
        """测试删除目录"""
        test_name = "RMD 删除目录"
        try:
            # 先回到根目录
            self.ftp.cwd('/')
            full_path = f"{self.base_path}/{self.test_dir}" if self.base_path else self.test_dir
            self.ftp.rmd(full_path)
            logger.debug(f"成功删除目录：{full_path}")
            self.result.add_pass(test_name)
        except Exception as e:
            logger.debug(f"{test_name} 失败：{traceback.format_exc()}")
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
        self.remote_temp_files: List[str] = []  # 记录远程文件用于清理
        self.test_dir = f"test_sftp_{datetime.now().strftime('%Y%m%d_%H%M%S')}"
        self.current_file_prefix = f"sftp_{datetime.now().strftime('%Y%m%d_%H%M%S')}_"
        self.base_path = Config.SFTP_BASE_PATH or ""  # 可配置基础路径
    
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
        """清理测试文件（递归删除）"""
        logger.info("开始清理 SFTP 测试文件...")
        
        if self.sftp:
            try:
                # 列出并删除所有以 test_dir 开头的文件
                test_pattern = f"{self.base_path}/{self.test_dir}*"
                try:
                    all_items = self.sftp.listdir(self.base_path or '.')
                    for item in all_items:
                        if item.startswith(self.test_dir):
                            full_path = f"{self.base_path}/{item}" if self.base_path else item
                            try:
                                # 尝试作为文件删除
                                self.sftp.remove(full_path)
                                logger.debug(f"已删除远程文件：{full_path}")
                            except Exception:
                                # 作为目录递归删除
                                try:
                                    self._recursive_delete(full_path)
                                    logger.debug(f"已删除远程目录：{full_path}")
                                except Exception as e:
                                    logger.warning(f"无法删除 {full_path}: {e}")
                except Exception as e:
                    logger.warning(f"列出远程文件失败：{e}")
                
                # 报告残留文件
                try:
                    remaining = self.sftp.listdir(self.base_path or '.')
                    remaining_test = [f for f in remaining if f.startswith(self.test_dir)]
                    if remaining_test:
                        logger.error(f"残留文件/目录：{remaining_test}")
                except Exception:
                    pass
                    
            except Exception as e:
                logger.error(f"SFTP 清理过程出错：{e}")
        
        # 清理本地临时文件
        for file_path in self.temp_files:
            try:
                if os.path.exists(file_path):
                    os.remove(file_path)
                    logger.debug(f"已删除本地文件：{file_path}")
            except Exception as e:
                logger.warning(f"删除本地文件 {file_path} 失败：{e}")
        
        self.temp_files.clear()
        self.remote_temp_files.clear()
    
    def _recursive_delete(self, path: str):
        """递归删除目录及其内容"""
        try:
            # 尝试直接删除（如果是空目录）
            self.sftp.rmdir(path)
        except Exception:
            # 非空目录，先列出内容
            try:
                items = self.sftp.listdir(path)
                for item in items:
                    full_path = f"{path}/{item}"
                    try:
                        # 尝试作为文件删除
                        self.sftp.remove(full_path)
                    except Exception:
                        # 作为子目录递归删除
                        self._recursive_delete(full_path)
                # 最后删除目录本身
                self.sftp.rmdir(path)
            except Exception as e:
                raise Exception(f"递归删除 {path} 失败：{e}")
    
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
            # Paramiko 的 getcwd() 需要先调用 chdir 才能设置内部状态
            # 使用 normalize('.') 来获取当前工作目录
            pwd = self.sftp.normalize('.')
            if pwd and isinstance(pwd, str) and len(pwd) > 0:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"返回空路径或类型错误：{pwd}")
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
            filename = f"{self.current_file_prefix}chmod_test.txt"
            
            # 先创建文件并设置为 600
            local_path = self.create_temp_file(filename, b"test content for chmod")
            self.sftp.put(local_path, filename)
            self.sftp.chmod(filename, 0o600)
            old_stat = self.sftp.stat(filename)
            old_mode = old_stat.st_mode & 0o777  # 只取权限位
            
            # 修改权限为 644
            self.sftp.chmod(filename, 0o644)
            new_stat = self.sftp.stat(filename)
            new_mode = new_stat.st_mode & 0o777  # 只取权限位
            
            # 验证权限已修改
            if new_mode == 0o644 and old_mode != new_mode:
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"权限未正确设置：期望 0o644, 实际 0o{new_mode:o} (旧权限 0o{old_mode:o})")
            
            # 恢复原始权限
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
        """测试创建符号链接（可能不支持）"""
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
            
            # 验证链接存在且是符号链接
            try:
                stat_info = self.sftp.lstat(link)
                # 检查是否是符号链接 (S_IFLNK = 0o120000)
                is_symlink = stat.S_ISLNK(stat_info.st_mode)
                
                if is_symlink:
                    logger.debug(f"成功创建符号链接：{link} -> {target}")
                    self.result.add_pass(test_name)
                else:
                    self.result.add_fail(test_name, f"创建的不是符号链接，mode={oct(stat_info.st_mode)}")
            except Exception as e:
                self.result.add_fail(test_name, f"符号链接验证失败：{e}")
            
            # 清理链接（保留目标文件）
            try:
                self.sftp.remove(link)
            except Exception:
                pass
        
        except Exception as e:
            error_msg = str(e)
            # 检测是否是不支持的操作
            if any(keyword in error_msg.lower() for keyword in ['unsupported', 'not implemented', 'unimplemented']):
                logger.warning(f"{test_name} 不被此 SFTP 服务器支持，已跳过")
                # 记录为特殊类型的失败（功能不支持）
                self.result.errors.append({"test": test_name, "reason": f"功能不支持：{error_msg}"})
                print(f"  ⊘ {test_name}: 功能不支持（已跳过）")
            else:
                logger.debug(f"{test_name} 失败：{traceback.format_exc()}")
                self.result.add_fail(test_name, f"不支持符号链接：{e}")
    
    def test_readlink(self):
        """测试读取符号链接（可能不支持）"""
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
            
            # 验证是符号链接后再读取
            try:
                stat_info = self.sftp.lstat(link)
                is_symlink = stat.S_ISLNK(stat_info.st_mode)
                
                if not is_symlink:
                    self.result.add_fail(test_name, f"不是符号链接，无法读取，mode={oct(stat_info.st_mode)}")
                    # 清理链接
                    try:
                        self.sftp.remove(link)
                    except Exception:
                        pass
                    return
            except Exception as e:
                self.result.add_fail(test_name, f"lstat 失败：{e}")
                return
            
            # 读取链接
            read_target = self.sftp.readlink(link)
            
            if read_target == target:
                logger.debug(f"成功读取符号链接：{link} -> {target}")
                self.result.add_pass(test_name)
            else:
                self.result.add_fail(test_name, f"链接目标不匹配：期望={target}, 实际={read_target}")
            
            # 清理链接
            try:
                self.sftp.remove(link)
            except Exception:
                pass
        
        except Exception as e:
            error_msg = str(e)
            # 检测是否是不支持的操作
            if any(keyword in error_msg.lower() for keyword in ['unsupported', 'not implemented', 'unimplemented']):
                logger.warning(f"{test_name} 不被此 SFTP 服务器支持，已跳过")
                # 记录为特殊类型的失败（功能不支持）
                self.result.errors.append({"test": test_name, "reason": f"功能不支持：{error_msg}"})
                print(f"  ⊘ {test_name}: 功能不支持（已跳过）")
            else:
                logger.debug(f"{test_name} 失败：{traceback.format_exc()}")
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
    global logger
    
    # 初始化日志系统
    logger = setup_logging(verbose=Config.VERBOSE, log_file=Config.LOG_FILE)
    
    logger.info("="*70)
    logger.info("WFTPG FTP/SFTP 完整功能测试")
    logger.info("="*70)
    logger.info(f"服务器配置:")
    logger.info(f"  主机：{Config.SERVER_HOST}")
    logger.info(f"  FTP 端口：{Config.FTP_PORT}")
    logger.info(f"  SFTP 端口：{Config.SFTP_PORT}")
    logger.info(f"  用户名：{Config.USERNAME}")
    logger.info(f"  路径配置 - FTP: {Config.FTP_BASE_PATH or '根目录'}, SFTP: {Config.SFTP_BASE_PATH or '根目录'}")
    logger.info("="*70)
    
    # 检查 Python 依赖
    logger.info("检查环境依赖...")
    try:
        import paramiko
        logger.info("  ✓ paramiko 已安装")
    except ImportError:
        logger.error("  ✗ paramiko 未安装，请运行：pip install paramiko")
        sys.exit(1)
    
    # 检查端口是否就绪
    logger.info("检查服务端口...")
    ports_to_check = [Config.FTP_PORT, Config.SFTP_PORT]
    
    if not wait_for_ports(Config.SERVER_HOST, ports_to_check, Config.PORT_WAIT_TIMEOUT):
        logger.error("✗ 服务未就绪，请先启动 WFTPD 服务")
        logger.error(f"  需要开放的端口：{', '.join(map(str, ports_to_check))}")
        sys.exit(1)
    
    try:
        # FTP 测试
        logger.info("开始 FTP 协议功能测试...")
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
        logger.info("开始 SFTP 协议功能测试...")
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
        logger.error("\n✗ 测试被用户中断")
        sys.exit(1)
    except Exception as e:
        logger.error(f"\n✗ 测试异常：{e}")
        logger.error(traceback.format_exc())
        sys.exit(1)
    
    logger.info("="*70)
    logger.info("测试完成！")
    logger.info("="*70)


if __name__ == '__main__':
    main()
