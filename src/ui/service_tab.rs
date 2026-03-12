use gtk::prelude::*;
use gtk::{Box, Orientation, Label, Button, Frame};
use gtk::glib::clone;
use std::sync::{Arc, Mutex as StdMutex};
use wftpg::AppState;

fn check_root_permission() -> bool {
    unsafe { libc::getuid() == 0 }
}

pub fn create(state: &Arc<StdMutex<AppState>>) -> Box {
    let container = Box::new(Orientation::Vertical, 10);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);

    let title_label = Label::new(Some("<b>系统服务</b>"));
    title_label.set_use_markup(true);
    container.pack_start(&title_label, false, false, 0);

    let status_label = create_control_frame(&container, state);
    create_status_frame(&container, state, &status_label);

    container
}

fn create_control_frame(container: &Box, state: &Arc<StdMutex<AppState>>) -> Label {
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
    let uninstall_btn = Button::with_label("卸载服务");
    let refresh_btn = Button::with_label("刷新状态");

    setup_service_buttons(state, &install_btn, &start_btn, &stop_btn, &uninstall_btn, &refresh_btn, &status_label);

    button_box.pack_start(&install_btn, false, false, 0);
    button_box.pack_start(&start_btn, false, false, 0);
    button_box.pack_start(&stop_btn, false, false, 0);
    button_box.pack_start(&uninstall_btn, false, false, 0);
    button_box.pack_start(&refresh_btn, false, false, 0);
    control_box.pack_start(&button_box, false, false, 0);

    control_frame.add(&control_box);
    container.pack_start(&control_frame, false, false, 0);

    status_label
}

fn setup_service_buttons(
    state: &Arc<StdMutex<AppState>>,
    install_btn: &Button,
    start_btn: &Button,
    stop_btn: &Button,
    uninstall_btn: &Button,
    refresh_btn: &Button,
    status_label: &Label,
) {
    let state_clone = Arc::clone(state);
    let status_label_clone = status_label.clone();
    install_btn.connect_clicked(clone!(@strong state_clone, @strong status_label_clone => move |_| {
        if !check_root_permission() {
            log::error!("Service installation requires root permission");
            status_label_clone.set_markup("<span foreground='red'>错误: 需要root权限</span>");
            return;
        }
        let state = state_clone.lock().unwrap();
        let exe_path = std::env::current_exe().unwrap_or_default();
        match state.service_manager.install_service(exe_path.to_str().unwrap_or("")) {
            Ok(_) => {
                log::info!("Service installed successfully");
                let _ = state.service_manager.reload_daemon();
                update_service_status(&state_clone, &status_label_clone);
            }
            Err(e) => {
                log::error!("Install error: {}", e);
                status_label_clone.set_markup(&format!("<span foreground='red'>安装失败: {}</span>", e));
            }
        }
    }));

    let state_clone = Arc::clone(state);
    let status_label_clone = status_label.clone();
    start_btn.connect_clicked(clone!(@strong state_clone, @strong status_label_clone => move |_| {
        if !check_root_permission() {
            log::error!("Service start requires root permission");
            status_label_clone.set_markup("<span foreground='red'>错误: 需要root权限</span>");
            return;
        }
        let state = state_clone.lock().unwrap();
        match state.service_manager.start_service() {
            Ok(_) => {
                log::info!("Service started successfully");
                update_service_status(&state_clone, &status_label_clone);
            }
            Err(e) => {
                log::error!("Start error: {}", e);
                status_label_clone.set_markup(&format!("<span foreground='red'>启动失败: {}</span>", e));
            }
        }
    }));

    let state_clone = Arc::clone(state);
    let status_label_clone = status_label.clone();
    stop_btn.connect_clicked(clone!(@strong state_clone, @strong status_label_clone => move |_| {
        if !check_root_permission() {
            log::error!("Service stop requires root permission");
            status_label_clone.set_markup("<span foreground='red'>错误: 需要root权限</span>");
            return;
        }
        let state = state_clone.lock().unwrap();
        match state.service_manager.stop_service() {
            Ok(_) => {
                log::info!("Service stopped successfully");
                update_service_status(&state_clone, &status_label_clone);
            }
            Err(e) => {
                log::error!("Stop error: {}", e);
                status_label_clone.set_markup(&format!("<span foreground='red'>停止失败: {}</span>", e));
            }
        }
    }));

    let state_clone = Arc::clone(state);
    let status_label_clone = status_label.clone();
    uninstall_btn.connect_clicked(clone!(@strong state_clone, @strong status_label_clone => move |_| {
        if !check_root_permission() {
            log::error!("Service uninstall requires root permission");
            status_label_clone.set_markup("<span foreground='red'>错误: 需要root权限</span>");
            return;
        }
        let state = state_clone.lock().unwrap();
        match state.service_manager.uninstall_service() {
            Ok(_) => {
                log::info!("Service uninstalled successfully");
                let _ = state.service_manager.reload_daemon();
                update_service_status(&state_clone, &status_label_clone);
            }
            Err(e) => {
                log::error!("Uninstall error: {}", e);
                status_label_clone.set_markup(&format!("<span foreground='red'>卸载失败: {}</span>", e));
            }
        }
    }));

    let state_clone = Arc::clone(state);
    let status_label_clone = status_label.clone();
    refresh_btn.connect_clicked(clone!(@strong state_clone, @strong status_label_clone => move |_| {
        update_service_status(&state_clone, &status_label_clone);
    }));
}

fn create_status_frame(container: &Box, state: &Arc<StdMutex<AppState>>, status_label: &Label) {
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
         • 卸载服务: 移除已安装的系统服务\n\n\
         <i>注意: 所有服务操作都需要root权限</i>"
    );
    status_box.pack_start(&info_label, false, false, 0);

    let perm_label = Label::new(None);
    if check_root_permission() {
        perm_label.set_markup("<span foreground='green'>✓ 当前以root权限运行</span>");
    } else {
        perm_label.set_markup("<span foreground='orange'>⚠ 当前非root权限，服务功能受限</span>");
    }
    status_box.pack_start(&perm_label, false, false, 0);

    status_frame.add(&status_box);
    container.pack_start(&status_frame, false, false, 0);

    update_service_status(state, status_label);
}

fn update_service_status(state: &Arc<StdMutex<AppState>>, status_label: &Label) {
    let state = state.lock().unwrap();
    let sm = &state.service_manager;
    
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
