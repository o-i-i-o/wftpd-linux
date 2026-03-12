use gtk::prelude::*;
use gtk::{
    Box, Orientation, Label, Button, Entry, Frame, Separator, SpinButton, Adjustment,
    CheckButton, glib,
};
use gtk::glib::clone;
use std::sync::{Arc, Mutex as StdMutex};
use wftpg::AppState;
use wftpg::ipc::IpcClient;

pub fn create(state: &Arc<StdMutex<AppState>>) -> Box {
    let container = Box::new(Orientation::Vertical, 10);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);

    let title_label = Label::new(Some("<b>服务器配置</b>"));
    title_label.set_use_markup(true);
    container.pack_start(&title_label, false, false, 0);

    let (ftp_status, sftp_status) = create_status_frame(&container);
    create_control_frame(&container, state, &ftp_status, &sftp_status);
    create_server_config_frame(&container, state);
    create_ftp_config_frame(&container, state);
    create_sftp_config_frame(&container, state);

    start_status_monitor(&ftp_status, &sftp_status);

    container
}

fn start_status_monitor(ftp_status: &Label, sftp_status: &Label) {
    let ftp_status_clone = ftp_status.clone();
    let sftp_status_clone = sftp_status.clone();
    
    glib::timeout_add_seconds_local(2, move || {
        match IpcClient::get_status() {
            Ok(response) => {
                if response.ftp_running {
                    ftp_status_clone.set_markup("<span foreground='green'>FTP: 运行中 ✓</span>");
                } else {
                    ftp_status_clone.set_text("FTP: 已停止");
                }
                
                if response.sftp_running {
                    sftp_status_clone.set_markup("<span foreground='green'>SFTP: 运行中 ✓</span>");
                } else {
                    sftp_status_clone.set_text("SFTP: 已停止");
                }
            }
            Err(_) => {
                ftp_status_clone.set_markup("<span foreground='gray'>FTP: 服务未运行</span>");
                sftp_status_clone.set_markup("<span foreground='gray'>SFTP: 服务未运行</span>");
            }
        }
        glib::ControlFlow::Continue
    });
}

fn create_status_frame(container: &Box) -> (Label, Label) {
    let status_frame = Frame::new(Some("服务状态"));
    let status_box = Box::new(Orientation::Horizontal, 20);
    status_box.set_margin_top(10);
    status_box.set_margin_bottom(10);
    status_box.set_margin_start(10);
    status_box.set_margin_end(10);

    let ftp_status = Label::new(Some("FTP: 检测中..."));
    let sftp_status = Label::new(Some("SFTP: 检测中..."));
    status_box.pack_start(&ftp_status, false, false, 0);
    status_box.pack_start(&sftp_status, false, false, 0);
    status_frame.add(&status_box);
    container.pack_start(&status_frame, false, false, 0);

    (ftp_status, sftp_status)
}

fn create_control_frame(
    container: &Box,
    state: &Arc<StdMutex<AppState>>,
    ftp_status: &Label,
    sftp_status: &Label,
) {
    let control_frame = Frame::new(Some("服务控制"));
    let control_box = Box::new(Orientation::Horizontal, 10);
    control_box.set_margin_top(10);
    control_box.set_margin_bottom(10);
    control_box.set_margin_start(10);
    control_box.set_margin_end(10);

    let ftp_btn = Button::with_label("启动FTP");
    let sftp_btn = Button::with_label("启动SFTP");
    let stop_ftp_btn = Button::with_label("停止FTP");
    let stop_sftp_btn = Button::with_label("停止SFTP");
    let stop_all_btn = Button::with_label("停止全部");

    setup_ftp_start_button(state, &ftp_btn, ftp_status);
    setup_sftp_start_button(state, &sftp_btn, sftp_status);
    setup_ftp_stop_button(state, &stop_ftp_btn, ftp_status);
    setup_sftp_stop_button(state, &stop_sftp_btn, sftp_status);
    setup_stop_all_button(state, &stop_all_btn, ftp_status, sftp_status);

    control_box.pack_start(&ftp_btn, false, false, 0);
    control_box.pack_start(&stop_ftp_btn, false, false, 0);
    control_box.pack_start(&Separator::new(Orientation::Vertical), false, false, 5);
    control_box.pack_start(&sftp_btn, false, false, 0);
    control_box.pack_start(&stop_sftp_btn, false, false, 0);
    control_box.pack_start(&Separator::new(Orientation::Vertical), false, false, 5);
    control_box.pack_start(&stop_all_btn, false, false, 0);
    control_frame.add(&control_box);
    container.pack_start(&control_frame, false, false, 0);
}

