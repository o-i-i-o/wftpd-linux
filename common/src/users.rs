use anyhow::{Context, Result};
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::Path;
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub username: String,
    pub password_hash: String,
    pub home_dir: String,
    pub permissions: Permissions,
    pub created_at: DateTime<Utc>,
    pub last_login: Option<DateTime<Utc>>,
    pub enabled: bool,
    pub is_admin: bool,
}

// 权限模型即布尔位集合，字段数量为领域固有，非设计缺陷
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct Permissions {
    pub can_read: bool,
    pub can_write: bool,
    pub can_delete: bool,
    pub can_list: bool,
    pub can_mkdir: bool,
    pub can_rmdir: bool,
    pub can_rename: bool,
    pub can_append: bool,
    pub quota_mb: Option<u64>,
    pub speed_limit_kbps: Option<u64>,
}

impl Permissions {
    #[must_use]
    pub fn full() -> Self {
        Permissions {
            can_read: true,
            can_write: true,
            can_delete: true,
            can_list: true,
            can_mkdir: true,
            can_rmdir: true,
            can_rename: true,
            can_append: true,
            quota_mb: None,
            speed_limit_kbps: None,
        }
    }
}

impl fmt::Display for Permissions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut perms = Vec::new();
        if self.can_read {
            perms.push("读");
        }
        if self.can_write {
            perms.push("写");
        }
        if self.can_delete {
            perms.push("删");
        }
        if self.can_list {
            perms.push("列表");
        }
        if self.can_mkdir {
            perms.push("建目录");
        }
        if self.can_rmdir {
            perms.push("删目录");
        }
        if self.can_rename {
            perms.push("重命名");
        }
        if self.can_append {
            perms.push("追加");
        }
        write!(f, "{}", perms.join(", "))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserManager {
    users: HashMap<String, User>,
}

impl UserManager {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 从 JSON 文件加载用户库
    ///
    /// 文件缺失、为空、读取失败或解析失败时均回退为空用户库并记录日志，
    /// 不视为致命错误。
    ///
    /// # Errors
    /// 当前实现不会返回 `Err`；保留 `Result` 签名以便未来引入可恢复错误
    pub fn load(path: &Path) -> Result<Self> {
        debug!("[UserManager] 尝试加载用户配置文件：{:?}", path);

        if !path.exists() {
            warn!(
                "[UserManager] 用户配置文件不存在：{:?}，将创建空用户管理器",
                path
            );
            return Ok(Self::new());
        }

        info!("[UserManager] 用户配置文件存在，开始读取：{:?}", path);

        let content = match fs::read_to_string(path) {
            Ok(c) => {
                debug!("[UserManager] 成功读取用户配置文件，大小：{} 字节", c.len());
                c
            }
            Err(e) => {
                error!("[UserManager] 读取用户配置文件失败：{} - {:?}", e, path);
                eprintln!("Warning: Failed to read users file: {e}");
                return Ok(Self::new());
            }
        };

        if content.trim().is_empty() {
            warn!("[UserManager] 用户配置文件内容为空：{:?}", path);
            return Ok(Self::new());
        }

        debug!("[UserManager] 开始解析 JSON 内容...");
        let manager: UserManager = serde_json::from_str(&content).unwrap_or_else(|e| {
            error!(
                "[UserManager] 解析用户配置文件 JSON 失败：{} - 内容预览：{}",
                e,
                &content[..content.len().min(200)]
            );
            eprintln!("Warning: Failed to parse users file: {e}");
            UserManager::new()
        });

        info!("[UserManager] 成功加载 {} 个用户", manager.users.len());
        for (username, user) in &manager.users {
            debug!(
                "[UserManager]   - 用户：{}, 启用：{}, 主目录：{}, 权限：{}",
                username, user.enabled, user.home_dir, user.permissions
            );
        }

        info!("[UserManager] 用户配置加载完成");
        Ok(manager)
    }

