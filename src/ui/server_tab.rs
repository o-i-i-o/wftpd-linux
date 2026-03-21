use gtk::prelude::*;
use gtk::{
    Box, Orientation, Label, Button, Entry, Frame, Separator, SpinButton, Adjustment,
    CheckButton, glib, ComboBoxText, FileChooserDialog, FileChooserAction,
};
use gtk::glib::clone;
use std::sync::{Arc, Mutex as StdMutex};
use std::path::Path;
use crate::AppState;
use crate::communication::ipc::IpcClient;

pub fn create(state: &Arc<StdMutex<AppState>>) -> Box {
    let container = Box::new(Orientation::Vertical, 10);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);

    let (ftp_status, sftp_status, ftp_btn, sftp_btn) = 
        create_status_and_control_frame(&container, state);
    
    create_ftp_config_frame(&container, state);
    create_sftp_config_frame(&container, state);

    start_status_monitor(&ftp_status, &sftp_status, &ftp_btn, &sftp_btn);

    container
}

fn start_status_monitor(
    ftp_status: &Label, 
    sftp_status: &Label, 
    ftp_btn: &Button, 
    sftp_btn: &Button
) {
    let ftp_status_clone = ftp_status.clone();
    let sftp_status_clone = sftp_status.clone();
    let ftp_btn_clone = ftp_btn.clone();
    let sftp_btn_clone = sftp_btn.clone();
    
    let mut last_ftp_status = false;
    let mut last_sftp_status = false;
    
    glib::timeout_add_seconds_local(5, move || {
        match IpcClient::get_status() {
            Ok(response) => {
                if response.ftp_running != last_ftp_status {
                    last_ftp_status = response.ftp_running;
                    if response.ftp_running {
                        ftp_status_clone.set_markup("<span foreground='green'>运行中 ✓</span>");
                        ftp_btn_clone.set_label("停止");
                    } else {
                        ftp_status_clone.set_text("已停止");
                        ftp_btn_clone.set_label("启动");
                    }
                }
                
                if response.sftp_running != last_sftp_status {
                    last_sftp_status = response.sftp_running;
                    if response.sftp_running {
                        sftp_status_clone.set_markup("<span foreground='green'>运行中 ✓</span>");
                        sftp_btn_clone.set_label("停止");
                    } else {
                        sftp_status_clone.set_text("已停止");
                        sftp_btn_clone.set_label("启动");
                    }
                }
            }
            Err(_) => {
                if last_ftp_status || last_sftp_status {
                    last_ftp_status = false;
                    last_sftp_status = false;
                    ftp_status_clone.set_markup("<span foreground='gray'>服务未运行</span>");
                    sftp_status_clone.set_markup("<span foreground='gray'>服务未运行</span>");
                    ftp_btn_clone.set_label("启动");
                    sftp_btn_clone.set_label("启动");
                }
            }
        }
        glib::ControlFlow::Continue
    });
}

fn create_status_and_control_frame(
    container: &Box,
    state: &Arc<StdMutex<AppState>>,
) -> (Label, Label, Button, Button) {
    let frame = Frame::new(Some("服务状态与控制"));
    let main_box = Box::new(Orientation::Vertical, 10);
    main_box.set_margin_top(10);
    main_box.set_margin_bottom(10);
    main_box.set_margin_start(10);
    main_box.set_margin_end(10);

    let row1 = Box::new(Orientation::Horizontal, 10);
    
    let ftp_label = Label::new(Some("<b>FTP:</b>"));
    ftp_label.set_use_markup(true);
    row1.pack_start(&ftp_label, false, false, 0);
    
    let ftp_status = Label::new(Some("检测中..."));
    row1.pack_start(&ftp_status, false, false, 0);
    
    let ftp_btn = Button::with_label("启动");
    row1.pack_start(&ftp_btn, false, false, 0);
    
    let restart_ftp_btn = Button::with_label("重启");
    row1.pack_start(&restart_ftp_btn, false, false, 0);
    
    row1.pack_start(&Separator::new(Orientation::Vertical), false, false, 10);
    
    let sftp_label = Label::new(Some("<b>SFTP:</b>"));
    sftp_label.set_use_markup(true);
    row1.pack_start(&sftp_label, false, false, 0);
    
    let sftp_status = Label::new(Some("检测中..."));
    row1.pack_start(&sftp_status, false, false, 0);
    
    let sftp_btn = Button::with_label("启动");
    row1.pack_start(&sftp_btn, false, false, 0);
    
    let restart_sftp_btn = Button::with_label("重启");
    row1.pack_start(&restart_sftp_btn, false, false, 0);
    
    main_box.pack_start(&row1, false, false, 0);
    frame.add(&main_box);
    container.pack_start(&frame, false, false, 0);

    setup_ftp_toggle_button(state, &ftp_btn, &ftp_status);
    setup_sftp_toggle_button(state, &sftp_btn, &sftp_status);
    setup_ftp_restart_button(state, &restart_ftp_btn, &ftp_status, &ftp_btn);
    setup_sftp_restart_button(state, &restart_sftp_btn, &sftp_status, &sftp_btn);

    (ftp_status, sftp_status, ftp_btn, sftp_btn)
}

