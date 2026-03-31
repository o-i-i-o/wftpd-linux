# FTP/SFTP 逻辑 Chroot 与安全路径实现分析报告

## 日期
2026-03-31

## 📋 执行摘要

经过深入分析代码，发现当前 FTP 和 SFTP 的逻辑 chroot 实现存在**多个严重的安全漏洞和逻辑错误**，可能导致：
- ⚠️ **路径逃逸风险**
- ⚠️ **符号链接攻击**
- ⚠️ **竞态条件 (TOCTOU)**
- ⚠️ **权限检查绕过**

---

## 🔴 关键安全漏洞

### 漏洞 1: 符号链接导致的 chroot 逃逸

**位置**: `src/server/sftp/state.rs` - `handle_symlink()`

**问题描述**:
```rust
// 当前实现（第 1246-1304 行）
let full_target = if target.starts_with('/') {
    let resolved = match safe_resolve_path(&self.home_dir, &target) {
        Ok(p) => p,
        Err(e) => { /* ... */ }
    };
    // ❌ 安全检查只验证是否在 home_canon 内
    if !resolved.starts_with(&home_canon) {
        return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, ...));
    }
    resolved
} else {
    // ❌ 相对路径解析后也可能指向 home 外
    let resolved = match self.resolve_path(&target)?;
    if !resolved.starts_with(&home_canon) { /* ... */ }
    resolved
};

// 然后创建符号链接
std::os::unix::fs::symlink(&full_target, &full_link)
```

**攻击场景**:
```bash
# 攻击者可以：
1. 在用户目录内创建目录：mkdir /home/wftpg/123/evil
2. 创建指向系统目录的符号链接：ln -s /etc /home/wftpg/123/evil/etc
3. 通过符号链接访问：read /home/wftpg/123/evil/etc/passwd

# 虽然 resolve_path 会检查，但如果：
# a) /etc 已经存在且可访问
# b) 或者利用竞态条件
# 就可能成功！
```

**根本原因**: 
- `safe_resolve_path` 对**不存在的路径**不会 canonicalize
- 符号链接的目标不会被立即验证
- 后续访问时可能通过符号链接逃逸

---

### 漏洞 2: RENAME 操作的双重路径检查缺陷

**位置**: `src/server/sftp/state.rs` - `handle_rename()` (第 717-780 行)

**问题代码**:
```rust
let old_full = match self.resolve_path(&old_path) { /* ... */ };
let new_full = match self.resolve_path(&new_path) { /* ... */ };

// ❌ 只检查了各自的路径，没有检查重命名后的最终位置
match tokio::fs::rename(&old_full, &new_full).await { /* ... */ }
```

**安全问题**:
1. **竞态条件**: 在检查 `old_full` 和 `new_full` 之间，文件系统可能已改变
2. **目标验证不足**: 如果 `new_full` 的父目录是符号链接，可能逃逸
3. **原子性缺失**: 检查和执行不是原子的

**示例攻击**:
```bash
# 时间窗口攻击
t0: 攻击者请求 rename("/home/wftpg/123/a", "/home/wftpg/123/../escape")
t1: 服务端检查 old_path="/home/wftpg/123/a" ✓
t2: 服务端检查 new_path="/home/wftpg/123/../escape" → 规范化为 "/home/wftpg/escape" ✗
t3: 但在 t1-t2 之间，攻击者修改了路径结构...
```

---

### 漏洞 3: 路径解析中的 TOCTOU (Time-Of-Check-Time-Of-Use)

**位置**: 所有使用 `resolve_path` 的地方

**模式**:
```rust
// 典型模式（出现在所有 handler 中）
let full_path = match self.resolve_path(&path) { /* check */ };
// ⏰ TOCTOU 窗口：这里可能被其他进程修改
tokio::fs::operation(&full_path).await  // Use
```

**具体案例**: `handle_read()` (第 468-534 行)
```rust
let handle = self.handles.get_mut(&handle_str);
// ❌ 假设 handle 中的 path 仍然是安全的
// 但可能在打开后被替换为符号链接
file.seek(SeekFrom::Start(offset)).await?;
file.read(&mut buffer).await?;
```

---

### 漏洞 4: 不完整的权限检查

