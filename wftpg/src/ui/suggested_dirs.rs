//! 主目录推荐目录选择对话框与共享目录权限设置。

use gtk::glib::clone;
use gtk::prelude::*;
use gtk::{
    Box, CellRendererText, Dialog, DialogFlags, Label, ListStore, Orientation, ResponseType,
    ScrolledWindow, TreeView, TreeViewColumn,
};
use std::os::unix::fs::PermissionsExt;
use tracing::{info, warn};

/// 弹出推荐目录选择对话框，把选中路径填入主目录输入框
pub(super) fn show_suggested_directories_dialog(home_entry: &gtk::Entry) {
    let dialog = Dialog::with_buttons(
        Some("选择推荐目录"),
        None::<&gtk::Window>,
        DialogFlags::MODAL,
        &[("取消", ResponseType::Cancel), ("确定", ResponseType::Ok)],
    );
    dialog.set_default_size(500, 400);

    let content = dialog.content_area();
    let box_ = Box::new(Orientation::Vertical, 10);
    box_.set_margin_top(10);
    box_.set_margin_bottom(10);
    box_.set_margin_start(10);
    box_.set_margin_end(10);

    let label = Label::new(Some(
        "以下是当前用户有权限访问的目录，选择一个作为用户主目录：",
    ));
    box_.pack_start(&label, false, false, 0);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(300)
        .build();

    let store = ListStore::new(&[
        gtk::glib::Type::STRING,
        gtk::glib::Type::STRING,
        gtk::glib::Type::STRING,
    ]);

    let suggested_dirs = get_suggested_directories();
    for (path, desc, perms) in suggested_dirs {
        let iter = store.append();
        store.set_value(&iter, 0, &path.to_value());
        store.set_value(&iter, 1, &desc.to_value());
        store.set_value(&iter, 2, &perms.to_value());
    }

    let tree = TreeView::with_model(&store);

    let col_path = TreeViewColumn::new();
    col_path.set_title("路径");
    col_path.set_resizable(true);
    col_path.set_min_width(200);
    let renderer_path = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_path, &renderer_path, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_path, &renderer_path, "text", 0);
    tree.append_column(&col_path);

    let col_desc = TreeViewColumn::new();
    col_desc.set_title("描述");
    col_desc.set_resizable(true);
    let renderer_desc = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_desc, &renderer_desc, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_desc, &renderer_desc, "text", 1);
    tree.append_column(&col_desc);

    let col_perms = TreeViewColumn::new();
    col_perms.set_title("权限");
    col_perms.set_resizable(true);
    let renderer_perms = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_perms, &renderer_perms, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_perms, &renderer_perms, "text", 2);
    tree.append_column(&col_perms);

    scrolled.add(&tree);
    box_.pack_start(&scrolled, true, true, 0);

    let tip_label = Label::new(Some(
        "提示: 选择目录后点击确定，或使用\"浏览...\"选择其他目录",
    ));
    tip_label.set_markup("<span foreground='gray' size='small'>提示: 选择目录后点击确定，或使用\"浏览...\"选择其他目录</span>");
    box_.pack_start(&tip_label, false, false, 0);

    content.add(&box_);
    content.show_all();

    let home_entry_clone = home_entry.clone();
    let tree_clone = tree.clone();

    dialog.connect_response(clone!(
        #[strong]
        home_entry_clone,
        #[strong]
        tree_clone,
        move |dlg, resp| {
            if resp == ResponseType::Ok {
                let selection = tree_clone.selection();
                if let Some((model, iter)) = selection.selected() {
                    let path: String = model.value(&iter, 0).get().unwrap_or_default();
                    home_entry_clone.set_text(&path);
                }
            }
            dlg.close();
        }
    ));

    dialog.run();
}

