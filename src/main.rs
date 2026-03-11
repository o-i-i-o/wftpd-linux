slint::include_modules!();

use std::rc::Rc;
use std::cell::RefCell;
use std::time::Duration;
use slint::{ModelRc, Weak};

use wftpg::AppState;

fn main() -> Result<(), slint::PlatformError> {
    env_logger::init();
    
    let state = Rc::new(RefCell::new(AppState::new()));
    
    let main_window = MainWindow::new()?;
    
    init_window_state(&main_window, &state.borrow());
    
    setup_ftp_callbacks(&main_window, &state);
    setup_sftp_callbacks(&main_window, &state);
    setup_config_callbacks(&main_window, &state);
    setup_service_callbacks(&main_window, &state);
    setup_user_callbacks(&main_window, &state);
    setup_ip_callbacks(&main_window, &state);
    setup_log_callbacks(&main_window, &state);
    
    main_window.run()
}

fn init_window_state(main_window: &MainWindow, state: &AppState) {
    let config = state.config.lock().unwrap();
    
    main_window.set_bind_ip(config.server.bind_ip.clone().into());
    main_window.set_ftp_port(config.server.ftp_port.to_string().into());
    main_window.set_sftp_port(config.server.sftp_port.to_string().into());
    main_window.set_max_connections(config.server.max_connections.to_string().into());
    main_window.set_connection_timeout(config.server.connection_timeout.to_string().into());
    main_window.set_idle_timeout(config.server.idle_timeout.to_string().into());
    
    main_window.set_ftp_home(config.ftp.default_home.clone().into());
    main_window.set_passive_port_min(config.ftp.passive_ports.0.to_string().into());
    main_window.set_passive_port_max(config.ftp.passive_ports.1.to_string().into());
    main_window.set_welcome_message(config.ftp.welcome_message.clone().into());
    main_window.set_allow_anonymous(config.ftp.allow_anonymous);
    main_window.set_anonymous_home(config.ftp.anonymous_home.clone().unwrap_or_default().into());
    
    main_window.set_sftp_home(config.sftp.default_home.clone().into());
    main_window.set_host_key_path(config.sftp.host_key_path.clone().into());
    main_window.set_max_auth_attempts(config.sftp.max_auth_attempts.to_string().into());
    main_window.set_auth_timeout(config.sftp.auth_timeout.to_string().into());
    
    main_window.set_max_login_attempts(config.security.max_login_attempts.to_string().into());
    main_window.set_ban_duration(config.security.ban_duration.to_string().into());
    main_window.set_require_ssl(config.security.require_ssl);
    main_window.set_cert_path(config.security.cert_path.clone().unwrap_or_default().into());
    main_window.set_key_path(config.security.key_path.clone().unwrap_or_default().into());
    
    main_window.set_log_dir(config.logging.log_dir.clone().into());
    main_window.set_log_level(config.logging.log_level.clone().into());
    main_window.set_max_log_size((config.logging.max_log_size / (1024 * 1024)).to_string().into());
    main_window.set_max_log_files(config.logging.max_log_files.to_string().into());
    main_window.set_log_to_file(config.logging.log_to_file);
    main_window.set_log_to_gui(config.logging.log_to_gui);
    
    main_window.set_ftp_running(state.is_ftp_running());
    main_window.set_sftp_running(state.is_sftp_running());
    main_window.set_service_installed(state.service_manager.service_exists());
    main_window.set_service_running(state.service_manager.is_service_running());
    main_window.set_auto_refresh_logs(true);
}

fn setup_ftp_callbacks(main_window: &MainWindow, state: &Rc<RefCell<AppState>>) {
    {
        let state = Rc::clone(state);
        let window = main_window.as_weak();
        
        main_window.on_start_ftp_clicked(move || {
            let state = state.borrow();
            match state.start_ftp() {
                Ok(_) => {
                    if let Some(window) = window.upgrade() {
                        window.set_ftp_running(true);
                        window.set_status_message("FTP服务器已启动".into());
                    }
                }
                Err(e) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(format!("FTP启动失败: {}", e).into());
                    }
                }
            }
        });
    }
    
    {
        let state = Rc::clone(state);
        let window = main_window.as_weak();
        
        main_window.on_stop_ftp_clicked(move || {
            let state = state.borrow();
            state.stop_ftp();
            if let Some(window) = window.upgrade() {
                window.set_ftp_running(false);
                window.set_status_message("FTP服务器已停止".into());
            }
        });
    }
}