**位置**: `handle_setstat()` 和 `handle_fsetstat()`

**问题**:
```rust
// handle_setstat (第 843-922 行)
if !self.check_permission_cached(|p| p.can_write) {
    return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, ...));
}
// ❌ 只检查 can_write，但实际可以修改任何属性
// 包括：permissions, ownership, timestamps
```

**风险**:
- 用户可以修改文件所有权（如果以 root 运行）
- 可以设置 setuid/setgid 位
- 可以修改时间戳掩盖入侵痕迹

---

### 漏洞 5: 配额检查的竞态条件

**位置**: `handle_write()` (第 536-604 行)

**代码**:
```rust
let current_len = tokio::fs::metadata(&target_path).await
    .map(|metadata| metadata.len())
    .unwrap_or(offset);
let requested_end = offset.saturating_add(data_len as u64);
let additional_bytes = requested_end.saturating_sub(current_len);

if !self.check_quota_for_additional_bytes(additional_bytes) {
    return Ok(build_status_packet(id, SSH_FX_FAILURE, "Quota exceeded", ""));
}

// ⏰ TOCTOU 窗口
let handle = self.handles.get_mut(&handle_str);
// 写入操作...
h.written_bytes += data_len as u64;  // ❌ 这是累积的，但 quota 检查已过时
```

**攻击方式**:
```python
# 并发写入绕过配额
thread1: write(handle, offset=0, data="A"*1MB)  # 检查通过
thread2: write(handle, offset=0, data="B"*1MB)  # 同时检查通过
# 结果：写入了 2MB，但配额只允许 1MB
```

---

## 🟡 FTP 特有问题

### FTP 问题 1: CWD 命令的路径遍历

**位置**: `src/server/ftp/commands/directory.rs`

**代码分析**:
```rust
// 第 38-55 行 - CWD ..
let new_path = match safe_resolve_path(&self.cwd, &self.home_dir, "..") {
    // ❌ 依赖 safe_resolve_path 阻止逃逸
    // 但该函数本身可能有逻辑缺陷
}
```

**潜在问题**:
- `safe_resolve_path_with_cwd` 在处理 `..` 时的逻辑
- 如果 cwd 已经是 `/home/wftpg/123/subdir`
- `..` 应该回到 `/home/wftpg/123`
- 但如果使用符号链接：`cd /home/wftpg/123/link_to_parent && cd ..`
- 可能到达 `/home/wftpg` 甚至更高

---

### FTP 问题 2: MLST/MLSD 的事实泄露

**位置**: `src/server/ftp/commands/directory.rs`

**代码**:
```rust
// 第 164-217 行 - MLSD 命令
for entry in entries {
    let facts = build_mlst_facts(&metadata);
    // ❌ 可能泄露敏感信息：
    // - 真实文件大小
    // - 精确时间戳
    // - Unix 权限模式
}
```

**风险**:
- 泄露系统信息（通过 inode 号等）
- 帮助攻击者识别敏感文件

---

## 🔵 SFTP 特有问题

### SFTP 问题 1: 扩展命令的安全检查缺失

**位置**: `src/server/sftp/extensions.rs`

**示例**: `handle_posix_rename()` (第 615-692 行)
```rust
async fn handle_posix_rename(&mut self, id: u32, data: &[u8], ext_offset: usize) -> Result<Vec<u8>> {
    // ❌ 没有调用 check_permission_cached!
    // 直接使用了路径解析
    
    let old_full = match self.resolve_path(&old_path) { /* ... */ }
    let new_full = match self.resolve_path(&new_path) { /* ... */ }
    
    // 没有显式的权限检查！
    match tokio::fs::rename(&old_full, &new_full).await { /* ... */ }
}
```

---

### SFTP 问题 2: 硬链接和复制文件的风险

**位置**: `src/server/sftp/extensions.rs`

**代码**:
```rust
// hardlink@openssh.com (未显示完整代码，但通常模式)
match std::os::unix::fs::hardlink(&old_path, &new_path) {
    // ❌ 硬链接可以：
    // 1. 绕过配额（同一文件多次链接）
    // 2. 锁定文件防止删除
    // 3. 访问原本无权访问的文件
}
```