fn setup_ftp_toggle_button(state: &Arc<StdMutex<AppState>>, button: &Button, status: &Label) {
    let status_clone = status.clone();
    let button_clone = button.clone();
    let state_clone = Arc::clone(state);
    
    button.connect_clicked(clone!(@strong status_clone, @strong button_clone, @strong state_clone => move |_| {
        let current_label = button_clone.label().map(|s| s.to_string()).unwrap_or_default();
        
        if current_label == "停止" {
            status_clone.set_text("FTP: 停止中...");
            let status = status_clone.clone();
            let state = Arc::clone(&state_clone);
            let btn = button_clone.clone();
            
            glib::MainContext::ref_thread_default().spawn_local(async move {
                match IpcClient::stop_ftp() {
                    Ok(response) => {
                        if response.success {
                            status.set_text("已停止");
                            btn.set_label("启动");
                            if let Ok(s) = state.try_lock()
                                && let Ok(mut log) = s.logger.try_lock() {
                                    log.info("FTP", "FTP服务已停止");
                                }
                        } else {
                            status.set_markup(&format!("<span foreground='red'>{}</span>", response.message));
                        }
                    }
                    Err(e) => {
                        status.set_markup(&format!("<span foreground='red'>错误 - {}</span>", e));
                    }
                }
            });
        } else {
            status_clone.set_text("FTP: 启动中...");
            let status = status_clone.clone();
            let state = Arc::clone(&state_clone);
            let btn = button_clone.clone();
            
            glib::MainContext::ref_thread_default().spawn_local(async move {
                match IpcClient::start_ftp() {
                    Ok(response) => {
                        if response.success {
                            status.set_markup("<span foreground='green'>运行中 ✓</span>");
                            btn.set_label("停止");
                            if let Ok(s) = state.try_lock()
                                && let Ok(mut log) = s.logger.try_lock() {
                                    let (bind_ip, ftp_port) = {
                                        if let Ok(cfg) = s.config.try_lock() {
                                            (cfg.server.bind_ip.clone(), cfg.server.ftp_port)
                                        } else {
                                            ("0.0.0.0".to_string(), 21)
                                        }
                                    };
                                    log.info("FTP", &format!("FTP服务已启动，监听 {}:{}", bind_ip, ftp_port));
                                }
                        } else {
                            status.set_markup(&format!("<span foreground='red'>{}</span>", response.message));
                        }
                    }
                    Err(e) => {
                        status.set_markup(&format!("<span foreground='red'>错误 - {}</span>", e));
                    }
                }
            });
        }
    }));
}

fn setup_sftp_toggle_button(state: &Arc<StdMutex<AppState>>, button: &Button, status: &Label) {
    let status_clone = status.clone();
    let button_clone = button.clone();
    let state_clone = Arc::clone(state);
    
    button.connect_clicked(clone!(@strong status_clone, @strong button_clone, @strong state_clone => move |_| {
        let current_label = button_clone.label().map(|s| s.to_string()).unwrap_or_default();
        
        if current_label == "停止" {
            status_clone.set_text("SFTP: 停止中...");
            let status = status_clone.clone();
            let state = Arc::clone(&state_clone);
            let btn = button_clone.clone();
            
            glib::MainContext::ref_thread_default().spawn_local(async move {
                match IpcClient::stop_sftp() {
                    Ok(response) => {
                        if response.success {
                            status.set_text("已停止");
                            btn.set_label("启动");
                            if let Ok(s) = state.try_lock()
                                && let Ok(mut log) = s.logger.try_lock() {
                                    log.info("SFTP", "SFTP服务已停止");
                                }
                        } else {
                            status.set_markup(&format!("<span foreground='red'>{}</span>", response.message));
                        }
                    }
                    Err(e) => {
                        status.set_markup(&format!("<span foreground='red'>错误 - {}</span>", e));
                    }
                }
            });
        } else {
            status_clone.set_text("SFTP: 启动中...");
            let status = status_clone.clone();
            let state = Arc::clone(&state_clone);
            let btn = button_clone.clone();
            
            glib::MainContext::ref_thread_default().spawn_local(async move {
                match IpcClient::start_sftp() {
                    Ok(response) => {
                        if response.success {
                            status.set_markup("<span foreground='green'>运行中 ✓</span>");
                            btn.set_label("停止");
                            if let Ok(s) = state.try_lock()
                                && let Ok(mut log) = s.logger.try_lock() {
                                    let (bind_ip, sftp_port) = {
                                        if let Ok(cfg) = s.config.try_lock() {
                                            (cfg.server.bind_ip.clone(), cfg.server.sftp_port)
                                        } else {
                                            ("0.0.0.0".to_string(), 22)
                                        }
                                    };
                                    log.info("SFTP", &format!("SFTP服务已启动，监听 {}:{}", bind_ip, sftp_port));
                                }
                        } else {
                            status.set_markup(&format!("<span foreground='red'>{}</span>", response.message));
                        }
                    }
                    Err(e) => {
                        status.set_markup(&format!("<span foreground='red'>错误 - {}</span>", e));
                    }
                }
            });
        }
    }));
}

