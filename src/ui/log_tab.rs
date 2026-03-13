use gtk::prelude::*;
use gtk::{
    Box, Orientation, Label, Button, TextView, ScrolledWindow, Entry, Frame, SpinButton,
    Adjustment, CheckButton, ComboBoxText, glib,
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

    create_log_config_frame(&container, state);

    let (refresh_btn, clear_btn, auto_refresh_cb) = create_control_buttons(&container);
    let text_view = create_log_view(&container, state);

    setup_button_handlers(state, &text_view, &refresh_btn, &clear_btn, &auto_refresh_cb);

    container
}

fn create_log_config_frame(container: &Box, state: &Arc<StdMutex<AppState>>) {
    let frame = Frame::new(Some("日志配置"));
    let box_ = Box::new(Orientation::Vertical, 5);
    box_.set_margin_top(10);
    box_.set_margin_bottom(10);
    box_.set_margin_start(10);
    box_.set_margin_end(10);

    let row1 = Box::new(Orientation::Horizontal, 5);
    row1.pack_start(&Label::new(Some("日志目录:")), false, false, 0);
    let log_dir_entry = Entry::new();
    log_dir_entry.set_hexpand(true);
    if let Ok(s) = state.try_lock() {
        if let Ok(config) = s.config.try_lock() {
            log_dir_entry.set_text(&config.logging.log_dir);
        }
    }
    row1.pack_start(&log_dir_entry, true, true, 0);
    box_.pack_start(&row1, false, false, 0);

    let row2 = Box::new(Orientation::Horizontal, 5);
    row2.pack_start(&Label::new(Some("日志级别:")), false, false, 0);
    let log_level_combo = ComboBoxText::new();
    log_level_combo.append(Some("debug"), "Debug");
    log_level_combo.append(Some("info"), "Info");
    log_level_combo.append(Some("warn"), "Warning");
    log_level_combo.append(Some("error"), "Error");
    if let Ok(s) = state.try_lock() {
        if let Ok(config) = s.config.try_lock() {
            let level = config.logging.log_level.as_str();
            let id = match level {
                "debug" => Some("debug"),
                "warn" => Some("warn"),
                "error" => Some("error"),
                _ => Some("info"),
            };
            log_level_combo.set_active_id(id);
        }
    }
    row2.pack_start(&log_level_combo, false, false, 0);

    row2.pack_start(&Label::new(Some("最大文件大小(MB):")), false, false, 0);
    let max_size_spin = create_spin_button(1.0, 1000.0, 1.0);
    if let Ok(s) = state.try_lock() {
        if let Ok(config) = s.config.try_lock() {
            max_size_spin.set_value((config.logging.max_log_size / (1024 * 1024)) as f64);
        }
    }
    row2.pack_start(&max_size_spin, false, false, 0);

    row2.pack_start(&Label::new(Some("最大文件数:")), false, false, 0);
    let max_files_spin = create_spin_button(1.0, 100.0, 1.0);
    if let Ok(s) = state.try_lock() {
        if let Ok(config) = s.config.try_lock() {
            max_files_spin.set_value(config.logging.max_log_files as f64);
        }
    }
    row2.pack_start(&max_files_spin, false, false, 0);
    box_.pack_start(&row2, false, false, 0);

    let row3 = Box::new(Orientation::Horizontal, 5);
    let log_to_file_cb = CheckButton::with_label("记录到文件");
    if let Ok(s) = state.try_lock() {
        if let Ok(config) = s.config.try_lock() {
            log_to_file_cb.set_active(config.logging.log_to_file);
        }
    }
    row3.pack_start(&log_to_file_cb, false, false, 0);

    let log_to_gui_cb = CheckButton::with_label("显示在界面");
    if let Ok(s) = state.try_lock() {
        if let Ok(config) = s.config.try_lock() {
            log_to_gui_cb.set_active(config.logging.log_to_gui);
        }
    }
    row3.pack_start(&log_to_gui_cb, false, false, 0);
    box_.pack_start(&row3, false, false, 0);

    let save_btn = Button::with_label("保存日志配置");
    let state_clone = Arc::clone(state);
    let log_dir_clone = log_dir_entry.clone();
    let log_level_clone = log_level_combo.clone();
    let max_size_clone = max_size_spin.clone();
    let max_files_clone = max_files_spin.clone();
    let log_to_file_clone = log_to_file_cb.clone();
    let log_to_gui_clone = log_to_gui_cb.clone();
    save_btn.connect_clicked(
        clone!(@strong state_clone, @strong log_dir_clone, @strong log_level_clone,
               @strong max_size_clone, @strong max_files_clone, @strong log_to_file_clone,
               @strong log_to_gui_clone => move |_| {
            let state = Arc::clone(&state_clone);
            let log_dir = log_dir_clone.text().to_string();
            let log_level = log_level_clone.active_id()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "info".to_string());
            let max_size = (max_size_clone.value() as u64) * 1024 * 1024;
            let max_files = max_files_clone.value() as usize;
            let log_to_file = log_to_file_clone.is_active();
            let log_to_gui = log_to_gui_clone.is_active();
            
            glib::MainContext::ref_thread_default().spawn_local(async move {
                if let Ok(s) = state.try_lock() {
                    if let Ok(mut config) = s.config.try_lock() {
                        config.logging.log_dir = log_dir;
                        config.logging.log_level = log_level;
                        config.logging.max_log_size = max_size;
                        config.logging.max_log_files = max_files;
                        config.logging.log_to_file = log_to_file;
                        config.logging.log_to_gui = log_to_gui;
                        let _ = config.save(&s.config_path);
                        log::info!("Logging configuration saved");
                    }
                }
            });
        }),
    );
    box_.pack_start(&save_btn, false, false, 0);

    frame.add(&box_);
    container.pack_start(&frame, false, false, 0);
}