    /// 将用户库序列化为 JSON 并写入 `path`（先写临时文件再原子重命名）
    ///
    /// # Errors
    /// 父目录创建、序列化、临时文件写入或重命名失败时返回错误
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create users directory")?;
        }

        let content = serde_json::to_string_pretty(self).context("Failed to serialize users")?;

        let temp_path = path.with_extension("tmp");
        fs::write(&temp_path, content).context("Failed to write temp users file")?;

        fs::rename(&temp_path, path).context("Failed to rename temp users file")?;

        Ok(())
    }

    /// 使用 Argon2id 生成口令哈希（自动随机盐）
    ///
    /// # Errors
    /// Argon2 哈希计算失败（如盐生成失败）时返回错误
    fn hash_password(password: &str) -> Result<String> {
        // password-hash 0.6 起 hash_password 自动生成随机盐，无需外部 RNG
        let hash = Argon2::default()
            .hash_password(password.as_bytes())
            .map_err(|e| anyhow::anyhow!("Failed to hash password: {e}"))?
            .to_string();
        Ok(hash)
    }

    fn verify_password(password: &str, hash: &str) -> bool {
        let parsed_hash = match PasswordHash::new(hash) {
            Ok(h) => h,
            Err(e) => {
                warn!("Failed to parse password hash: {}", e);
                return false;
            }
        };
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok()
    }

    /// 新增用户
    ///
    /// # Errors
    /// 用户名为空、用户已存在、家目录为空/不存在/不是目录，或口令哈希失败时返回错误
    pub fn add_user(
        &mut self,
        username: String,
        password: &str,
        home_dir: String,
        permissions: Permissions,
        is_admin: bool,
    ) -> Result<()> {
        if username.is_empty() {
            return Err(anyhow::anyhow!("用户名不能为空"));
        }

        if self.users.contains_key(&username) {
            return Err(anyhow::anyhow!("用户已存在: {username}"));
        }

        if home_dir.trim().is_empty() {
            return Err(anyhow::anyhow!("家目录不能为空"));
        }

        let home_path = Path::new(&home_dir);
        if !home_path.exists() {
            return Err(anyhow::anyhow!("家目录不存在: {home_dir}"));
        }
        if !home_path.is_dir() {
            return Err(anyhow::anyhow!("家目录路径不是目录: {home_dir}"));
        }

        let password_hash = Self::hash_password(password)?;
        let user = User {
            username: username.clone(),
            password_hash,
            home_dir,
            permissions,
            created_at: Utc::now(),
            last_login: None,
            enabled: true,
            is_admin,
        };

        self.users.insert(username, user);
        Ok(())
    }

    /// 删除用户
    ///
    /// # Errors
    /// 用户不存在时返回错误
    pub fn remove_user(&mut self, username: &str) -> Result<()> {
        if self.users.remove(username).is_none() {
            return Err(anyhow::anyhow!("用户不存在: {username}"));
        }
        Ok(())
    }

    /// 更新用户口令
    ///
    /// # Errors
    /// 用户不存在或新口令哈希失败时返回错误
    pub fn update_password(&mut self, username: &str, new_password: &str) -> Result<()> {
        let user = self
            .users
            .get_mut(username)
            .ok_or_else(|| anyhow::anyhow!("用户不存在: {username}"))?;

        user.password_hash = Self::hash_password(new_password)?;
        Ok(())
    }

    /// 更新用户主目录
    ///
    /// # Errors
    /// 家目录为空/不存在/不是目录，或用户不存在时返回错误
    pub fn update_home_dir(&mut self, username: &str, home_dir: String) -> Result<()> {
        if home_dir.trim().is_empty() {
            return Err(anyhow::anyhow!("家目录不能为空"));
        }

        let home_path = Path::new(&home_dir);
        if !home_path.exists() {
            return Err(anyhow::anyhow!("家目录不存在: {home_dir}"));
        }
        if !home_path.is_dir() {
            return Err(anyhow::anyhow!("家目录路径不是目录: {home_dir}"));
        }

        let user = self
            .users
            .get_mut(username)
            .ok_or_else(|| anyhow::anyhow!("用户不存在: {username}"))?;

        user.home_dir = home_dir;
        Ok(())
    }

    /// 覆盖用户权限
    ///
    /// # Errors
    /// 用户不存在时返回错误
    pub fn update_permissions(&mut self, username: &str, permissions: Permissions) -> Result<()> {
        let user = self
            .users
            .get_mut(username)
            .ok_or_else(|| anyhow::anyhow!("用户不存在: {username}"))?;

        user.permissions = permissions;
        Ok(())
    }

    /// 启用或禁用用户
    ///
    /// # Errors
    /// 用户不存在时返回错误
    pub fn set_user_enabled(&mut self, username: &str, enabled: bool) -> Result<()> {
        let user = self
            .users
            .get_mut(username)
            .ok_or_else(|| anyhow::anyhow!("用户不存在: {username}"))?;

        user.enabled = enabled;
        Ok(())
    }

    /// 校验用户口令；成功时更新 `last_login` 并落盘用户库
    ///
    /// 用户不存在、已禁用或口令不匹配均返回 `Ok(false)`。
    ///
    /// # Errors
    /// 当前实现不会返回 `Err`；保留 `Result` 签名以便未来引入可恢复错误
    pub fn authenticate(&mut self, username: &str, password: &str) -> Result<bool> {
        let Some(user) = self.users.get_mut(username) else {
            return Ok(false);
        };

        if !user.enabled {
            return Ok(false);
        }

        if Self::verify_password(password, &user.password_hash) {
            user.last_login = Some(Utc::now());

            let users_path = crate::paths::users_path();
            if let Err(e) = self.save(&users_path) {
                eprintln!("Warning: Failed to persist last_login: {e}");
            }

            return Ok(true);
        }

        Ok(false)
    }

    /// 从磁盘重新加载用户库并整体替换内存副本
    ///
    /// 文件缺失、读取失败或解析失败时保留/回退为空库并记录日志，不视为致命错误。
    ///
    /// # Errors
    /// 当前实现不会返回 `Err`；保留 `Result` 签名以便未来引入可恢复错误
    pub fn reload(&mut self, path: &Path) -> Result<()> {
        debug!("[UserManager::reload] 尝试重新加载用户配置文件：{:?}", path);

        if !path.exists() {
            warn!("[UserManager::reload] 用户配置文件不存在：{:?}", path);
            return Ok(());
        }

        info!("[UserManager::reload] 开始重新加载用户配置文件：{:?}", path);

        let content = match fs::read_to_string(path) {
            Ok(c) => {
                debug!("[UserManager::reload] 成功读取文件，大小：{} 字节", c.len());
                c
            }
            Err(e) => {
                error!("[UserManager::reload] 读取文件失败：{} - {:?}", e, path);
                warn!("Failed to read users file during reload: {}", e);
                return Ok(());
            }
        };

        if content.trim().is_empty() {
            warn!("[UserManager::reload] 文件内容为空：{:?}", path);
            return Ok(());
        }

        debug!("[UserManager::reload] 开始解析 JSON...");
        let manager: UserManager = serde_json::from_str(&content).unwrap_or_else(|e| {
            error!(
                "[UserManager::reload] 解析 JSON 失败：{} - 内容预览：{}",
                e,
                &content[..content.len().min(200)]
            );
            warn!("Failed to parse users file during reload: {}", e);
            UserManager::new()
        });

        info!(
            "[UserManager::reload] 成功重新加载 {} 个用户 (原用户数：{})",
            manager.users.len(),
            self.users.len()
        );
        for (username, user) in &manager.users {
            debug!(
                "[UserManager::reload]   - 用户：{}, 启用：{}, 主目录：{}",
                username, user.enabled, user.home_dir
            );
        }

        let old_count = self.users.len();
        self.users = manager.users;
        info!(
            "[UserManager::reload] 重新加载完成，用户数变化：{} -> {}",
            old_count,
            self.users.len()
        );
        Ok(())
    }

    #[must_use]
    pub fn get_user(&self, username: &str) -> Option<&User> {
        self.users.get(username)
    }

    #[must_use]
    pub fn get_users(&self) -> &std::collections::HashMap<String, User> {
        &self.users
    }

    #[must_use]
    pub fn get_all_users(&self) -> Vec<User> {
        self.users.values().cloned().collect()
    }

    pub fn list_users(&self) -> impl Iterator<Item = (&String, &User)> {
        self.users.iter()
    }

    /// 校验匿名 FTP 主目录配置
    ///
    /// # Errors
    /// 目录为空、不存在或不是目录时返回错误
    pub fn validate_anonymous_home(home_dir: &str) -> Result<()> {
        if home_dir.trim().is_empty() {
            return Err(anyhow::anyhow!("匿名用户目录不能为空"));
        }

        let home_path = Path::new(home_dir);
        if !home_path.exists() {
            return Err(anyhow::anyhow!("匿名用户目录不存在: {home_dir}"));
        }
        if !home_path.is_dir() {
            return Err(anyhow::anyhow!("匿名用户目录路径不是目录: {home_dir}"));
        }

        Ok(())
    }
}
