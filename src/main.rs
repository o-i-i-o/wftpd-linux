mod lib;

use std::os::raw::{c_char, c_int, c_void};
use std::ffi::{CStr, CString};
use std::sync::Arc;

use lib::AppState;
use lib::users::Permissions;

#[no_mangle]
pub extern "C" fn wftpg_init() -> *mut c_void {
    let state = Arc::new(AppState::new());
    Arc::into_raw(state) as *mut c_void
}

#[no_mangle]
pub extern "C" fn wftpg_free(state: *mut c_void) {
    if !state.is_null() {
        unsafe {
            let _ = Arc::from_raw(state as *const AppState);
        }
    }
}

#[no_mangle]
pub extern "C" fn wftpg_start_ftp(state: *mut c_void) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    let result = state.start_ftp();
    std::mem::forget(state);
    if result.is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn wftpg_stop_ftp(state: *mut c_void) {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    state.stop_ftp();
    std::mem::forget(state);
}

#[no_mangle]
pub extern "C" fn wftpg_start_sftp(state: *mut c_void) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    let result = state.start_sftp();
    std::mem::forget(state);
    if result.is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn wftpg_stop_sftp(state: *mut c_void) {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    state.stop_sftp();
    std::mem::forget(state);
}

#[no_mangle]
pub extern "C" fn wftpg_is_ftp_running(state: *mut c_void) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    let running = state.is_ftp_running();
    std::mem::forget(state);
    if running { 1 } else { 0 }
}

#[no_mangle]
pub extern "C" fn wftpg_is_sftp_running(state: *mut c_void) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    let running = state.is_sftp_running();
    std::mem::forget(state);
    if running { 1 } else { 0 }
}

#[no_mangle]
pub extern "C" fn wftpg_add_user(state: *mut c_void, username: *const c_char, 
                                  password: *const c_char, home_dir: *const c_char,
                                  is_admin: c_int) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    
    let username = unsafe { CStr::from_ptr(username).to_string_lossy().to_string() };
    let password = unsafe { CStr::from_ptr(password).to_string_lossy().to_string() };
    let home_dir = unsafe { CStr::from_ptr(home_dir).to_string_lossy().to_string() };
    
    let mut users = state.user_manager.lock().unwrap();
    let result = users.add_user(&username, &password, &home_dir, is_admin != 0);
    drop(users);
    
    let _ = state.save_users();
    std::mem::forget(state);
    
    if result.is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn wftpg_remove_user(state: *mut c_void, username: *const c_char) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    
    let username = unsafe { CStr::from_ptr(username).to_string_lossy().to_string() };
    
    let mut users = state.user_manager.lock().unwrap();
    let result = users.remove_user(&username);
    drop(users);
    
    let _ = state.save_users();
    std::mem::forget(state);
    
    if result.is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn wftpg_update_user_password(state: *mut c_void, username: *const c_char,
                                              new_password: *const c_char) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    
    let username = unsafe { CStr::from_ptr(username).to_string_lossy().to_string() };
    let new_password = unsafe { CStr::from_ptr(new_password).to_string_lossy().to_string() };
    
    let mut users = state.user_manager.lock().unwrap();
    let result = users.update_password(&username, &new_password);
    drop(users);
    
    let _ = state.save_users();
    std::mem::forget(state);
    
    if result.is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn wftpg_get_user_count(state: *mut c_void) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    let users = state.user_manager.lock().unwrap();
    let count = users.user_count() as c_int;
    std::mem::forget(state);
    count
}

#[no_mangle]
pub extern "C" fn wftpg_get_user_list(state: *mut c_void, callback: extern "C" fn(*const c_char, *const c_char, c_int, c_int)) {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    let users = state.user_manager.lock().unwrap();
    
    for user in users.list_users() {
        let username = CString::new(user.username.as_str()).unwrap();
        let home_dir = CString::new(user.home_dir.as_str()).unwrap();
        callback(username.as_ptr(), home_dir.as_ptr(), if user.enabled { 1 } else { 0 }, if user.is_admin { 1 } else { 0 });
    }
    
    std::mem::forget(state);
}

