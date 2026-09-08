use crate::AppState;
use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, Label, Notebook};
use std::sync::{Arc, Mutex as StdMutex};

use super::file_log_tab;
use super::log_tab;
use super::security_tab;
use super::server_tab;
use super::service_tab;
use super::user_tab;
use super::utils::setup_window_for_uos;

pub fn build_ui(app: &Application) {
    // 前端使用 new_for_gui()，不直接写入日志文件，通过 IPC 从 wftpd 获取日志
    let state = match AppState::new_for_gui() {
        Ok(s) => Arc::new(StdMutex::new(s)),
        Err(e) => {
            eprintln!("Failed to initialize application state: {e}");
            // 创建一个临时窗口来显示错误对话框
            let window = ApplicationWindow::builder()
                .application(app)
                .title("WFTPG - 错误")
                .default_width(400)
                .default_height(200)
                .build();
            window.present();

            let error_msg = format!("初始化失败: {e}");
            let dialog = gtk::MessageDialog::new(
                Some(&window),
                gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
                gtk::MessageType::Error,
                gtk::ButtonsType::Ok,
                &error_msg,
            );
            dialog.set_title("启动错误");
            dialog.run();
            dialog.close();
            window.close();
            return;
        }
    };

    let window = ApplicationWindow::builder()
        .application(app)
        .title("WFTPG - SFTP/FTP管理工具")
        .default_width(1000)
        .default_height(750)
        .resizable(true)
        .decorated(true)
        .build();

    setup_window_for_uos(&window);

    window.connect_delete_event(|_, _| {
        // GUI 关闭时不停止后台服务，仅退出界面
        // wftpd 服务由 systemd 管理，独立于 GUI 运行
        gtk::glib::Propagation::Proceed
    });

    let notebook = Notebook::new();
    notebook.set_tab_pos(gtk::PositionType::Top);
    notebook.set_scrollable(true);

    append_tab(&notebook, &server_tab::create(&state), "服务器配置");
    append_tab(&notebook, &user_tab::create(&state), "用户管理");
    append_tab(&notebook, &security_tab::create(&state), "安全设置");
    append_tab(&notebook, &service_tab::create(&state), "系统服务");
    append_tab(&notebook, &log_tab::create(&state), "日志查看");
    append_tab(&notebook, &file_log_tab::create(&state), "文件操作日志");

    window.add(&notebook);
    window.show_all();

    let state_for_dirs = Arc::clone(&state);
    gtk::glib::MainContext::default().spawn_local(async move {
        ensure_default_directories(&state_for_dirs);
    });
}

fn append_tab<P: gtk::prelude::IsA<gtk::Widget>>(notebook: &Notebook, page: &P, label: &str) {
    let tab_label = Label::new(Some(label));
    tab_label.set_markup(&format!("<span size='large'>{label}</span>"));
    notebook.append_page(page, Some(&tab_label));
}

fn ensure_default_directories(state: &Arc<StdMutex<AppState>>) {
    let dirs_to_create: Vec<String> = {
        match state.try_lock() {
            Ok(s) => match s.user_manager.try_lock() {
                Ok(users) => {
                    let users_list = users.list_users();
                    users_list
                        .into_iter()
                        .map(|(_, user)| user.home_dir.clone())
                        .collect()
                }
                Err(_) => return,
            },
            Err(_) => return,
        }
    };

    for dir in dirs_to_create {
        let path = std::path::Path::new(&dir);
        if !path.exists()
            && let Err(e) = std::fs::create_dir_all(path)
        {
            eprintln!("Failed to create directory {dir}: {e}");
        }
    }
}
