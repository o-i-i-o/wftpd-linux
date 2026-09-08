use crate::AppState;
use gtk::glib::clone;
use gtk::prelude::*;
use gtk::{
    Adjustment, Box, Button, CellRendererText, CheckButton, ComboBoxText, Entry, Frame, Label,
    ListStore, Orientation, ScrolledWindow, SpinButton, TreeView, TreeViewColumn, glib,
};
use std::fs;
use std::sync::{Arc, Mutex as StdMutex};
use tracing::info;

pub fn create(state: &Arc<StdMutex<AppState>>) -> Box {
    let container = Box::new(Orientation::Vertical, 10);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);

    create_log_config_frame(&container, state);

    let (refresh_btn, clear_btn, auto_refresh_cb, log_file_combo) =
        create_control_buttons(&container, state);
    let tree_view = create_log_view(&container);

    setup_button_handlers(
        &tree_view,
        &refresh_btn,
        &clear_btn,
        &auto_refresh_cb,
        &log_file_combo,
    );

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
    if let Ok(s) = state.try_lock()
        && let Ok(config) = s.config.try_lock()
    {
        log_dir_entry.set_text(&config.logging.log_dir);
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
    if let Ok(s) = state.try_lock()
        && let Ok(config) = s.config.try_lock()
    {
        let level = config.logging.log_level.as_str();
        let id = match level {
            "debug" => Some("debug"),
            "warn" => Some("warn"),
            "error" => Some("error"),
            _ => Some("info"),
        };
        log_level_combo.set_active_id(id);
    }
    row2.pack_start(&log_level_combo, false, false, 0);

    row2.pack_start(&Label::new(Some("最大文件大小(MB):")), false, false, 0);
    let max_size_spin = create_spin_button(1.0, 1000.0, 1.0);
    if let Ok(s) = state.try_lock()
        && let Ok(config) = s.config.try_lock()
    {
        max_size_spin.set_value(super::utils::spin_f64_from(
            config.logging.max_log_size / (1024 * 1024),
        ));
    }
    row2.pack_start(&max_size_spin, false, false, 0);

    row2.pack_start(&Label::new(Some("最大文件数:")), false, false, 0);
    let max_files_spin = create_spin_button(1.0, 100.0, 1.0);
    if let Ok(s) = state.try_lock()
        && let Ok(config) = s.config.try_lock()
    {
        max_files_spin.set_value(super::utils::spin_f64_from(
            u64::try_from(config.logging.max_log_files).unwrap_or(u64::MAX),
        ));
    }
    row2.pack_start(&max_files_spin, false, false, 0);
    box_.pack_start(&row2, false, false, 0);

    let row3 = Box::new(Orientation::Horizontal, 5);
    let log_to_gui_cb = CheckButton::with_label("启用前端日志显示");
    if let Ok(s) = state.try_lock()
        && let Ok(config) = s.config.try_lock()
    {
        log_to_gui_cb.set_active(config.logging.enable_gui_logging);
    }
    row3.pack_start(&log_to_gui_cb, false, false, 0);
    box_.pack_start(&row3, false, false, 0);

    let save_btn = Button::with_label("保存");
    let state_clone = Arc::clone(state);
    let log_dir_clone = log_dir_entry.clone();
    let log_level_clone = log_level_combo.clone();
    let max_size_clone = max_size_spin.clone();
    let max_files_clone = max_files_spin.clone();
    let enable_gui_clone = log_to_gui_cb.clone();
    save_btn.connect_clicked(
        clone!(@strong state_clone, @strong log_dir_clone, @strong log_level_clone,
               @strong max_size_clone, @strong max_files_clone, @strong enable_gui_clone => move |_| {
            let state = Arc::clone(&state_clone);
            let log_dir = log_dir_clone.text().to_string();
            let log_level = log_level_clone.active_id().map_or_else(|| "info".to_string(), |s| s.to_string());
            let max_size = super::utils::spin_u64(max_size_clone.value()) * 1024 * 1024;
            let max_files = super::utils::spin_usize(max_files_clone.value());
            let enable_gui = enable_gui_clone.is_active();

            glib::MainContext::ref_thread_default().spawn_local(async move {
                if let Ok(s) = state.try_lock()
                    && let Ok(mut config) = s.config.try_lock() {
                        config.logging.log_dir = log_dir;
                        config.logging.log_level = log_level;
                        config.logging.max_log_size = max_size;
                        config.logging.max_log_files = max_files;
                        config.logging.enable_gui_logging = enable_gui;
                        let _ = config.save(&wftpd_common::Config::get_config_path());
                        info!("Logging configuration saved");
                    }
            });
        }),
    );
    box_.pack_start(&save_btn, false, false, 0);

    frame.add(&box_);
    container.pack_start(&frame, false, false, 0);
}

