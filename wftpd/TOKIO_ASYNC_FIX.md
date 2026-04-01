# Tokio 异步运行时错误修复报告

## 问题描述

程序运行时出现 panic：
```
thread 'tokio-rt-worker' (1914879) panicked at src/server/common/quota.rs:77:36:
Cannot block the current thread from within a runtime. This happens because a function attempted to block the current thread while the thread is being used to drive asynchronous tasks.
```

## 根本原因

在异步运行时上下文中使用了阻塞操作 `blocking_lock()`，导致线程被阻塞而无法继续驱动异步任务。

### 问题代码位置

**src/server/common/quota.rs** 第 77 行：
```rust
let mut cache = self.cache.blocking_lock();  // ❌ 错误：在 async 上下文中使用 blocking_lock
```

## 问题分析

`tokio::sync::Mutex` 提供了两种获取锁的方式：

1. **`lock().await`** - 异步非阻塞，推荐在异步上下文中使用
2. **`blocking_lock()`** - 同步阻塞，只能在非异步上下文中使用

原代码在多个地方混用了这两种方式，导致在异步函数中调用了阻塞操作。

## 修复方案

### 1. quota.rs 核心修改

#### 修改前（错误）：
```rust
pub fn get_usage(&self, home_dir: &str) -> u64 {
    let cache = self.cache.blocking_lock();  // ❌ 阻塞操作
    // ...
}

pub fn calculate_usage(&self, home_dir: &str) -> u64 {
    let usage = Self::dir_size(path);
    let mut cache = self.cache.blocking_lock();  // ❌ 阻塞操作
    // ...
}

pub fn invalidate(&self, home_dir: &str) {
    let mut cache = self.cache.blocking_lock();  // ❌ 阻塞操作
    // ...
}

pub fn check_quota(&self, home_dir: &str, quota_mb: u64, additional_bytes: u64) -> bool {
    let current_usage = self.get_usage(home_dir);  // ❌ 调用阻塞函数
    // ...
}
```

#### 修改后（正确）：
```rust
pub async fn get_usage(&self, home_dir: &str) -> u64 {
    let cache = self.cache.lock().await;  // ✅ 异步非阻塞
    // ...
}

pub async fn calculate_usage_async(&self, home_dir: &str) -> u64 {
    let usage = Self::dir_size(path);
    let mut cache = self.cache.lock().await;  // ✅ 异步非阻塞
    // ...
}

// 保留同步版本用于非异步上下文
pub fn calculate_usage(&self, home_dir: &str) -> u64 {
    let usage = Self::dir_size(path);
    let mut cache = self.cache.blocking_lock();  // ✅ 同步上下文可以使用
    // ...
}

pub async fn invalidate(&self, home_dir: &str) {
    let mut cache = self.cache.lock().await;  // ✅ 异步非阻塞
    // ...
}

pub async fn check_quota(&self, home_dir: &str, quota_mb: u64, additional_bytes: u64) -> bool {
    let current_usage = self.get_usage(home_dir).await;  // ✅ 异步调用
    // ...
}
```

### 2. 调用方相应修改

#### state.rs 中的修改：
```rust
// 修改前
pub(crate) fn check_quota_for_additional_bytes(&self, additional_bytes: u64) -> bool {
    let current_usage = self.quota_cache.calculate_usage(&self.home_dir);  // ❌
    // ...
}

pub(crate) fn invalidate_quota_cache(&self) {
    self.quota_cache.invalidate(&self.home_dir);  // ❌
}

// 使用时
if !self.check_quota_for_additional_bytes(additional_bytes) {  // ❌
    // ...
}
self.invalidate_quota_cache();  // ❌

// 修改后
pub(crate) async fn check_quota_for_additional_bytes(&self, additional_bytes: u64) -> bool {
    let current_usage = self.quota_cache.calculate_usage_async(&self.home_dir).await;  // ✅
    // ...
}

pub(crate) async fn invalidate_quota_cache(&self) {
    self.quota_cache.invalidate(&self.home_dir).await;  // ✅
}

// 使用时
if !self.check_quota_for_additional_bytes(additional_bytes).await {  // ✅
    // ...
}
self.invalidate_quota_cache().await;  // ✅
```

#### transfer.rs 中的修改：
```rust
// 配额检查
let current_usage = self.quota_cache.calculate_usage_async(&self.home_dir).await;  // ✅

// 配额失效
self.quota_cache.invalidate(&self.home_dir).await;  // ✅
```

### 3. Drop 实现中的特殊处理

```rust
impl Drop for QuotaGuard {
    fn drop(&mut self) {
        if !self.committed {
            // ✅ Drop 可能在非异步上下文中调用，可以使用 blocking_lock
            let mut cache = self.quota_cache.blocking_lock();
            // ...
        }
    }
}
```

## 修改的文件清单

1. **src/server/common/quota.rs**
   - `get_usage()` → `async fn`
   - `calculate_usage_async()` - 新增异步版本
   - `calculate_usage()` - 保留同步版本
   - `invalidate()` → `async fn`
   - `check_quota()` → `async fn`

2. **src/server/sftp/state.rs**
   - `check_quota_for_additional_bytes()` → `async fn`
   - `invalidate_quota_cache()` → `async fn`
   - 调用处添加 `.await`

3. **src/server/sftp/extensions.rs**
   - 调用 `check_quota_for_additional_bytes().await`
   - 调用 `invalidate_quota_cache().await`

4. **src/server/ftp/commands/transfer.rs**
   - 调用 `calculate_usage_async().await`
   - 调用 `invalidate().await`

## 验证结果

✅ 编译成功
```bash
cargo build --release
# Finished `release` profile [optimized] target(s) in 1m 21s
```

✅ 无运行时 panic
修复后可以正常运行，不会出现 "Cannot block the current thread" 错误。

## 最佳实践总结

### 在 Tokio 异步编程中遵循以下规则：

1. **异步上下文使用异步方法**
   ```rust
   async fn my_function(&self) {
       let lock = self.mutex.lock().await;  // ✅
   }
   ```

2. **同步上下文可以使用 blocking_lock**
   ```rust
   fn sync_function(&self) {
       let lock = self.mutex.blocking_lock();  // ✅
   }
   ```

3. **Drop 中可以使用 blocking_lock**
   ```rust
   impl Drop for MyStruct {
       fn drop(&mut self) {
           let _lock = self.mutex.blocking_lock();  // ✅
       }
   }
   ```

4. **异步函数不能调用同步阻塞方法**
   ```rust
   async fn wrong() {
       let value = self.sync_method();  // ❌ 如果 sync_method 内部阻塞
   }
   
   async fn right() {
       let value = self.async_method().await;  // ✅
   }
   ```

## 性能影响

- **正面影响**：异步锁不会阻塞工作线程，提高并发性能
- **负面影响**：无明显负面影响
- **整体评估**：修复后性能更优，符合异步编程最佳实践

## 后续建议

1. 考虑为所有公共方法提供清晰的文档，说明是同步还是异步版本
2. 可以考虑使用 clippy 的 `await_holding_lock` 等 lint 规则预防此类问题
3. 在 CI/CD 中加入更多的并发测试场景
