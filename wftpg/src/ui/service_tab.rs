//! 系统服务页：FTP/SFTP 服务控制（gRPC）与后端守护进程管理（systemctl --user）。

use gtk::prelude::*;
use gtk::{Box, Button, Frame, Label, Orientation, glib};
use std::sync::{Arc, Mutex as StdMutex};
use wftpd_proto::wftpd::v1::service_selector::Which;

use crate::AppState;
use crate::communication;

pub fn create(_state: &Arc<StdMutex<AppState>>) -> Box {
    let container = Box::new(Orientation::Vertical, 10);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);

    // ---- 状态区 ----
    let status_frame = Frame::new(Some("服务状态"));
    let status_box = Box::new(Orientation::Vertical, 6);
    status_box.set_margin_top(8);
    status_box.set_margin_bottom(8);
    status_box.set_margin_start(8);
    status_box.set_margin_end(8);

    let ftp_status = Label::new(Some("FTP: 未知"));
    let sftp_status = Label::new(Some("SFTP: 未知"));
    let version_status = Label::new(None);
    ftp_status.set_halign(gtk::Align::Start);
    sftp_status.set_halign(gtk::Align::Start);
    version_status.set_halign(gtk::Align::Start);

    status_box.pack_start(&ftp_status, false, false, 0);
    status_box.pack_start(&sftp_status, false, false, 0);
    status_box.pack_start(&version_status, false, false, 0);

    let refresh_button = Button::with_label("刷新状态");
    {
        let ftp_status = ftp_status.clone();
        let sftp_status = sftp_status.clone();
        let version_status = version_status.clone();
        refresh_button.connect_clicked(move |_| {
            refresh_status(
                ftp_status.clone(),
                sftp_status.clone(),
                version_status.clone(),
            );
        });
    }
    status_box.pack_start(&refresh_button, false, false, 0);
    status_frame.add(&status_box);
    container.pack_start(&status_frame, false, false, 0);

    // ---- FTP / SFTP 控制 ----
    let services_frame = Frame::new(Some("协议服务控制（通过后端 gRPC 接口）"));
    let services_box = Box::new(Orientation::Vertical, 6);
    services_box.set_margin_top(8);
    services_box.set_margin_bottom(8);
    services_box.set_margin_start(8);
    services_box.set_margin_end(8);

    for (name, which) in [("FTP", Which::Ftp), ("SFTP", Which::Sftp)] {
        let row = Box::new(Orientation::Horizontal, 6);
        let label = Label::new(Some(name));
        label.set_halign(gtk::Align::Start);
        row.pack_start(&label, true, true, 0);

        for (text, action) in [("启动", "start"), ("停止", "stop"), ("重启", "restart")] {
            let button = Button::with_label(text);
            let action = action.to_string();
            button.connect_clicked(move |_| run_service_action(which, &action));
            row.pack_start(&button, false, false, 0);
        }

        services_box.pack_start(&row, false, false, 0);
    }
    services_frame.add(&services_box);
    container.pack_start(&services_frame, false, false, 0);

    // ---- 后端守护进程管理 ----
    let daemon_frame = Frame::new(Some("后端守护进程（systemd 用户服务）"));
    let daemon_box = Box::new(Orientation::Vertical, 6);
    daemon_box.set_margin_top(8);
    daemon_box.set_margin_bottom(8);
    daemon_box.set_margin_start(8);
    daemon_box.set_margin_end(8);

    let daemon_row = Box::new(Orientation::Horizontal, 6);
    for (text, verb) in [("启动", "start"), ("停止", "stop"), ("重启", "restart")] {
        let button = Button::with_label(text);
        let verb = verb.to_string();
        button.connect_clicked(move |_| run_systemctl(&verb));
        daemon_row.pack_start(&button, false, false, 0);
    }
    daemon_box.pack_start(&daemon_row, false, false, 0);

    let hint = Label::new(None);
    hint.set_markup(
        "wftpd 以当前桌面用户的 systemd 服务运行，无需 root 权限：\n\
         • 启动：<tt>systemctl --user start wftpd</tt>\n\
         • 停止：<tt>systemctl --user stop wftpd</tt>\n\
         • 开机自启：<tt>systemctl --user enable wftpd</tt>\n\
         • 查看状态：<tt>systemctl --user status wftpd</tt>",
    );
    hint.set_halign(gtk::Align::Start);
    daemon_box.pack_start(&hint, false, false, 0);
    daemon_frame.add(&daemon_box);
    container.pack_start(&daemon_frame, false, false, 0);

    // 初始加载状态
    refresh_status(ftp_status, sftp_status, version_status);

    container
}

