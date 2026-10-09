# WFTPD — 桌面级 FTP/SFTP 服务套件

WFTPD 是一个基于 Rust 的 FTP/FTPS/SFTP 服务器套件，采用**前后端分离**架构：

- **wftpd** — 后端守护进程，运行 FTP/SFTP 服务并通过 gRPC（Unix Domain Socket）暴露管理接口；
- **wftp-gui** — GTK 桌面前端，用于管理服务器配置、用户、服务启停与日志查看。

前后端进程级隔离：GUI 不持有任何服务状态，所有写入均通过 gRPC 提交给后端；后端不依赖任何 GUI 组件，可脱离前端独立运行。

## 主要特性

- **FTP / FTPS** — 基于 libunftp，支持主/被动模式、TLS 加密（可强制）、编码转换、限速
- **SFTP** — 基于 russh/russh-sftp，SSH2 协议，Ed25519 主机密钥（缺失时自动生成）
- **用户管理** — JSON 用户库，Argon2id 密码哈希（参数自动升级），目录配额与带宽限制
- **细粒度权限** — 读/写/删除/列目录/建目录/删目录/改名/续传逐项控制
- **安全控制** — IP 白名单/黑名单（CIDR）、登录尝试限制与封禁、并发与空闲超时
- **审计日志** — 文件操作审计日志、运行日志（环形缓冲，GUI 实时查看）
- **服务管理** — systemd 用户服务，GUI 内可启停/重启/开机自启

## 架构通信

```
┌──────────────┐  UDS gRPC (tonic)   ┌─────────────────────┐
│  wftp-gui    │ ──────────────────► │  wftpd 守护进程      │
│  GTK 前端    │  /run/user/<uid>/   │  · gRPC 控制面       │
│  (配置/监控) │   wftpd.sock        │  · FTP/FTPS (2121)  │
└──────────────┘  套接字权限 0600     │  · SFTP     (2222)  │
                                     └─────────────────────┘
```

后端依据 `config.toml` 中各协议的 `enabled` 开关决定是否启动对应服务，两者可独立启停。

## 编译

需要 Rust（edition 2024）及 GTK 3 开发头文件：

```bash
sudo apt install libgtk-3-dev pkg-config
cargo build --release
```

产物：

- `target/release/wftpd` — 后端守护进程（含 FTP + SFTP，按配置启停）
- `target/release/wftp-gui` — 桌面管理前端

### deb 打包

```bash
./debian/build-deb.sh
```

生成的 deb 包含两个二进制、systemd 用户单元及预置配置（安装时拷贝到
`~/.config/wftpd/config.toml` 并替换其中的 `~` 为实际主目录）。

## 运行

后端以当前桌面用户的 systemd 用户服务运行，无需 root：

```bash
systemctl --user start wftpd      # 启动
systemctl --user enable wftpd     # 登录后自启
sudo loginctl enable-linger $USER # 未登录也保持运行
```

前端直接启动：`wftp-gui`（或 `./target/release/wftp-gui`）。

## 配置文件

配置文件按以下顺序解析：

1. 环境变量 `WFTPD_CONFIG_DIR` 指定的 `config.toml`（测试/多实例覆盖）；
2. 二进制同目录 `config.toml`——直接运行编译产物（如 `./target/release/wftpd`）
   时默认配置就生成在这里，便于测试；
3. 用户配置目录 `~/.config/wftpd/config.toml`（deb 安装形态）。

文件不存在时自动落盘一份默认配置。完整示例（与默认值一致）：

