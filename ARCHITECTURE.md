# WFTPG 架构说明

## 架构概述

WFTPG 采用 **前后端分离** 的架构设计，确保后台服务可以独立于 GUI 管理程序持续运行。

### 核心组件

```
┌─────────────────┐         ┌──────────────────┐
│  wftp-gui       │         │   wftpd.service  │
│  GTK 管理界面   │◄───────►│   systemd 守护进程│
│  (前端)         │  Unix   │   (后端)         │
│                 │  Socket │                  │
└─────────────────┘         └──────────────────┘
                                   │
                            ┌──────┴──────┐
                            │             │
                         FTP Server   SFTP Server
```

## 组件说明

### 1. wftpd (后台守护进程)

**运行方式:**
- 由 systemd 管理和自动启动
- 系统启动后自动运行（如果已启用）
- 关闭 GUI 不会影响其运行

**主要功能:**
- ✅ 提供 FTP/SFTP 文件传输服务
- ✅ 通过 Unix Socket 提供 IPC 接口
- ✅ 处理所有业务逻辑（配置管理、用户管理、日志记录）
- ✅ 持久化运行，不受 GUI 启停影响

**配置文件位置:**
- 主配置：`/etc/wftpg/config.toml`
- 用户数据：`/var/lib/wftpg/users.json`
- 日志文件：`/var/log/wftpg/`

### 2. wftp-gui (前端管理界面)

**运行方式:**
- 用户手动启动和关闭
- 仅作为 IPC 客户端，不持有服务状态
- 通过 IPC 与 wftpd 通信获取实时状态

**主要功能:**
- ✅ 配置管理（通过 IPC 读写配置文件）
- ✅ 用户管理（通过 IPC 操作用户数据）
- ✅ 状态展示（从 IPC 获取服务状态）
- ✅ 日志查看（从 IPC 获取日志数据）
- ❌ **不直接控制服务启停**（由 systemd 管理）

**重要：**
- GUI 关闭时**不会停止**后台服务
- 所有配置修改通过 IPC 发送给 wftpd 执行
- 仅作为"控制面板"，不是服务的必要条件

### 3. systemd 服务管理

**服务名称:** `wftpd.service`

**关键配置:**
```ini
Type=simple           # 简单的前台运行模式
Restart=always        # 总是自动重启
RestartSec=3          # 重启间隔 3 秒
TimeoutStopSec=10     # 优雅关闭超时 10 秒
```

**管理命令:**
```bash
# 启动服务
sudo systemctl start wftpd

# 停止服务
sudo systemctl stop wftpd

# 重启服务
sudo systemctl restart wftpd

# 查看状态
systemctl status wftpd

# 启用开机自启
sudo systemctl enable wftpd

# 禁用开机自启
sudo systemctl disable wftpd

# 查看日志
journalctl -u wftpd -f
```

**便捷工具:**
使用提供的 `wftpgctl` 脚本：
```bash
sudo ./debian/wftpgctl start    # 启动
sudo ./debian/wftpgctl stop     # 停止
sudo ./debian/wftpgctl restart  # 重启
./debian/wftpgctl status        # 查看状态
```

## 工作流程

### 启动流程

```
系统启动
    │
    ▼
systemd 自动启动 wftpd.service (如果已 enable)
    │
    ├──► wftpd 读取配置文件
    ├──► wftpd 加载用户数据
    ├──► 启动 FTP Server (如果配置启用)
    ├──► 启动 SFTP Server (如果配置启用)
    └──► 启动 IPC Server (Unix Socket)
    
用户启动 wftp-gui
    │
    └──► 连接到 IPC Socket
         └──► 获取当前状态
              └──► 显示管理界面
```

### 配置修改流程

```
用户在 GUI 修改配置
    │
    ▼
GUI 通过 IPC 发送配置更新请求
    │
    ▼
wftpd 接收请求并验证
    │
    ├──► 写入配置文件 (/etc/wftpg/config.toml)
    ├──► 更新内存中的配置
    └──► 返回成功响应
    
GUI 收到响应并显示成功消息
```

### 服务重启流程

```
需要重启服务
    │
    ▼
方法 1: 使用 systemctl
    sudo systemctl restart wftpd

方法 2: 使用 wftpgctl 工具
    sudo ./wftpgctl restart

方法 3: 在 GUI 中点击"重启服务"按钮
    (实际调用 systemctl restart wftpd)
    │
    ▼
systemd 停止 wftpd
    │
    ▼
systemd 重新启动 wftpd
    │
    └──► wftpd 根据最新配置启动服务
```

## 权限管理

### Unix Socket 权限

IPC Socket 路径：`/run/wftpd/wftpg.sock`