fn refresh_status(ftp_status: Label, sftp_status: Label, version_status: Label) {
    use std::sync::mpsc;

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(communication::get_status());
    });

    // glib 主循环轮询后台线程结果（与 log_tab 相同的模式）
    glib_poll(move || {
        match rx.try_recv() {
            Ok(result) => match result {
                Ok(status) => {
                    set_state_label(&ftp_status, "FTP", status.ftp_running);
                    set_state_label(&sftp_status, "SFTP", status.sftp_running);
                    version_status.set_text(&format!("后端版本: wftpd v{}", status.version));
                }
                Err(e) => {
                    ftp_status.set_markup("<b>FTP:</b> <span foreground='red'>后端未连接</span>");
                    sftp_status.set_markup("<b>SFTP:</b> <span foreground='red'>后端未连接</span>");
                    version_status.set_text(&format!("错误: {e}"));
                }
            },
            Err(_) => return glib::ControlFlow::Continue,
        }
        glib::ControlFlow::Break
    });
}

/// 在 glib 主循环上以 100ms 间隔轮询闭包；闭包返回 `ControlFlow::Break` 时自动停止
fn glib_poll<F: FnMut() -> glib::ControlFlow + 'static>(f: F) {
    let f = std::cell::RefCell::new(f);
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        (f.borrow_mut())()
    });
}

fn set_state_label(label: &Label, name: &str, running: bool) {
    if running {
        label.set_markup(&format!(
            "<b>{name}:</b> <span foreground='green'>● 运行中</span>"
        ));
    } else {
        label.set_markup(&format!(
            "<b>{name}:</b> <span foreground='gray'>○ 已停止</span>"
        ));
    }
}

fn run_service_action(which: Which, action: &str) {
    use std::sync::mpsc;

    let action = action.to_string();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = match action.as_str() {
            "start" => communication::start_service(which),
            "stop" => communication::stop_service(which),
            _ => communication::restart_service(which),
        };
        let _ = tx.send(result);
    });

    glib_poll(move || match rx.try_recv() {
        Ok(Err(e)) => {
            show_error_dialog(&format!("操作失败: {e}"));
            glib::ControlFlow::Break
        }
        Ok(Ok(())) => glib::ControlFlow::Break,
        Err(_) => glib::ControlFlow::Continue,
    });
}

fn run_systemctl(verb: &str) {
    use std::sync::mpsc;

    let verb = verb.to_string();
    let verb_msg = verb.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let status = std::process::Command::new("systemctl")
            .args(["--user", &verb, "wftpd"])
            .status();
        let _ = tx.send(status);
    });

    glib_poll(move || match rx.try_recv() {
        Ok(Ok(s)) => {
            if !s.success() {
                show_error_dialog(&format!("systemctl --user {verb_msg} wftpd 失败: {s}"));
            }
            glib::ControlFlow::Break
        }
        Ok(Err(e)) => {
            show_error_dialog(&format!("无法执行 systemctl: {e}"));
            glib::ControlFlow::Break
        }
        Err(_) => glib::ControlFlow::Continue,
    });
}

fn show_error_dialog(message: &str) {
    let dialog = gtk::MessageDialog::new(
        None::<&gtk::Window>,
        gtk::DialogFlags::MODAL,
        gtk::MessageType::Error,
        gtk::ButtonsType::Ok,
        message,
    );
    dialog.run();
    dialog.close();
}