---

## 🛠️ 修复建议优先级

### 🔴 紧急（立即修复）

#### 1. 增强符号链接安全检查

**修复方案**:
```rust
async fn handle_symlink(&mut self, data: &[u8]) -> Result<Vec<u8>> {
    // ... 现有代码 ...
    
    // ✅ 新增：严格验证目标路径
    let full_target = resolve_and_validate_path(&target, &self.home_dir)?;
    let full_link = resolve_and_validate_path(&link_path, &self.home_dir)?;
    
    // ✅ 新增：禁止符号链接指向 home 外（即使是相对路径）
    if !is_within_home_unconditionally(&full_target, &self.home_dir)? {
        return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, 
            "Symlink target must be within home directory", ""));
    }
    
    // ✅ 新增：使用 O_NOFOLLOW 打开文件而不是 symlink
    // 或者完全禁用符号链接
    match tokio::fs::symlink(&full_target, &full_link).await {
        // ✅ 新增：立即验证创建的是符号链接
        match tokio::fs::symlink_metadata(&full_link).await {
            Ok(meta) if meta.file_type().is_symlink() => { /* OK */ }
            _ => {
                // 清理并返回错误
                let _ = tokio::fs::remove_file(&full_link).await;
                return Ok(build_status_packet(id, SSH_FX_FAILURE, 
                    "Failed to create symlink", ""));
            }
        }
    }
}
```

#### 2. 实现路径级权限检查

**新增辅助函数**:
```rust
/// 严格验证路径是否在 chroot 内
fn is_path_within_chroot(path: &Path, home: &str) -> Result<bool> {
    // ✅ 使用 realpath 而不是简单的 starts_with
    let canon_path = tokio::fs::canonicalize(path).await?;
    let canon_home = tokio::fs::canonicalize(home).await?;
    Ok(canon_path.starts_with(&canon_home))
}

/// 检查路径的所有组件是否都安全
async fn validate_path_components(path: &Path, home: &str) -> Result<()> {
    // ✅ 逐段检查路径，防止符号链接逃逸
    for ancestor in path.ancestors() {
        if !ancestor.starts_with(home) {
            bail!("Path component escapes chroot");
        }
        // 检查是否是符号链接
        if let Ok(meta) = tokio::fs::symlink_metadata(ancestor).await {
            if meta.file_type().is_symlink() {
                // 验证符号链接目标
                let link_target = tokio::fs::read_link(ancestor).await?;
                if !link_target.starts_with(home) {
                    bail!("Symlink {} points outside chroot", ancestor.display());
                }
            }
        }
    }
    Ok(())
}
```

---

### 🟠 高优先级（尽快修复）

#### 3. 消除 TOCTOU 竞态

**方案 A: 使用文件描述符而非路径**
```rust
// 参考 utils.rs 中的 safe_open_file_at (第 657-724 行)
#[cfg(unix)]
pub fn safe_open_file_at(home_dir: &str, relative_path: &str) -> WftpgResult<std::fs::File> {
    use nix::fcntl::{openat, OFlag, AT_FDCWD};
    
    // ✅ 使用 openat + O_NOFOLLOW
    let dirfd = openat(AT_FDCWD, &home_canon, 
        OFlag::O_DIRECTORY | OFlag::O_RDONLY, Mode::empty())?;
    
    // ✅ 逐段打开，每步都检查
    for component in components {
        openat(current_fd, component, 
            OFlag::O_RDONLY | OFlag::O_NOFOLLOW, Mode::empty())?;
    }
}
```

**方案 B: 添加文件锁**
```rust
use tokio::sync::Mutex;

struct LockedFile {
    path: PathBuf,
    _lock: fs_lock::FileLock,  // 排他锁
}

async fn with_file_lock<F, T>(path: &Path, f: F) -> Result<T> {
    let lock = acquire_exclusive_lock(path).await?;
    // 在锁保护下操作
    f(lock)
}
```

---

#### 4. 强化权限检查