fn setup_ftp_restart_button(state: &Arc<StdMutex<AppState>>, button: &Button, status: &Label, toggle_btn: &Button) {
    let status_clone = status.clone();
    let button_clone = button.clone();
    let state_clone = Arc::clone(state);
    let toggle_btn_clone = toggle_btn.clone();
    
    button.connect_clicked(clone!(@strong status_clone, @strong button_clone, @strong state_clone, @strong toggle_btn_clone => move |_| {
        status_clone.set_text("FTP: 重启中...");
        let status = status_clone.clone();
        let state = Arc::clone(&state_clone);
        let toggle = toggle_btn_clone.clone();
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            let _ = IpcClient::stop_ftp();
            std::thread::sleep(std::time::Duration::from_millis(500));
            
            match IpcClient::start_ftp() {
                Ok(response) => {
                    if response.success {
                        status.set_markup("<span foreground='green'>运行中 ✓</span>");
                        toggle.set_label("停止");
                        if let Ok(s) = state.try_lock()
                            && let Ok(mut log) = s.logger.try_lock() {
                                log.info("FTP", "FTP服务已重启");
                            }
                    } else {
                        status.set_markup(&format!("<span foreground='red'>{}</span>", response.message));
                        toggle.set_label("启动");
                    }
                }
                Err(e) => {
                    status.set_markup(&format!("<span foreground='red'>错误 - {}</span>", e));
                    toggle.set_label("启动");
                }
            }
        });
    }));
}

fn setup_sftp_restart_button(state: &Arc<StdMutex<AppState>>, button: &Button, status: &Label, toggle_btn: &Button) {
    let status_clone = status.clone();
    let button_clone = button.clone();
    let state_clone = Arc::clone(state);
    let toggle_btn_clone = toggle_btn.clone();
    
    button.connect_clicked(clone!(@strong status_clone, @strong button_clone, @strong state_clone, @strong toggle_btn_clone => move |_| {
        status_clone.set_text("SFTP: 重启中...");
        let status = status_clone.clone();
        let state = Arc::clone(&state_clone);
        let toggle = toggle_btn_clone.clone();
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            let _ = IpcClient::stop_sftp();
            std::thread::sleep(std::time::Duration::from_millis(500));
            
            match IpcClient::start_sftp() {
                Ok(response) => {
                    if response.success {
                        status.set_markup("<span foreground='green'>运行中 ✓</span>");
                        toggle.set_label("停止");
                        if let Ok(s) = state.try_lock()
                            && let Ok(mut log) = s.logger.try_lock() {
                                log.info("SFTP", "SFTP服务已重启");
                            }
                    } else {
                        status.set_markup(&format!("<span foreground='red'>{}</span>", response.message));
                        toggle.set_label("启动");
                    }
                }
                Err(e) => {
                    status.set_markup(&format!("<span foreground='red'>错误 - {}</span>", e));
                    toggle.set_label("启动");
                }
            }
        });
    }));
}