fn create_control_buttons(
    container: &Box,
    state: &Arc<StdMutex<AppState>>,
) -> (Button, Button, CheckButton, ComboBoxText) {
    let control_box = Box::new(Orientation::Horizontal, 10);

    control_box.pack_start(&Label::new(Some("日志文件:")), false, false, 0);

    let log_file_combo = ComboBoxText::new();
    log_file_combo.set_hexpand(true);
    populate_log_files(&log_file_combo, state);
    control_box.pack_start(&log_file_combo, true, true, 0);

    let refresh_btn = Button::with_label("刷新");
    let clear_btn = Button::with_label("清空显示");
    let auto_refresh_cb = CheckButton::with_label("自动刷新");
    auto_refresh_cb.set_active(true);

    control_box.pack_start(&refresh_btn, false, false, 0);
    control_box.pack_start(&clear_btn, false, false, 0);
    control_box.pack_start(&auto_refresh_cb, false, false, 0);
    container.pack_start(&control_box, false, false, 0);

    (refresh_btn, clear_btn, auto_refresh_cb, log_file_combo)
}

fn populate_log_files(combo: &ComboBoxText, state: &Arc<StdMutex<AppState>>) {
    combo.remove_all();

    let log_dir = if let Ok(s) = state.try_lock() {
        if let Ok(config) = s.config.try_lock() {
            config.logging.log_dir.clone()
        } else {
            "/var/log/wftpg".to_string()
        }
    } else {
        "/var/log/wftpg".to_string()
    };

    combo.append(Some("current"), "当前日志 (内存缓冲)");

    if let Ok(entries) = fs::read_dir(&log_dir) {
        let mut log_files: Vec<(String, String)> = entries
            .filter_map(std::result::Result::ok)
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                name.starts_with("wftpg-") && name.ends_with(".log")
            })
            .map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                let path = e.path().to_string_lossy().to_string();
                (name, path)
            })
            .collect();

        log_files.sort_by(|a, b| b.0.cmp(&a.0));

        for (name, path) in log_files {
            combo.append(Some(&path), &name);
        }
    }

    combo.set_active_id(Some("current"));
}

fn create_log_view(container: &Box) -> TreeView {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();

    let store = ListStore::new(&[
        gtk::glib::Type::STRING,
        gtk::glib::Type::STRING,
        gtk::glib::Type::STRING,
        gtk::glib::Type::STRING,
        gtk::glib::Type::STRING,
        gtk::glib::Type::STRING,
    ]);

    populate_log_store(&store, "current");

    let tree = TreeView::with_model(&store);

    let col_time = TreeViewColumn::new();
    col_time.set_title("时间");
    col_time.set_resizable(true);
    col_time.set_min_width(150);
    let renderer_time = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_time, &renderer_time, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_time, &renderer_time, "text", 0);
    tree.append_column(&col_time);

    let col_level = TreeViewColumn::new();
    col_level.set_title("级别");
    col_level.set_resizable(true);
    col_level.set_min_width(60);
    let renderer_level = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_level, &renderer_level, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_level, &renderer_level, "text", 1);
    tree.append_column(&col_level);

    let col_source = TreeViewColumn::new();
    col_source.set_title("来源");
    col_source.set_resizable(true);
    col_source.set_min_width(80);
    let renderer_source = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_source, &renderer_source, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_source, &renderer_source, "text", 2);
    tree.append_column(&col_source);

    let col_message = TreeViewColumn::new();
    col_message.set_title("消息");
    col_message.set_resizable(true);
    col_message.set_min_width(300);
    let renderer_message = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_message, &renderer_message, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_message, &renderer_message, "text", 3);
    tree.append_column(&col_message);

    let col_client = TreeViewColumn::new();
    col_client.set_title("客户端IP");
    col_client.set_resizable(true);
    col_client.set_min_width(120);
    let renderer_client = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_client, &renderer_client, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_client, &renderer_client, "text", 4);
    tree.append_column(&col_client);

    let col_action = TreeViewColumn::new();
    col_action.set_title("操作");
    col_action.set_resizable(true);
    col_action.set_min_width(80);
    let renderer_action = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_action, &renderer_action, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_action, &renderer_action, "text", 5);
    tree.append_column(&col_action);

    scrolled.add(&tree);
    container.pack_start(&scrolled, true, true, 0);

    tree
}

