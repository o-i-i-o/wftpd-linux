use gtk::prelude::*;
use gtk::{Box, Orientation, Label, Button, Frame, glib, CheckButton};
use gtk::glib::clone;
use std::sync::{Arc, Mutex as StdMutex};
use crate::AppState;
use tracing::{info, error};

pub fn create(state: &Arc<StdMutex<AppState>>) -> Box {
    let container = Box::new(Orientation::Vertical, 10);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);

    let (status_label, autostart_btn) = create_control_frame(&container, state);
    create_status_frame(&container, state, &status_label, &autostart_btn);

    container
}

fn create_control_frame(container: &Box, state: &Arc<StdMutex<AppState>>) -> (Label, CheckButton) {
    let control_frame = Frame::new(Some("服务控制"));
    let control_box = Box::new(Orientation::Vertical, 10);
    control_box.set_margin_top(10);
    control_box.set_margin_bottom(10);
    control_box.set_margin_start(10);
    control_box.set_margin_end(10);

    let status_label = Label::new(None);
    update_service_status(state, &status_label);
    control_box.pack_start(&status_label, false, false, 0);

    let button_box = Box::new(Orientation::Horizontal, 10);

    let install_btn = Button::with_label("安装服务");
    let start_btn = Button::with_label("启动服务");
    let stop_btn = Button::with_label("停止服务");
    let restart_btn = Button::with_label("重启服务");
    let uninstall_btn = Button::with_label("卸载服务");
    let refresh_btn = Button::with_label("刷新状态");

    let autostart_btn = CheckButton::with_label("开机自启");
    update_autostart_status(state, &autostart_btn);

    setup_service_buttons(state, &install_btn, &start_btn, &stop_btn, &restart_btn, &uninstall_btn, &refresh_btn, &autostart_btn, &status_label);

    button_box.pack_start(&install_btn, false, false, 0);
    button_box.pack_start(&start_btn, false, false, 0);
    button_box.pack_start(&stop_btn, false, false, 0);
    button_box.pack_start(&restart_btn, false, false, 0);
    button_box.pack_start(&uninstall_btn, false, false, 0);
    button_box.pack_start(&refresh_btn, false, false, 0);
    control_box.pack_start(&button_box, false, false, 0);

    let autostart_box = Box::new(Orientation::Horizontal, 10);
    autostart_box.pack_start(&autostart_btn, false, false, 0);
    control_box.pack_start(&autostart_box, false, false, 0);

    control_frame.add(&control_box);
    container.pack_start(&control_frame, false, false, 0);

    (status_label, autostart_btn)
}

