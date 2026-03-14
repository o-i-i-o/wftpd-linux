use gtk::prelude::*;
use gtk::{
    Box, Orientation, Label, Button, ScrolledWindow, ComboBoxText, TreeView, ListStore, 
    CellRendererText, TreeViewColumn, glib, CheckButton,
};
use gtk::glib::clone;
use std::sync::{Arc, Mutex as StdMutex};
use crate::AppState;
use std::fs;

pub fn create(state: &Arc<StdMutex<AppState>>) -> Box {
    let container = Box::new(Orientation::Vertical, 10);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);

    let (refresh_btn, clear_btn, auto_refresh_cb, log_file_combo) = create_control_buttons(&container, state);
    let tree_view = create_log_view(&container, state);

    setup_button_handlers(state, &tree_view, &refresh_btn, &clear_btn, &auto_refresh_cb, &log_file_combo);

    container
}

fn create_control_buttons(container: &Box, state: &Arc<StdMutex<AppState>>) -> (Button, Button, CheckButton, ComboBoxText) {
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
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                name.starts_with("file-ops-") && name.ends_with(".log")
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

fn create_log_view(container: &Box, state: &Arc<StdMutex<AppState>>) -> TreeView {
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
        gtk::glib::Type::STRING,
        gtk::glib::Type::STRING,
        gtk::glib::Type::STRING,
    ]);

    populate_log_store(&store, state, "current");

    let tree = TreeView::with_model(&store);

    let col_time = TreeViewColumn::new();
    col_time.set_title("时间");
    col_time.set_resizable(true);
    col_time.set_min_width(150);
    let renderer_time = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_time, &renderer_time, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_time, &renderer_time, "text", 0);
    tree.append_column(&col_time);

    let col_user = TreeViewColumn::new();
    col_user.set_title("用户");
    col_user.set_resizable(true);
    col_user.set_min_width(80);
    let renderer_user = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_user, &renderer_user, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_user, &renderer_user, "text", 1);
    tree.append_column(&col_user);

    let col_ip = TreeViewColumn::new();
    col_ip.set_title("客户端IP");
    col_ip.set_resizable(true);
    col_ip.set_min_width(120);
    let renderer_ip = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_ip, &renderer_ip, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_ip, &renderer_ip, "text", 2);
    tree.append_column(&col_ip);

    let col_op = TreeViewColumn::new();
    col_op.set_title("操作");
    col_op.set_resizable(true);
    col_op.set_min_width(80);
    let renderer_op = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_op, &renderer_op, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_op, &renderer_op, "text", 3);
    tree.append_column(&col_op);

    let col_path = TreeViewColumn::new();
    col_path.set_title("文件路径");
    col_path.set_resizable(true);
    col_path.set_min_width(300);
    let renderer_path = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_path, &renderer_path, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_path, &renderer_path, "text", 4);
    tree.append_column(&col_path);

    let col_size = TreeViewColumn::new();
    col_size.set_title("大小");
    col_size.set_resizable(true);
    col_size.set_min_width(80);
    let renderer_size = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_size, &renderer_size, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_size, &renderer_size, "text", 5);
    tree.append_column(&col_size);

    let col_protocol = TreeViewColumn::new();
    col_protocol.set_title("协议");
    col_protocol.set_resizable(true);
    col_protocol.set_min_width(60);
    let renderer_protocol = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_protocol, &renderer_protocol, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_protocol, &renderer_protocol, "text", 6);
    tree.append_column(&col_protocol);

    let col_status = TreeViewColumn::new();
    col_status.set_title("状态");
    col_status.set_resizable(true);
    col_status.set_min_width(60);
    let renderer_status = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_status, &renderer_status, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_status, &renderer_status, "text", 7);
    tree.append_column(&col_status);

    let col_msg = TreeViewColumn::new();
    col_msg.set_title("消息");
    col_msg.set_resizable(true);
    col_msg.set_min_width(150);
    let renderer_msg = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_msg, &renderer_msg, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_msg, &renderer_msg, "text", 8);
    tree.append_column(&col_msg);

    scrolled.add(&tree);
    container.pack_start(&scrolled, true, true, 0);

    tree
}

fn format_file_size(size: u64) -> String {
    if size == 0 {
        return "-".to_string();
    }
    
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    
    if size >= GB {
        format!("{:.2} GB", size as f64 / GB as f64)
    } else if size >= MB {
        format!("{:.2} MB", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.2} KB", size as f64 / KB as f64)
    } else {
        format!("{} B", size)
    }
}

fn get_operation_display(op: &str) -> String {
    match op {
        "UPLOAD" => "上传",
        "DOWNLOAD" => "下载",
        "UPDATE" => "更新",
        "DELETE" => "删除",
        "RENAME" => "重命名",
        "MKDIR" => "创建目录",
        "RMDIR" => "删除目录",
        "APPEND" => "追加",
        "COPY" => "复制",
        "SYMLINK" => "符号链接",
        "HARDLINK" => "硬链接",
        _ => op,
    }.to_string()
}