**访问权限:**
- 所有者：`root:wftpg`
- 权限位：`0660` (所有者和组可读写)
- 允许访问的用户：
  - `root` 用户
  - `wftpg` 组成员

**创建 wftpg 组并添加用户:**
```bash
# 创建组
sudo groupadd wftpg

# 将用户添加到组
sudo usermod -aG wftpg $USER

# 注销并重新登录后生效
```

## 目录结构

```
/etc/wftpg/              # 配置文件目录
├── config.toml          # 主配置文件
└── users.json           # 用户数据

/var/lib/wftpg/          # 运行时数据目录
├── cache/               # 缓存目录
└── config/              # 配置链接

/var/log/wftpg/          # 日志目录
├── wftpg-YYYY-MM-DD.log # 系统日志
└── file-ops-*.log       # 文件操作日志

/run/wftpd/              # 运行时目录
└── wftpg.sock           # IPC Socket
```

## 安全特性

### 1. 权限隔离

- wftpd 以 `wftpg` 用户身份运行（非 root）
- 通过 capabilities 机制授予必要的网络权限
- 使用 `NoNewPrivileges=true` 防止提权

### 2. 文件系统保护

- `ProtectSystem=strict`: 系统目录只读
- `ProtectHome=false`: 允许访问用户家目录（FTP/SFTP 需要）
- `PrivateTmp=yes`: 独立的临时目录
- `ReadWritePaths`: 明确指定可写路径

### 3. IPC 安全

- Unix Socket 限制访问权限（0660）
- 基于 UID/GID 的权限检查
- 仅允许 wftpg 组成员访问

## 常见问题

### Q: 为什么关闭 GUI 后服务还在运行？

**A:** 这是设计行为。wftpd 是由 systemd 管理的独立服务，GUI 只是一个管理界面。就像关闭了"服务管理器"窗口不会停止 Windows 服务一样。

### Q: 如何完全停止 FTP/SFTP 服务？

**A:** 使用 systemd 命令：
```bash
sudo systemctl stop wftpd
```

### Q: 配置修改后立即生效吗？

**A:** 是的。配置通过 IPC 发送给 wftpd 后会立即更新到内存和磁盘，无需重启服务。

### Q: 如何查看服务日志？

**A:** 有三种方式：
1. 在 GUI 的"日志查看"标签页查看（推荐）
2. 使用 journalctl: `journalctl -u wftpd -f`
3. 直接查看日志文件：`tail -f /var/log/wftpg/wftpg-*.log`

### Q: 服务无法启动怎么办？

**A:** 按以下步骤排查：
1. 查看详细错误：`sudo journalctl -u wftpd -n 50`
2. 检查配置文件语法：`cat /etc/wftpg/config.toml`
3. 检查端口占用：`sudo netstat -tlnp | grep :21`
4. 检查权限：`ls -la /run/wftpd/`

## 开发说明

### 代码结构

```
src/
├── bin/
│   ├── wftp-gui.rs      # GUI 入口
│   └── wftpd.rs         # 守护进程入口
├── core/                # 核心功能模块
│   ├── config.rs        # 配置管理
│   ├── users.rs         # 用户管理
│   ├── server_manager.rs# 服务器管理
│   └── ...
├── server/              # FTP/SFTP 服务器实现
│   ├── ftp/
│   └── sftp/
├── communication/       # IPC 通信
│   ├── server.rs        # IPC 服务端
│   ├── client.rs        # IPC 客户端
│   └── protocol.rs      # 通信协议
└── ui/                  # GUI 界面
    ├── main_window.rs
    └── ...
```

### IPC 协议

基于 JSON-RPC 的简化版本：

**请求格式:**
```json
{
  "id": 1,
  "command": "GetStatus"
}
```

**响应格式:**
```json
{
  "id": 1,
  "result": {
    "ftp_running": true,
    "sftp_running": true
  }
}
```

### 添加新的 IPC 命令

1. 在 `communication/protocol.rs` 定义命令枚举
2. 在 `communication/server.rs` 实现处理函数
3. 在 `communication/client.rs` 添加客户端封装
4. 在 UI 中调用客户端 API

## 总结

WFTPG 的架构设计遵循 Linux 守护进程的最佳实践：

✅ **前后端分离**: GUI 仅作为管理界面，不影响后台服务
✅ **systemd 集成**: 利用 systemd 实现自动启动、重启、日志管理
✅ **权限最小化**: 非 root 运行，限制文件系统访问
✅ **模块化设计**: 清晰的职责划分，易于维护和扩展

这种架构确保了服务的**高可用性**和**易管理性**，适合生产环境部署。