fn create_ftp_config_frame(
    container: &Box, 
    state: &Arc<StdMutex<AppState>>,
) {
    let config_frame = Frame::new(Some("FTP配置"));
    let config_box = Box::new(Orientation::Vertical, 5);
    config_box.set_margin_top(10);
    config_box.set_margin_bottom(10);
    config_box.set_margin_start(10);
    config_box.set_margin_end(10);

    let (ftp_enabled_cb, anon_cb, bind_ip_entry, ftp_port_spin, passive_start_spin, passive_end_spin, welcome_entry, 
         anon_home_entry, max_speed_spin, encoding_combo, anon_status_label, masquerade_ip_entry, 
         max_conn_spin, idle_timeout_spin) = load_ftp_config(state);

    let row1 = Box::new(Orientation::Horizontal, 5);
    row1.pack_start(&ftp_enabled_cb, false, false, 0);
    row1.pack_start(&Label::new(Some("绑定地址:")), false, false, 0);
    row1.pack_start(&bind_ip_entry, false, false, 0);
    row1.pack_start(&Label::new(Some("端口:")), false, false, 0);
    row1.pack_start(&ftp_port_spin, false, false, 0);
    
    let save_btn = Button::with_label("保存配置");
    row1.pack_end(&save_btn, false, false, 0);
    config_box.pack_start(&row1, false, false, 0);

    let row2 = Box::new(Orientation::Horizontal, 5);
    row2.pack_start(&Label::new(Some("被动端口范围:")), false, false, 0);
    row2.pack_start(&passive_start_spin, false, false, 0);
    row2.pack_start(&Label::new(Some("-")), false, false, 0);
    row2.pack_start(&passive_end_spin, false, false, 0);
    config_box.pack_start(&row2, false, false, 0);

    let row_masq = Box::new(Orientation::Horizontal, 5);
    let masq_label = Label::new(Some("对外公开IP:"));
    row_masq.pack_start(&masq_label, false, false, 0);
    row_masq.pack_start(&masquerade_ip_entry, false, false, 0);
    let masq_hint = Label::new(Some("(NAT环境下PASV模式返回的IP地址)"));
    masq_hint.set_markup("<span foreground='gray' size='small'>(NAT环境下PASV模式返回的IP地址)</span>");
    row_masq.pack_start(&masq_hint, false, false, 0);
    config_box.pack_start(&row_masq, false, false, 0);

    let row3 = Box::new(Orientation::Horizontal, 5);
    row3.pack_start(&Label::new(Some("欢迎消息:")), false, false, 0);
    welcome_entry.set_hexpand(true);
    row3.pack_start(&welcome_entry, true, true, 0);
    config_box.pack_start(&row3, false, false, 0);

    let anon_row = Box::new(Orientation::Horizontal, 5);
    anon_row.pack_start(&anon_cb, false, false, 0);
    config_box.pack_start(&anon_row, false, false, 0);

    let row4 = Box::new(Orientation::Horizontal, 5);
    row4.pack_start(&Label::new(Some("匿名用户目录:")), false, false, 0);
    anon_home_entry.set_hexpand(true);
    row4.pack_start(&anon_home_entry, true, true, 0);
    
    let browse_btn = Button::with_label("浏览...");
    row4.pack_start(&browse_btn, false, false, 0);
    config_box.pack_start(&row4, false, false, 0);
    
    let anon_status_clone = anon_status_label.clone();
    let anon_home_clone = anon_home_entry.clone();
    browse_btn.connect_clicked(clone!(@strong anon_status_clone, @strong anon_home_clone => move |_| {
        let dialog = FileChooserDialog::new(
            Some("选择匿名用户主目录"),
            None::<&gtk::Window>,
            FileChooserAction::SelectFolder,
        );
        dialog.add_button("取消", gtk::ResponseType::Cancel);
        dialog.add_button("选择", gtk::ResponseType::Accept);
        
        let entry = anon_home_clone.clone();
        let status = anon_status_clone.clone();
        dialog.connect_response(clone!(@strong entry, @strong status, @strong dialog => move |dlg, resp| {
            if resp == gtk::ResponseType::Accept
                && let Some(path) = dlg.filename() {
                    let path_str = path.to_string_lossy().to_string();
                    entry.set_text(&path_str);
                    validate_anonymous_home(&entry, &status, true);
                }
            dlg.close();
        }));
        
        dialog.run();
    }));

    let row4_status = Box::new(Orientation::Horizontal, 5);
    row4_status.pack_start(&anon_status_label, false, false, 0);
    config_box.pack_start(&row4_status, false, false, 0);

    let row5 = Box::new(Orientation::Horizontal, 5);
    row5.pack_start(&Label::new(Some("最大传输速度(KB/s):")), false, false, 0);
    row5.pack_start(&max_speed_spin, false, false, 0);
    row5.pack_start(&Label::new(Some("编码:")), false, false, 0);
    row5.pack_start(&encoding_combo, false, false, 0);
    config_box.pack_start(&row5, false, false, 0);

    let row_conn = Box::new(Orientation::Horizontal, 5);
    row_conn.pack_start(&Label::new(Some("最大连接数:")), false, false, 0);
    row_conn.pack_start(&max_conn_spin, false, false, 0);
    row_conn.pack_start(&Label::new(Some("空闲超时(秒):")), false, false, 0);
    row_conn.pack_start(&idle_timeout_spin, false, false, 0);
    let conn_hint = Label::new(None);
    conn_hint.set_markup("<span foreground='gray' size='small'>(连接空闲超过此时长将自动断开)</span>");
    row_conn.pack_start(&conn_hint, false, false, 0);
    config_box.pack_start(&row_conn, false, false, 0);

    let anon_cb_for_signal = anon_cb.clone();
    let anon_home_for_signal = anon_home_entry.clone();
    let anon_status_for_signal = anon_status_label.clone();
    anon_cb.connect_toggled(clone!(@strong anon_cb_for_signal, @strong anon_home_for_signal, @strong anon_status_for_signal => move |_| {
        validate_anonymous_home(&anon_home_for_signal, &anon_status_for_signal, anon_cb_for_signal.is_active());
    }));

    let anon_home_for_change = anon_home_entry.clone();
    let anon_status_for_change = anon_status_label.clone();
    let anon_cb_for_change = anon_cb.clone();
    anon_home_entry.connect_changed(clone!(@strong anon_home_for_change, @strong anon_status_for_change, @strong anon_cb_for_change => move |_| {
        validate_anonymous_home(&anon_home_for_change, &anon_status_for_change, anon_cb_for_change.is_active());
    }));

    setup_ftp_save_button(
        state, &save_btn, &ftp_enabled_cb, &anon_cb, &bind_ip_entry, &ftp_port_spin,
        &passive_start_spin, &passive_end_spin, &welcome_entry, 
        &anon_home_entry, &max_speed_spin, &encoding_combo, &anon_status_label, &masquerade_ip_entry,
        &max_conn_spin, &idle_timeout_spin,
    );

    config_frame.add(&config_box);
    container.pack_start(&config_frame, false, false, 0);
}