fn get_suggested_directories() -> Vec<(String, String, String)> {
    let mut dirs = Vec::new();

    if let Ok(home) = std::env::var("HOME") {
        let home_path = std::path::Path::new(&home);

        if check_dir_permission(&home) {
            dirs.push((home.clone(), "用户主目录".to_string(), "读写".to_string()));
        }

        let desktop = home_path.join("Desktop");
        if desktop.exists() && check_dir_permission(&desktop.to_string_lossy()) {
            dirs.push((
                desktop.to_string_lossy().to_string(),
                "桌面".to_string(),
                "读写".to_string(),
            ));
        }

        let documents = home_path.join("Documents");
        if documents.exists() && check_dir_permission(&documents.to_string_lossy()) {
            dirs.push((
                documents.to_string_lossy().to_string(),
                "文档".to_string(),
                "读写".to_string(),
            ));
        }

        let downloads = home_path.join("Downloads");
        if downloads.exists() && check_dir_permission(&downloads.to_string_lossy()) {
            dirs.push((
                downloads.to_string_lossy().to_string(),
                "下载".to_string(),
                "读写".to_string(),
            ));
        }

        let share_dir = home_path.join("Desktop/文件共享");
        if share_dir.exists() && check_dir_permission(&share_dir.to_string_lossy()) {
            dirs.push((
                share_dir.to_string_lossy().to_string(),
                "默认共享目录".to_string(),
                "读写".to_string(),
            ));
        } else if check_dir_permission(&home) {
            dirs.push((
                share_dir.to_string_lossy().to_string(),
                "默认共享目录(待创建)".to_string(),
                "读写".to_string(),
            ));
        }
    }

    if dirs.is_empty() {
        dirs.push((
            "/tmp".to_string(),
            "临时目录".to_string(),
            "读写".to_string(),
        ));
    }

    dirs
}

fn check_dir_permission(path: &str) -> bool {
    use std::os::unix::fs::MetadataExt;

    let path = std::path::Path::new(path);

    if !path.exists() {
        if let Some(parent) = path.parent() {
            return parent.exists() && check_dir_permission(&parent.to_string_lossy());
        }
        return false;
    }

    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };

    if !metadata.is_dir() {
        return false;
    }

    let mode = metadata.permissions().mode();
    let uid_of_dir = metadata.uid();
    let gid_of_dir = metadata.gid();

    let world_readable = (mode & 0o004) != 0;
    let world_executable = (mode & 0o001) != 0;

    if world_readable && world_executable {
        return true;
    }

    let current_uid = nix::unistd::getuid().as_raw();
    if uid_of_dir == current_uid {
        return (mode & 0o400) != 0 && (mode & 0o100) != 0;
    }

    let in_file_group = if let Ok(groups) = nix::unistd::getgroups() {
        groups.iter().any(|gid| gid.as_raw() == gid_of_dir)
    } else {
        warn!("Failed to get groups, falling back to world permissions");
        return world_readable && world_executable;
    };

    if in_file_group {
        return (mode & 0o040) != 0 && (mode & 0o010) != 0;
    }

    world_readable && world_executable
}

/// 把用户主目录设置为 wftpg 组可读写的共享目录（chgrp + 2770 + 默认 ACL）
pub(super) fn setup_shared_directory_permissions(path: &str) {
    let path = std::path::Path::new(path);

    if !path.exists() {
        return;
    }

    let wftpg_group_exists = std::process::Command::new("getent")
        .args(["group", "wftpg"])
        .status()
        .is_ok_and(|s| s.success());

    if !wftpg_group_exists {
        warn!("wftpg group not found, skipping permission setup");
        return;
    }

    let chown_result = std::process::Command::new("chgrp")
        .args(["wftpg", &path.to_string_lossy()])
        .status();

    match chown_result {
        Ok(status) if status.success() => {
            info!("Changed group ownership to wftpg for {}", path.display());
        }
        _ => {
            warn!("Failed to chgrp directory to wftpg group");
        }
    }

    let chmod_result = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o2770));
    match chmod_result {
        Ok(()) => {
            info!("Set directory {} permissions to 2770", path.display());
        }
        Err(e) => {
            warn!("Failed to set directory permissions: {}", e);
        }
    }

    let setfacl_result = std::process::Command::new("setfacl")
        .args(["-d", "-m", "u::rw-,g::rw-,o::---", &path.to_string_lossy()])
        .status();

    match setfacl_result {
        Ok(status) if status.success() => {
            info!(
                "Set default ACL for directory {} (files: rw-, no execute)",
                path.display()
            );
        }
        _ => {
            info!("setfacl not available, using umask for file permissions");
        }
    }
}
