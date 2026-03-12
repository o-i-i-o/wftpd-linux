use gtk::prelude::*;
use gtk::{Box, Orientation, Label, Button, Frame};
use gtk::glib::clone;
use std::sync::{Arc, Mutex as StdMutex};
use wftpg::AppState;

pub fn create(state: &Arc<StdMutex<AppState>>) -> Box {
    let container = Box::new(Orientation::Vertical, 10);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);

    let title_label = Label::new(Some("<b>系统服务</b>"));
    title_label.set_use_markup(true);
    container.pack_start(&title_label, false, false, 0);

    create_control_frame(&container, state);
    create_status_frame(&container);

    container
}

fn create_control_frame(container: &Box, state: &Arc<StdMutex<AppState>>) {
    let control_frame = Frame::new(Some("服务控制"));
    let control_box = Box::new(Orientation::Vertical, 10);
    control_box.set_margin_top(10);
    control_box.set_margin_bottom(10);
    control_box.set_margin_start(10);
    control_box.set_margin_end(10);

    let button_box = Box::new(Orientation::Horizontal, 10);

    let install_btn = Button::with_label("安装服务");
    let start_btn = Button::with_label("启动服务");
    let stop_btn = Button::with_label("停止服务");
    let uninstall_btn = Button::with_label("卸载服务");

    setup_service_buttons(state, &install_btn, &start_btn, &stop_btn, &uninstall_btn);

    button_box.pack_start(&install_btn, false, false, 0);
    button_box.pack_start(&start_btn, false, false, 0);
    button_box.pack_start(&stop_btn, false, false, 0);
    button_box.pack_start(&uninstall_btn, false, false, 0);
    control_box.pack_start(&button_box, false, false, 0);

    control_frame.add(&control_box);
    container.pack_start(&control_frame, false, false, 0);
}

fn setup_service_buttons(
    state: &Arc<StdMutex<AppState>>,
    install_btn: &Button,
    start_btn: &Button,
    stop_btn: &Button,
    uninstall_btn: &Button,
) {
    let state_clone = Arc::clone(state);
    install_btn.connect_clicked(clone!(@strong state_clone => move |_| {
        let state = state_clone.lock().unwrap();
        let exe_path = std::env::current_exe().unwrap_or_default();
        match state.service_manager.install_service(exe_path.to_str().unwrap_or("")) {
            Ok(_) => log::info!("Service installed"),
            Err(e) => log::error!("Install error: {}", e),
        }
    }));

    let state_clone = Arc::clone(state);
    start_btn.connect_clicked(clone!(@strong state_clone => move |_| {
        let state = state_clone.lock().unwrap();
        match state.service_manager.start_service() {
            Ok(_) => log::info!("Service started"),
            Err(e) => log::error!("Start error: {}", e),
        }
    }));

    let state_clone = Arc::clone(state);
    stop_btn.connect_clicked(clone!(@strong state_clone => move |_| {
        let state = state_clone.lock().unwrap();
        match state.service_manager.stop_service() {
            Ok(_) => log::info!("Service stopped"),
            Err(e) => log::error!("Stop error: {}", e),
        }
    }));

    let state_clone = Arc::clone(state);
    uninstall_btn.connect_clicked(clone!(@strong state_clone => move |_| {
        let state = state_clone.lock().unwrap();
        match state.service_manager.uninstall_service() {
            Ok(_) => log::info!("Service uninstalled"),
            Err(e) => log::error!("Uninstall error: {}", e),
        }
    }));
}

fn create_status_frame(container: &Box) {
    let status_frame = Frame::new(Some("服务状态"));
    let status_box = Box::new(Orientation::Vertical, 5);
    status_box.set_margin_top(10);
    status_box.set_margin_bottom(10);
    status_box.set_margin_start(10);
    status_box.set_margin_end(10);

    let status_label = Label::new(Some("服务状态: 未安装"));
    status_box.pack_start(&status_label, false, false, 0);

    let info_label = Label::new(None);
    info_label.set_markup("<i>系统服务功能需要root权限运行</i>");
    status_box.pack_start(&info_label, false, false, 0);

    status_frame.add(&status_box);
    container.pack_start(&status_frame, false, false, 0);
}