fn setup_sftp_callbacks(main_window: &MainWindow, state: &Rc<RefCell<AppState>>) {
    {
        let state = Rc::clone(state);
        let window = main_window.as_weak();
        
        main_window.on_start_sftp_clicked(move || {
            let state = state.borrow();
            match state.start_sftp() {
                Ok(_) => {
                    if let Some(window) = window.upgrade() {
                        window.set_sftp_running(true);
                        window.set_status_message("SFTP服务器已启动".into());
                    }
                }
                Err(e) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(format!("SFTP启动失败: {}", e).into());
                    }
                }
            }
        });
    }
    
    {
        let state = Rc::clone(state);
        let window = main_window.as_weak();
        
        main_window.on_stop_sftp_clicked(move || {
            let state = state.borrow();
            state.stop_sftp();
            if let Some(window) = window.upgrade() {
                window.set_sftp_running(false);
                window.set_status_message("SFTP服务器已停止".into());
            }
        });
    }
}

fn setup_config_callbacks(main_window: &MainWindow, state: &Rc<RefCell<AppState>>) {
    let state = Rc::clone(state);
    let window = main_window.as_weak();
    
    main_window.on_save_config_clicked(move || {
        let state = state.borrow();
        
        if let Some(window) = window.upgrade() {
            let mut config = state.config.lock().unwrap();
            config.server.bind_ip = window.get_bind_ip().to_string();
            config.server.ftp_port = window.get_ftp_port().to_string().parse().unwrap_or(21);
            config.server.sftp_port = window.get_sftp_port().to_string().parse().unwrap_or(22);
            config.server.max_connections = window.get_max_connections().to_string().parse().unwrap_or(100);
            config.server.connection_timeout = window.get_connection_timeout().to_string().parse().unwrap_or(300);
            config.server.idle_timeout = window.get_idle_timeout().to_string().parse().unwrap_or(600);
            
            config.ftp.default_home = window.get_ftp_home().to_string();
            let port_min: u16 = window.get_passive_port_min().to_string().parse().unwrap_or(50000);
            let port_max: u16 = window.get_passive_port_max().to_string().parse().unwrap_or(51000);
            config.ftp.passive_ports = (port_min, port_max);
            config.ftp.welcome_message = window.get_welcome_message().to_string();
            config.ftp.allow_anonymous = window.get_allow_anonymous();
            let anon_home = window.get_anonymous_home().to_string();
            config.ftp.anonymous_home = if anon_home.is_empty() { None } else { Some(anon_home) };
            
            config.sftp.default_home = window.get_sftp_home().to_string();
            config.sftp.host_key_path = window.get_host_key_path().to_string();
            config.sftp.max_auth_attempts = window.get_max_auth_attempts().to_string().parse().unwrap_or(3);
            config.sftp.auth_timeout = window.get_auth_timeout().to_string().parse().unwrap_or(60);
            
            config.security.max_login_attempts = window.get_max_login_attempts().to_string().parse().unwrap_or(5);
            config.security.ban_duration = window.get_ban_duration().to_string().parse().unwrap_or(300);
            config.security.require_ssl = window.get_require_ssl();
            let cert_path = window.get_cert_path().to_string();
            config.security.cert_path = if cert_path.is_empty() { None } else { Some(cert_path) };
            let key_path = window.get_key_path().to_string();
            config.security.key_path = if key_path.is_empty() { None } else { Some(key_path) };
            
            config.logging.log_dir = window.get_log_dir().to_string();
            config.logging.log_level = window.get_log_level().to_string();
            let max_size_mb: u64 = window.get_max_log_size().to_string().parse().unwrap_or(10);
            config.logging.max_log_size = max_size_mb * 1024 * 1024;
            config.logging.max_log_files = window.get_max_log_files().to_string().parse().unwrap_or(10);
            config.logging.log_to_file = window.get_log_to_file();
            config.logging.log_to_gui = window.get_log_to_gui();
        }
        
        match state.save_config() {
            Ok(_) => {
                if let Some(window) = window.upgrade() {
                    window.set_status_message("配置已保存".into());
                }
            }
            Err(e) => {
                if let Some(window) = window.upgrade() {
                    window.set_status_message(format!("保存失败: {}", e).into());
                }
            }
        }
    });
}

