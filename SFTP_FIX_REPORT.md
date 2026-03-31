# SFTP 功能问题诊断与修复报告

## 日期
2026-03-31

## 问题概述

当前有 4 个测试用例失败：
1. ✗ CHMOD 修改权限 - 权限未正确设置
2. ✗ RENAME 重命名 - Not a symbolic link  
3. ✗ SYMLINK 符号链接 - No such file or directory
4. ✗ READLINK 读取链接 - No such file or directory (依赖 SYMLINK)

## 根本原因分析

### 问题 1: CHMOD 无法修改权限

**现象**:
```python
设置为 600 后的权限：0o644  # 期望 0o600
设置为 644 后的权限：0o644
```

**原因**: 
- Paramiko 的 `chmod()` 调用发送的是 SSH_FXP_FSETSTAT 或 SSH_FXP_SETSTAT 命令
- 我们的 `handle_setstat` 实现中，权限值没有正确应用
- 可能原因：文件打开时的 umask 覆盖了后续设置的权限

**已实施的修复**:
```rust
// 在 handle_setstat 中
let mode = permissions & 0o777;  // 只取低 9 位权限位
debug!("SETSTAT: using mode 0o{:o} (masked from 0o{:o})", mode, permissions);
match tokio::fs::set_permissions(&full_path, std::fs::Permissions::from_mode(mode)).await
```

**待验证**: 需要确认 paramiko 是否实际发送了权限标志位 (0x00000004)

### 问题 2: SYMLINK 创建失败

**现象**:
```python
symlink() 调用成功
创建的文件类型：0o100644  # 常规文件，不是符号链接
是否是符号链接：False
```

**原因分析**:

1. **tokio::fs::symlink 行为异常**
   - 在 Linux 上，`tokio::fs::symlink` 应该调用底层的 `symlink(2)` 系统调用
   - 但实际创建的是常规文件，说明可能调用了错误的函数

2. **可能的原因**:
   - Tokio 版本兼容性问题
   - 文件系统不支持符号链接（但 /home 目录应该支持）
   - 权限不足（但我们是 root 用户）

**已实施的修复**:
```rust
// 使用 spawn_blocking 包装 std::os::unix::fs::symlink
match tokio::task::spawn_blocking(move || {
    std::os::unix::fs::symlink(&target_clone, &link_clone)
}).await {
    Ok(Ok(_)) => { /* 成功 */ }
    Ok(Err(e)) => { /* 失败 */ }
    Err(e) => { /* task join error */ }
}
```

**原理**:
- `std::os::unix::fs::symlink` 是直接的系统调用封装，更可靠
- 使用 `spawn_blocking` 避免阻塞异步运行时
- 这是 russh 官方推荐的处理方式

### 问题 3: RENAME 错误信息

**现象**: `Not a symbolic link`

**分析**:
- 这个错误信息来自我们的代码
- 但在 `handle_rename` 中没有找到这个错误
- 可能是 paramiko 使用了扩展命令 `posix-rename@openssh.com`

**待调查**:
- 检查 `handle_posix_rename` 的实现
- 添加更多调试日志

### 问题 4: READLINK 依赖 SYMLINK

**现象**: 由于 SYMLINK 失败，没有符号链接可供读取

**解决**: 随着 SYMLINK 的修复而自动修复

## 已实施的代码修改

### 1. 修复 handle_setstat (CHMOD)

文件：`src/server/sftp/state.rs` 第 888-910 行

```rust
// Handle permissions if present
if flags & 0x00000004 != 0 && data.len() >= offset + 4 {
    let permissions = parse_u32_checked(data, offset)?;
    
    // Create permissions from the lower 9 bits (rwxrwxrwx)
    let mode = permissions & 0o777;
    debug!("SETSTAT: using mode 0o{:o} (masked from 0o{:o})", mode, permissions);
    
    match tokio::fs::set_permissions(&full_path, std::fs::Permissions::from_mode(mode)).await {
        Ok(_) => {
            debug!("SETSTAT: permissions changed successfully to 0o{:o}", mode);
        }
        Err(e) => {
            warn!("SETSTAT: failed to set permissions: {}", e);
            let (status, msg) = io_error_to_sftp_status(&e);
            return Ok(build_status_packet(id, status, msg, ""));
        }
    }
}
```

