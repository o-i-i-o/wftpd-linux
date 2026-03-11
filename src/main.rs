#![allow(dead_code)]

slint::include_modules!();

use std::rc::Rc;
use std::cell::RefCell;

use wftpg::AppState;

fn main() -> Result<(), slint::PlatformError> {
    env_logger::init();
    
    let state = Rc::new(RefCell::new(AppState::new()));
    
    let main_window = MainWindow::new()?;
    
    {
        let state = state.borrow();
        let config = state.config.lock().unwrap();
        
        main_window.set_bind_ip(config.server.bind_ip.clone().into());
        main_window.set_ftp_port(config.server.ftp_port.to_string().into());
        main_window.set_sftp_port(config.server.sftp_port.to_string().into());
        main_window.set_ftp_home(config.ftp.default_home.clone().into());
        main_window.set_sftp_home(config.sftp.default_home.clone().into());
        main_window.set_ftp_running(state.is_ftp_running());
        main_window.set_sftp_running(state.is_sftp_running());
        main_window.set_service_installed(state.service_manager.service_exists());
        main_window.set_service_running(state.service_manager.is_service_running());
    }
    
    {
        let state = Rc::clone(&state);
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
        let state = Rc::clone(&state);
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
    
    {
        let state = Rc::clone(&state);
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
        let state = Rc::clone(&state);
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
    
    {
        let state = Rc::clone(&state);
        let window = main_window.as_weak();
        
        main_window.on_save_config_clicked(move || {
            let state = state.borrow();
            
            if let Some(window) = window.upgrade() {
                let mut config = state.config.lock().unwrap();
                config.server.bind_ip = window.get_bind_ip().to_string();
                config.server.ftp_port = window.get_ftp_port().to_string().parse().unwrap_or(21);
                config.server.sftp_port = window.get_sftp_port().to_string().parse().unwrap_or(22);
                config.ftp.default_home = window.get_ftp_home().to_string();
                config.sftp.default_home = window.get_sftp_home().to_string();
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
    
    {
        let state = Rc::clone(&state);
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
        let state = Rc::clone(&state);
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
        let state = Rc::clone(&state);
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
        let state = Rc::clone(&state);
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
    
    {
        let state = Rc::clone(&state);
        let window = main_window.as_weak();
        
        main_window.on_refresh_logs_clicked(move || {
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
                window.set_logs(slint::Model::new(log_entries));
            }
        });
    }
    
    main_window.run()
}