fn load_ftp_config(state: &Arc<StdMutex<AppState>>) -> (CheckButton, CheckButton, Entry, SpinButton, SpinButton, SpinButton, Entry, Entry, SpinButton, ComboBoxText, Label, Entry, SpinButton, SpinButton) {
    let ftp_enabled_cb = CheckButton::with_label("启用FTP服务");
    let anon_cb = CheckButton::with_label("允许匿名访问");
    let bind_ip_entry = Entry::new();
    bind_ip_entry.set_width_chars(15);
    bind_ip_entry.set_placeholder_text(Some("0.0.0.0"));
    let ftp_port_spin = create_spin_button(1.0, 65535.0, 1.0);
    let passive_start_spin = create_spin_button(1024.0, 65535.0, 1.0);
    let passive_end_spin = create_spin_button(1024.0, 65535.0, 1.0);
    let welcome_entry = Entry::new();
    let anon_home_entry = Entry::new();
    let max_speed_spin = create_spin_button(0.0, 102400.0, 100.0);
    let encoding_combo = ComboBoxText::new();
    encoding_combo.append(Some("utf-8"), "UTF-8");
    encoding_combo.append(Some("gbk"), "GBK");
    encoding_combo.append(Some("gb2312"), "GB2312");
    encoding_combo.set_active_id(Some("utf-8"));
    let anon_status_label = Label::new(None);
    let masquerade_ip_entry = Entry::new();
    masquerade_ip_entry.set_width_chars(15);
    masquerade_ip_entry.set_placeholder_text(Some("如: 192.168.1.100"));
    let max_conn_spin = create_spin_button(1.0, 10000.0, 10.0);
    let idle_timeout_spin = create_spin_button(60.0, 86400.0, 60.0);

    if let Ok(s) = state.try_lock()
        && let Ok(cfg) = s.config.try_lock() {
            ftp_enabled_cb.set_active(cfg.ftp.enabled);
            anon_cb.set_active(cfg.ftp.allow_anonymous);
            bind_ip_entry.set_text(&cfg.ftp.bind_ip);
            ftp_port_spin.set_value(cfg.server.ftp_port as f64);
            passive_start_spin.set_value(cfg.ftp.passive_ports.0 as f64);
            passive_end_spin.set_value(cfg.ftp.passive_ports.1 as f64);
            welcome_entry.set_text(&cfg.ftp.welcome_message);
            if let Some(ref anon_home) = cfg.ftp.anonymous_home {
                anon_home_entry.set_text(anon_home);
            }
            max_speed_spin.set_value(cfg.ftp.max_speed_kbps as f64);
            let encoding_id = cfg.ftp.encoding.to_lowercase();
            encoding_combo.set_active_id(Some(&encoding_id));
            if let Some(ref masq_ip) = cfg.ftp.masquerade_ip {
                masquerade_ip_entry.set_text(masq_ip);
            }
            max_conn_spin.set_value(cfg.server.max_connections as f64);
            idle_timeout_spin.set_value(cfg.server.idle_timeout as f64);
        }

    validate_anonymous_home(&anon_home_entry, &anon_status_label, anon_cb.is_active());

    (ftp_enabled_cb, anon_cb, bind_ip_entry, ftp_port_spin, passive_start_spin, passive_end_spin, welcome_entry, anon_home_entry, max_speed_spin, encoding_combo, anon_status_label, masquerade_ip_entry, max_conn_spin, idle_timeout_spin)
}

