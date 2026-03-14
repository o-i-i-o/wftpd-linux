use anyhow::{Context, Result};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::Path;

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
        if self.can_read { perms.push("读"); }
        if self.can_write { perms.push("写"); }
        if self.can_delete { perms.push("删"); }
        if self.can_list { perms.push("列表"); }
        if self.can_mkdir { perms.push("建目录"); }
        if self.can_rmdir { perms.push("删目录"); }
        if self.can_rename { perms.push("重命名"); }
        if self.can_append { perms.push("追加"); }
        write!(f, "{}", perms.join(","))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserManager {
    users: HashMap<String, User>,
}

impl UserManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::new());
        }

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Warning: Failed to read users file: {}", e);
                return Ok(Self::new());
            }
        };

        if content.trim().is_empty() {
            return Ok(Self::new());
        }

        let manager: UserManager = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Warning: Failed to parse users file: {}", e);
                return Ok(Self::new());
            }
        };

        Ok(manager)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create users directory")?;
        }

        let content = serde_json::to_string_pretty(self).context("Failed to serialize users")?;

        fs::write(path, content).context("Failed to write users file")?;

        Ok(())
    }

    fn hash_password(password: &str) -> Result<String> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| anyhow::anyhow!("Failed to hash password: {}", e))?
            .to_string();
        Ok(hash)
    }

    fn verify_password(password: &str, hash: &str) -> bool {
        let parsed_hash = match PasswordHash::new(hash) {
            Ok(h) => h,
            Err(_) => return false,
        };
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok()
    }

    pub fn add_user(
        &mut self,
        username: &str,
        password: &str,
        home_dir: &str,
        is_admin: bool,
    ) -> Result<()> {
        if self.users.contains_key(username) {
            anyhow::bail!("User already exists: {}", username);
        }

        let password_hash = Self::hash_password(password)?;
        let user = User {
            username: username.to_string(),
            password_hash,
            home_dir: home_dir.to_string(),
            permissions: Permissions::full(),
            created_at: Utc::now(),
            last_login: None,
            enabled: true,
            is_admin,
        };

        self.users.insert(username.to_string(), user);
        Ok(())
    }

    pub fn remove_user(&mut self, username: &str) -> Result<()> {
        if self.users.remove(username).is_none() {
            anyhow::bail!("User not found: {}", username);
        }
        Ok(())
    }

    pub fn update_password(&mut self, username: &str, new_password: &str) -> Result<()> {
        let user = self
            .users
            .get_mut(username)
            .ok_or_else(|| anyhow::anyhow!("User not found: {}", username))?;

        user.password_hash = Self::hash_password(new_password)?;
        Ok(())
    }

    pub fn update_home_dir(&mut self, username: &str, home_dir: &str) -> Result<()> {
        let user = self
            .users
            .get_mut(username)
            .ok_or_else(|| anyhow::anyhow!("User not found: {}", username))?;

        user.home_dir = home_dir.to_string();
        Ok(())
    }

    pub fn update_permissions(
        &mut self,
        username: &str,
        permissions: Permissions,
    ) -> Result<()> {
        let user = self
            .users
            .get_mut(username)
            .ok_or_else(|| anyhow::anyhow!("User not found: {}", username))?;

        user.permissions = permissions;
        Ok(())
    }

    pub fn set_user_enabled(&mut self, username: &str, enabled: bool) -> Result<()> {
        let user = self
            .users
            .get_mut(username)
            .ok_or_else(|| anyhow::anyhow!("User not found: {}", username))?;

        user.enabled = enabled;
        Ok(())
    }

    pub fn authenticate(&mut self, username: &str, password: &str) -> Result<bool> {
        let user = self
            .users
            .get_mut(username)
            .ok_or_else(|| anyhow::anyhow!("User not found: {}", username))?;

        if !user.enabled {
            return Ok(false);
        }

        if Self::verify_password(password, &user.password_hash) {
            user.last_login = Some(Utc::now());
            return Ok(true);
        }

        Ok(false)
    }

    pub fn reload(&mut self, path: &Path) -> Result<()> {
        if !path.exists() {
            return Ok(());
        }

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };

        if content.trim().is_empty() {
            return Ok(());
        }

        let manager: UserManager = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(_) => return Ok(()),
        };

        self.users = manager.users;
        Ok(())
    }

    pub fn get_user(&self, username: &str) -> Option<&User> {
        self.users.get(username)
    }

    pub fn get_users(&self) -> &std::collections::HashMap<String, User> {
        &self.users
    }

    pub fn get_all_users(&self) -> Vec<User> {
        self.users.values().cloned().collect()
    }

    pub fn list_users(&self) -> impl Iterator<Item = (&String, &User)> {
        self.users.iter()
    }
}