```toml
[ftp]
enabled = true
bind_ip = "0.0.0.0"
port = 2121
passive_ports = [50000, 51000]
welcome_message = "Welcome to WFTPG FTP Server"
allow_anonymous = false
# anonymous_home = "/srv/ftp"     # 启用匿名访问时必填且必须是存在的目录
max_speed_kbps = 0                # 0 = 不限速
encoding = "UTF-8"
data_timeout = 300
# masquerade_ip = "203.0.113.10"  # 被动模式对外公告的 IP

[sftp]
enabled = true
bind_ip = "0.0.0.0"
port = 2222
host_key_path = "~/.local/state/wftpd/ssh/ssh_host_ed25519_key"
max_auth_attempts = 3
auth_timeout = 60
log_level = "info"

[security]
allowed_ips = ["0.0.0.0/0"]
denied_ips = []                   # 黑名单优先于白名单
max_login_attempts = 5
ban_duration = 300
require_ssl = false               # FTPS：强制 TLS（拒绝未升级的客户端）
# cert_path = "/path/to/cert"     # FTPS 证书与私钥
# key_path = "/path/to/key"
max_connections = 100
connection_timeout = 300
idle_timeout = 600

[logging]
log_dir = "~/.local/state/wftpd/logs"
log_level = "info"
max_log_size = 10485760
max_log_files = 10
enable_json = false
enable_gui_logging = true         # 允许日志实时推送到前端
```

## 用户配置

用户数据位于 `~/.config/wftpd/users.json`（与 GUI 共享同一份类型定义）：

```json
{
  "users": {
    "alice": {
      "username": "alice",
      "password_hash": "$argon2id$v=19$m=19456,t=2,p=1$...",
      "home_dir": "/home/alice/ftp",
      "permissions": {
        "can_read": true,
        "can_write": true,
        "can_delete": true,
        "can_list": true,
        "can_mkdir": true,
        "can_rmdir": true,
        "can_rename": true,
        "can_append": true,
        "quota_mb": 1024,
        "speed_limit_kbps": null
      },
      "created_at": "2026-03-14T06:50:12.347486227Z",
      "last_login": null,
      "enabled": true,
      "is_admin": false
    }
  }
}
```

`quota_mb = 0` 表示不限制配额；`speed_limit_kbps = null` 表示不限制带宽。
密码哈希参数低于当前策略时，用户登录成功后会自动升级哈希参数。

## 数据目录（XDG）

| 路径 | 内容 |
|---|---|
| `~/.config/wftpd/` | `config.toml`、`users.json` |
| `~/.local/state/wftpd/logs/` | 运行日志、文件操作审计日志 |
| `~/.local/state/wftpd/ssh/` | SFTP 主机密钥 |
| `/run/user/<uid>/wftpd.sock` | 前后端 gRPC 控制套接字（0600） |

## 测试

```bash
cargo test --workspace
```

手动验证服务（后端启动后）：

```bash
curl ftp://localhost:2121/
sftp -P 2222 alice@localhost
```

## 主要依赖

- `tokio` — 异步运行时
- `libunftp` / `unftp-core` — FTP 协议实现
- `russh` + `russh-sftp` — SSH/SFTP 协议实现
- `tonic` + `prost` — gRPC（前后端 IPC）
- `gtk` + `glib` — 桌面前端
- `rustls` — TLS 支持
- `argon2` — 密码哈希
- `serde` + `toml` — 配置与用户数据序列化
- `tracing` — 结构化日志

## 安全性

- 密码使用 Argon2id 哈希存储，参数随策略自动升级
- FTPS（FTP over TLS）支持强制模式；SFTP 使用 SSH2，支持 Ed25519/RSA 主机密钥
- IP 访问控制列表（CIDR 格式，黑名单优先）
- 登录尝试次数限制与自动封禁
- 连接数、连接超时、空闲超时多重限制
- 目录配额与带宽限速（FTP/SFTP 共用同一套设施）
- 用户被 chroot 到各自主目录，无法越权访问
- 配置与用户数据原子写入（临时文件 + rename），避免写坏
- 前后端仅通过本机 UDS 通信，套接字权限 0600

## 许可证

本项目采用 GPL v3 许可证，详见 [Cargo.toml](Cargo.toml) 中的 `license` 字段。