fn populate_log_store(store: &ListStore, source: &str) {
    use std::sync::mpsc;

    store.clear();

    // 显示加载中提示
    let loading_iter = store.append();
    store.set_value(&loading_iter, 3, &"正在加载日志...".to_value());

    let source_string = source.to_string();

    // 创建 channel 用于在线程间通信
    let (tx, rx) = mpsc::channel();

    // 在新线程中执行阻塞的 IPC 调用
    std::thread::spawn(move || {
        let result: Result<Vec<_>, _> = if source_string == "current" {
            crate::communication::client::get_logs(500)
        } else {
            crate::communication::client::get_log_file_content(&source_string, 500)
        };
        let _ = tx.send(result);
    });

    // 使用 glib 的超时轮询检查结果
    let store_weak = store.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        if let Ok(result) = rx.try_recv() {
            // IPC 完成，更新 UI
            if let Ok(entries) = result {
                store_weak.clear();
                for entry in entries.into_iter().rev() {
                    let iter = store_weak.append();
                    store_weak.set_value(&iter, 0, &entry.timestamp.to_value());
                    store_weak.set_value(&iter, 1, &entry.level.to_value());
                    store_weak.set_value(&iter, 2, &entry.source.to_value());
                    store_weak.set_value(&iter, 3, &entry.message.to_value());
                    store_weak.set_value(
                        &iter,
                        4,
                        &entry
                            .client_ip
                            .unwrap_or_else(|| "-".to_string())
                            .to_value(),
                    );
                    store_weak.set_value(
                        &iter,
                        5,
                        &entry.action.unwrap_or_else(|| "-".to_string()).to_value(),
                    );
                }
            } else if let Err(e) = result {
                store_weak.clear();
                let iter = store_weak.append();
                store_weak.set_value(&iter, 3, &format!("❌ 无法获取日志：{e}").to_value());
            }
            glib::ControlFlow::Break // 停止定时器
        } else {
            glib::ControlFlow::Continue // 继续等待
        }
    });
}

fn setup_button_handlers(
    tree_view: &TreeView,
    refresh_btn: &Button,
    clear_btn: &Button,
    auto_refresh_cb: &CheckButton,
    log_file_combo: &ComboBoxText,
) {
    let tree_view_clone = tree_view.clone();
    let log_file_combo_clone = log_file_combo.clone();
    refresh_btn.connect_clicked(
        clone!(@strong tree_view_clone, @strong log_file_combo_clone => move |_| {
            let source = log_file_combo_clone.active_id().map_or_else(|| "current".to_string(), |s| s.to_string());
            if let Some(store) = tree_view_clone.model()
                && let Ok(store) = store.downcast::<ListStore>() {
                    populate_log_store(&store, &source);
                }
        }),
    );

    let tree_view_clone = tree_view.clone();
    clear_btn.connect_clicked(clone!(@strong tree_view_clone => move |_| {
        if let Some(store) = tree_view_clone.model()
            && let Ok(store) = store.downcast::<ListStore>() {
                store.clear();
            }
    }));

    let tree_view_clone = tree_view.clone();
    let auto_refresh_cb_clone = auto_refresh_cb.clone();
    let log_file_combo_clone = log_file_combo.clone();

    glib::timeout_add_seconds_local(2, move || {
        if auto_refresh_cb_clone.is_active() {
            let source = log_file_combo_clone
                .active_id()
                .map_or_else(|| "current".to_string(), |s| s.to_string());

            if source == "current"
                && let Some(store) = tree_view_clone.model()
                && let Ok(store) = store.downcast::<ListStore>()
            {
                populate_log_store(&store, &source);
            }
        }
        glib::ControlFlow::Continue
    });

    let tree_view_clone = tree_view.clone();
    log_file_combo.connect_changed(
        clone!(@strong tree_view_clone, @strong log_file_combo => move |_| {
            let source = log_file_combo.active_id().map_or_else(|| "current".to_string(), |s| s.to_string());
            if let Some(store) = tree_view_clone.model()
                && let Ok(store) = store.downcast::<ListStore>() {
                    populate_log_store(&store, &source);
                }
        }),
    );
}

fn create_spin_button(min: f64, max: f64, step: f64) -> SpinButton {
    let adjustment = Adjustment::new(min, min, max, step, step * 10.0, 0.0);
    SpinButton::builder()
        .adjustment(&adjustment)
        .digits(0)
        .width_chars(6)
        .build()
}
