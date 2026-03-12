mod ui;

use gtk::prelude::*;
use gtk::Application;
use std::env;
use std::path::Path;

fn main() {
    if let Err(e) = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).try_init() {
        eprintln!("Failed to initialize logger: {}", e);
    }

    if unsafe { libc::getuid() } == 0 {
        setup_display_environment();
    }

    let app = Application::builder()
        .application_id("com.wftpg.app")
        .build();

    app.connect_activate(ui::build_ui);

    let args: Vec<String> = std::env::args().collect();
    let args_str: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    app.run_with_args(&args_str);
}

fn setup_display_environment() {
    if env::var("DISPLAY").is_err() {
        env::set_var("DISPLAY", ":0");
    }

    if env::var("XAUTHORITY").is_err() {
        if let Ok(home_dirs) = std::fs::read_dir("/home") {
            for entry in home_dirs.flatten() {
                let xauth_path = entry.path().join(".Xauthority");
                if xauth_path.exists() {
                    env::set_var("XAUTHORITY", xauth_path);
                    break;
                }
            }
        }
    }

    if env::var("XDG_RUNTIME_DIR").is_err() {
        if let Ok(entries) = std::fs::read_dir("/run/user") {
            for entry in entries.flatten() {
                let runtime_dir = entry.path();
                if runtime_dir.exists() {
                    env::set_var("XDG_RUNTIME_DIR", runtime_dir);
                    break;
                }
            }
        }
    }

    if env::var("WAYLAND_DISPLAY").is_err()
        && Path::new("/run/user/1000/wayland-0").exists()
    {
        env::set_var("WAYLAND_DISPLAY", "wayland-0");
    }

    if env::var("DBUS_SESSION_BUS_ADDRESS").is_err() {
        if let Ok(entries) = std::fs::read_dir("/run/user") {
            for entry in entries.flatten() {
                let dbus_path = entry.path().join("bus");
                if dbus_path.exists() {
                    env::set_var(
                        "DBUS_SESSION_BUS_ADDRESS",
                        format!("unix:path={}", dbus_path.display()),
                    );
                    break;
                }
            }
        }
    }
}