fn setup_service_callbacks(main_window: &MainWindow, state: &Rc<RefCell<AppState>>) {
    {
        let state = Rc::clone(state);
        let window = main_window.as_weak();
        
        main_window.on_install_service_clicked(move || {
            let state = state.borrow();
            let exe_path = std::env::current_exe().unwrap_or_default();
            match state.service_manager.install_service(&exe_path.to_string_lossy()) {
                Ok(_) => {
                    let _ = state.service_manager.reload_daemon();
                    if let Some(window) = window.upgrade() {
                        window.set_service_installed(true);
                        window.set_status_message("服务已安装".into());
                    }
                }
                Err(e) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(format!("安装失败: {}", e).into());
                    }
                }
            }
        });
    }
    
    {
        let state = Rc::clone(state);
        let window = main_window.as_weak();
        
        main_window.on_uninstall_service_clicked(move || {
            let state = state.borrow();
            match state.service_manager.uninstall_service() {
                Ok(_) => {
                    let _ = state.service_manager.reload_daemon();
                    if let Some(window) = window.upgrade() {
                        window.set_service_installed(false);
                        window.set_service_running(false);
                        window.set_status_message("服务已卸载".into());
                    }
                }
                Err(e) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(format!("卸载失败: {}", e).into());
                    }
                }
            }
        });
    }
    
    {
        let state = Rc::clone(state);
        let window = main_window.as_weak();
        
        main_window.on_start_service_clicked(move || {
            let state = state.borrow();
            match state.service_manager.start_service() {
                Ok(_) => {
                    if let Some(window) = window.upgrade() {
                        window.set_service_running(true);
                        window.set_status_message("服务已启动".into());
                    }
                }
                Err(e) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(format!("启动失败: {}", e).into());
                    }
                }
            }
        });
    }
    
    {
        let state = Rc::clone(state);
        let window = main_window.as_weak();
        
        main_window.on_stop_service_clicked(move || {
            let state = state.borrow();
            match state.service_manager.stop_service() {
                Ok(_) => {
                    if let Some(window) = window.upgrade() {
                        window.set_service_running(false);
                        window.set_status_message("服务已停止".into());
                    }
                }
                Err(e) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(format!("停止失败: {}", e).into());
                    }
                }
            }
        });
    }
}

