//! WFTPD - SFTP/FTP Server Daemon
//! 
//! This is the main daemon that runs in the background and manages
//! FTP and SFTP services. It listens on a named pipe for IPC commands.

#![windows_subsystem = "windows"]
extern crate windows_service;

use wftpg::AppState;
use wftpg::core::ipc::{IpcServer, Command, Response, CommandData};
use wftpg::core::windows_ipc::PIPE_NAME;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::ffi::OsString;
use std::time::Duration;
use windows_service::{
    define_windows_service,
    service::{
        ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceType,
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_manager::{ServiceManager, ServiceManagerAccess},
    service_dispatcher,
};

// === Command Handlers ===

fn handle_status(state: &AppState) -> Response {
    Response::ok(state.is_ftp_running(), state.is_sftp_running())
}

fn handle_start_ftp(state: &AppState) -> Response {
    if state.is_ftp_running() {
        return Response::ok(true, state.is_sftp_running());
    }
    
    match state.start_ftp() {
        Ok(()) => {
            log_service_start(state, "FTP");
            Response::ok(true, state.is_sftp_running())
        }
        Err(e) => Response::error(&format!("FTP启动失败: {e}")),
    }
}

fn handle_start_sftp(state: &AppState) -> Response {
    if state.is_sftp_running() {
        return Response::ok(state.is_ftp_running(), true);
    }
    
    match state.start_sftp() {
        Ok(()) => {
            log_service_start(state, "SFTP");
            Response::ok(state.is_ftp_running(), true)
        }
        Err(e) => Response::error(&format!("SFTP启动失败: {e}")),
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
    if let Ok(mut log) = state.logger.try_lock()
        && let Ok(cfg) = state.config.try_lock() {
            let (bind_ip, port) = if service == "FTP" {
                (cfg.server.bind_ip.clone(), cfg.server.ftp_port)
            } else {
                (cfg.server.bind_ip.clone(), cfg.server.sftp_port)
            };
            log.info(service, &format!("{service}服务已启动，监听 {bind_ip}:{port}"));
        }
}

fn log_info(state: &AppState, source: &str, message: &str) {
    if let Ok(mut log) = state.logger.try_lock() {
        log.info(source, message);
    }
}

fn handle_command(state: &AppState, cmd: &Command) -> Response {
    match cmd.action.as_str() {
        "status" => handle_status(state),
        "start" => handle_start_action(state, cmd),
        "stop" => handle_stop_action(state, cmd),
        "reload" => handle_reload(state),
        "get_logs" => handle_get_logs(state, cmd),
        "get_file_logs" => handle_get_file_logs(state, cmd),
        _ => Response::error("未知命令"),
    }
}

fn handle_reload(state: &AppState) -> Response {
    let config_msg = match state.reload_config() {
        Ok(()) => "配置已重新加载".to_string(),
        Err(e) => format!("配置重新加载失败: {e}"),
    };
    
    let users_msg = match state.reload_users() {
        Ok(()) => "用户配置已重新加载".to_string(),
        Err(e) => format!("用户配置重新加载失败: {e}"),
    };
    
    let message = format!("{config_msg}; {users_msg}");
    
    if config_msg.contains("失败") || users_msg.contains("失败") {
        Response {
            success: false,
            message,
            ftp_running: state.is_ftp_running(),
            sftp_running: state.is_sftp_running(),
            logs: None,
            file_logs: None,
        }
    } else {
        Response {
            success: true,
            message,
            ftp_running: state.is_ftp_running(),
            sftp_running: state.is_sftp_running(),
            logs: None,
            file_logs: None,
        }
    }
}

fn handle_get_logs(state: &AppState, cmd: &Command) -> Response {
    let count = cmd.data.as_ref()
        .and_then(|d| match d {
            CommandData::GetLogs { count, .. } => Some(*count),
            CommandData::GetFileLogs { .. } => None,
        })
        .unwrap_or(100);
    
    let logs = state.get_logs(count);
    Response::with_logs(state.is_ftp_running(), state.is_sftp_running(), logs)
}

fn handle_get_file_logs(state: &AppState, cmd: &Command) -> Response {
    let count = cmd.data.as_ref()
        .and_then(|d| match d {
            CommandData::GetFileLogs { count } => Some(*count),
            CommandData::GetLogs { .. } => None,
        })
        .unwrap_or(100);
    
    let file_logs = state.get_file_logs(count);
    Response::with_file_logs(state.is_ftp_running(), state.is_sftp_running(), file_logs)
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

// === Service Implementation ===

const SERVICE_NAME: &str = "wftpd";
const SERVICE_DISPLAY_NAME: &str = "WFTPD SFTP/FTP Server";
const SERVICE_DESCRIPTION: &str = "SFTP and FTP server daemon with GUI management";

define_windows_service!(ffi_service_main, my_service_main);

fn my_service_main(_arguments: Vec<OsString>) {
    if let Err(e) = run_service() {
        log::error!("Service failed: {e}");
    }
}

fn run_service() -> windows_service::Result<()> {
    init_logger();

    log::info!(
        "WFTPD Service - SFTP/FTP Server Daemon v{}",
        env!("CARGO_PKG_VERSION")
    );

    // 首先注册服务控制处理器并报告 Running 状态，避免 1053 超时错误
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);

    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop => {
                log::info!("Service stopping...");
                running_clone.store(false, Ordering::SeqCst);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Pause | ServiceControl::Continue | ServiceControl::Interrogate => {
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

    // 立即报告 Running 状态（必须在 30 秒内完成）
    let running_status = ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    };
    status_handle.set_service_status(running_status)?;

    // 在后台线程中初始化服务和运行主循环
    let service_thread = thread::spawn(move || {
        let state = match create_app_state() {
            Ok(s) => s,
            Err(e) => {
                log::error!("Failed to initialize: {e}");
                return;
            }
        };

        let ipc_server = match create_ipc_server() {
            Ok(s) => s,
            Err(e) => {
                log::error!("Failed to create IPC server: {e}");
                return;
            }
        };

        start_enabled_services(&state);
        log::info!("Service ready to accept connections on named pipe: {PIPE_NAME}");

        run_main_loop_with_shutdown(&state, &ipc_server, &running);

        state.stop_all();
        log::info!("Service stopped");
    });

    // 等待服务线程完成
    let _ = service_thread.join();

    // 报告 Stopped 状态
    let stopped_status = ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    };
    status_handle.set_service_status(stopped_status)?;

    Ok(())
}

fn run_main_loop_with_shutdown(state: &Arc<AppState>, ipc_server: &IpcServer, running: &Arc<AtomicBool>) {
    while running.load(Ordering::SeqCst) {
        match ipc_server.accept_timeout(Duration::from_millis(100)) {
            Ok(Some((stream, cmd))) => {
                let state_clone = Arc::clone(state);
                thread::spawn(move || {
                    let response = handle_command(&state_clone, &cmd);
                    if let Err(e) = IpcServer::send_response(&stream, &response) {
                        log::error!("Failed to send response: {e}");
                    }
                });
            }
            Ok(None) => {
                // Timeout, continue loop to check running flag
            }
            Err(e) => {
                log::error!("Failed to accept IPC connection: {e}");
            }
        }
    }
}

fn install_service() -> anyhow::Result<()> {
    let current_exe = std::env::current_exe()?;
    let exe_dir = current_exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("无法获取当前程序目录"))?;

    let wftpd_exe = exe_dir.join("wftpd.exe");
    if !wftpd_exe.exists() {
        return Err(anyhow::anyhow!(
            "在当前目录未找到 wftpd.exe，请确保 wftpd.exe 与 wftp-gui.exe 在同一目录"
        ));
    }

    log::info!("Using wftpd.exe at {}", wftpd_exe.display());
    
    let manager_access = ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;
    
    let service_info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: wftpd_exe,
        launch_arguments: vec![],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };
    
    let service = service_manager.create_service(&service_info, ServiceAccess::CHANGE_CONFIG)?;
    if let Err(e) = service.set_description(SERVICE_DESCRIPTION) {
        log::warn!("设置服务描述失败（可忽略）: {e:?}");
    }
    
    log::info!("Service installed successfully");
    Ok(())
}

