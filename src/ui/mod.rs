pub mod server_tab;
pub mod user_tab;
pub mod security_tab;
pub mod service_tab;
pub mod log_tab;

use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, Notebook, Label};
use std::sync::{Arc, Mutex as StdMutex};
use wftpg::AppState;

pub fn build_ui(app: &Application) {
    let state = match AppState::new() {
        Ok(s) => Arc::new(StdMutex::new(s)),
        Err(e) => {
            eprintln!("Failed to initialize application state: {}", e);
            show_error_dialog(&format!("初始化失败: {}", e));
            return;
        }
    };

    if let Err(e) = ensure_default_directories(&state) {
        eprintln!("Failed to create default directories: {}", e);
    }

    let window = ApplicationWindow::builder()
        .application(app)
        .title("WFTPG - SFTP/FTP管理工具")
        .default_width(1000)
        .default_height(750)
        .build();

    let state_clone = Arc::clone(&state);
    window.connect_delete_event(move |_, _| {
        let state = state_clone.lock().unwrap();
        state.stop_all();
        gtk::glib::Propagation::Proceed
    });

    let notebook = Notebook::new();

    let server_box = server_tab::create(&state);
    notebook.append_page(&server_box, Some(&Label::new(Some("服务器配置"))));

    let user_box = user_tab::create(&state);
    notebook.append_page(&user_box, Some(&Label::new(Some("用户管理"))));

    let security_box = security_tab::create(&state);
    notebook.append_page(&security_box, Some(&Label::new(Some("安全设置"))));

    let service_box = service_tab::create(&state);
    notebook.append_page(&service_box, Some(&Label::new(Some("系统服务"))));

    let log_box = log_tab::create(&state);
    notebook.append_page(&log_box, Some(&Label::new(Some("日志查看"))));

    window.add(&notebook);
    window.show_all();
}

fn ensure_default_directories(state: &Arc<StdMutex<AppState>>) -> anyhow::Result<()> {
    let state_guard = state.lock().unwrap();
    let users = state_guard.user_manager.lock().unwrap();
    for (_, user) in users.list_users() {
        let share_dir = std::path::Path::new(&user.home_dir);
        if !share_dir.exists() {
            std::fs::create_dir_all(share_dir)?;
        }
    }
    Ok(())
}

fn show_error_dialog(message: &str) {
    let dialog = gtk::MessageDialog::new(
        None::<&gtk::Window>,
        gtk::DialogFlags::empty(),
        gtk::MessageType::Error,
        gtk::ButtonsType::Ok,
        message,
    );
    dialog.connect_response(|dialog, _| {
        dialog.close();
    });
    dialog.show();
}
