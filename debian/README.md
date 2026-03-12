# WFTPG DEB打包说明

## 文件结构

```
wftpg/
├── debian/
│   ├── build-deb.sh          # 主打包脚本
│   ├── create-icon.sh        # 图标生成脚本
│   ├── control               # DEB包控制文件
│   ├── postinst              # 安装后脚本
│   ├── prerm                 # 卸载前脚本
│   ├── postrm                # 卸载后脚本
│   └── com.wftpg.pkexec.policy  # PolicyKit权限配置
├── wftpg.desktop             # 桌面快捷方式文件
└── ui/                       # 图标资源目录
    └── wftpg.png/svg         # 应用图标
```

## 构建DEB包

### 前置要求

1. 确保已安装Rust工具链
2. 确保已安装必要的依赖：
   ```bash
   sudo apt install build-essential libgtk-3-dev libssl-dev policykit-1
   ```

### 构建步骤

1. **（可选）生成应用图标**
   ```bash
   cd debian
   chmod +x create-icon.sh
   ./create-icon.sh
   ```

2. **构建DEB包（需要root权限）**
   ```bash
   cd debian
   sudo ./build-deb.sh
   ```

3. **查看生成的DEB包**
   ```bash
   ls -lh build/wftpg_2.0.0_arm64.deb
   ```

## 安装和使用

### 安装DEB包

```bash
sudo dpkg -i build/wftpg_2.0.0_arm64.deb
# 或者
sudo apt install build/wftpg_2.0.0_arm64.deb
```

### 启动应用

安装后可以通过以下方式启动：

1. **从应用菜单启动**
   - 在UOS/Deepin的应用启动器中找到"WFTPG文件传输服务器"
   - 点击图标启动（会弹出PolicyKit认证对话框）

2. **从命令行启动**
   ```bash
   wftpg
   # 或者使用pkexec获取root权限
   pkexec wftpg
   ```

### 卸载应用

```bash
sudo apt remove wftpg
# 或者完全清除包括配置文件
sudo apt purge wftpg
```

## 功能特性

### 1. 系统集成

- **桌面快捷方式**: 安装到 `/usr/share/applications/wftpg.desktop`
- **应用图标**: 安装到 `/usr/share/icons/hicolor/`
- **可执行文件**: 安装到 `/usr/bin/wftpg`

### 2. Root权限管理

- 使用PolicyKit (pkexec) 进行权限提升
- 配置文件位于 `/usr/share/polkit-1/actions/com.wftpg.pkexec.policy`
- 启动时会弹出系统认证对话框

### 3. 配置和数据目录

- **系统配置**: `/etc/wftpg/`
- **日志目录**: `/var/log/wftpg/`
- **用户配置**: `~/.config/wftpg/` (用户首次运行时创建)
- **用户缓存**: `~/.cache/wftpg/` (日志缓存)

### 4. UOS/Deepin兼容性

- 符合UOS应用打包规范
- 支持Deepin应用商店管理
- 支持系统菜单集成
- 支持中文界面

## 自定义配置

### 修改版本号

编辑 `debian/control` 和 `debian/build-deb.sh` 中的版本号：
```bash
Version: 2.0.0  # 修改此处
```

### 修改架构

默认为 `arm64`，如需修改其他架构：
```bash
Architecture: arm64  # 可改为 amd64, i386 等
```

### 添加依赖

编辑 `debian/control` 文件：
```
Depends: libgtk-3-0, libc6, libssl3, policykit-1, your-dependency
```

## 故障排除

### 1. 权限问题

如果启动时提示权限不足：
```bash
# 检查PolicyKit配置
ls -l /usr/share/polkit-1/actions/com.wftpg.pkexec.policy

# 手动授权
pkexec wftpg
```

### 2. 图标不显示

```bash
# 更新图标缓存
sudo gtk-update-icon-cache /usr/share/icons/hicolor

# 更新桌面数据库
sudo update-desktop-database /usr/share/applications
```

### 3. 应用菜单中找不到

```bash
# 检查desktop文件
ls -l /usr/share/applications/wftpg.desktop

# 重新安装
sudo dpkg -r wftpg
sudo dpkg -i build/wftpg_2.0.0_arm64.deb
```

## 开发说明

### 构建流程

1. 编译Rust程序（release模式）
2. 创建DEB包目录结构
3. 复制可执行文件、桌面文件、图标等
4. 生成控制文件和脚本
5. 打包为DEB文件

### 文件权限

- 可执行文件: 755 (root:root)
- 桌面文件: 644 (root:root)
- 配置目录: 755 (root:root)
- PolicyKit配置: 644 (root:root)

## 许可证

MIT License

## 联系方式

- 项目主页: https://github.com/wftpg/wftpg
- 问题反馈: https://github.com/wftpg/wftpg/issues
