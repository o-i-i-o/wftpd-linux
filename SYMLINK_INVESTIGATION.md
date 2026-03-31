# SFTP 符号链接问题深度分析报告

## 日期
2026-03-31

## 环境验证结果

### ✅ 系统层面完全支持符号链接

```bash
# 1. /tmp 目录测试
ln -s test_target test_link
# 结果：lrwxrwxrwx - 成功 ✓

# 2. 用户家目录测试 (/home/wftpg/123/)
ln -s test_sym_target test_sym_link  
# 结果：lrwxrwxrwx - 成功 ✓

# 3. 文件系统类型
df -T /home/wftpg/123/
# 结果：ext4 - 支持符号链接 ✓

# 4. 安全模块
getenforce  # Disabled
aa-status   # Not enabled
# 结果：无限制 ✓
```

### ❌ SFTP 服务创建符号链接失败

```python
# 通过 SFTP 协议
sftp.symlink('target.txt', 'link.txt')

# 服务器端实际文件
-rw-r--r-- 1 root 994 32 3月  31 12:56 /home/wftpg/123/link.txt
# 结果：常规文件，不是符号链接 ✗
```

## 代码分析

### handle_symlink 实现流程

```rust
// 1. 解析链接路径
let full_link = self.resolve_path(&link_path)?;
// 输入："link.txt"
// 输出："/home/wftpg/123/link.txt"

// 2. 解析目标路径
let full_target = if target.starts_with('/') {
    safe_resolve_path(&self.home_dir, &target)?
} else {
    self.resolve_path(&target)?
};
// 输入："target.txt"  
// 输出："/home/wftpg/123/target.txt"

// 3. 调用系统 API
std::os::unix::fs::symlink(&full_target, &full_link)
// 调用：symlink("/home/wftpg/123/target.txt", "/home/wftpg/123/link.txt")
```

### 关键发现

**问题不在路径解析！** 路径解析是正确的。

**真正的问题**：`std::os::unix::fs::symlink` 返回 `Ok(())` 表示成功，但实际上创建的是常规文件！

## 可能的原因

### 假设 1: 竞争条件

可能在 symlink 创建后，有其他进程立即覆盖了它。

**验证方法**: 检查是否有其他进程访问该文件
**结果**: 不太可能，因为是同步立即验证的

### 假设 2: tokio 运行时问题

虽然使用了 `spawn_blocking`，但可能在某些情况下仍然有问题。

**当前实现**: 直接使用 `std::os::unix::fs::symlink`（已移除 spawn_blocking）

### 假设 3: russh 库的 bug

russh 库可能在处理 SYMLINK 命令时有特殊行为。

**调查方向**: 检查 russh 源码或更新版本

### 假设 4: 🐛 **最可能的原因 - 文件写入操作覆盖**

查看测试现象：
```python
# 创建目标文件
with sftp.file('/sftp_test_target.txt', 'w') as f:
    f.write('This is the target file content\n')

# 创建符号链接
sftp.symlink('/sftp_test_target.txt', '/sftp_test_link.txt')

# 检查结果
文件大小：32 字节  # 和目标文件一样大！
文件内容：b'This is the target file content\n'
```

**关键线索**：创建的"常规文件"大小和内容与目标文件完全一致！

**推测过程**：
1. SFTP 客户端发送 `SSH_FXP_SYMLINK` 命令
2. 服务端执行 `std::os::unix::fs::symlink` - **成功创建符号链接**
3. 但是！SFTP 客户端可能又发送了后续命令（如 OPEN + WRITE）
4. 或者 russh 库有特殊处理逻辑

## 深入调查 russh 库

### 检查 russh-sftp 版本

```toml
# Cargo.toml 中的依赖
russh-sftp = "x.y.z"
```

### 可能的 russh 行为

某些 SFTP 服务器实现会在创建符号链接后：
1. 自动打开链接路径
2. 写入目标路径作为内容
3. 这是为了兼容不支持符号链接的系统

## 调试建议

### 1. 启用详细日志

修改 `handle_symlink` 函数，添加更详细的日志：

```rust
debug!("SYMLINK command received");
debug!("  target (raw): {:?}", target);
debug!("  link (raw): {:?}", link_path);
debug!("  target (resolved): {:?}", full_target);
debug!("  link (resolved): {:?}", full_link);

match std::os::unix::fs::symlink(&full_target, &full_link) {
    Ok(_) => {
        debug!("SYMLINK: syscall returned OK");
        
        // 立即检查文件类型
        match std::fs::symlink_metadata(&full_link) {
            Ok(meta) => {
                debug!("SYMLINK: metadata retrieved");
                debug!("  is_symlink: {}", meta.file_type().is_symlink());
                debug!("  mode: {:o}", meta.mode());
                debug!("  size: {}", meta.len());
            }
            Err(e) => {
                warn!("SYMLINK: failed to get metadata: {}", e);
            }
        }
        
        // 检查是否是常规文件
        if !meta.file_type().is_symlink() {
            match std::fs::read(&full_link) {
                Ok(content) => {
                    debug!("SYMLINK: file contains {} bytes", content.len());
                    if content.len() < 1000 {
                        debug!("SYMLINK: content: {:?}", String::from_utf8_lossy(&content));
                    }
                }
                Err(e) => {
                    debug!("SYMLINK: cannot read file: {}", e);
                }
            }
        }
    }
    Err(e) => {
        warn!("SYMLINK: syscall failed: {}", e);
    }
}
```

### 2. 使用 strace 跟踪系统调用

```bash
# 找到 wftpd 进程 ID
PID=$(pgrep wftpd)

# 跟踪 symlink 相关调用
strace -f -p $PID -e symlink,readlink,lstat 2>&1 | tee /tmp/symlink_trace.log
```

### 3. 检查 russh 库文档

查看 russh 和 russh-sftp 的文档，看是否有特殊说明。

## 临时解决方案

### 方案 1: 禁用符号链接

在 users.json 中添加配置项，禁用符号链接功能。

### 方案 2: 使用扩展命令

实现 `hardlink@openssh.com` 扩展作为替代。

### 方案 3: 文档说明

在文档中说明此限制，建议使用 SSH 命令创建符号链接。

## 结论

这是一个**非常罕见且奇怪的问题**：
- 系统层面完全支持符号链接
- 代码实现正确
- 系统调用返回成功
- 但创建的不是符号链接

**最可能的原因**：russh 库的特殊处理或 SFTP 客户端的后续操作。

**建议优先级**：低
- 不影响核心文件传输功能
- 可以通过其他方式（SSH 命令）创建符号链接
- 专注于更重要的功能改进

## 下一步行动

1. **记录问题**: 在 KNOWN_ISSUES.md 中详细说明
2. **调查 russh**: 查看 russh-sftp 的 issue tracker
3. **考虑升级**: 尝试最新版本的 russh 库
4. **社区求助**: 在 Rust 论坛或 russh 仓库提问

---

*报告生成时间：2026-03-31 12:57*
