#!/bin/bash
# 权限配置验证脚本

set -e

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "========================================"
echo "  WFTPG 权限配置验证"
echo "========================================"
echo ""

# 检查是否以 root 运行
if [ "$EUID" -ne 0 ]; then 
    echo -e "${YELLOW}提示：请使用 sudo 运行此脚本以获得完整的权限信息${NC}"
    echo ""
fi

# 检查 wftpg 用户是否存在
echo "1. 检查 wftpg 用户和组..."
if id "wftpg" > /dev/null 2>&1; then
    echo -e "${GREEN}✓${NC} wftpg 用户存在"
    if getent group wftpg > /dev/null 2>&1; then
        echo -e "${GREEN}✓${NC} wftpg 组存在"
    else
        echo -e "${RED}✗${NC} wftpg 组不存在"
    fi
else
    echo -e "${YELLOW}⚠${NC} wftpg 用户不存在（可能还未安装）"
fi
echo ""

# 检查目录权限
echo "2. 检查目录权限..."
DIRECTORIES=("/etc/wftpg" "/etc/wftpg/keys" "/var/log/wftpg" "/var/lib/wftpg" "/var/lib/wftpg/ssh" "/var/lib/wftpg/share")

for dir in "${DIRECTORIES[@]}"; do
    if [ -d "$dir" ]; then
        owner=$(stat -c '%U:%G' "$dir" 2>/dev/null || echo "unknown")
        perms=$(stat -c '%a' "$dir" 2>/dev/null || echo "unknown")
        
        if [ "$owner" = "wftpg:wftpg" ]; then
            echo -e "${GREEN}✓${NC} $dir - 所有者：$owner, 权限：$perms"
        elif [ "$owner" = "root:root" ]; then
            echo -e "${YELLOW}⚠${NC} $dir - 所有者：$owner (应该是 wftpg:wftpg)"
        else
            echo -e "${RED}✗${NC} $dir - 所有者：$owner (应该是 wftpg:wftpg)"
        fi
    else
        echo -e "${YELLOW}⚠${NC} $dir - 目录不存在"
    fi
done
echo ""

# 检查配置文件权限
echo "3. 检查配置文件权限..."
CONFIG_FILES=("/etc/wftpg/config.toml" "/etc/wftpg/users.json")

for file in "${CONFIG_FILES[@]}"; do
    if [ -f "$file" ]; then
        owner=$(stat -c '%U:%G' "$file" 2>/dev/null || echo "unknown")
        perms=$(stat -c '%a' "$file" 2>/dev/null || echo "unknown")
        
        if [ "$owner" = "wftpg:wftpg" ] && [ "$perms" = "644" ]; then
            echo -e "${GREEN}✓${NC} $file - 所有者：$owner, 权限：$perms"
        else
            echo -e "${YELLOW}⚠${NC} $file - 所有者：$owner, 权限：$perms (应该是 wftpg:wftpg, 644)"
        fi
    else
        echo -e "${YELLOW}⚠${NC} $file - 文件不存在"
    fi
done
echo ""

# 检查可执行文件权限
echo "4. 检查可执行文件权限..."
EXEC_FILES=("/usr/bin/wftpd" "/usr/bin/wftp-gui")

for file in "${EXEC_FILES[@]}"; do
    if [ -f "$file" ]; then
        owner=$(stat -c '%U:%G' "$file" 2>/dev/null || echo "unknown")
        perms=$(stat -c '%a' "$file" 2>/dev/null || echo "unknown")
        
        if [ "$owner" = "root:root" ] && [ "$perms" = "755" ]; then
            echo -e "${GREEN}✓${NC} $file - 所有者：$owner, 权限：$perms"
        else
            echo -e "${YELLOW}⚠${NC} $file - 所有者：$owner, 权限：$perms (应该是 root:root, 755)"
        fi
    else
        echo -e "${YELLOW}⚠${NC} $file - 文件不存在"
    fi
done
echo ""

# 检查 SSH 密钥目录权限
echo "5. 检查 SSH 密钥目录权限..."
SSH_DIR="/var/lib/wftpg/ssh"
if [ -d "$SSH_DIR" ]; then
    owner=$(stat -c '%U:%G' "$SSH_DIR" 2>/dev/null || echo "unknown")
    perms=$(stat -c '%a' "$SSH_DIR" 2>/dev/null || echo "unknown")
    
    if [ "$owner" = "wftpg:wftpg" ] && [ "$perms" = "700" ]; then
        echo -e "${GREEN}✓${NC} $SSH_DIR - 所有者：$owner, 权限：$perms (高安全)"
    else
        echo -e "${YELLOW}⚠${NC} $SSH_DIR - 所有者：$owner, 权限：$perms (应该是 wftpg:wftpg, 700)"
    fi
    
    # 检查 SSH 密钥文件
    for key_file in "$SSH_DIR"/ssh_host_*_key; do
        if [ -f "$key_file" ]; then
            owner=$(stat -c '%U:%G' "$key_file" 2>/dev/null || echo "unknown")
            perms=$(stat -c '%a' "$key_file" 2>/dev/null || echo "unknown")
            
            if [ "$perms" = "600" ]; then
                echo -e "${GREEN}✓${NC} $(basename $key_file) - 权限：$perms (私有)"
            else
                echo -e "${YELLOW}⚠${NC} $(basename $key_file) - 权限：$perms (应该是 600)"
            fi
        fi
    done
else
    echo -e "${YELLOW}⚠${NC} $SSH_DIR - 目录不存在"
fi
echo ""

# 总结
echo "========================================"
echo "  验证完成"
echo "========================================"
echo ""
echo "说明:"
echo "- ✓ 表示配置正确"
echo "- ⚠ 表示配置可能需要调整"
echo "- ✗ 表示配置错误"
echo ""
echo "如需修复权限，可以运行:"
echo "  sudo chown -R wftpg:wftpg /etc/wftpg /var/log/wftpg /var/lib/wftpg"
echo "  sudo chmod 700 /var/lib/wftpg/ssh"
echo "  sudo chmod 644 /etc/wftpg/config.toml /etc/wftpg/users.json"
echo ""
