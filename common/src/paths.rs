//! XDG 路径解析（用户态运行模型）
//!
//! 前后端程序均以当前桌面登录用户运行，所有可写路径都遵循 XDG 规范：
//!
//! | 用途     | 路径                                          |
//! |----------|-----------------------------------------------|
//! | 配置     | `$XDG_CONFIG_HOME/wftpd`（默认 `~/.config/wftpd`） |
//! | 持久状态 | `$XDG_STATE_HOME/wftpd`（默认 `~/.local/state/wftpd`，日志、SSH 主机密钥） |
//! | 运行时   | `$XDG_RUNTIME_DIR/wftpd`（默认 `/tmp/wftpd-$UID`，仅存放 UDS 套接字） |
//!
//! 环境变量 `WFTPD_CONFIG_DIR` / `WFTPD_STATE_DIR` 可强制覆盖配置与状态目录，
//! 便于测试与多实例共存。

use std::path::PathBuf;

fn xdg_dir(env_var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(env_var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/"));
            home.join(fallback)
        })
}

fn override_dir(env_var: &str) -> Option<PathBuf> {
    std::env::var_os(env_var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// 配置目录（config.toml、users.json、用户公钥目录）
pub fn config_dir() -> PathBuf {
    override_dir("WFTPD_CONFIG_DIR")
        .unwrap_or_else(|| xdg_dir("XDG_CONFIG_HOME", ".config").join("wftpd"))
}

/// 持久状态目录（日志、SSH 主机密钥等跨重启保留的数据）
pub fn state_dir() -> PathBuf {
    override_dir("WFTPD_STATE_DIR")
        .unwrap_or_else(|| xdg_dir("XDG_STATE_HOME", ".local/state").join("wftpd"))
}

/// 运行时目录（UDS 套接字等生命周期与登录会话一致的数据）
pub fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("wftpd"),
        None => {
            let uid = nix::unistd::getuid();
            PathBuf::from(format!("/tmp/wftpd-{}", uid))
        }
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn users_path() -> PathBuf {
    config_dir().join("users.json")
}

/// SFTP 公钥认证的用户密钥目录（`<keys_dir>/<username>/authorized_keys`）
pub fn keys_dir() -> PathBuf {
    config_dir().join("keys")
}

pub fn default_log_dir() -> PathBuf {
    state_dir().join("logs")
}

pub fn default_host_key_path() -> PathBuf {
    state_dir().join("ssh").join("ssh_host_ed25519_key")
}

/// 前后端 gRPC(UDS) 套接字路径
pub fn socket_path() -> PathBuf {
    runtime_dir().join("wftpd.sock")
}

pub fn audit_log_path() -> PathBuf {
    default_log_dir().join("audit.log")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paths_are_scoped_to_wftpd() {
        assert!(config_path().ends_with("config.toml"));
        assert!(users_path().ends_with("users.json"));
        assert!(socket_path().ends_with("wftpd.sock"));
        assert!(default_host_key_path().ends_with("ssh_host_ed25519_key"));
    }
}