fn validate_anonymous_home(entry: &Entry, status_label: &Label, allow_anon: bool) {
    let home = entry.text().to_string();
    
    if !allow_anon {
        status_label.set_markup("<span foreground='gray' size='small'>匿名访问未启用</span>");
        return;
    }
    
    if home.trim().is_empty() {
        status_label.set_markup("<span foreground='red' size='small'>⚠ 启用匿名访问必须配置匿名用户目录</span>");
        return;
    }
    
    let path = Path::new(&home);
    
    if !path.exists() {
        status_label.set_markup(&format!(
            "<span foreground='red' size='small'>⚠ 目录不存在: {}</span>",
            home
        ));
        return;
    }
    
    if !path.is_dir() {
        status_label.set_markup(&format!(
            "<span foreground='red' size='small'>⚠ 路径不是目录: {}</span>",
            home
        ));
        return;
    }
    
    status_label.set_markup("<span foreground='green' size='small'>✓ 目录有效</span>");
}

#[allow(clippy::too_many_arguments)]
fn setup_ftp_save_button(
    state: &Arc<StdMutex<AppState>>,
    save_btn: &Button,
    ftp_enabled_cb: &CheckButton,
    anon_cb: &CheckButton,
    bind_ip_entry: &Entry,
    ftp_port_spin: &SpinButton,
    passive_start_spin: &SpinButton,
    passive_end_spin: &SpinButton,
    welcome_entry: &Entry,
    anon_home_entry: &Entry,
    max_speed_spin: &SpinButton,
    encoding_combo: &ComboBoxText,
    anon_status_label: &Label,
    masquerade_ip_entry: &Entry,
    max_conn_spin: &SpinButton,
    idle_timeout_spin: &SpinButton,
) {
    let state_clone = Arc::clone(state);
    let ftp_enabled_clone = ftp_enabled_cb.clone();
    let anon_clone = anon_cb.clone();
    let bind_ip_clone = bind_ip_entry.clone();
    let ftp_port_clone = ftp_port_spin.clone();
    let passive_start_clone = passive_start_spin.clone();
    let passive_end_clone = passive_end_spin.clone();
    let welcome_clone = welcome_entry.clone();
    let anon_home_clone = anon_home_entry.clone();
    let max_speed_clone = max_speed_spin.clone();
    let encoding_clone = encoding_combo.clone();
    let anon_status_clone = anon_status_label.clone();
    let masquerade_ip_clone = masquerade_ip_entry.clone();
    let max_conn_clone = max_conn_spin.clone();
    let idle_timeout_clone = idle_timeout_spin.clone();
    
    save_btn.connect_clicked(clone!(@strong state_clone, @strong ftp_enabled_clone, @strong anon_clone,
               @strong bind_ip_clone, @strong ftp_port_clone, @strong passive_start_clone, @strong passive_end_clone, 
               @strong welcome_clone, @strong anon_home_clone, @strong max_speed_clone, 
               @strong encoding_clone, @strong anon_status_clone, @strong masquerade_ip_clone,
               @strong max_conn_clone, @strong idle_timeout_clone => move |_| {
        let enabled = ftp_enabled_clone.is_active();
        let anon = anon_clone.is_active();
        let bind_ip = bind_ip_clone.text().to_string();
        let ftp_port = ftp_port_clone.value() as u16;
        let start = passive_start_clone.value() as u16;
        let end = passive_end_clone.value() as u16;
        let welcome = welcome_clone.text().to_string();
        let anon_home = anon_home_clone.text().to_string();
        let max_speed = max_speed_clone.value() as u64;
        let encoding = encoding_clone.active_text().map(|s| s.to_string()).unwrap_or_else(|| "UTF-8".to_string());
        let masquerade_ip = masquerade_ip_clone.text().to_string();
        let masquerade_ip_opt = if masquerade_ip.trim().is_empty() { None } else { Some(masquerade_ip.trim().to_string()) };
        let max_conn = max_conn_clone.value() as usize;
        let idle_timeout = idle_timeout_clone.value() as u64;
        
        if anon {
            if anon_home.trim().is_empty() {
                anon_status_clone.set_markup("<span foreground='red' size='small'>⚠ 启用匿名访问必须配置匿名用户目录</span>");
                if let Ok(s) = state_clone.try_lock()
                    && let Ok(mut log) = s.logger.try_lock() {
                        log.error("CONFIG", "保存失败: 启用匿名访问必须配置匿名用户目录");
                    }
                return;
            }
            
            let path = Path::new(&anon_home);
            if !path.exists() {
                anon_status_clone.set_markup(&format!(
                    "<span foreground='red' size='small'>⚠ 目录不存在: {}</span>",
                    anon_home
                ));
                if let Ok(s) = state_clone.try_lock()
                    && let Ok(mut log) = s.logger.try_lock() {
                        log.error("CONFIG", &format!("保存失败: 匿名用户目录不存在: {}", anon_home));
                    }
                return;
            }
            
            if !path.is_dir() {
                anon_status_clone.set_markup(&format!(
                    "<span foreground='red' size='small'>⚠ 路径不是目录: {}</span>",
                    anon_home
                ));
                if let Ok(s) = state_clone.try_lock()
                    && let Ok(mut log) = s.logger.try_lock() {
                        log.error("CONFIG", &format!("保存失败: 匿名用户目录路径不是目录: {}", anon_home));
                    }
                return;
            }
        }
        
        let config_str = {
            if let Ok(s) = state_clone.try_lock() {
                if let Ok(mut cfg) = s.config.try_lock() {
                    cfg.ftp.enabled = enabled;
                    cfg.ftp.allow_anonymous = anon;
                    cfg.ftp.bind_ip = if bind_ip.is_empty() { "0.0.0.0".to_string() } else { bind_ip };
                    cfg.server.ftp_port = ftp_port;
                    cfg.ftp.passive_ports = (start.min(end), start.max(end));
                    cfg.ftp.welcome_message = welcome;
                    cfg.ftp.anonymous_home = if anon_home.is_empty() { None } else { Some(anon_home) };
                    cfg.ftp.max_speed_kbps = max_speed;
                    cfg.ftp.encoding = encoding.clone();
                    cfg.ftp.masquerade_ip = masquerade_ip_opt;
                    cfg.server.max_connections = max_conn;
                    cfg.server.idle_timeout = idle_timeout;
                    toml::to_string_pretty(&*cfg).unwrap_or_default()
                } else { return; }
            } else { return; }
        };
        
        match crate::communication::dbus::write_config(&config_str) {
            Ok(()) => {
                if anon {
                    anon_status_clone.set_markup("<span foreground='green' size='small'>✓ 目录有效</span>");
                } else {
                    anon_status_clone.set_markup("<span foreground='gray' size='small'>匿名访问未启用</span>");
                }
                if let Ok(s) = state_clone.try_lock()
                    && let Ok(mut log) = s.logger.try_lock() {
                        log.info("CONFIG", &format!(
                            "FTP配置已保存: 启用={}, 绑定={}, 端口={}, 编码={}, 匿名={}, 对外IP={}, 最大连接={}, 空闲超时={}s",
                            if enabled { "是" } else { "否" },
                            bind_ip_clone.text(),
                            ftp_port,
                            encoding,
                            if anon { "是" } else { "否" },
                            masquerade_ip_clone.text(),
                            max_conn,
                            idle_timeout
                        ));
                    }
            }
            Err(e) => {
                if let Ok(s) = state_clone.try_lock()
                    && let Ok(mut log) = s.logger.try_lock() {
                        log.error("CONFIG", &format!("保存配置失败: {}", e));
                    }
            }
        }
    }));
}

