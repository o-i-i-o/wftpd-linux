//! WFTPD - SFTP/FTP Server Daemon
//!
//! This is the main daemon that runs in the background and manages
//! FTP and SFTP services. It listens on a Unix socket for IPC commands.

use wftpg::AppState;
use wftpg::ipc::{IpcServer, Command, Response, SOCKET_PATH};
use std::sync::Arc;
use std::thread;

// === Command Handlers ===

fn handle_status(state: &AppState) -> Response {
    Response::ok(state.is_ftp_running(), state.is_sftp_running())
}

fn handle_start_ftp(state: &AppState) -> Response {
    if state.is_ftp_running() {
        return Response::ok(true, state.is_sftp_running());
    }
    
    match state.start_ftp() {
        Ok(_) => {
            log_service_start(state, "FTP");
            Response::ok(true, state.is_sftp_running())
        }
        Err(e) => Response::error(&format!("FTP启动失败: {}", e)),
    }
}

fn handle_start_sftp(state: &AppState) -> Response {
    if state.is_sftp_running() {
        return Response::ok(state.is_ftp_running(), true);
    }
    
    match state.start_sftp() {
        Ok(_) => {
            log_service_start(state, "SFTP");
            Response::ok(state.is_ftp_running(), true)
        }
        Err(e) => Response::error(&format!("SFTP启动失败: {}", e)),
    }
}

fn handle_start_all(state: &AppState) -> Response {
    let mut ftp_ok = state.is_ftp_running();
    let mut sftp_ok = state.is_sftp_running();
    
    if !ftp_ok {
        ftp_ok = state.start_ftp().is_ok();
    }
    if !sftp_ok {
        sftp_ok = state.start_sftp().is_ok();
    }
    
    if ftp_ok && sftp_ok {
        log_info(state, "SERVER", "所有服务已启动");
        Response::ok(true, true)
    } else {
        Response::error("部分服务启动失败")
    }
}

fn handle_stop_ftp(state: &AppState) -> Response {
    state.stop_ftp();
    log_info(state, "FTP", "FTP服务已停止");
    Response::ok(false, state.is_sftp_running())
}

fn handle_stop_sftp(state: &AppState) -> Response {
    state.stop_sftp();
    log_info(state, "SFTP", "SFTP服务已停止");
    Response::ok(state.is_ftp_running(), false)
}

fn handle_stop_all(state: &AppState) -> Response {
    state.stop_all();
    log_info(state, "SERVER", "所有服务已停止");
    Response::ok(false, false)
}

// === Helper Functions ===

fn log_service_start(state: &AppState, service: &str) {
    if let Ok(mut log) = state.logger.try_lock() {
        if let Ok(cfg) = state.config.try_lock() {
            let (bind_ip, port) = if service == "FTP" {
                (cfg.server.bind_ip.clone(), cfg.server.ftp_port)
            } else {
                (cfg.server.bind_ip.clone(), cfg.server.sftp_port)
            };
            log.info(service, &format!("{}服务已启动，监听 {}:{}", service, bind_ip, port));
        }
    }
}

fn log_info(state: &AppState, source: &str, message: &str) {
    if let Ok(mut log) = state.logger.try_lock() {
        log.info(source, message);
    }
}

fn handle_command(state: &AppState, cmd: Command) -> Response {
    match cmd.action.as_str() {
        "status" => handle_status(state),
        "start" => handle_start_action(state, &cmd),
        "stop" => handle_stop_action(state, &cmd),
        "reload" => handle_reload(state),
        _ => Response::error("未知命令"),
    }
}

fn handle_reload(state: &AppState) -> Response {
    let config_msg = match state.reload_config() {
        Ok(_) => "配置已重新加载".to_string(),
        Err(e) => format!("配置重新加载失败: {}", e),
    };
    
    let users_msg = match state.reload_users() {
        Ok(_) => "用户配置已重新加载".to_string(),
        Err(e) => format!("用户配置重新加载失败: {}", e),
    };
    
    let message = format!("{}; {}", config_msg, users_msg);
    
    if config_msg.contains("失败") || users_msg.contains("失败") {
        Response {
            success: false,
            message,
            ftp_running: state.is_ftp_running(),
            sftp_running: state.is_sftp_running(),
        }
    } else {
        Response {
            success: true,
            message,
            ftp_running: state.is_ftp_running(),
            sftp_running: state.is_sftp_running(),
        }
    }
}

fn handle_start_action(state: &AppState, cmd: &Command) -> Response {
    match cmd.service.as_deref().unwrap_or("all") {
        "ftp" => handle_start_ftp(state),
        "sftp" => handle_start_sftp(state),
        "all" => handle_start_all(state),
        _ => Response::error("未知服务"),
    }
}

fn handle_stop_action(state: &AppState, cmd: &Command) -> Response {
    match cmd.service.as_deref().unwrap_or("all") {
        "ftp" => handle_stop_ftp(state),
        "sftp" => handle_stop_sftp(state),
        "all" => handle_stop_all(state),
        _ => Response::error("未知服务"),
    }
}

// === Main Entry Point ===

fn main() {
    init_logger();
    
    log::info!("WFTPD - SFTP/FTP Server Daemon v{}", env!("CARGO_PKG_VERSION"));
    
    let state = match create_app_state() {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to initialize: {}", e);
            std::process::exit(1);
        }
    };
    
    let ipc_server = match create_ipc_server() {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to create IPC server: {}", e);
            std::process::exit(1);
        }
    };
    
    setup_signal_handler(&state);
    start_enabled_services(&state);
    
    log::info!("Ready to accept connections on {}", SOCKET_PATH);
    
    run_main_loop(&state, &ipc_server);
}

fn init_logger() {
    if let Err(e) = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info")
    ).try_init() {
        eprintln!("Failed to initialize logger: {}", e);
    }
}

fn create_app_state() -> anyhow::Result<Arc<AppState>> {
    Ok(Arc::new(AppState::new()?))
}

fn create_ipc_server() -> anyhow::Result<IpcServer> {
    IpcServer::new()
}

fn setup_signal_handler(state: &Arc<AppState>) {
    let state_clone = Arc::clone(state);
    ctrlc::set_handler(move || {
        log::info!("Shutting down...");
        state_clone.stop_all();
        let _ = std::fs::remove_file(SOCKET_PATH);
        std::process::exit(0);
    }).expect("Error setting Ctrl-C handler");
}

fn start_enabled_services(state: &Arc<AppState>) {
    let (ftp_enabled, sftp_enabled) = get_enabled_services(state);
    
    if ftp_enabled || sftp_enabled {
        if let Err(e) = state.start_all() {
            log::error!("Failed to start services: {}", e);
        }
    }
}

fn get_enabled_services(state: &Arc<AppState>) -> (bool, bool) {
    if let Ok(cfg) = state.config.try_lock() {
        (cfg.ftp.enabled, cfg.sftp.enabled)
    } else {
        (false, false)
    }
}

fn run_main_loop(state: &Arc<AppState>, ipc_server: &IpcServer) {
    loop {
        match ipc_server.accept() {
            Ok((stream, cmd)) => {
                let state_clone = Arc::clone(state);
                thread::spawn(move || {
                    let response = handle_command(&state_clone, cmd);
                    if let Err(e) = IpcServer::send_response(&stream, &response) {
                        log::error!("Failed to send response: {}", e);
                    }
                });
            }
            Err(e) => {
                log::error!("Failed to accept IPC connection: {}", e);
            }
        }
    }
}