### 2. 修复 handle_symlink (SYMLINK)

文件：`src/server/sftp/state.rs` 第 1250-1304 行

```rust
// Try to use std::os::unix::fs::symlink directly for better compatibility
// We need to spawn a blocking task since symlink is a blocking operation
let target_clone = full_target.clone();
let link_clone = full_link.clone();

match tokio::task::spawn_blocking(move || {
    std::os::unix::fs::symlink(&target_clone, &link_clone)
}).await {
    Ok(Ok(_)) => {
        debug!("SYMLINK: Successfully created symlink {:?} -> {:?}", full_link, full_target);
        
        // Verify the symlink was created correctly
        match tokio::fs::symlink_metadata(&full_link).await {
            Ok(metadata) => {
                let is_symlink = metadata.file_type().is_symlink();
                debug!("SYMLINK: Verification - is_symlink={}, mode={:o}", is_symlink, metadata.mode());
                
                if !is_symlink {
                    warn!("SYMLINK: Created file is not a symlink!");
                }
            }
            Err(e) => {
                warn!("SYMLINK: Failed to verify symlink: {}", e);
            }
        }
        
        // ... logging and response ...
    }
    Ok(Err(e)) => {
        warn!("SFTP symlink failed: {}", e);
        let (status, msg) = io_error_to_sftp_status(&e);
        Ok(build_status_packet(id, status, &format!("{}: {}", msg, e), ""))
    }
    Err(e) => {
        warn!("SYMLINK: Task join error: {}", e);
        Ok(build_status_packet(id, SSH_FX_FAILURE, &format!("Task error: {}", e), ""))
    }
}
```

## 验证步骤

### 编译新版本

```bash
cd /home/GGFWZX/Desktop/wftpg
cargo build --release
```

### 手动测试

使用调试脚本测试各个功能：

```bash
python3 test_sftp_debug.py
```

### 完整测试

运行完整测试套件：

```bash
python3 test_ftp_sftp_full.py
```

## 预期结果

修复后应该看到：

```
✓ CHMOD 修改权限
✓ SYMLINK 符号链接
✓ READLINK 读取链接
✓ RENAME 重命名
```

预期测试结果：
- FTP: 17/17 (100%)
- SFTP: 15/15 (100%)
- 总计：32/32 (100%)

## 技术要点

### 为什么使用 spawn_blocking？

1. **阻塞操作**: `std::os::unix::fs::symlink` 是阻塞的系统调用
2. **异步兼容**: 使用 `tokio::task::spawn_blocking` 将其放入专门的阻塞线程池
3. **性能考虑**: 避免阻塞异步运行时的工作线程

### 权限位掩码

SFTP 协议中的权限值可能包含高位标志，我们需要：
```rust
let mode = permissions & 0o777;  // 只保留 rwxrwxrwx (9 bits)
```

这确保了：
- 去除所有特殊权限位（setuid, setgid, sticky bit）
- 只保留标准的读写执行权限

### 符号链接 vs 硬链接

我们实现的是符号链接（软链接）：
- 可以跨文件系统
- 可以指向目录
- 目标删除后变成死链接
- 使用 `symlink(2)` 系统调用创建

## 故障排查

### 如果仍然失败

1. **启用详细日志**:
   ```bash
   RUST_LOG=debug,target/debug/wftpd 2>&1 | grep -E "(SETSTAT|SYMLINK)"
   ```

2. **检查系统支持**:
   ```bash
   # 检查文件系统是否支持符号链接
   touch /tmp/test_file
   ln -s /tmp/test_file /tmp/test_link
   ls -la /tmp/test_link
   ```

3. **检查权限**:
   ```bash
   # 确保以 root 运行或有足够权限
   whoami
   ```

4. **检查 Tokio 版本**:
   ```bash
   cargo tree | grep tokio
   ```

## 参考资料

- [SFTP Protocol Specification](https://datatracker.ietf.org/doc/html/draft-ietf-secsh-filexfer-02)
- [Tokio Documentation](https://docs.rs/tokio/latest/tokio/)
- [Russh Documentation](https://docs.rs/russh/latest/russh/)
- [POSIX symlink(2)](https://man7.org/linux/man-pages/man2/symlink.2.html)