fn populate_log_store(store: &ListStore, state: &Arc<StdMutex<AppState>>, source: &str) {
    store.clear();
    
    if source == "current" {
        if let Ok(s) = state.try_lock() {
            if let Ok(file_logger) = s.file_logger.try_lock() {
                let entries = file_logger.get_recent_logs(500);
                for entry in entries.into_iter().rev() {
                    let iter = store.append();
                    store.set_value(&iter, 0, &entry.timestamp.format("%Y-%m-%d %H:%M:%S").to_string().to_value());
                    store.set_value(&iter, 1, &entry.username.to_value());
                    store.set_value(&iter, 2, &entry.client_ip.to_value());
                    store.set_value(&iter, 3, &get_operation_display(&entry.operation).to_value());
                    store.set_value(&iter, 4, &entry.file_path.to_value());
                    store.set_value(&iter, 5, &format_file_size(entry.file_size).to_value());
                    store.set_value(&iter, 6, &entry.protocol.to_value());
                    store.set_value(&iter, 7, &if entry.success { "成功" } else { "失败" }.to_value());
                    store.set_value(&iter, 8, &entry.message.to_value());
                }
            }
        }
    } else {
        match fs::read_to_string(source) {
            Ok(content) => {
                for line in content.lines().rev().take(500) {
                    if let Ok(entry) = serde_json::from_str::<crate::core::file_logger::FileLogEntry>(line) {
                        let iter = store.append();
                        store.set_value(&iter, 0, &entry.timestamp.format("%Y-%m-%d %H:%M:%S").to_string().to_value());
                        store.set_value(&iter, 1, &entry.username.to_value());
                        store.set_value(&iter, 2, &entry.client_ip.to_value());
                        store.set_value(&iter, 3, &get_operation_display(&entry.operation).to_value());
                        store.set_value(&iter, 4, &entry.file_path.to_value());
                        store.set_value(&iter, 5, &format_file_size(entry.file_size).to_value());
                        store.set_value(&iter, 6, &entry.protocol.to_value());
                        store.set_value(&iter, 7, &if entry.success { "成功" } else { "失败" }.to_value());
                        store.set_value(&iter, 8, &entry.message.to_value());
                    }
                }
            }
            Err(e) => {
                let iter = store.append();
                store.set_value(&iter, 4, &format!("无法读取日志文件: {}", e).to_value());
            }
        }
    }
}

fn setup_button_handlers(
    state: &Arc<StdMutex<AppState>>,
    tree_view: &TreeView,
    refresh_btn: &Button,
    clear_btn: &Button,
    auto_refresh_cb: &CheckButton,
    log_file_combo: &ComboBoxText,
) {
    let state_clone = Arc::clone(state);
    let tree_view_clone = tree_view.clone();
    let log_file_combo_clone = log_file_combo.clone();
    refresh_btn.connect_clicked(clone!(@strong state_clone, @strong tree_view_clone, @strong log_file_combo_clone => move |_| {
        let source = log_file_combo_clone.active_id()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "current".to_string());
        if let Some(store) = tree_view_clone.model() {
            if let Ok(store) = store.downcast::<ListStore>() {
                populate_log_store(&store, &state_clone, &source);
            }
        }
    }));

    let tree_view_clone = tree_view.clone();
    clear_btn.connect_clicked(clone!(@strong tree_view_clone => move |_| {
        if let Some(store) = tree_view_clone.model() {
            if let Ok(store) = store.downcast::<ListStore>() {
                store.clear();
            }
        }
    }));

    let state_clone = Arc::clone(state);
    let tree_view_clone = tree_view.clone();
    let auto_refresh_cb_clone = auto_refresh_cb.clone();
    let log_file_combo_clone = log_file_combo.clone();
    
    glib::timeout_add_seconds_local(2, move || {
        if auto_refresh_cb_clone.is_active() {
            let source = log_file_combo_clone.active_id()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "current".to_string());
            
            if source == "current" {
                if let Some(store) = tree_view_clone.model() {
                    if let Ok(store) = store.downcast::<ListStore>() {
                        populate_log_store(&store, &state_clone, &source);
                    }
                }
            }
        }
        glib::ControlFlow::Continue
    });

    let state_clone = Arc::clone(state);
    let tree_view_clone = tree_view.clone();
    log_file_combo.connect_changed(clone!(@strong state_clone, @strong tree_view_clone, @strong log_file_combo => move |_| {
        let source = log_file_combo.active_id()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "current".to_string());
        if let Some(store) = tree_view_clone.model() {
            if let Ok(store) = store.downcast::<ListStore>() {
                populate_log_store(&store, &state_clone, &source);
            }
        }
    }));
}