fn uninstall_service() -> anyhow::Result<()> {
    let manager_access = ServiceManagerAccess::CONNECT;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;
    
    let service = service_manager.open_service(SERVICE_NAME, ServiceAccess::STOP | ServiceAccess::DELETE)?;
    service.stop()?;
    service.delete()?;
    
    log::info!("Service uninstalled successfully");
    Ok(())
}

// === Main Entry Point ===

fn main() {
    let args: Vec<String> = std::env::args().collect();
    
    if args.len() > 1 {
        match args[1].as_str() {
            "--install" => {
                init_logger();
                if let Err(e) = install_service() {
                    log::error!("Failed to install service: {e}");
                    std::process::exit(1);
                }
                return;
            }
            "--uninstall" => {
                init_logger();
                if let Err(e) = uninstall_service() {
                    log::error!("Failed to uninstall service: {e}");
                    std::process::exit(1);
                }
                return;
            }
            _ => {}
        }
    }
    
    // Run as Windows service (started by service dispatcher) or console application
    if let Err(_e) = service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        // If service_dispatcher::start fails, run as console application
        run_console_application();
    }
}

fn run_console_application() {
    init_logger();
    
    log::info!("WFTPD - SFTP/FTP Server Daemon v{}", env!("CARGO_PKG_VERSION"));
    
    let state = match create_app_state() {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to initialize: {e}");
            std::process::exit(1);
        }
    };
    
    let ipc_server = match create_ipc_server() {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to create IPC server: {e}");
            std::process::exit(1);
        }
    };
    
    setup_signal_handler(&state);
    start_enabled_services(&state);
    
    log::info!("Ready to accept connections on named pipe: {PIPE_NAME}");
    
    run_main_loop(&state, &ipc_server);
}

fn init_logger() {
    let log_dir = std::path::PathBuf::from("C:\\ProgramData\\wftpg\\logs");
    std::fs::create_dir_all(&log_dir).unwrap_or(());
    
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .try_init();
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
        std::process::exit(0);
    }).expect("Error setting Ctrl-C handler");
}

fn start_enabled_services(state: &Arc<AppState>) {
    let (ftp_enabled, sftp_enabled) = get_enabled_services(state);
    
    if (ftp_enabled || sftp_enabled)
        && let Err(e) = state.start_all() {
            log::error!("Failed to start services: {e}");
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
                    let response = handle_command(&state_clone, &cmd);
                    if let Err(e) = IpcServer::send_response(&stream, &response) {
                        log::error!("Failed to send response: {e}");
                    }
                });
            }
            Err(e) => {
                log::error!("Failed to accept IPC connection: {e}");
            }
        }
    }
}