fn setup_user_callbacks(main_window: &MainWindow, state: &Rc<RefCell<AppState>>) {
    {
        let state = Rc::clone(state);
        let window = main_window.as_weak();
        
        main_window.on_load_users_clicked(move || {
            load_users_to_window(&window, &state);
        });
    }
    
    {
        let state = Rc::clone(state);
        let window = main_window.as_weak();
        
        main_window.on_add_user_clicked(move |username, password, home_dir, is_admin| {
            let username = username.to_string();
            let password = password.to_string();
            let home_dir = if home_dir.is_empty() {
                
                {
                    let state = state.borrow();
                    let config = state.config.lock().unwrap();
                    config.sftp.default_home.clone()
                }
            } else {
                home_dir.to_string()
            };
            
            let result = {
                let state = state.borrow_mut();
                let mut users = state.user_manager.lock().unwrap();
                
                match users.add_user(&username, &password, &home_dir, is_admin) {
                    Ok(_) => {
                        match users.save(&state.users_path) {
                            Ok(_) => Ok(()),
                            Err(e) => Err(format!("保存用户失败: {}", e)),
                        }
                    }
                    Err(e) => Err(format!("添加用户失败: {}", e)),
                }
            };
            
            match result {
                Ok(_) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(format!("用户 {} 已添加", username).into());
                        load_users_to_window(&window.as_weak(), &state);
                    }
                }
                Err(e) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(e.into());
                    }
                }
            }
        });
    }
    
    {
        let state = Rc::clone(state);
        let window = main_window.as_weak();
        
        main_window.on_remove_user_clicked(move |username| {
            let username = username.to_string();
            
            let result = {
                let state = state.borrow_mut();
                let mut users = state.user_manager.lock().unwrap();
                
                match users.remove_user(&username) {
                    Ok(_) => {
                        match users.save(&state.users_path) {
                            Ok(_) => Ok(()),
                            Err(e) => Err(format!("保存用户失败: {}", e)),
                        }
                    }
                    Err(e) => Err(format!("删除用户失败: {}", e)),
                }
            };
            
            match result {
                Ok(_) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(format!("用户 {} 已删除", username).into());
                        load_users_to_window(&window.as_weak(), &state);
                    }
                }
                Err(e) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(e.into());
                    }
                }
            }
        });
    }
    
    {
        let state = Rc::clone(state);
        let window = main_window.as_weak();
        
        main_window.on_toggle_user_enabled_clicked(move |username, enabled| {
            let username = username.to_string();
            
            let result = {
                let state = state.borrow_mut();
                let mut users = state.user_manager.lock().unwrap();
                
                match users.set_user_enabled(&username, enabled) {
                    Ok(_) => {
                        match users.save(&state.users_path) {
                            Ok(_) => Ok(()),
                            Err(e) => Err(format!("保存用户失败: {}", e)),
                        }
                    }
                    Err(e) => Err(format!("操作失败: {}", e)),
                }
            };
            
            match result {
                Ok(_) => {
                    if let Some(window) = window.upgrade() {
                        let status = if enabled { "已启用" } else { "已禁用" };
                        window.set_status_message(format!("用户 {} {}", username, status).into());
                        load_users_to_window(&window.as_weak(), &state);
                    }
                }
                Err(e) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(e.into());
                    }
                }
            }
        });
    }
}

fn load_users_to_window(window: &Weak<MainWindow>, state: &Rc<RefCell<AppState>>) {
    let state = state.borrow();
    let users = state.user_manager.lock().unwrap();
    
    let user_list: Vec<UserInfo> = users.get_all_users()
        .into_iter()
        .map(|u| UserInfo {
            username: u.username.into(),
            home_dir: u.home_dir.into(),
            enabled: u.enabled,
            is_admin: u.is_admin,
        })
        .collect();
    
    if let Some(window) = window.upgrade() {
        window.set_users(ModelRc::from(user_list.as_slice()));
    }
}

