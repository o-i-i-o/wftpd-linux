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

    let window = ApplicationWindow::builder()
        .application(app)
        .title("WFTPG - SFTP/FTP管理工具")
        .default_width(1000)
        .default_height(750)
        .resizable(true)
        .decorated(true)
        .build();

    setup_window_for_uos(&window);

    let state_clone = Arc::clone(&state);
    window.connect_delete_event(move |_, _| {
        if let Ok(s) = state_clone.lock() {
            s.stop_all();
        }
        gtk::glib::Propagation::Proceed
    });

    let notebook = Notebook::new();
    notebook.set_tab_pos(gtk::PositionType::Top);
    notebook.set_scrollable(true);

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

    let state_for_dirs = Arc::clone(&state);
    gtk::glib::MainContext::default().spawn_local(async move {
        ensure_default_directories_async(&state_for_dirs).await;
    });
}

fn setup_window_for_uos(window: &ApplicationWindow) {
    set_window_icon(window);
    
    let display = gtk::gdk::Display::default();
    if let Some(display) = display {
        let monitor = display.primary_monitor();
        if let Some(monitor) = monitor {
            let geometry = monitor.geometry();
            let scale_factor = monitor.scale_factor();
            
            if scale_factor > 1 {
                let width = (geometry.width() as f32 * 0.7 / scale_factor as f32) as i32;
                let height = (geometry.height() as f32 * 0.7 / scale_factor as f32) as i32;
                window.set_default_size(width.min(1200), height.min(900));
            }
        }
    }
}

fn set_window_icon(window: &ApplicationWindow) {
    let icon_paths = [
        "/usr/share/icons/hicolor/scalable/apps/wftpg.svg",
        "/usr/share/icons/hicolor/256x256/apps/wftpg.png",
        "/usr/share/icons/hicolor/48x48/apps/wftpg.png",
        "/usr/share/pixmaps/wftpg.svg",
        "/usr/share/pixmaps/wftpg.png",
    ];

    for path in &icon_paths {
        let icon_path = std::path::Path::new(path);
        if icon_path.exists() {
            if let Ok(pixbuf) = gtk::gdk_pixbuf::Pixbuf::from_file(icon_path) {
                window.set_icon(Some(&pixbuf));
                return;
            }
        }
    }

    if let Some(icon_theme) = gtk::IconTheme::default() {
        for size in [48, 64, 128, 256] {
            if let Ok(Some(pixbuf)) = icon_theme.load_icon("wftpg", size, gtk::IconLookupFlags::empty()) {
                window.set_icon(Some(&pixbuf));
                return;
            }
        }
    }
}

async fn ensure_default_directories_async(state: &Arc<StdMutex<AppState>>) {
    let dirs_to_create: Vec<String> = {
        match state.try_lock() {
            Ok(s) => {
                match s.user_manager.try_lock() {
                    Ok(users) => {
                        let users_list = users.list_users();
                        users_list.into_iter()
                            .map(|(_, user)| user.home_dir.clone())
                            .collect()
                    }
                    Err(_) => return,
                }
            }
            Err(_) => return,
        }
    };

    for dir in dirs_to_create {
        let path = std::path::Path::new(&dir);
        if !path.exists() {
            if let Err(e) = std::fs::create_dir_all(path) {
                eprintln!("Failed to create directory {}: {}", dir, e);
            }
        }
    }
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
