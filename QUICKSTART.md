# WFTPG 快速入门指南

## 安装步骤

### 1. 安装依赖

```bash
# Ubuntu/Debian 系统
sudo apt update
sudo apt install -y libgtk-3-0 policykit-1

# 可选：安装 ImageMagick（用于生成图标）
sudo apt install -y imagemagick
```

### 2. 创建用户和组

```bash
# 创建 wftpg 组
sudo groupadd wftpg

# 创建 wftpg 用户（可选，systemd 服务会使用该用户）
sudo useradd -r -g wftpg -s /bin/false -d /var/lib/wftpg wftpg

# 将当前用户添加到 wftpg 组（以便访问 IPC Socket）
sudo usermod -aG wftpg $USER

# 注销并重新登录使组设置生效
```

### 3. 安装 DEB 包

```bash
# 假设你已经构建了 DEB 包
cd /path/to/wftpg/debian
sudo dpkg -i wftpg_*.deb

# 如果有依赖问题
sudo apt install -f
```

### 4. 启用服务

```bash
# 启用开机自启
sudo systemctl enable wftpd

# 启动服务
sudo systemctl start wftpd

# 查看状态
systemctl status wftpd
```

---

## 使用方法

### 启动 GUI 管理界面

```bash
wftp-gui
```

或在应用程序菜单中找到 "WFTPG"。

### 使用命令行工具

```bash
# 查看服务状态
wftpgctl status

# 启动服务（需要 root）
sudo wftpgctl start

# 重启服务（需要 root）
sudo wftpgctl restart

# 停止服务（需要 root）
sudo wftpgctl stop
```

### 使用 systemctl 命令

```bash
# 查看所有相关服务
systemctl list-units | grep wftpg

# 查看详细状态
systemctl status wftpd

# 启动/停止/重启
sudo systemctl start wftpd
sudo systemctl stop wftpd
sudo systemctl restart wftpd

# 启用/禁用开机自启
sudo systemctl enable wftpd
sudo systemctl disable wftpd

# 查看日志
journalctl -u wftpd -f
```

---

## 首次配置

### 1. 配置 FTP/SFTP 服务

编辑配置文件：
```bash
sudo nano /etc/wftpg/config.toml
```

基本配置示例：
```toml
[ftp]
enabled = true
bind_ip = "0.0.0.0"
default_home = "/var/lib/wftpg/share"
passive_ports = [50000, 51000]

[sftp]
enabled = true
bind_ip = "0.0.0.0"
default_home = "/var/lib/wftpg/share"
```

### 2. 创建用户

方法一：使用 GUI
- 打开 wftp-gui
- 切换到"用户管理"标签页
- 点击"添加用户"
- 填写用户名、密码、家目录等信息

方法二：直接编辑 users.json
```bash
sudo nano /var/lib/wftpg/users.json
```

示例内容：
```json
{
  "users": {
    "testuser": {
      "password_hash": "$argon2id$v=19$m=4096,t=3,p=1$...",
      "home_dir": "/home/testuser",
      "permissions": ["read", "write", "delete"]
    }
  }
}
```

### 3. 测试连接

**FTP 测试：**
```bash
# 使用 ftp 客户端
ftp localhost

# 或使用 lftp
lftp -u testuser localhost
```

**SFTP 测试：**
```bash
sftp -P 22 testuser@localhost
```

---

## 常见问题排查

### Q1: GUI 无法连接到服务

**症状：** 打开 GUI 显示"无法连接到 IPC Socket"

**解决方法：**
```bash
# 检查服务是否运行
systemctl status wftpd

# 检查 Socket 是否存在
ls -la /run/wftpd/wftpg.sock

# 检查用户是否在 wftpg 组中
groups $USER

# 如果不在组中，添加后重新登录
sudo usermod -aG wftpg $USER
```

### Q2: 服务无法启动

**症状：** `systemctl start wftpd` 失败

**排查步骤：**
```bash
# 查看详细错误日志
sudo journalctl -u wftpd -n 50 --no-pager

# 检查配置文件语法
cat /etc/wftpg/config.toml

# 检查端口占用
sudo netstat -tlnp | grep :21
sudo netstat -tlnp | grep :22

# 检查权限
ls -la /etc/wftpg/
ls -la /var/lib/wftpg/
```

### Q3: FTP 被动模式连接失败

**解决方法：**
1. 确保防火墙开放了被动端口范围
```bash
sudo ufw allow 50000:51000/tcp
```

2. 如果在 NAT 后面，配置外部 IP
```toml
[ftp]
pasv_address = "your.external.ip"
```

### Q4: 权限问题

**症状：** 用户无法上传/删除文件

**解决方法：**
```bash
# 确保用户家目录存在并有正确权限
sudo mkdir -p /home/testuser
sudo chown testuser:wftpg /home/testuser
sudo chmod 2770 /home/testuser

# 如果使用共享目录
sudo chown -R wftpg:wftpg /var/lib/wftpg/share
sudo chmod -R 2770 /var/lib/wftpg/share
```

---

## 高级配置

### 配置 SSL/TLS（FTPS）

在 `config.toml` 中添加：
```toml
[ftp.tls]
enabled = true
cert_path = "/etc/wftpg/keys/server.crt"
key_path = "/etc/wftpg/keys/server.key"
```

生成自签名证书：
```bash
sudo mkdir -p /etc/wftpg/keys
openssl req -x509 -nodes -days 365 -newkey rsa:2048 \
  -keyout /etc/wftpg/keys/server.key \
  -out /etc/wftpg/keys/server.crt
```

### 配置 SFTP Host Key

```bash
# 生成 SSH Host Key
sudo ssh-keygen -t rsa -b 4096 -f /var/lib/wftpg/ssh/ssh_host_rsa_key -N ""

# 设置权限
sudo chown wftpg:wftpg /var/lib/wftpg/ssh/ssh_host_rsa_key
sudo chmod 600 /var/lib/wftpg/ssh/ssh_host_rsa_key
```

然后在 `config.toml` 中配置：
```toml
[sftp]
host_key_path = "/var/lib/wftpg/ssh/ssh_host_rsa_key"
```

### 限制用户到家目录

在 `users.json` 中为用户添加 `chroot` 设置：
```json
{
  "users": {
    "restricted": {
      "password_hash": "...",
      "home_dir": "/var/lib/wftpg/share/restricted",
      "permissions": ["read", "write"],
      "chroot": true
    }
  }
}
```

---

## 卸载

```bash
# 停止服务
sudo systemctl stop wftpd

# 卸载软件
sudo apt remove wftpg

# 完全清除（包括配置文件）
sudo apt purge wftpg

# 手动删除残留数据（可选）
sudo rm -rf /etc/wftpg
sudo rm -rf /var/lib/wftpg
sudo rm -rf /var/log/wftpg
```

---

## 获取帮助

- 查看完整文档：`man wftpg`（如果已安装）
- 查看架构说明：`cat /usr/share/doc/wftpg/ARCHITECTURE.md`
- GitHub Issues: https://github.com/wftpg/wftpg/issues
- 邮件列表：support@wftpg.com

---

## 快捷键（GUI）

- `Ctrl+Q`: 退出程序
- `Ctrl+S`: 保存配置
- `F5`: 刷新状态
- `F1`: 打开帮助

---

祝您使用愉快！🎉
