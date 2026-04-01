# WFTPG GUI 无头模式使用指南

## 📋 概述

本指南介绍如何在无头（Headless）环境中运行 WFTPG GUI，使用 Xvfb（X Virtual Frame Buffer）创建虚拟显示环境。

## ✅ 前置条件

1. **已安装 Xvfb**
   ```bash
   sudo apt install xvfb  # Debian/Ubuntu
   sudo yum install xorg-x11-server-Xvfb  # RHEL/CentOS
   ```

2. **wftpd 后台服务正在运行**
   ```bash
   ./target/release/wftpd &
   ```

3. **GUI 已编译完成**
   ```bash
   cargo build --release
   ```

## 🚀 快速启动

### 启动 GUI（无头模式）

```bash
./start-headless-gui.sh
```

启动后会显示：
- ✅ Xvfb 进程信息（PID, Display: :99）
- ✅ wftp-gui 进程信息
- ✅ IPC 连接测试方法

### 查看运行状态

```bash
# 查看进程
ps aux | grep -E 'Xvfb|wftp-gui'

# 输出示例：
# root  1080850  0.6  0.5 693000 44780 pts/12 Sl 17:02 0:00 Xvfb :99 -screen 0 1920x1080x24
# root  1080873  1.3  0.5 861164 43956 pts/12 Sl 17:02 0:00 /home/GGFWZX/Desktop/wftpg/target/release/wftp-gui
```

### 测试 IPC 连接

```bash
DISPLAY=:99 python3 test_socket.py
```

## 🛑 停止 GUI

```bash
./stop-headless-gui.sh
```

## ⚙️ 配置选项

### 修改虚拟显示参数

编辑 `start-headless-gui.sh`：

```bash
XVFB_DISPLAY=":99"              # 显示编号
XVFB_SCREEN="1920x1080x24"      # 分辨率@色深
```

### 支持的参数

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `XVFB_DISPLAY` | `:99` | 虚拟显示编号 |
| `XVFB_SCREEN` | `1920x1080x24` | 屏幕分辨率和色深 |

## 🔍 故障排查

### 问题 1：Xvfb 启动失败

**症状：** 脚本报错 "Xvfb 启动失败"

**解决：**
```bash
# 检查端口是否被占用
netstat -tuln | grep :99

# 手动清理旧进程
pkill -9 Xvfb
pkill -9 wftp-gui

# 重试
./start-headless-gui.sh
```

### 问题 2：GUI 无法连接 IPC Socket

**症状：** GUI 提示 "IPC socket not found"

**解决：**
```bash
# 检查 socket 文件
ls -la /run/wftpd/wftpg.sock

# 检查权限（应该是 0666）
stat -c "%a" /run/wftpd/wftpg.sock

# 如果权限不对，重启 wftpd 服务
pkill wftpd
./target/release/wftpd &
```

### 问题 3：GUI 进程崩溃

**症状：** GUI 进程意外退出

**解决：**
```bash
# 查看详细错误
DISPLAY=:99 ./target/release/wftp-gui

# 常见错误：
# - Gtk-WARNING: cannot open display → DISPLAY 环境变量未设置
# - IPC error → wftpd 服务未运行
```

## 📊 进程管理

### 查看所有相关进程

```bash
ps aux | grep -E 'wftpd|Xvfb|wftp-gui' | grep -v grep
```

### 手动清理所有进程

```bash
pkill -9 wftpd
pkill -9 Xvfb
pkill -9 wftp-gui
```

### 检查 PID 文件

```bash
ls -la /tmp/wftp-gui.pid /tmp/xvfb-wftpg.pid
cat /tmp/wftp-gui.pid
cat /tmp/xvfb-wftpg.pid
```

## 🎯 使用场景

### 场景 1：自动化测试

```bash
#!/bin/bash
# 启动服务
./target/release/wftpd &
sleep 2

# 启动 GUI（无头）
./start-headless-gui.sh &
sleep 3

# 运行测试脚本
python3 automated_tests.py

# 清理
./stop-headless-gui.sh
pkill wftpd
```

### 场景 2：CI/CD 集成

```yaml
# GitHub Actions 示例
- name: Setup Xvfb
  run: sudo apt-get install xvfb

- name: Start GUI in headless mode
  run: |
    ./start-headless-gui.sh &
    sleep 5

- name: Run tests
  run: python3 test_gui_functionality.py
```

### 场景 3：远程服务器管理

```bash
# SSH 连接到远程服务器
ssh user@server

# 启动 GUI（无头）
./start-headless-gui.sh

# 通过 IPC 工具管理
python3 manage_wftpg.py --status
python3 manage_wftpg.py --restart-services
```

## �� 技术细节

### Xvfb 工作原理

Xvfb（X Virtual Frame Buffer）是一个 X server，它在内存中执行所有的绘图操作，不需要显示输出设备。

- **优点：**
  - 无需物理显示器
  - 支持多实例隔离
  - 资源占用低
  
- **缺点：**
  - 无法直接看到界面（需要截图或 VNC）
  - 不支持硬件加速

### IPC 通信机制

```
wftp-gui (GTK)
     ↓
Unix Socket (/run/wftpd/wftpg.sock)
     ↓
wftpd (Backend)
     ↓
FTP/SFTP Servers
```

## 🔒 安全注意事项

⚠️ **重要：** 无头模式下的 GUI 以 root 用户运行，请注意：

1. **生产环境** 建议使用普通用户 + systemd service
2. **Socket 权限** 应设置为 0660 并配合用户组管理
3. **Xvfb 访问** 可以通过 `-ac` 参数限制

## 📚 相关文档

- [WFTPG 系统架构](README.md)
- [IPC 通信协议](docs/IPC_PROTOCOL.md)
- [systemd 服务配置](debian/wftpd.service)

## 💡 常见问题

**Q: 可以在多个不同的 display 上运行多个 GUI 实例吗？**  
A: 可以！修改 `XVFB_DISPLAY=":99"` 为不同编号即可。

**Q: 如何截图或录制视频？**  
A: 使用 `xwd` 或 `ffmpeg`：
```bash
# 截图
xwd -root -display :99 | convert xwd:- screenshot.png

# 录屏
ffmpeg -f x11grab -video_size 1920x1080 -i :99 output.mp4
```

**Q: 可以通过 VNC 查看虚拟桌面吗？**  
A: 可以！配合 x11vnc：
```bash
x11vnc -display :99 -forever -shared &
# 然后用 VNC 客户端连接 :5999 端口
```

---

**最后更新：** 2026-03-31  
**版本：** v1.0
