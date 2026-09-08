//! 系统服务页：FTP/SFTP 服务控制（gRPC）与后端守护进程管理（systemctl --user）。

use gtk::prelude::*;
use gtk::{Box, Button, Frame, Label, Orientation, glib};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use wftpd_proto::wftpd::v1::service_selector::Which;

use crate::AppState;
use crate::communication;

/// 状态自动刷新间隔（秒）：同步外部引起的状态变化
/// （终端执行 systemctl、服务异常退出、其他客户端操作等）
const AUTO_REFRESH_SECS: u32 = 2;

/// 状态区标签集合；GTK 控件为引用计数对象，clone 后仍指向同一控件
#[derive(Clone)]
struct StatusLabels {
    ftp: Label,
    sftp: Label,
    version: Label,
}

pub fn create(_state: &Arc<StdMutex<AppState>>) -> Box {
    let container = Box::new(Orientation::Vertical, 10);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);

    // 标记一次状态刷新是否在途，避免周期刷新与手动刷新叠加请求
    let refreshing = Arc::new(AtomicBool::new(false));

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

    let labels = StatusLabels {
        ftp: ftp_status,
        sftp: sftp_status,
        version: version_status,
    };

    let refresh_button = Button::with_label("刷新状态");
    {
        let labels = labels.clone();
        let refreshing = Arc::clone(&refreshing);
        refresh_button.connect_clicked(move |_| {
            refresh_status(&labels, &refreshing);
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
            let labels = labels.clone();
            let refreshing = Arc::clone(&refreshing);
            button
                .connect_clicked(move |_| run_service_action(which, &action, &labels, &refreshing));
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
        let labels = labels.clone();
        let refreshing = Arc::clone(&refreshing);
        button.connect_clicked(move |_| run_systemctl(&verb, &labels, &refreshing));
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

    // 周期自动刷新：状态变化（无论来源）都会同步到界面
    {
        let labels = labels.clone();
        let refreshing = Arc::clone(&refreshing);
        glib::timeout_add_seconds_local(AUTO_REFRESH_SECS, move || {
            refresh_status(&labels, &refreshing);
            glib::ControlFlow::Continue
        });
    }

    // 初始加载状态
    refresh_status(&labels, &refreshing);

    container
}

/// 查询后端状态并更新标签；同一时刻只允许一次刷新在途，避免请求堆积
fn refresh_status(labels: &StatusLabels, refreshing: &Arc<AtomicBool>) {
    use std::sync::mpsc;

    if refreshing.swap(true, Ordering::SeqCst) {
        return; // 上一次刷新尚未完成，跳过本次
    }

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(communication::get_status());
    });

    // glib 主循环轮询后台线程结果（与 log_tab 相同的模式）
    let labels = labels.clone();
    let refreshing = Arc::clone(refreshing);
    glib_poll(move || {
        let done = match rx.try_recv() {
            Ok(Ok(status)) => {
                set_state_label(&labels.ftp, "FTP", status.ftp_running);
                set_state_label(&labels.sftp, "SFTP", status.sftp_running);
                labels
                    .version
                    .set_text(&format!("后端版本: wftpd v{}", status.version));
                true
            }
            Ok(Err(e)) => {
                labels
                    .ftp
                    .set_markup("<b>FTP:</b> <span foreground='red'>后端未连接</span>");
                labels
                    .sftp
                    .set_markup("<b>SFTP:</b> <span foreground='red'>后端未连接</span>");
                labels.version.set_text(&format!("错误: {e}"));
                true
            }
            // 查询线程异常终止：结束本轮轮询，等待下一次刷新
            Err(std::sync::mpsc::TryRecvError::Disconnected) => true,
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
        };
        if done {
            refreshing.store(false, Ordering::SeqCst);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
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

fn run_service_action(
    which: Which,
    action: &str,
    labels: &StatusLabels,
    refreshing: &Arc<AtomicBool>,
) {
    use std::sync::mpsc;

    let action = action.to_string();
    let labels = labels.clone();
    let refreshing = Arc::clone(refreshing);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = match action.as_str() {
            "start" => communication::start_service(which),
            "stop" => communication::stop_service(which),
            _ => communication::restart_service(which),
        };
        let _ = tx.send(result);
    });

    // 操作结束后（无论成败）立即刷新状态，保证显示与后端一致
    glib_poll(move || match rx.try_recv() {
        Ok(Err(e)) => {
            show_error_dialog(&format!("操作失败: {e}"));
            refresh_status(&labels, &refreshing);
            glib::ControlFlow::Break
        }
        Ok(Ok(())) => {
            refresh_status(&labels, &refreshing);
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Disconnected) => {
            show_error_dialog("服务操作线程异常终止");
            refresh_status(&labels, &refreshing);
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
    });
}

fn run_systemctl(verb: &str, labels: &StatusLabels, refreshing: &Arc<AtomicBool>) {
    use std::sync::mpsc;

    let verb = verb.to_string();
    let verb_msg = verb.clone();
    let labels = labels.clone();
    let refreshing = Arc::clone(refreshing);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let status = std::process::Command::new("systemctl")
            .args(["--user", &verb, "wftpd"])
            .status();
        let _ = tx.send(status);
    });

    // systemctl 返回后立即刷新；守护进程刚启动时套接字可能尚未就绪，
    // 此时显示"后端未连接"，由周期自动刷新在数秒内纠正
    glib_poll(move || match rx.try_recv() {
        Ok(Ok(s)) => {
            if !s.success() {
                show_error_dialog(&format!("systemctl --user {verb_msg} wftpd 失败: {s}"));
            }
            refresh_status(&labels, &refreshing);
            glib::ControlFlow::Break
        }
        Ok(Err(e)) => {
            show_error_dialog(&format!("无法执行 systemctl: {e}"));
            refresh_status(&labels, &refreshing);
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Disconnected) => {
            refresh_status(&labels, &refreshing);
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
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
