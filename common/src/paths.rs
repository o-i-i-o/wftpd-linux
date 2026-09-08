//! XDG 路径解析（用户态运行模型）
//!
//! 前后端程序均以当前桌面登录用户运行，所有可写路径都遵循 XDG 规范：
//!
//! | 用途     | 路径                                          |
//! |----------|-----------------------------------------------|
//! | 配置     | 二进制同目录 `config.toml`（存在即用、可写即默认生成，便于测试）；否则 `$XDG_CONFIG_HOME/wftpd`（默认 `~/.config/wftpd`，deb 安装由 postinst 预置） |
//! | 持久状态 | `$XDG_STATE_HOME/wftpd`（默认 `~/.local/state/wftpd`，日志、SSH 主机密钥） |
//! | 运行时   | `$XDG_RUNTIME_DIR/wftpd`（默认 `/tmp/wftpd-$UID`，仅存放 UDS 套接字） |
//!
//! 环境变量 `WFTPD_CONFIG_DIR` / `WFTPD_STATE_DIR` 可强制覆盖配置与状态目录，
//! 便于测试与多实例共存。

use std::path::PathBuf;

/// 当前用户主目录（`HOME` 未设置时回退到 `/`）
#[must_use]
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

fn xdg_dir(env_var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(env_var)
        .filter(|v| !v.is_empty())
        .map_or_else(|| home_dir().join(fallback), PathBuf::from)
}

fn override_dir(env_var: &str) -> Option<PathBuf> {
    std::env::var_os(env_var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// 配置目录（users.json、用户公钥目录；config.toml 见 [`config_path`]）
#[must_use]
pub fn config_dir() -> PathBuf {
    override_dir("WFTPD_CONFIG_DIR")
        .unwrap_or_else(|| xdg_dir("XDG_CONFIG_HOME", ".config").join("wftpd"))
}

/// 配置文件路径，按顺序解析：
///
/// 1. `WFTPD_CONFIG_DIR` 环境变量覆盖（测试/多实例）；
/// 2. 二进制同目录 `config.toml`——存在即使用；目录可写（如 `cargo build`
///    产物所在的 `target/`）时，缺失的默认配置也生成到这里，便于直接运行
///    二进制做测试。安装形态的二进制位于 `/usr/bin`，对桌面用户不可写，
///    自然落到 3；root 跳过本层，避免污染系统目录；
/// 3. 用户 XDG 配置目录（deb 安装形态，postinst 已预置初始配置）。
#[must_use]
pub fn config_path() -> PathBuf {
    if let Some(dir) = override_dir("WFTPD_CONFIG_DIR") {
        return dir.join("config.toml");
    }

    if !nix::unistd::getuid().is_root()
        && let Some(exe_dir) = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
    {
        let candidate = exe_dir.join("config.toml");
        let dir_writable = nix::unistd::access(&exe_dir, nix::unistd::AccessFlags::W_OK).is_ok();
        if candidate.is_file() || dir_writable {
            return candidate;
        }
    }

    config_dir().join("config.toml")
}

/// 持久状态目录（日志、SSH 主机密钥等跨重启保留的数据）
#[must_use]
pub fn state_dir() -> PathBuf {
    override_dir("WFTPD_STATE_DIR")
        .unwrap_or_else(|| xdg_dir("XDG_STATE_HOME", ".local/state").join("wftpd"))
}

/// 运行时目录（UDS 套接字等生命周期与登录会话一致的数据）
#[must_use]
pub fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        PathBuf::from(dir).join("wftpd")
    } else {
        let uid = nix::unistd::getuid();
        PathBuf::from(format!("/tmp/wftpd-{uid}"))
    }
}

#[must_use]
pub fn users_path() -> PathBuf {
    config_dir().join("users.json")
}

/// SFTP 公钥认证的用户密钥目录（`<keys_dir>/<username>/authorized_keys`）
#[must_use]
pub fn keys_dir() -> PathBuf {
    config_dir().join("keys")
}

#[must_use]
pub fn default_log_dir() -> PathBuf {
    state_dir().join("logs")
}

#[must_use]
pub fn default_host_key_path() -> PathBuf {
    state_dir().join("ssh").join("ssh_host_ed25519_key")
}

/// 前后端 gRPC(UDS) 套接字路径
#[must_use]
pub fn socket_path() -> PathBuf {
    runtime_dir().join("wftpd.sock")
}

#[must_use]
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

    #[test]
    fn test_config_path_binary_dir_resolution() {
        // cargo test 的二进制位于 target/ 下：目录可写且非 root 时，
        // 配置解析到二进制同目录；否则（如 root、只读目录）落到用户 XDG 目录
        let exe_dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let binary_dir_usable = !nix::unistd::getuid().is_root()
            && nix::unistd::access(&exe_dir, nix::unistd::AccessFlags::W_OK).is_ok();

        if binary_dir_usable {
            assert_eq!(config_path(), exe_dir.join("config.toml"));
        } else {
            assert_eq!(config_path(), config_dir().join("config.toml"));
        }
    }
}