#[no_mangle]
pub extern "C" fn wftpg_set_user_enabled(state: *mut c_void, username: *const c_char, enabled: c_int) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    
    let username = unsafe { CStr::from_ptr(username).to_string_lossy().to_string() };
    
    let mut users = state.user_manager.lock().unwrap();
    let result = users.set_user_enabled(&username, enabled != 0);
    drop(users);
    
    let _ = state.save_users();
    std::mem::forget(state);
    
    if result.is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn wftpg_update_user_permissions(state: *mut c_void, username: *const c_char,
                                                  can_read: c_int, can_write: c_int,
                                                  can_delete: c_int, can_list: c_int,
                                                  can_mkdir: c_int, can_rmdir: c_int,
                                                  can_rename: c_int, can_append: c_int) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    
    let username = unsafe { CStr::from_ptr(username).to_string_lossy().to_string() };
    
    let permissions = Permissions {
        can_read: can_read != 0,
        can_write: can_write != 0,
        can_delete: can_delete != 0,
        can_list: can_list != 0,
        can_mkdir: can_mkdir != 0,
        can_rmdir: can_rmdir != 0,
        can_rename: can_rename != 0,
        can_append: can_append != 0,
        quota_mb: None,
        speed_limit_kbps: None,
    };
    
    let mut users = state.user_manager.lock().unwrap();
    let result = users.update_permissions(&username, permissions);
    drop(users);
    
    let _ = state.save_users();
    std::mem::forget(state);
    
    if result.is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn wftpg_get_config(state: *mut c_void, 
                                    bind_ip: *mut *mut c_char,
                                    ftp_port: *mut c_int,
                                    sftp_port: *mut c_int,
                                    ftp_enabled: *mut c_int,
                                    sftp_enabled: *mut c_int,
                                    ftp_home: *mut *mut c_char,
                                    sftp_home: *mut *mut c_char) {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    let config = state.config.lock().unwrap();
    
    unsafe {
        *bind_ip = CString::new(config.server.bind_ip.as_str()).unwrap().into_raw();
        *ftp_port = config.server.ftp_port as c_int;
        *sftp_port = config.server.sftp_port as c_int;
        *ftp_enabled = if config.ftp.enabled { 1 } else { 0 };
        *sftp_enabled = if config.sftp.enabled { 1 } else { 0 };
        *ftp_home = CString::new(config.ftp.default_home.as_str()).unwrap().into_raw();
        *sftp_home = CString::new(config.sftp.default_home.as_str()).unwrap().into_raw();
    }
    
    std::mem::forget(state);
}

#[no_mangle]
pub extern "C" fn wftpg_set_config(state: *mut c_void,
                                    bind_ip: *const c_char,
                                    ftp_port: c_int,
                                    sftp_port: c_int,
                                    ftp_enabled: c_int,
                                    sftp_enabled: c_int,
                                    ftp_home: *const c_char,
                                    sftp_home: *const c_char) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    
    {
        let mut config = state.config.lock().unwrap();
        config.server.bind_ip = unsafe { CStr::from_ptr(bind_ip).to_string_lossy().to_string() };
        config.server.ftp_port = ftp_port as u16;
        config.server.sftp_port = sftp_port as u16;
        config.ftp.enabled = ftp_enabled != 0;
        config.sftp.enabled = sftp_enabled != 0;
        config.ftp.default_home = unsafe { CStr::from_ptr(ftp_home).to_string_lossy().to_string() };
        config.sftp.default_home = unsafe { CStr::from_ptr(sftp_home).to_string_lossy().to_string() };
    }
    
    let result = state.save_config();
    std::mem::forget(state);
    
    if result.is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn wftpg_add_allowed_ip(state: *mut c_void, ip: *const c_char) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    
    let ip = unsafe { CStr::from_ptr(ip).to_string_lossy().to_string() };
    
    {
        let mut config = state.config.lock().unwrap();
        if !config.security.allowed_ips.contains(&ip) {
            config.security.allowed_ips.push(ip);
        }
    }
    
    let result = state.save_config();
    std::mem::forget(state);
    
    if result.is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn wftpg_remove_allowed_ip(state: *mut c_void, ip: *const c_char) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    
    let ip = unsafe { CStr::from_ptr(ip).to_string_lossy().to_string() };
    
    {
        let mut config = state.config.lock().unwrap();
        config.security.allowed_ips.retain(|x: &String| x != &ip);
    }
    
    let result = state.save_config();
    std::mem::forget(state);
    
    if result.is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn wftpg_add_denied_ip(state: *mut c_void, ip: *const c_char) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    
    let ip = unsafe { CStr::from_ptr(ip).to_string_lossy().to_string() };
    
    {
        let mut config = state.config.lock().unwrap();
        if !config.security.denied_ips.contains(&ip) {
            config.security.denied_ips.push(ip);
        }
    }
    
    let result = state.save_config();
    std::mem::forget(state);
    
    if result.is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn wftpg_remove_denied_ip(state: *mut c_void, ip: *const c_char) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    
    let ip = unsafe { CStr::from_ptr(ip).to_string_lossy().to_string() };
    
    {
        let mut config = state.config.lock().unwrap();
        config.security.denied_ips.retain(|x: &String| x != &ip);
    }
    
    let result = state.save_config();
    std::mem::forget(state);
    
    if result.is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn wftpg_get_allowed_ips(state: *mut c_void, callback: extern "C" fn(*const c_char)) {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    let config = state.config.lock().unwrap();
    
    for ip in &config.security.allowed_ips {
        let ip_c = CString::new(ip.as_str()).unwrap();
        callback(ip_c.as_ptr());
    }
    
    std::mem::forget(state);
}

