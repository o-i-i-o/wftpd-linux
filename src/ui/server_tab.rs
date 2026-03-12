use gtk::prelude::*;
use gtk::{
    Box, Orientation, Label, Button, Entry, Frame, Separator,
};
use gtk::glib::clone;
use std::sync::{Arc, Mutex as StdMutex};
use wftpg::AppState;

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
    create_config_frame(&container, state);

    container
}

fn create_status_frame(container: &Box) -> (Label, Label) {
    let status_frame = Frame::new(Some("服务状态"));
    let status_box = Box::new(Orientation::Horizontal, 20);
    status_box.set_margin_top(10);
    status_box.set_margin_bottom(10);
    status_box.set_margin_start(10);
    status_box.set_margin_end(10);

    let ftp_status = Label::new(Some("FTP: 已停止"));
    let sftp_status = Label::new(Some("SFTP: 已停止"));
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

    setup_ftp_start_button(state, &ftp_btn, ftp_status);
    setup_sftp_start_button(state, &sftp_btn, sftp_status);
    setup_ftp_stop_button(state, &stop_ftp_btn, ftp_status);
    setup_sftp_stop_button(state, &stop_sftp_btn, sftp_status);

    control_box.pack_start(&ftp_btn, false, false, 0);
    control_box.pack_start(&stop_ftp_btn, false, false, 0);
    control_box.pack_start(&Separator::new(Orientation::Vertical), false, false, 5);
    control_box.pack_start(&sftp_btn, false, false, 0);
    control_box.pack_start(&stop_sftp_btn, false, false, 0);
    control_frame.add(&control_box);
    container.pack_start(&control_frame, false, false, 0);
}

fn setup_ftp_start_button(state: &Arc<StdMutex<AppState>>, button: &Button, status: &Label) {
    let state_clone = Arc::clone(state);
    let status_clone = status.clone();
    button.connect_clicked(clone!(@strong state_clone, @strong status_clone => move |_| {
        let state = state_clone.lock().unwrap();
        match state.start_ftp() {
            Ok(_) => {
                status_clone.set_text("FTP: 运行中");
                log::info!("FTP started");
            }
            Err(e) => {
                status_clone.set_text(&format!("FTP: 启动失败 - {}", e));
                log::error!("FTP start error: {}", e);
            }
        }
    }));
}

fn setup_sftp_start_button(state: &Arc<StdMutex<AppState>>, button: &Button, status: &Label) {
    let state_clone = Arc::clone(state);
    let status_clone = status.clone();
    button.connect_clicked(clone!(@strong state_clone, @strong status_clone => move |_| {
        let state = state_clone.lock().unwrap();
        match state.start_sftp() {
            Ok(_) => {
                status_clone.set_text("SFTP: 运行中");
                log::info!("SFTP started");
            }
            Err(e) => {
                status_clone.set_text(&format!("SFTP: 启动失败 - {}", e));
                log::error!("SFTP start error: {}", e);
            }
        }
    }));
}

fn setup_ftp_stop_button(state: &Arc<StdMutex<AppState>>, button: &Button, status: &Label) {
    let state_clone = Arc::clone(state);
    let status_clone = status.clone();
    button.connect_clicked(clone!(@strong state_clone, @strong status_clone => move |_| {
        let state = state_clone.lock().unwrap();
        state.stop_ftp();
        status_clone.set_text("FTP: 已停止");
        log::info!("FTP stopped");
    }));
}

fn setup_sftp_stop_button(state: &Arc<StdMutex<AppState>>, button: &Button, status: &Label) {
    let state_clone = Arc::clone(state);
    let status_clone = status.clone();
    button.connect_clicked(clone!(@strong state_clone, @strong status_clone => move |_| {
        let state = state_clone.lock().unwrap();
        state.stop_sftp();
        status_clone.set_text("SFTP: 已停止");
        log::info!("SFTP stopped");
    }));
}

fn create_config_frame(container: &Box, state: &Arc<StdMutex<AppState>>) {
    let config_frame = Frame::new(Some("网络配置"));
    let config_box = Box::new(Orientation::Vertical, 5);
    config_box.set_margin_top(10);
    config_box.set_margin_bottom(10);
    config_box.set_margin_start(10);
    config_box.set_margin_end(10);

    let (ip_entry, ftp_port_entry, sftp_port_entry) = create_config_entries(&config_box, state);
    create_save_button(&config_box, state, &ip_entry, &ftp_port_entry, &sftp_port_entry);

    config_frame.add(&config_box);
    container.pack_start(&config_frame, false, false, 0);
}

fn create_config_entries(
    config_box: &Box,
    state: &Arc<StdMutex<AppState>>,
) -> (Entry, Entry, Entry) {
    let ip_box = Box::new(Orientation::Horizontal, 5);
    ip_box.pack_start(&Label::new(Some("绑定IP:")), false, false, 0);
    let ip_entry = Entry::new();
    ip_entry.set_hexpand(true);
    {
        let state = state.lock().unwrap();
        let config = state.config.lock().unwrap();
        ip_entry.set_text(&config.server.bind_ip);
    }
    ip_box.pack_start(&ip_entry, true, true, 0);
    config_box.pack_start(&ip_box, false, false, 0);

    let port_box = Box::new(Orientation::Horizontal, 5);
    port_box.pack_start(&Label::new(Some("FTP端口:")), false, false, 0);
    let ftp_port_entry = Entry::new();
    ftp_port_entry.set_width_chars(8);
    {
        let state = state.lock().unwrap();
        let config = state.config.lock().unwrap();
        ftp_port_entry.set_text(&config.server.ftp_port.to_string());
    }
    port_box.pack_start(&ftp_port_entry, false, false, 0);

    port_box.pack_start(&Label::new(Some("SFTP端口:")), false, false, 0);
    let sftp_port_entry = Entry::new();
    sftp_port_entry.set_width_chars(8);
    {
        let state = state.lock().unwrap();
        let config = state.config.lock().unwrap();
        sftp_port_entry.set_text(&config.server.sftp_port.to_string());
    }
    port_box.pack_start(&sftp_port_entry, false, false, 0);
    config_box.pack_start(&port_box, false, false, 0);

    (ip_entry, ftp_port_entry, sftp_port_entry)
}

fn create_save_button(
    config_box: &Box,
    state: &Arc<StdMutex<AppState>>,
    ip_entry: &Entry,
    ftp_port_entry: &Entry,
    sftp_port_entry: &Entry,
) {
    let save_btn = Button::with_label("保存配置");
    let state_clone = Arc::clone(state);
    let ip_entry_clone = ip_entry.clone();
    let ftp_port_entry_clone = ftp_port_entry.clone();
    let sftp_port_entry_clone = sftp_port_entry.clone();
    save_btn.connect_clicked(
        clone!(@strong state_clone, @strong ip_entry_clone, @strong ftp_port_entry_clone, @strong sftp_port_entry_clone => move |_| {
            let state = state_clone.lock().unwrap();
            let mut config = state.config.lock().unwrap();
            config.server.bind_ip = ip_entry_clone.text().to_string();
            if let Ok(port) = ftp_port_entry_clone.text().to_string().parse() {
                config.server.ftp_port = port;
            }
            if let Ok(port) = sftp_port_entry_clone.text().to_string().parse() {
                config.server.sftp_port = port;
            }
            let _ = state.save_config();
            log::info!("Configuration saved");
        }),
    );
    config_box.pack_start(&save_btn, false, false, 0);
}