fn setup_ftp_start_button(state: &Arc<StdMutex<AppState>>, button: &Button, status: &Label) {
    let status_clone = status.clone();
    let state_clone = Arc::clone(state);
    button.connect_clicked(clone!(@strong status_clone, @strong state_clone => move |_| {
        status_clone.set_text("FTP: 启动中...");
        let status = status_clone.clone();
        let state = Arc::clone(&state_clone);
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            match IpcClient::start_ftp() {
                Ok(response) => {
                    if response.success {
                        status.set_markup("<span foreground='green'>FTP: 运行中 ✓</span>");
                        if let Ok(s) = state.try_lock() {
                            if let Ok(mut log) = s.logger.try_lock() {
                                let (bind_ip, ftp_port) = {
                                    if let Ok(cfg) = s.config.try_lock() {
                                        (cfg.server.bind_ip.clone(), cfg.server.ftp_port)
                                    } else {
                                        ("0.0.0.0".to_string(), 2121)
                                    }
                                };
                                log.info("FTP", &format!("FTP服务已启动，监听 {}:{}", bind_ip, ftp_port));
                            }
                        }
                    } else {
                        status.set_markup(&format!("<span foreground='red'>FTP: {}</span>", response.message));
                    }
                }
                Err(e) => {
                    status.set_markup(&format!("<span foreground='red'>FTP: 错误 - {}</span>", e));
                }
            }
        });
    }));
}

fn setup_sftp_start_button(state: &Arc<StdMutex<AppState>>, button: &Button, status: &Label) {
    let status_clone = status.clone();
    let state_clone = Arc::clone(state);
    button.connect_clicked(clone!(@strong status_clone, @strong state_clone => move |_| {
        status_clone.set_text("SFTP: 启动中...");
        let status = status_clone.clone();
        let state = Arc::clone(&state_clone);
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            match IpcClient::start_sftp() {
                Ok(response) => {
                    if response.success {
                        status.set_markup("<span foreground='green'>SFTP: 运行中 ✓</span>");
                        if let Ok(s) = state.try_lock() {
                            if let Ok(mut log) = s.logger.try_lock() {
                                let (bind_ip, sftp_port) = {
                                    if let Ok(cfg) = s.config.try_lock() {
                                        (cfg.server.bind_ip.clone(), cfg.server.sftp_port)
                                    } else {
                                        ("0.0.0.0".to_string(), 2222)
                                    }
                                };
                                log.info("SFTP", &format!("SFTP服务已启动，监听 {}:{}", bind_ip, sftp_port));
                            }
                        }
                    } else {
                        status.set_markup(&format!("<span foreground='red'>SFTP: {}</span>", response.message));
                    }
                }
                Err(e) => {
                    status.set_markup(&format!("<span foreground='red'>SFTP: 错误 - {}</span>", e));
                }
            }
        });
    }));
}

fn setup_ftp_stop_button(state: &Arc<StdMutex<AppState>>, button: &Button, status: &Label) {
    let status_clone = status.clone();
    let state_clone = Arc::clone(state);
    button.connect_clicked(clone!(@strong status_clone, @strong state_clone => move |_| {
        status_clone.set_text("FTP: 停止中...");
        let status = status_clone.clone();
        let state = Arc::clone(&state_clone);
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            match IpcClient::stop_ftp() {
                Ok(response) => {
                    if response.success {
                        status.set_text("FTP: 已停止");
                        if let Ok(s) = state.try_lock() {
                            if let Ok(mut log) = s.logger.try_lock() {
                                log.info("FTP", "FTP服务已停止");
                            }
                        }
                    } else {
                        status.set_markup(&format!("<span foreground='red'>FTP: {}</span>", response.message));
                    }
                }
                Err(e) => {
                    status.set_markup(&format!("<span foreground='red'>FTP: 错误 - {}</span>", e));
                }
            }
        });
    }));
}

