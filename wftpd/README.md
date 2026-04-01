# WFTPD - 纯后端 FTP/SFTP 服务

WFTPD 是一个轻量级的 FTP 和 SFTP 服务器管理工具，完全使用 Rust 编写。此版本已移除所有前端相关代码，成为独立但完整的纯后端程序。

## 主要特性

- ✅ **FTP 服务器** - 支持主动/被动模式、TLS 加密、速度限制
- ✅ **SFTP 服务器** - 基于 SSH2 协议、支持公钥认证
- ✅ **配置管理** - 通过 TOML 配置文件控制服务行为
- ✅ **用户管理** - JSON 格式的用户数据库，支持密码哈希
- ✅ **日志记录** - 文件操作审计日志、系统日志
- ✅ **安全控制** - IP 白名单/黑名单、连接数限制、登录尝试限制

## 架构设计

```
wftpd/
├── src/
│   ├── bin/
│   │   └── wftpd.rs          # 主程序入口
│   ├── core/                  # 核心功能模块
│   │   ├── config.rs         # 配置管理
│   │   ├── users.rs          # 用户管理
│   │   ├── file_logger.rs    # 文件日志
│   │   ├── logger.rs         # 日志管理
│   │   └── tracing_logger.rs # 分布式追踪
│   ├── server/                # 服务器实现
│   │   ├── ftp/              # FTP 服务器模块
│   │   │   ├── server.rs
│   │   │   ├── handler.rs
│   │   │   └── commands/     # FTP 命令实现
│   │   └── sftp/             # SFTP 服务器模块
│   │       ├── server.rs
│   │       ├── handler.rs
│   │       └── packet.rs
│   └── lib.rs                 # 库根（AppState）
├── Cargo.toml
└── config_template.toml
```

## 编译与运行

### 编译

```bash
cd /home/GGFWZX/Desktop/wftpg/wftpd
cargo build --release
```

### 运行

直接运行：
```bash
sudo ./target/release/wftpd
```

或使用 systemd 服务：
```bash
sudo systemctl start wftpd
sudo systemctl enable wftpd  # 开机自启
```

## 配置文件

配置文件位于 `/etc/wftpg/config.toml`：

```toml
[ftp]
enabled = true
bind_ip = "0.0.0.0"
port = 2121
passive_ports = [50000, 51000]
welcome_message = "Welcome to WFTPG FTP Server"
allow_anonymous = false
max_speed_kbps = 0
encoding = "UTF-8"

[sftp]
enabled = true
bind_ip = "0.0.0.0"
port = 2222
host_key_path = "/var/lib/wftpg/ssh/ssh_host_rsa_key"
max_auth_attempts = 3
auth_timeout = 60

[security]
allowed_ips = ["0.0.0.0/0"]
denied_ips = []
max_login_attempts = 5
ban_duration = 300
max_connections = 100

[logging]
log_dir = "/var/log/wftpg"
log_level = "info"
max_log_size = 10485760
max_log_files = 10
enable_json = false
```

## 用户配置

用户数据位于 `/etc/wftpg/users.json`：

```json
{
  "users": {
    "username": {
      "password_hash": "argon2...",
      "home_dir": "/path/to/home",
      "permissions": {
        "read": true,
        "write": true,
        "delete": true,
        "rename": true,
        "create_dir": true,
        "delete_dir": true
      },
      "quota_bytes": 0
    }
  }
}
```

## 测试

使用提供的测试脚本：

```bash
chmod +x test_backend.sh
./test_backend.sh
```

手动测试 FTP：
```bash
curl ftp://localhost:2121/
```

手动测试 SFTP：
```bash
sftp -P 2222 user@localhost
```

## 与服务端交互

由于移除了 IPC 通信模块，现在服务直接由命令行启动和管理：

- **启动服务**: `sudo ./target/release/wftpd`
- **停止服务**: Ctrl+C 或 `sudo systemctl stop wftpd`
- **重启服务**: `sudo systemctl restart wftpd`
- **查看状态**: `sudo systemctl status wftpd`
- **查看日志**: `journalctl -u wftpd -f` 或查看 `/var/log/wftpg/` 目录

## 主要变化（相比 GUI 版本）

### 移除的模块
- ❌ UI 模块（GTK 前端）
- ❌ Communication 模块（IPC 客户端/服务端）
- ❌ Service 模块（服务守护进程）
- ❌ ServerManager（服务器管理器）
- ❌ Logger（UI 日志缓冲）

### 简化的 AppState
```rust
pub struct AppState {
    pub config: Arc<Mutex<Config>>,
    pub user_manager: Arc<Mutex<UserManager>>,
    pub file_logger: Arc<Mutex<FileLogger>>,
    pub ftp_server: Option<FtpServer>,
    pub sftp_server: Option<SftpServer>,
}
```

### 独立的 FTP/SFTP 服务
FTP 和 SFTP 服务现在作为独立组件，可以通过配置文件控制是否启动：
- `ftp.enabled = true/false` - 控制是否启动 FTP
- `sftp.enabled = true/false` - 控制是否启动 SFTP
- 可以同时启动，也可以只启动其中一个

## 依赖项

主要依赖：
- `tokio` - 异步运行时
- `russh` - SSH/SFTP 协议实现
- `rustls` - TLS 支持
- `serde` + `toml` - 配置序列化
- `tracing` - 日志记录
- `argon2` - 密码哈希

## 安全性

- 密码使用 Argon2 算法哈希存储
- 支持 FTPS (FTP over TLS)
- SFTP 使用 SSH2 协议，支持 Ed25519/RSA 密钥
- IP 访问控制列表（白名单/黑名单）
- 连接数和登录尝试限制
- 防止暴力破解攻击

## 许可证

本项目采用 MIT 许可证。

## 贡献

欢迎提交 Issue 和 Pull Request！

## 联系方式

如有问题或建议，请通过 GitHub Issues 联系我们。