#[allow(clippy::too_many_arguments)]
fn setup_service_buttons(
    state: &Arc<StdMutex<AppState>>,
    install_btn: &Button,
    start_btn: &Button,
    stop_btn: &Button,
    restart_btn: &Button,
    uninstall_btn: &Button,
    refresh_btn: &Button,
    autostart_btn: &CheckButton,
    status_label: &Label,
) {
    let state_clone = Arc::clone(state);
    let status_label_clone = status_label.clone();
    let autostart_btn_clone = autostart_btn.clone();
    install_btn.connect_clicked(clone!(@strong state_clone, @strong status_label_clone, @strong autostart_btn_clone => move |_| {
        let state = Arc::clone(&state_clone);
        let status_label = status_label_clone.clone();
        let autostart_btn = autostart_btn_clone.clone();
        let exe_path = std::env::current_exe().unwrap_or_default();
        let exe_path_str = exe_path.to_string_lossy().to_string();
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            if let Ok(s) = state.try_lock() {
                match s.service_manager.install_service(&exe_path_str) {
                    Ok(_) => {
                        info!("Service installed successfully");
                        let _ = s.service_manager.reload_daemon();
                        update_service_status(&state, &status_label);
                        update_autostart_status(&state, &autostart_btn);
                    }
                    Err(e) => {
                        error!("Install error: {}", e);
                        status_label.set_markup(&format!("<span foreground='red'>安装失败: {}</span>", e));
                    }
                }
            }
        });
    }));

    let state_clone = Arc::clone(state);
    let status_label_clone = status_label.clone();
    start_btn.connect_clicked(clone!(@strong state_clone, @strong status_label_clone => move |_| {
        let state = Arc::clone(&state_clone);
        let status_label = status_label_clone.clone();
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            if let Ok(s) = state.try_lock() {
                match s.service_manager.start_service() {
                    Ok(_) => {
                        info!("Service started successfully");
                        update_service_status(&state, &status_label);
                    }
                    Err(e) => {
                        error!("Start error: {}", e);
                        status_label.set_markup(&format!("<span foreground='red'>启动失败: {}</span>", e));
                    }
                }
            }
        });
    }));

    let state_clone = Arc::clone(state);
    let status_label_clone = status_label.clone();
    stop_btn.connect_clicked(clone!(@strong state_clone, @strong status_label_clone => move |_| {
        let state = Arc::clone(&state_clone);
        let status_label = status_label_clone.clone();
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            if let Ok(s) = state.try_lock() {
                match s.service_manager.stop_service() {
                    Ok(_) => {
                        info!("Service stopped successfully");
                        update_service_status(&state, &status_label);
                    }
                    Err(e) => {
                        error!("Stop error: {}", e);
                        status_label.set_markup(&format!("<span foreground='red'>停止失败: {}</span>", e));
                    }
                }
            }
        });
    }));

    let state_clone = Arc::clone(state);
    let status_label_clone = status_label.clone();
    restart_btn.connect_clicked(clone!(@strong state_clone, @strong status_label_clone => move |_| {
        let state = Arc::clone(&state_clone);
        let status_label = status_label_clone.clone();
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            if let Ok(s) = state.try_lock() {
                match s.service_manager.restart_service() {
                    Ok(_) => {
                        info!("Service restarted successfully");
                        update_service_status(&state, &status_label);
                    }
                    Err(e) => {
                        error!("Restart error: {}", e);
                        status_label.set_markup(&format!("<span foreground='red'>重启失败: {}</span>", e));
                    }
                }
            }
        });
    }));

    let state_clone = Arc::clone(state);
    let status_label_clone = status_label.clone();
    let autostart_btn_clone = autostart_btn.clone();
    uninstall_btn.connect_clicked(clone!(@strong state_clone, @strong status_label_clone, @strong autostart_btn_clone => move |_| {
        let state = Arc::clone(&state_clone);
        let status_label = status_label_clone.clone();
        let autostart_btn = autostart_btn_clone.clone();
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            if let Ok(s) = state.try_lock() {
                match s.service_manager.uninstall_service() {
                    Ok(_) => {
                        info!("Service uninstalled successfully");
                        let _ = s.service_manager.reload_daemon();
                        update_service_status(&state, &status_label);
                        update_autostart_status(&state, &autostart_btn);
                    }
                    Err(e) => {
                        error!("Uninstall error: {}", e);
                        status_label.set_markup(&format!("<span foreground='red'>卸载失败: {}</span>", e));
                    }
                }
            }
        });
    }));

    let state_clone = Arc::clone(state);
    let status_label_clone = status_label.clone();
    let autostart_btn_clone = autostart_btn.clone();
    refresh_btn.connect_clicked(clone!(@strong state_clone, @strong status_label_clone, @strong autostart_btn_clone => move |_| {
        update_service_status(&state_clone, &status_label_clone);
        update_autostart_status(&state_clone, &autostart_btn_clone);
    }));

    let state_clone = Arc::clone(state);
    let autostart_btn_clone = autostart_btn.clone();
    autostart_btn.connect_toggled(clone!(@strong state_clone, @strong autostart_btn_clone => move |_| {
        let state = Arc::clone(&state_clone);
        let autostart_btn = autostart_btn_clone.clone();
        let is_active = autostart_btn.is_active();
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            if let Ok(s) = state.try_lock() {
                let result = if is_active {
                    s.service_manager.enable_service()
                } else {
                    s.service_manager.disable_service()
                };
                
                match result {
                    Ok(_) => {
                        info!("Autostart {} successfully", if is_active { "enabled" } else { "disabled" });
                    }
                    Err(e) => {
                        error!("Autostart error: {}", e);
                        update_autostart_status(&state, &autostart_btn);
                    }
                }
            }
        });
    }));
}

fn create_status_frame(container: &Box, state: &Arc<StdMutex<AppState>>, status_label: &Label, autostart_btn: &CheckButton) {
    let status_frame = Frame::new(Some("服务信息"));
    let status_box = Box::new(Orientation::Vertical, 5);
    status_box.set_margin_top(10);
    status_box.set_margin_bottom(10);
    status_box.set_margin_start(10);
    status_box.set_margin_end(10);

    let info_label = Label::new(None);
    info_label.set_markup(
        "<b>系统服务说明:</b>\n\
         • 安装服务: 将程序注册为systemd系统服务\n\
         • 启动服务: 启动已安装的系统服务\n\
         • 停止服务: 停止正在运行的系统服务\n\
         • 开机自启: 设置服务是否随系统启动自动运行\n\n\
         <i>注意: 服务操作通过PolicyKit进行权限认证</i>"
    );
    status_box.pack_start(&info_label, false, false, 0);

    status_frame.add(&status_box);
    container.pack_start(&status_frame, false, false, 0);

    update_service_status(state, status_label);
    update_autostart_status(state, autostart_btn);
}

fn update_service_status(state: &Arc<StdMutex<AppState>>, status_label: &Label) {
    if let Ok(s) = state.try_lock() {
        let sm = &s.service_manager;
        
        let service_exists = sm.service_exists();
        let is_running = if service_exists { sm.is_service_running() } else { false };
        
        let status_text = if !service_exists {
            "<span foreground='gray'>服务状态: 未安装</span>".to_string()
        } else if is_running {
            "<span foreground='green'>服务状态: 运行中 ✓</span>".to_string()
        } else {
            "<span foreground='orange'>服务状态: 已安装但未运行</span>".to_string()
        };
        
        status_label.set_markup(&status_text);
    }
}

fn update_autostart_status(state: &Arc<StdMutex<AppState>>, autostart_btn: &CheckButton) {
    if let Ok(s) = state.try_lock() {
        let sm = &s.service_manager;
        let is_enabled = sm.service_exists() && sm.is_service_enabled();
        
        autostart_btn.set_active(is_enabled);
        autostart_btn.set_sensitive(sm.service_exists());
    }
}