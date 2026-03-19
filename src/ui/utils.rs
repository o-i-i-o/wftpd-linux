use gtk::prelude::*;
use gtk::ApplicationWindow;

pub fn setup_window_for_uos(window: &ApplicationWindow) {
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

pub fn set_window_icon(window: &ApplicationWindow) {
    let icon_paths = [
        "/usr/share/icons/hicolor/scalable/apps/wftpg.svg",
        "/usr/share/icons/hicolor/256x256/apps/wftpg.png",
        "/usr/share/icons/hicolor/48x48/apps/wftpg.png",
        "/usr/share/pixmaps/wftpg.svg",
        "/usr/share/pixmaps/wftpg.png",
    ];

    for path in &icon_paths {
        let icon_path = std::path::Path::new(path);
        if icon_path.exists()
            && let Ok(pixbuf) = gtk::gdk_pixbuf::Pixbuf::from_file(icon_path) {
                window.set_icon(Some(&pixbuf));
                return;
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

pub fn show_error_dialog(message: &str) {
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