fn create_control_buttons(container: &Box) -> (Button, Button, CheckButton) {
    let control_box = Box::new(Orientation::Horizontal, 10);

    let refresh_btn = Button::with_label("刷新日志");
    let clear_btn = Button::with_label("清空显示");
    let auto_refresh_cb = CheckButton::with_label("自动刷新");
    auto_refresh_cb.set_active(true);

    control_box.pack_start(&refresh_btn, false, false, 0);
    control_box.pack_start(&clear_btn, false, false, 0);
    control_box.pack_start(&auto_refresh_cb, false, false, 0);
    container.pack_start(&control_box, false, false, 0);

    (refresh_btn, clear_btn, auto_refresh_cb)
}

fn create_log_view(container: &Box, state: &Arc<StdMutex<AppState>>) -> TextView {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();

    let text_view = TextView::new();
    text_view.set_editable(false);
    text_view.set_monospace(true);
    text_view.set_left_margin(10);
    text_view.set_right_margin(10);
    text_view.set_top_margin(10);
    text_view.set_bottom_margin(10);

    populate_log_view(&text_view, state);

    scrolled.add(&text_view);
    container.pack_start(&scrolled, true, true, 0);

    text_view
}

fn format_log_entries(state: &Arc<StdMutex<AppState>>) -> String {
    if let Ok(s) = state.try_lock() {
        if let Ok(logger) = s.logger.try_lock() {
            let entries = logger.get_recent_logs(100);
            
            if entries.is_empty() {
                return "暂无日志记录".to_string();
            }
            
            let mut text = String::with_capacity(entries.len() * 100);
            for entry in entries {
                text.push_str(&format!(
                    "[{}] {} - {} - {} - {}\n",
                    entry.timestamp.format("%Y-%m-%d %H:%M:%S"),
                    entry.level,
                    entry.source,
                    entry.message,
                    entry.action.unwrap_or_default()
                ));
            }
            return text;
        }
    }
    "无法获取日志".to_string()
}

fn populate_log_view(text_view: &TextView, state: &Arc<StdMutex<AppState>>) {
    let text = format_log_entries(state);
    if let Some(buffer) = text_view.buffer() {
        buffer.set_text(&text);
    }
}

fn setup_button_handlers(
    state: &Arc<StdMutex<AppState>>,
    text_view: &TextView,
    refresh_btn: &Button,
    clear_btn: &Button,
    auto_refresh_cb: &CheckButton,
) {
    let state_clone = Arc::clone(state);
    let text_view_clone = text_view.clone();
    refresh_btn.connect_clicked(clone!(@strong state_clone, @strong text_view_clone => move |_| {
        let state = Arc::clone(&state_clone);
        let text_view = text_view_clone.clone();
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            let text = format_log_entries(&state);
            if let Some(buffer) = text_view.buffer() {
                buffer.set_text(&text);
            }
        });
    }));

    let text_view_clone = text_view.clone();
    clear_btn.connect_clicked(clone!(@strong text_view_clone => move |_| {
        if let Some(buffer) = text_view_clone.buffer() {
            buffer.set_text("");
        }
    }));

    let state_clone = Arc::clone(state);
    let text_view_clone = text_view.clone();
    let auto_refresh_cb_clone = auto_refresh_cb.clone();
    
    glib::timeout_add_seconds_local(2, move || {
        if auto_refresh_cb_clone.is_active() {
            let text = format_log_entries(&state_clone);
            if let Some(buffer) = text_view_clone.buffer() {
                buffer.set_text(&text);
            }
        }
        glib::ControlFlow::Continue
    });
}

fn create_spin_button(min: f64, max: f64, step: f64) -> SpinButton {
    let adjustment = Adjustment::new(min, min, max, step, step * 10.0, 0.0);
    SpinButton::builder()
        .adjustment(&adjustment)
        .digits(0)
        .width_chars(6)
        .build()
}