#[no_mangle]
pub extern "C" fn wftpg_get_denied_ips(state: *mut c_void, callback: extern "C" fn(*const c_char)) {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    let config = state.config.lock().unwrap();
    
    for ip in &config.security.denied_ips {
        let ip_c = CString::new(ip.as_str()).unwrap();
        callback(ip_c.as_ptr());
    }
    
    std::mem::forget(state);
}

#[no_mangle]
pub extern "C" fn wftpg_get_logs(state: *mut c_void, count: c_int,
                                  callback: extern "C" fn(*const c_char, *const c_char, *const c_char, *const c_char, *const c_char)) {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    let logger = state.logger.lock().unwrap();
    let logs = logger.get_recent_logs(count as usize);
    
    for entry in logs {
        let timestamp = CString::new(entry.timestamp.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap();
        let level = CString::new(entry.level.to_string()).unwrap();
        let source = CString::new(entry.source.as_str()).unwrap();
        let message = CString::new(entry.message.as_str()).unwrap();
        let client_ip = CString::new(entry.client_ip.unwrap_or_else(|| "-".to_string()).as_str()).unwrap();
        callback(timestamp.as_ptr(), level.as_ptr(), source.as_ptr(), message.as_ptr(), client_ip.as_ptr());
    }
    
    std::mem::forget(state);
}

#[no_mangle]
pub extern "C" fn wftpg_install_service(state: *mut c_void, binary_path: *const c_char) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    let binary_path = unsafe { CStr::from_ptr(binary_path).to_string_lossy().to_string() };
    
    let result = state.service_manager.install_service(&binary_path);
    if result.is_ok() {
        let _ = state.service_manager.reload_daemon();
    }
    
    std::mem::forget(state);
    
    if result.is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn wftpg_uninstall_service(state: *mut c_void) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    
    let result = state.service_manager.uninstall_service();
    if result.is_ok() {
        let _ = state.service_manager.reload_daemon();
    }
    
    std::mem::forget(state);
    
    if result.is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn wftpg_service_start(state: *mut c_void) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    let result = state.service_manager.start_service();
    std::mem::forget(state);
    
    if result.is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn wftpg_service_stop(state: *mut c_void) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    let result = state.service_manager.stop_service();
    std::mem::forget(state);
    
    if result.is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn wftpg_service_enable(state: *mut c_void) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    let result = state.service_manager.enable_service();
    std::mem::forget(state);
    
    if result.is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn wftpg_service_disable(state: *mut c_void) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    let result = state.service_manager.disable_service();
    std::mem::forget(state);
    
    if result.is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn wftpg_is_service_running(state: *mut c_void) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    let running = state.service_manager.is_service_running();
    std::mem::forget(state);
    
    if running { 1 } else { 0 }
}

#[no_mangle]
pub extern "C" fn wftpg_is_service_enabled(state: *mut c_void) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    let enabled = state.service_manager.is_service_enabled();
    std::mem::forget(state);
    
    if enabled { 1 } else { 0 }
}

#[no_mangle]
pub extern "C" fn wftpg_service_exists(state: *mut c_void) -> c_int {
    let state = unsafe { Arc::from_raw(state as *const AppState) };
    let exists = state.service_manager.service_exists();
    std::mem::forget(state);
    
    if exists { 1 } else { 0 }
}

#[no_mangle]
pub extern "C" fn wftpg_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            let _ = CString::from_raw(s);
        }
    }
}

fn main() {
    println!("WFTPG - SFTP/FTP Server Management Tool");
    println!("This is a library for Qt5 GUI. Run the GUI application instead.");
}
