mod ui;

use gtk::prelude::*;
use gtk::{Application, Settings};
use std::env;

fn main() {
    if let Err(e) = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).try_init() {
        eprintln!("Failed to initialize logger: {}", e);
    }

    let app = Application::builder()
        .application_id("com.wftpg.app")
        .build();

    app.connect_startup(|app| {
        setup_gtk_settings(app);
    });

    app.connect_activate(ui::build_ui);

    let args: Vec<String> = std::env::args().collect();
    let args_str: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    app.run_with_args(&args_str);
}

fn setup_gtk_settings(app: &Application) {
    if let Some(settings) = Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(false);
        settings.set_gtk_theme_name(Some("deepin"));
        
        #[cfg(target_arch = "aarch64")]
        {
            settings.set_gtk_xft_antialias(1);
            settings.set_gtk_xft_hinting(1);
            settings.set_gtk_xft_rgba(Some("rgb"));
            settings.set_gtk_xft_hintstyle(Some("hintslight"));
        }
    }
    
    env::set_var("GTK_CSD", "0");
    
    let _ = app;
}