fn setup_sftp_stop_button(state: &Arc<StdMutex<AppState>>, button: &Button, status: &Label) {
    let status_clone = status.clone();
    let state_clone = Arc::clone(state);
    button.connect_clicked(clone!(@strong status_clone, @strong state_clone => move |_| {
        status_clone.set_text("SFTP: 停止中...");
        let status = status_clone.clone();
        let state = Arc::clone(&state_clone);
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            match IpcClient::stop_sftp() {
                Ok(response) => {
                    if response.success {
                        status.set_text("SFTP: 已停止");
                        if let Ok(s) = state.try_lock() {
                            if let Ok(mut log) = s.logger.try_lock() {
                                log.info("SFTP", "SFTP服务已停止");
                            }
                        }
                    } else {
                        status.set_markup(&format!("<span foreground='red'>SFTP: {}</span>", response.message));
                    }
                }
                Err(e) => {
                    status.set_markup(&format!("<span foreground='red'>SFTP: 错误 - {}</span>", e));
                }
            }
        });
    }));
}

fn setup_stop_all_button(
    state: &Arc<StdMutex<AppState>>,
    button: &Button,
    ftp_status: &Label,
    sftp_status: &Label,
) {
    let ftp_status_clone = ftp_status.clone();
    let sftp_status_clone = sftp_status.clone();
    let state_clone = Arc::clone(state);
    button.connect_clicked(clone!(@strong ftp_status_clone, @strong sftp_status_clone, @strong state_clone => move |_| {
        ftp_status_clone.set_text("FTP: 停止中...");
        sftp_status_clone.set_text("SFTP: 停止中...");
        let ftp_status = ftp_status_clone.clone();
        let sftp_status = sftp_status_clone.clone();
        let state = Arc::clone(&state_clone);
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            match IpcClient::stop_all() {
                Ok(response) => {
                    if response.success {
                        ftp_status.set_text("FTP: 已停止");
                        sftp_status.set_text("SFTP: 已停止");
                        if let Ok(s) = state.try_lock() {
                            if let Ok(mut log) = s.logger.try_lock() {
                                log.info("SERVER", "所有服务已停止");
                            }
                        }
                    } else {
                        ftp_status.set_markup(&format!("<span foreground='red'>FTP: {}</span>", response.message));
                        sftp_status.set_markup(&format!("<span foreground='red'>SFTP: {}</span>", response.message));
                    }
                }
                Err(e) => {
                    ftp_status.set_markup(&format!("<span foreground='red'>FTP: 错误 - {}</span>", e));
                    sftp_status.set_markup(&format!("<span foreground='red'>SFTP: 错误 - {}</span>", e));
                }
            }
        });
    }));
}