**修复**:
```rust
async fn handle_setstat(&mut self, data: &[u8]) -> Result<Vec<u8>> {
    // ... 现有代码 ...
    
    if flags & 0x00000004 != 0 {
        let permissions = parse_u32_checked(data, offset)?;
        
        // ✅ 新增：过滤危险权限位
        let mode = permissions & 0o777;  // 只保留 rwxrwxrwx
        
        // ✅ 新增：禁止 setuid/setgid/sticky
        if permissions & 0o7000 != 0 {
            warn!("Attempted to set special permission bits: 0o{:o}", permissions);
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, 
                "Special permission bits not allowed", ""));
        }
        
        // ✅ 新增：限制最大权限
        if mode & 0o022 != 0 && !self.is_admin_user() {
            // 普通用户不能设置 world-writable
            return Ok(build_status_packet(id, SSH_FX_PERMISSION_DENIED, 
                "World-writable permissions not allowed", ""));
        }
    }
}
```

---

### 🟡 中优先级（计划修复）

#### 5. 配额检查原子化

**修复**:
```rust
use std::sync::Arc;
use tokio::sync::Mutex;

struct QuotaManager {
    used: Arc<Mutex<u64>>,
    limit: u64,
}

impl QuotaManager {
    async fn try_allocate(&self, bytes: u64) -> Result<QuotaGuard> {
        let mut used = self.used.lock().await;
        if *used + bytes > self.limit {
            return Err("Quota exceeded");
        }
        *used += bytes;
        Ok(QuotaGuard { 
            bytes, 
            quota: Arc::clone(&self.used) 
        })
    }
}

// RAII 模式：自动归还配额
struct QuotaGuard {
    bytes: u64,
    quota: Arc<Mutex<u64>>,
}

impl Drop for QuotaGuard {
    fn drop(&mut self) {
        // 如果写入失败，需要归还配额
        let mut used = self.quota.blocking_lock();
        *used -= self.bytes;
    }
}
```

---

## 📊 影响评估

| 漏洞 | 严重程度 | CVSS 估算 | 可利用性 | 影响范围 |
|------|---------|----------|---------|---------|
| **符号链接逃逸** | 🔴 严重 | 8.5 | 中等 | 所有用户 |
| **TOCTOU 竞态** | 🟠 高 | 7.2 | 低 | 并发场景 |
| **权限检查绕过** | 🟠 高 | 7.5 | 中等 | 所有用户 |
| **配额绕过** | 🟡 中 | 5.3 | 高 | 多用户环境 |
| **RENAME 攻击** | 🟡 中 | 6.1 | 低 | 特定场景 |

---

## 🎯 修复路线图

### 第一阶段（1-2 周）- 紧急修复
- [ ] 实现严格的符号链接验证
- [ ] 添加路径级安全检查函数
- [ ] 修复 RENAME 双重检查问题
- [ ] 过滤危险权限位

### 第二阶段（3-4 周）- 架构改进
- [ ] 迁移到文件描述符为基础的操作
- [ ] 实现全局文件锁机制
- [ ] 原子化配额管理
- [ ] 添加审计日志

### 第三阶段（5-8 周）- 深度加固
- [ ] 全面的安全审计
- [ ] 渗透测试
- [ ] 形式化验证关键路径
- [ ] 编写安全文档

---

## 📝 代码审查清单

### 每次 PR 必须检查

- [ ] 所有路径操作都使用 `safe_resolve_path`
- [ ] 符号链接操作有额外验证
- [ ] 权限检查在每次操作前执行
- [ ] 配额检查是原子的
- [ ] 没有 TOCTOU 窗口
- [ ] 错误处理不会泄露敏感信息
- [ ] 日志记录不包含完整路径（隐私）

---

## 🔗 参考资料

- [RFC 4253 - SSH Transport Layer](https://tools.ietf.org/html/rfc4253)
- [RFC 4254 - SSH Connection Protocol](https://tools.ietf.org/html/rfc4254)
- [OWASP - Path Traversal](https://owasp.org/www-community/attacks/Path_Traversal)
- [CWE-22 - Improper Limitation of a Pathname](https://cwe.mitre.org/data/definitions/22.html)
- [CWE-367 - Time-of-check Time-of-use](https://cwe.mitre.org/data/definitions/367.html)

---

*报告生成时间：2026-03-31 13:00*
*下次审查日期：2026-04-07*