fn create_sftp_config_frame(
    container: &Box, 
    state: &Arc<StdMutex<AppState>>,
) {
    let config_frame = Frame::new(Some("SFTP配置"));
    let config_box = Box::new(Orientation::Vertical, 5);
    config_box.set_margin_top(10);
    config_box.set_margin_bottom(10);
    config_box.set_margin_start(10);
    config_box.set_margin_end(10);

    let (sftp_enabled_cb, sftp_port_spin, max_auth_spin, auth_timeout_spin, host_key_entry, 
         log_level_combo) = load_sftp_config(state);

    let row1 = Box::new(Orientation::Horizontal, 5);
    row1.pack_start(&sftp_enabled_cb, false, false, 0);
    row1.pack_start(&Label::new(Some("端口:")), false, false, 0);
    row1.pack_start(&sftp_port_spin, false, false, 0);
    row1.pack_start(&Label::new(Some("最大认证尝试:")), false, false, 0);
    row1.pack_start(&max_auth_spin, false, false, 0);
    
    let save_btn = Button::with_label("保存配置");
    row1.pack_end(&save_btn, false, false, 0);
    config_box.pack_start(&row1, false, false, 0);

    let row2 = Box::new(Orientation::Horizontal, 5);
    row2.pack_start(&Label::new(Some("认证超时(秒):")), false, false, 0);
    row2.pack_start(&auth_timeout_spin, false, false, 0);
    row2.pack_start(&Label::new(Some("日志级别:")), false, false, 0);
    row2.pack_start(&log_level_combo, false, false, 0);
    config_box.pack_start(&row2, false, false, 0);

    let row3 = Box::new(Orientation::Horizontal, 5);
    row3.pack_start(&Label::new(Some("主机密钥路径:")), false, false, 0);
    host_key_entry.set_hexpand(true);
    row3.pack_start(&host_key_entry, true, true, 0);
    config_box.pack_start(&row3, false, false, 0);

    setup_sftp_save_button(
        state, &save_btn, &sftp_enabled_cb, &sftp_port_spin,
        &max_auth_spin, &auth_timeout_spin, &host_key_entry, 
        &log_level_combo,
    );

    config_frame.add(&config_box);
    container.pack_start(&config_frame, false, false, 0);
}