fn create_server_config_frame(container: &Box, state: &Arc<StdMutex<AppState>>) {
    let config_frame = Frame::new(Some("基本配置"));
    let config_box = Box::new(Orientation::Vertical, 5);
    config_box.set_margin_top(10);
    config_box.set_margin_bottom(10);
    config_box.set_margin_start(10);
    config_box.set_margin_end(10);

    let (ip_entry, ftp_port_spin, sftp_port_spin, max_conn_spin, conn_timeout_spin, idle_timeout_spin) = 
        load_server_config(state);

    let row1 = Box::new(Orientation::Horizontal, 5);
    row1.pack_start(&Label::new(Some("绑定IP:")), false, false, 0);
    ip_entry.set_hexpand(true);
    row1.pack_start(&ip_entry, true, true, 0);
    config_box.pack_start(&row1, false, false, 0);

    let row2 = Box::new(Orientation::Horizontal, 5);
    row2.pack_start(&Label::new(Some("FTP端口:")), false, false, 0);
    row2.pack_start(&ftp_port_spin, false, false, 0);
    row2.pack_start(&Label::new(Some("SFTP端口:")), false, false, 0);
    row2.pack_start(&sftp_port_spin, false, false, 0);
    config_box.pack_start(&row2, false, false, 0);

    let row3 = Box::new(Orientation::Horizontal, 5);
    row3.pack_start(&Label::new(Some("最大连接数:")), false, false, 0);
    row3.pack_start(&max_conn_spin, false, false, 0);
    row3.pack_start(&Label::new(Some("连接超时(秒):")), false, false, 0);
    row3.pack_start(&conn_timeout_spin, false, false, 0);
    row3.pack_start(&Label::new(Some("空闲超时(秒):")), false, false, 0);
    row3.pack_start(&idle_timeout_spin, false, false, 0);
    config_box.pack_start(&row3, false, false, 0);

    let save_btn = Button::with_label("保存基本配置");
    let state_clone = Arc::clone(state);
    let ip_entry_clone = ip_entry.clone();
    let ftp_port_clone = ftp_port_spin.clone();
    let sftp_port_clone = sftp_port_spin.clone();
    let max_conn_clone = max_conn_spin.clone();
    let conn_timeout_clone = conn_timeout_spin.clone();
    let idle_timeout_clone = idle_timeout_spin.clone();
    save_btn.connect_clicked(clone!(@strong state_clone, @strong ip_entry_clone, @strong ftp_port_clone, @strong sftp_port_clone,
               @strong max_conn_clone, @strong conn_timeout_clone, @strong idle_timeout_clone => move |_| {
        let state = Arc::clone(&state_clone);
        let bind_ip = ip_entry_clone.text().to_string();
        let ftp_port = ftp_port_clone.value() as u16;
        let sftp_port = sftp_port_clone.value() as u16;
        let max_conn = max_conn_clone.value() as usize;
        let conn_timeout = conn_timeout_clone.value() as u64;
        let idle_timeout = idle_timeout_clone.value() as u64;
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            if let Ok(s) = state.try_lock() {
                if let Ok(mut cfg) = s.config.try_lock() {
                    cfg.server.bind_ip = bind_ip.clone();
                    cfg.server.ftp_port = ftp_port;
                    cfg.server.sftp_port = sftp_port;
                    cfg.server.max_connections = max_conn;
                    cfg.server.connection_timeout = conn_timeout;
                    cfg.server.idle_timeout = idle_timeout;
                    let _ = cfg.save(&s.config_path);
                    if let Ok(mut log) = s.logger.try_lock() {
                        log.info("CONFIG", &format!(
                            "基本配置已保存: 绑定IP={}, FTP端口={}, SFTP端口={}, 最大连接={}",
                            bind_ip, ftp_port, sftp_port, max_conn
                        ));
                    }
                }
            }
        });
    }));
    config_box.pack_start(&save_btn, false, false, 0);

    config_frame.add(&config_box);
    container.pack_start(&config_frame, false, false, 0);
}

fn load_server_config(state: &Arc<StdMutex<AppState>>) -> (Entry, SpinButton, SpinButton, SpinButton, SpinButton, SpinButton) {
    let ip_entry = Entry::new();
    let ftp_port_spin = create_spin_button(1.0, 65535.0, 1.0);
    let sftp_port_spin = create_spin_button(1.0, 65535.0, 1.0);
    let max_conn_spin = create_spin_button(1.0, 10000.0, 1.0);
    let conn_timeout_spin = create_spin_button(10.0, 3600.0, 10.0);
    let idle_timeout_spin = create_spin_button(10.0, 7200.0, 10.0);

    if let Ok(s) = state.try_lock() {
        if let Ok(cfg) = s.config.try_lock() {
            ip_entry.set_text(&cfg.server.bind_ip);
            ftp_port_spin.set_value(cfg.server.ftp_port as f64);
            sftp_port_spin.set_value(cfg.server.sftp_port as f64);
            max_conn_spin.set_value(cfg.server.max_connections as f64);
            conn_timeout_spin.set_value(cfg.server.connection_timeout as f64);
            idle_timeout_spin.set_value(cfg.server.idle_timeout as f64);
        }
    }

    (ip_entry, ftp_port_spin, sftp_port_spin, max_conn_spin, conn_timeout_spin, idle_timeout_spin)
}