fn setup_ip_callbacks(main_window: &MainWindow, state: &Rc<RefCell<AppState>>) {
    {
        let state = Rc::clone(state);
        let window = main_window.as_weak();
        
        main_window.on_load_ip_lists_clicked(move || {
            load_ip_lists_to_window(&window, &state);
        });
    }
    
    {
        let state = Rc::clone(state);
        let window = main_window.as_weak();
        
        main_window.on_add_allowed_ip_clicked(move |ip| {
            let ip = ip.to_string();
            if ip.is_empty() {
                return;
            }
            
            let result = {
                let state = state.borrow_mut();
                let mut config = state.config.lock().unwrap();
                config.security.allowed_ips.push(ip.clone());
                config.save(&state.config_path)
            };
            
            match result {
                Ok(_) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(format!("已添加白名单: {}", ip).into());
                        load_ip_lists_to_window(&window.as_weak(), &state);
                    }
                }
                Err(e) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(format!("保存配置失败: {}", e).into());
                    }
                }
            }
        });
    }
    
    {
        let state = Rc::clone(state);
        let window = main_window.as_weak();
        
        main_window.on_remove_allowed_ip_clicked(move |ip| {
            let ip = ip.to_string();
            
            let result = {
                let state = state.borrow_mut();
                let mut config = state.config.lock().unwrap();
                config.security.allowed_ips.retain(|x| x != &ip);
                config.save(&state.config_path)
            };
            
            match result {
                Ok(_) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(format!("已移除白名单: {}", ip).into());
                        load_ip_lists_to_window(&window.as_weak(), &state);
                    }
                }
                Err(e) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(format!("保存配置失败: {}", e).into());
                    }
                }
            }
        });
    }
    
    {
        let state = Rc::clone(state);
        let window = main_window.as_weak();
        
        main_window.on_add_denied_ip_clicked(move |ip| {
            let ip = ip.to_string();
            if ip.is_empty() {
                return;
            }
            
            let result = {
                let state = state.borrow_mut();
                let mut config = state.config.lock().unwrap();
                config.security.denied_ips.push(ip.clone());
                config.save(&state.config_path)
            };
            
            match result {
                Ok(_) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(format!("已添加黑名单: {}", ip).into());
                        load_ip_lists_to_window(&window.as_weak(), &state);
                    }
                }
                Err(e) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(format!("保存配置失败: {}", e).into());
                    }
                }
            }
        });
    }
    
    {
        let state = Rc::clone(state);
        let window = main_window.as_weak();
        
        main_window.on_remove_denied_ip_clicked(move |ip| {
            let ip = ip.to_string();
            
            let result = {
                let state = state.borrow_mut();
                let mut config = state.config.lock().unwrap();
                config.security.denied_ips.retain(|x| x != &ip);
                config.save(&state.config_path)
            };
            
            match result {
                Ok(_) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(format!("已移除黑名单: {}", ip).into());
                        load_ip_lists_to_window(&window.as_weak(), &state);
                    }
                }
                Err(e) => {
                    if let Some(window) = window.upgrade() {
                        window.set_status_message(format!("保存配置失败: {}", e).into());
                    }
                }
            }
        });
    }
}

fn load_ip_lists_to_window(window: &Weak<MainWindow>, state: &Rc<RefCell<AppState>>) {
    let state = state.borrow();
    let config = state.config.lock().unwrap();
    
    let allowed: Vec<slint::SharedString> = config.security.allowed_ips
        .iter()
        .map(|s| s.clone().into())
        .collect();
    
    let denied: Vec<slint::SharedString> = config.security.denied_ips
        .iter()
        .map(|s| s.clone().into())
        .collect();
    
    if let Some(window) = window.upgrade() {
        window.set_allowed_ips(ModelRc::from(allowed.as_slice()));
        window.set_denied_ips(ModelRc::from(denied.as_slice()));
    }
}

fn setup_log_callbacks(main_window: &MainWindow, state: &Rc<RefCell<AppState>>) {
    let state = Rc::clone(state);
    let window = main_window.as_weak();
    let window2 = main_window.as_weak();
    let state2 = Rc::clone(&state);
    
    main_window.on_refresh_logs_clicked(move || {
        refresh_logs(&window, &state);
    });
    
    start_auto_refresh_timer(window2, state2);
}

fn refresh_logs(window: &Weak<MainWindow>, state: &Rc<RefCell<AppState>>) {
    let state = state.borrow();
    let logger = state.logger.lock().unwrap();
    let logs = logger.get_recent_logs(100);
    
    let log_entries: Vec<LogEntry> = logs
        .into_iter()
        .map(|entry| LogEntry {
            timestamp: entry.timestamp.format("%Y-%m-%d %H:%M:%S").to_string().into(),
            level: entry.level.to_string().into(),
            source: entry.source.into(),
            message: entry.message.into(),
            client_ip: entry.client_ip.unwrap_or_default().into(),
        })
        .collect();
    
    if let Some(window) = window.upgrade() {
        window.set_logs(ModelRc::from(log_entries.as_slice()));
    }
}

fn start_auto_refresh_timer(window: Weak<MainWindow>, state: Rc<RefCell<AppState>>) {
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_secs(2),
        move || {
            let auto_refresh = {
                if let Some(window) = window.upgrade() {
                    window.get_auto_refresh_logs()
                } else {
                    false
                }
            };
            
            if auto_refresh {
                refresh_logs(&window, &state);
            }
        },
    );
    
    std::mem::forget(timer);
}