fn load_sftp_config(state: &Arc<StdMutex<AppState>>) -> (CheckButton, SpinButton, SpinButton, SpinButton, Entry, ComboBoxText) {
    let sftp_enabled_cb = CheckButton::with_label("启用SFTP服务");
    let sftp_port_spin = create_spin_button(1.0, 65535.0, 1.0);
    let max_auth_spin = create_spin_button(1.0, 10.0, 1.0);
    let auth_timeout_spin = create_spin_button(10.0, 300.0, 5.0);
    let host_key_entry = Entry::new();
    let log_level_combo = ComboBoxText::new();
    log_level_combo.append(Some("debug"), "Debug");
    log_level_combo.append(Some("info"), "Info");
    log_level_combo.append(Some("warn"), "Warning");
    log_level_combo.append(Some("error"), "Error");
    log_level_combo.set_active_id(Some("info"));

    if let Ok(s) = state.try_lock()
        && let Ok(cfg) = s.config.try_lock() {
            sftp_enabled_cb.set_active(cfg.sftp.enabled);
            sftp_port_spin.set_value(cfg.server.sftp_port as f64);
            max_auth_spin.set_value(cfg.sftp.max_auth_attempts as f64);
            auth_timeout_spin.set_value(cfg.sftp.auth_timeout as f64);
            host_key_entry.set_text(&cfg.sftp.host_key_path);
            let log_level_id = cfg.sftp.log_level.to_lowercase();
            log_level_combo.set_active_id(Some(&log_level_id));
        }

    (sftp_enabled_cb, sftp_port_spin, max_auth_spin, auth_timeout_spin, host_key_entry, log_level_combo)
}

#[allow(clippy::too_many_arguments)]
fn setup_sftp_save_button(
    state: &Arc<StdMutex<AppState>>,
    save_btn: &Button,
    sftp_enabled_cb: &CheckButton,
    sftp_port_spin: &SpinButton,
    max_auth_spin: &SpinButton,
    auth_timeout_spin: &SpinButton,
    host_key_entry: &Entry,
    log_level_combo: &ComboBoxText,
) {
    let state_clone = Arc::clone(state);
    let sftp_enabled_clone = sftp_enabled_cb.clone();
    let sftp_port_clone = sftp_port_spin.clone();
    let max_auth_clone = max_auth_spin.clone();
    let auth_timeout_clone = auth_timeout_spin.clone();
    let host_key_clone = host_key_entry.clone();
    let log_level_clone = log_level_combo.clone();
    
    save_btn.connect_clicked(clone!(@strong state_clone, @strong sftp_enabled_clone, @strong sftp_port_clone,
               @strong max_auth_clone, @strong auth_timeout_clone, @strong host_key_clone, 
               @strong log_level_clone => move |_| {
        let enabled = sftp_enabled_clone.is_active();
        let sftp_port = sftp_port_clone.value() as u16;
        let max_auth = max_auth_clone.value() as u32;
        let auth_timeout = auth_timeout_clone.value() as u64;
        let host_key = host_key_clone.text().to_string();
        let log_level = log_level_clone.active_text().map(|s| s.to_string()).unwrap_or_else(|| "info".to_string());
        
        let config_str = {
            if let Ok(s) = state_clone.try_lock() {
                if let Ok(mut cfg) = s.config.try_lock() {
                    cfg.sftp.enabled = enabled;
                    cfg.server.sftp_port = sftp_port;
                    cfg.sftp.max_auth_attempts = max_auth;
                    cfg.sftp.auth_timeout = auth_timeout;
                    cfg.sftp.host_key_path = host_key;
                    cfg.sftp.log_level = log_level.clone();
                    toml::to_string_pretty(&*cfg).unwrap_or_default()
                } else { return; }
            } else { return; }
        };
        
        match crate::communication::dbus::write_config(&config_str) {
            Ok(()) => {
                if let Ok(s) = state_clone.try_lock()
                    && let Ok(mut log) = s.logger.try_lock() {
                        log.info("CONFIG", &format!(
                            "SFTP配置已保存: 启用={}, 端口={}, 日志级别={}",
                            if enabled { "是" } else { "否" },
                            sftp_port,
                            log_level
                        ));
                    }
            }
            Err(e) => {
                if let Ok(s) = state_clone.try_lock()
                    && let Ok(mut log) = s.logger.try_lock() {
                        log.error("CONFIG", &format!("保存配置失败: {}", e));
                    }
            }
        }
    }));
}

fn create_spin_button(min: f64, max: f64, step: f64) -> SpinButton {
    let adjustment = Adjustment::new(min, min, max, step, step * 10.0, 0.0);
    SpinButton::builder()
        .adjustment(&adjustment)
        .digits(0)
        .width_chars(6)
        .build()
}