fn create_ftp_config_frame(container: &Box, state: &Arc<StdMutex<AppState>>) {
    let config_frame = Frame::new(Some("FTP配置"));
    let config_box = Box::new(Orientation::Vertical, 5);
    config_box.set_margin_top(10);
    config_box.set_margin_bottom(10);
    config_box.set_margin_start(10);
    config_box.set_margin_end(10);

    let (ftp_enabled_cb, anon_cb, passive_start_spin, passive_end_spin, welcome_entry) = 
        load_ftp_config(state);

    let row1 = Box::new(Orientation::Horizontal, 5);
    row1.pack_start(&ftp_enabled_cb, false, false, 0);
    row1.pack_start(&anon_cb, false, false, 0);
    config_box.pack_start(&row1, false, false, 0);

    let row2 = Box::new(Orientation::Horizontal, 5);
    row2.pack_start(&Label::new(Some("被动端口范围:")), false, false, 0);
    row2.pack_start(&passive_start_spin, false, false, 0);
    row2.pack_start(&Label::new(Some("-")), false, false, 0);
    row2.pack_start(&passive_end_spin, false, false, 0);
    config_box.pack_start(&row2, false, false, 0);

    let row3 = Box::new(Orientation::Horizontal, 5);
    row3.pack_start(&Label::new(Some("欢迎消息:")), false, false, 0);
    welcome_entry.set_hexpand(true);
    row3.pack_start(&welcome_entry, true, true, 0);
    config_box.pack_start(&row3, false, false, 0);

    let save_btn = Button::with_label("保存FTP配置");
    let state_clone = Arc::clone(state);
    let ftp_enabled_clone = ftp_enabled_cb.clone();
    let anon_clone = anon_cb.clone();
    let passive_start_clone = passive_start_spin.clone();
    let passive_end_clone = passive_end_spin.clone();
    let welcome_clone = welcome_entry.clone();
    save_btn.connect_clicked(clone!(@strong state_clone, @strong ftp_enabled_clone, @strong anon_clone,
               @strong passive_start_clone, @strong passive_end_clone, @strong welcome_clone => move |_| {
        let state = Arc::clone(&state_clone);
        let enabled = ftp_enabled_clone.is_active();
        let anon = anon_clone.is_active();
        let start = passive_start_clone.value() as u16;
        let end = passive_end_clone.value() as u16;
        let welcome = welcome_clone.text().to_string();
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            if let Ok(s) = state.try_lock() {
                if let Ok(mut cfg) = s.config.try_lock() {
                    cfg.ftp.enabled = enabled;
                    cfg.ftp.allow_anonymous = anon;
                    cfg.ftp.passive_ports = (start.min(end), start.max(end));
                    cfg.ftp.welcome_message = welcome;
                    let _ = cfg.save(&s.config_path);
                    if let Ok(mut log) = s.logger.try_lock() {
                        log.info("CONFIG", &format!(
                            "FTP配置已保存: 启用={}, 匿名访问={}, 被动端口={}-{}",
                            if enabled { "是" } else { "否" },
                            if anon { "是" } else { "否" },
                            start.min(end), start.max(end)
                        ));
                    }
                }
            }
        });
    }));
    config_box.pack_start(&save_btn, false, false, 0);

    config_frame.add(&config_box);
    container.pack_start(&config_frame, false, false, 0);
}

fn load_ftp_config(state: &Arc<StdMutex<AppState>>) -> (CheckButton, CheckButton, SpinButton, SpinButton, Entry) {
    let ftp_enabled_cb = CheckButton::with_label("启用FTP服务");
    let anon_cb = CheckButton::with_label("允许匿名访问");
    let passive_start_spin = create_spin_button(1024.0, 65535.0, 1.0);
    let passive_end_spin = create_spin_button(1024.0, 65535.0, 1.0);
    let welcome_entry = Entry::new();

    if let Ok(s) = state.try_lock() {
        if let Ok(cfg) = s.config.try_lock() {
            ftp_enabled_cb.set_active(cfg.ftp.enabled);
            anon_cb.set_active(cfg.ftp.allow_anonymous);
            passive_start_spin.set_value(cfg.ftp.passive_ports.0 as f64);
            passive_end_spin.set_value(cfg.ftp.passive_ports.1 as f64);
            welcome_entry.set_text(&cfg.ftp.welcome_message);
        }
    }

    (ftp_enabled_cb, anon_cb, passive_start_spin, passive_end_spin, welcome_entry)
}

fn create_sftp_config_frame(container: &Box, state: &Arc<StdMutex<AppState>>) {
    let config_frame = Frame::new(Some("SFTP配置"));
    let config_box = Box::new(Orientation::Vertical, 5);
    config_box.set_margin_top(10);
    config_box.set_margin_bottom(10);
    config_box.set_margin_start(10);
    config_box.set_margin_end(10);

    let (sftp_enabled_cb, max_auth_spin, auth_timeout_spin, host_key_entry) = 
        load_sftp_config(state);

    let row1 = Box::new(Orientation::Horizontal, 5);
    row1.pack_start(&sftp_enabled_cb, false, false, 0);
    row1.pack_start(&Label::new(Some("最大认证尝试:")), false, false, 0);
    row1.pack_start(&max_auth_spin, false, false, 0);
    row1.pack_start(&Label::new(Some("认证超时(秒):")), false, false, 0);
    row1.pack_start(&auth_timeout_spin, false, false, 0);
    config_box.pack_start(&row1, false, false, 0);

    let row2 = Box::new(Orientation::Horizontal, 5);
    row2.pack_start(&Label::new(Some("主机密钥路径:")), false, false, 0);
    host_key_entry.set_hexpand(true);
    row2.pack_start(&host_key_entry, true, true, 0);
    config_box.pack_start(&row2, false, false, 0);

    let save_btn = Button::with_label("保存SFTP配置");
    let state_clone = Arc::clone(state);
    let sftp_enabled_clone = sftp_enabled_cb.clone();
    let max_auth_clone = max_auth_spin.clone();
    let auth_timeout_clone = auth_timeout_spin.clone();
    let host_key_clone = host_key_entry.clone();
    save_btn.connect_clicked(clone!(@strong state_clone, @strong sftp_enabled_clone, @strong max_auth_clone,
               @strong auth_timeout_clone, @strong host_key_clone => move |_| {
        let state = Arc::clone(&state_clone);
        let enabled = sftp_enabled_clone.is_active();
        let max_auth = max_auth_clone.value() as u32;
        let auth_timeout = auth_timeout_clone.value() as u64;
        let host_key = host_key_clone.text().to_string();
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            if let Ok(s) = state.try_lock() {
                if let Ok(mut cfg) = s.config.try_lock() {
                    cfg.sftp.enabled = enabled;
                    cfg.sftp.max_auth_attempts = max_auth;
                    cfg.sftp.auth_timeout = auth_timeout;
                    cfg.sftp.host_key_path = host_key;
                    let _ = cfg.save(&s.config_path);
                    if let Ok(mut log) = s.logger.try_lock() {
                        log.info("CONFIG", &format!(
                            "SFTP配置已保存: 启用={}, 最大认证尝试={}",
                            if enabled { "是" } else { "否" },
                            max_auth
                        ));
                    }
                }
            }
        });
    }));
    config_box.pack_start(&save_btn, false, false, 0);

    config_frame.add(&config_box);
    container.pack_start(&config_frame, false, false, 0);
}

fn load_sftp_config(state: &Arc<StdMutex<AppState>>) -> (CheckButton, SpinButton, SpinButton, Entry) {
    let sftp_enabled_cb = CheckButton::with_label("启用SFTP服务");
    let max_auth_spin = create_spin_button(1.0, 10.0, 1.0);
    let auth_timeout_spin = create_spin_button(10.0, 300.0, 5.0);
    let host_key_entry = Entry::new();

    if let Ok(s) = state.try_lock() {
        if let Ok(cfg) = s.config.try_lock() {
            sftp_enabled_cb.set_active(cfg.sftp.enabled);
            max_auth_spin.set_value(cfg.sftp.max_auth_attempts as f64);
            auth_timeout_spin.set_value(cfg.sftp.auth_timeout as f64);
            host_key_entry.set_text(&cfg.sftp.host_key_path);
        }
    }

    (sftp_enabled_cb, max_auth_spin, auth_timeout_spin, host_key_entry)
}

fn create_spin_button(min: f64, max: f64, step: f64) -> SpinButton {
    let adjustment = Adjustment::new(min, min, max, step, step * 10.0, 0.0);
    SpinButton::builder()
        .adjustment(&adjustment)
        .digits(0)
        .width_chars(6)
        .build()
}