use crate::AppState;
use gtk::glib::{self, clone};
use gtk::prelude::*;
use gtk::{
    Adjustment, Box, Button, CellRendererText, CellRendererToggle, CheckButton, Dialog,
    DialogFlags, Entry, FileChooserAction, FileChooserDialog, Frame, Label, ListStore, Orientation,
    ResponseType, ScrolledWindow, SpinButton, TreeView, TreeViewColumn,
};
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex as StdMutex};
use tracing::{error, info, warn};

pub fn create(state: &Arc<StdMutex<AppState>>) -> Box {
    let container = Box::new(Orientation::Vertical, 10);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);

    let list_frame = Frame::new(Some("用户列表"));
    let list_box = Box::new(Orientation::Vertical, 5);
    list_box.set_margin_top(10);
    list_box.set_margin_bottom(10);
    list_box.set_margin_start(10);
    list_box.set_margin_end(10);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(300)
        .build();

    let store = ListStore::new(&[
        gtk::glib::Type::STRING,
        gtk::glib::Type::STRING,
        gtk::glib::Type::BOOL,
        gtk::glib::Type::STRING,
        gtk::glib::Type::STRING,
    ]);

    refresh_user_list(&store, state);

    let tree = TreeView::with_model(&store);

    let col_username = TreeViewColumn::new();
    col_username.set_title("用户名");
    col_username.set_resizable(true);
    col_username.set_min_width(100);
    let renderer_username = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_username, &renderer_username, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_username, &renderer_username, "text", 0);
    tree.append_column(&col_username);

    let col_home = TreeViewColumn::new();
    col_home.set_title("主目录");
    col_home.set_resizable(true);
    col_home.set_min_width(200);
    let renderer_home = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_home, &renderer_home, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_home, &renderer_home, "text", 1);
    tree.append_column(&col_home);

    let col_enabled = TreeViewColumn::new();
    col_enabled.set_title("启用");
    let renderer_enabled = CellRendererToggle::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_enabled, &renderer_enabled, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_enabled, &renderer_enabled, "active", 2);
    tree.append_column(&col_enabled);

    let col_quota = TreeViewColumn::new();
    col_quota.set_title("配额");
    col_quota.set_resizable(true);
    col_quota.set_min_width(80);
    let renderer_quota = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_quota, &renderer_quota, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_quota, &renderer_quota, "text", 4);
    tree.append_column(&col_quota);

    let col_perms = TreeViewColumn::new();
    col_perms.set_title("权限");
    col_perms.set_resizable(true);
    let renderer_perms = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_perms, &renderer_perms, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_perms, &renderer_perms, "text", 3);
    tree.append_column(&col_perms);

    scrolled.add(&tree);
    list_box.pack_start(&scrolled, true, true, 0);

    let btn_box = Box::new(Orientation::Horizontal, 10);
    let add_btn = Button::with_label("新建用户");
    let edit_btn = Button::with_label("编辑用户");
    let delete_btn = Button::with_label("删除用户");
    let toggle_btn = Button::with_label("启用/禁用");
    let refresh_btn = Button::with_label("刷新列表");
    btn_box.pack_start(&add_btn, false, false, 0);
    btn_box.pack_start(&edit_btn, false, false, 0);
    btn_box.pack_start(&delete_btn, false, false, 0);
    btn_box.pack_start(&toggle_btn, false, false, 0);
    btn_box.pack_start(&refresh_btn, false, false, 0);
    list_box.pack_start(&btn_box, false, false, 0);

    list_frame.add(&list_box);
    container.pack_start(&list_frame, true, true, 0);

    let state_clone = Arc::clone(state);
    let store_clone = store.clone();
    add_btn.connect_clicked(
        clone!(@strong state_clone, @strong store_clone => move |_| {
            show_user_dialog(None, &state_clone, &store_clone);
        }),
    );

    let state_clone = Arc::clone(state);
    let store_clone = store.clone();
    let tree_clone = tree.clone();
    edit_btn.connect_clicked(
        clone!(@strong state_clone, @strong store_clone, @strong tree_clone => move |_| {
            let selection = tree_clone.selection();
            if let Some((model, iter)) = selection.selected() {
                let username: String = model.value(&iter, 0).get().unwrap_or_default();
                show_user_dialog(Some(&username), &state_clone, &store_clone);
            }
        }),
    );

    let state_clone = Arc::clone(state);
    let store_clone = store.clone();
    let tree_clone = tree.clone();
    delete_btn.connect_clicked(
        clone!(@strong state_clone, @strong store_clone, @strong tree_clone => move |_| {
            let selection = tree_clone.selection();
            if let Some((model, iter)) = selection.selected() {
                let username: String = model.value(&iter, 0).get().unwrap_or_default();
                show_confirm_dialog(&username, &state_clone, &store_clone);
            }
        }),
    );

    let state_clone = Arc::clone(state);
    let store_clone = store.clone();
    let tree_clone = tree.clone();
    toggle_btn.connect_clicked(
        clone!(@strong state_clone, @strong store_clone, @strong tree_clone => move |_| {
            let selection = tree_clone.selection();
            if let Some((model, iter)) = selection.selected() {
                let username: String = model.value(&iter, 0).get().unwrap_or_default();
                let current_enabled: bool = model.value(&iter, 2).get().unwrap_or(true);

                let store = store_clone.clone();
                let uname = username.clone();
                let new_enabled = !current_enabled;

                let users_json = {
                    if let Ok(s) = state_clone.try_lock() {
                        if let Ok(mut users) = s.user_manager.try_lock() {
                            if users.set_user_enabled(&uname, new_enabled).is_ok() {
                                serde_json::to_string(&*users).unwrap_or_default()
                            } else { return; }
                        } else { return; }
                    } else { return; }
                };

                match crate::communication::write_users(&users_json) {
                    Ok(_) => {
                        let action = if new_enabled { "enabled" } else { "disabled" };
                        let _ = crate::communication::write_audit_log(
                            "gui-user",
                            "USER_TOGGLE",
                            &uname,
                            &format!("User {uname} status changed to {action}")
                        );
                        info!("User {} enabled status toggled to {}", uname, new_enabled);
                    }
                    Err(e) => {
                        error!("Failed to save users: {}", e);
                    }
                }
                refresh_user_list(&store, &state_clone);
            }
        }),
    );

    let state_clone = Arc::clone(state);
    let store_clone = store.clone();
    refresh_btn.connect_clicked(
        clone!(@strong state_clone, @strong store_clone => move |_| {
            refresh_user_list(&store_clone, &state_clone);
        }),
    );

    container
}

fn show_user_dialog(username: Option<&str>, state: &Arc<StdMutex<AppState>>, store: &ListStore) {
    let dialog = Dialog::with_buttons(
        Some(if username.is_some() {
            "编辑用户"
        } else {
            "新建用户"
        }),
        None::<&gtk::Window>,
        DialogFlags::MODAL,
        &[("取消", ResponseType::Cancel), ("确定", ResponseType::Ok)],
    );
    dialog.set_default_size(400, 350);

    let content = dialog.content_area();
    let box_ = Box::new(Orientation::Vertical, 10);
    box_.set_margin_top(10);
    box_.set_margin_bottom(10);
    box_.set_margin_start(10);
    box_.set_margin_end(10);

    let row1 = Box::new(Orientation::Horizontal, 5);
    row1.pack_start(&Label::new(Some("用户名:")), false, false, 0);
    let username_entry = Entry::new();
    username_entry.set_width_chars(20);
    if let Some(name) = username {
        username_entry.set_text(name);
        username_entry.set_sensitive(false);
    }
    row1.pack_start(&username_entry, true, true, 0);
    box_.pack_start(&row1, false, false, 0);

    let row2 = Box::new(Orientation::Horizontal, 5);
    row2.pack_start(&Label::new(Some("密码:")), false, false, 0);
    let password_entry = Entry::new();
    password_entry.set_width_chars(20);
    password_entry.set_visibility(false);
    if username.is_some() {
        password_entry.set_placeholder_text(Some("留空则不修改密码"));
    }
    row2.pack_start(&password_entry, true, true, 0);
    box_.pack_start(&row2, false, false, 0);

    let row3 = Box::new(Orientation::Horizontal, 5);
    row3.pack_start(&Label::new(Some("主目录:")), false, false, 0);
    let home_entry = Entry::new();
    home_entry.set_width_chars(20);
    home_entry.set_placeholder_text(Some("留空则使用默认目录"));
    row3.pack_start(&home_entry, true, true, 0);

    let browse_btn = Button::with_label("浏览...");
    let home_entry_clone = home_entry.clone();
    browse_btn.connect_clicked(clone!(@strong home_entry_clone => move |_| {
        let dialog = FileChooserDialog::new(
            Some("选择用户主目录"),
            None::<&gtk::Window>,
            FileChooserAction::SelectFolder,
        );
        dialog.add_button("取消", ResponseType::Cancel);
        dialog.add_button("选择", ResponseType::Accept);

        let entry = home_entry_clone.clone();
        dialog.connect_response(clone!(@strong entry, @strong dialog => move |dlg, resp| {
            if resp == ResponseType::Accept
                && let Some(path) = dlg.filename() {
                    entry.set_text(&path.to_string_lossy());
                }
            dlg.close();
        }));

        dialog.run();
    }));
    row3.pack_start(&browse_btn, false, false, 0);

    let suggest_btn = Button::with_label("推荐目录");
    let home_entry_for_suggest = home_entry.clone();
    suggest_btn.connect_clicked(clone!(@strong home_entry_for_suggest => move |_| {
        show_suggested_directories_dialog(&home_entry_for_suggest);
    }));
    row3.pack_start(&suggest_btn, false, false, 0);
    box_.pack_start(&row3, false, false, 0);

    let perm_frame = Frame::new(Some("权限设置"));
    let perm_box = Box::new(Orientation::Horizontal, 10);
    perm_box.set_margin_top(5);
    perm_box.set_margin_bottom(5);
    perm_box.set_margin_start(5);
    perm_box.set_margin_end(5);

    let read_cb = CheckButton::with_label("读取");
    let write_cb = CheckButton::with_label("写入");
    let delete_cb = CheckButton::with_label("删除");
    let list_cb = CheckButton::with_label("列表");
    let mkdir_cb = CheckButton::with_label("建目录");
    let rmdir_cb = CheckButton::with_label("删目录");
    let rename_cb = CheckButton::with_label("重命名");
    let append_cb = CheckButton::with_label("追加");

    read_cb.set_active(true);
    write_cb.set_active(true);
    delete_cb.set_active(true);
    list_cb.set_active(true);
    mkdir_cb.set_active(true);
    rmdir_cb.set_active(true);
    rename_cb.set_active(true);
    append_cb.set_active(true);

    perm_box.pack_start(&read_cb, false, false, 0);
    perm_box.pack_start(&write_cb, false, false, 0);
    perm_box.pack_start(&delete_cb, false, false, 0);
    perm_box.pack_start(&list_cb, false, false, 0);
    perm_box.pack_start(&mkdir_cb, false, false, 0);
    perm_box.pack_start(&rmdir_cb, false, false, 0);
    perm_box.pack_start(&rename_cb, false, false, 0);
    perm_box.pack_start(&append_cb, false, false, 0);
    perm_frame.add(&perm_box);
    box_.pack_start(&perm_frame, false, false, 0);

    let quota_frame = Frame::new(Some("配额设置"));
    let quota_box = Box::new(Orientation::Horizontal, 10);
    quota_box.set_margin_top(5);
    quota_box.set_margin_bottom(5);
    quota_box.set_margin_start(5);
    quota_box.set_margin_end(5);

    let quota_cb = CheckButton::with_label("启用配额");
    let quota_spin = create_spin_button(1.0, 1_000_000.0, 100.0);
    quota_box.pack_start(&quota_cb, false, false, 0);
    quota_box.pack_start(&Label::new(Some("配额(MB):")), false, false, 0);
    quota_box.pack_start(&quota_spin, false, false, 0);
    quota_frame.add(&quota_box);
    box_.pack_start(&quota_frame, false, false, 0);

    let speed_frame = Frame::new(Some("速度限制"));
    let speed_box = Box::new(Orientation::Horizontal, 10);
    speed_box.set_margin_top(5);
    speed_box.set_margin_bottom(5);
    speed_box.set_margin_start(5);
    speed_box.set_margin_end(5);

    let speed_limit_cb = CheckButton::with_label("启用限速");
    let speed_limit_spin = create_spin_button(1.0, 102_400.0, 100.0);
    speed_box.pack_start(&speed_limit_cb, false, false, 0);
    speed_box.pack_start(&Label::new(Some("速度限制(KB/s):")), false, false, 0);
    speed_box.pack_start(&speed_limit_spin, false, false, 0);
    let speed_hint = Label::new(None);
    speed_hint.set_markup("<span foreground='gray' size='small'>(0=不限制)</span>");
    speed_box.pack_start(&speed_hint, false, false, 0);
    speed_frame.add(&speed_box);
    box_.pack_start(&speed_frame, false, false, 0);

    if let Some(name) = username
        && let Ok(s) = state.try_lock()
        && let Ok(users) = s.user_manager.try_lock()
        && let Some(user) = users.get_user(name)
    {
        home_entry.set_text(&user.home_dir);
        read_cb.set_active(user.permissions.can_read);
        write_cb.set_active(user.permissions.can_write);
        delete_cb.set_active(user.permissions.can_delete);
        list_cb.set_active(user.permissions.can_list);
        mkdir_cb.set_active(user.permissions.can_mkdir);
        rmdir_cb.set_active(user.permissions.can_rmdir);
        rename_cb.set_active(user.permissions.can_rename);
        append_cb.set_active(user.permissions.can_append);

        if let Some(quota) = user.permissions.quota_mb {
            quota_cb.set_active(true);
            quota_spin.set_value(super::utils::spin_f64_from(quota));
        }

        if let Some(speed_limit) = user.permissions.speed_limit_kbps {
            speed_limit_cb.set_active(true);
            speed_limit_spin.set_value(super::utils::spin_f64_from(speed_limit));
        }
    }

    content.add(&box_);
    content.show_all();

    let state_clone = Arc::clone(state);
    let store_clone = store.clone();
    let username_entry_clone = username_entry.clone();
    let password_entry_clone = password_entry.clone();
    let home_entry_clone = home_entry.clone();
    let read_cb_clone = read_cb.clone();
    let write_cb_clone = write_cb.clone();
    let delete_cb_clone = delete_cb.clone();
    let list_cb_clone = list_cb.clone();
    let mkdir_cb_clone = mkdir_cb.clone();
    let rmdir_cb_clone = rmdir_cb.clone();
    let rename_cb_clone = rename_cb.clone();
    let append_cb_clone = append_cb.clone();
    let quota_cb_clone = quota_cb.clone();
    let quota_spin_clone = quota_spin.clone();
    let speed_limit_cb_clone = speed_limit_cb.clone();
    let speed_limit_spin_clone = speed_limit_spin.clone();
    let edit_username = username.map(std::string::ToString::to_string);

    dialog.connect_response(clone!(@strong state_clone, @strong store_clone, @strong username_entry_clone, @strong password_entry_clone, @strong home_entry_clone,
           @strong read_cb_clone, @strong write_cb_clone, @strong delete_cb_clone, @strong list_cb_clone,
           @strong mkdir_cb_clone, @strong rmdir_cb_clone, @strong rename_cb_clone, @strong append_cb_clone,
           @strong quota_cb_clone, @strong quota_spin_clone, @strong speed_limit_cb_clone, @strong speed_limit_spin_clone, @strong edit_username => move |dlg, resp| {
        if resp == ResponseType::Ok {
            let uname = username_entry_clone.text().to_string();
            let password = password_entry_clone.text().to_string();
            let home = home_entry_clone.text().to_string();

            if uname.is_empty() {
                return;
            }

            let perms = wftpd_common::Permissions {
                can_read: read_cb_clone.is_active(),
                can_write: write_cb_clone.is_active(),
                can_delete: delete_cb_clone.is_active(),
                can_list: list_cb_clone.is_active(),
                can_mkdir: mkdir_cb_clone.is_active(),
                can_rmdir: rmdir_cb_clone.is_active(),
                can_rename: rename_cb_clone.is_active(),
                can_append: append_cb_clone.is_active(),
                quota_mb: if quota_cb_clone.is_active() {
                    Some(super::utils::spin_u64(quota_spin_clone.value()))
                } else {
                    None
                },
                speed_limit_kbps: if speed_limit_cb_clone.is_active() {
                    Some(super::utils::spin_u64(speed_limit_spin_clone.value()))
                } else {
                    None
                },
            };

            let state = Arc::clone(&state_clone);
            let store = store_clone.clone();
            let home_dir = home.clone();
            let pwd = password.clone();
            let username = uname.clone();
            let is_edit = edit_username.is_some();

            if home_dir.is_empty() {
                glib::idle_add_local_once(move || {
                    let err_dialog = gtk::MessageDialog::new(
                        None::<&gtk::Window>,
                        gtk::DialogFlags::MODAL,
                        gtk::MessageType::Error,
                        gtk::ButtonsType::Ok,
                        "主目录不能为空",
                    );
                    err_dialog.run();
                    err_dialog.close();
                });
                return;
            }

            let home_path = std::path::Path::new(&home_dir);
            if !home_path.exists() {
                let home_dir_msg = home_dir.clone();
                glib::idle_add_local_once(move || {
                    let err_dialog = gtk::MessageDialog::new(
                        None::<&gtk::Window>,
                        gtk::DialogFlags::MODAL,
                        gtk::MessageType::Error,
                        gtk::ButtonsType::Ok,
                        &format!("主目录不存在: {home_dir_msg}"),
                    );
                    err_dialog.run();
                    err_dialog.close();
                });
                return;
            }
            if !home_path.is_dir() {
                let home_dir_msg = home_dir.clone();
                glib::idle_add_local_once(move || {
                    let err_dialog = gtk::MessageDialog::new(
                        None::<&gtk::Window>,
                        gtk::DialogFlags::MODAL,
                        gtk::MessageType::Error,
                        gtk::ButtonsType::Ok,
                        &format!("主目录路径不是目录: {home_dir_msg}"),
                    );
                    err_dialog.run();
                    err_dialog.close();
                });
                return;
            }

            let users_json = {
                if let Ok(s) = state.try_lock() {
                    if let Ok(mut users) = s.user_manager.try_lock() {
                        let success = if is_edit {
                            if !pwd.is_empty() {
                                let _ = users.update_password(&username, &pwd);
                            }
                            let _ = users.update_home_dir(&username, home_dir.clone());
                            let _ = users.update_permissions(&username, perms);
                            true
                        } else if pwd.is_empty() {
                            false
                        } else {
                            users.add_user(username.clone(), &pwd, home_dir.clone(), perms, false).is_ok()
                        };

                        if success {
                            setup_shared_directory_permissions(&home_dir);

                            serde_json::to_string(&*users).unwrap_or_default()
                        } else { return; }
                    } else { return; }
                } else { return; }
            };

            match crate::communication::write_users(&users_json) {
                Ok(_) => {
                    let action_type = if is_edit { "USER_MODIFY" } else { "USER_CREATE" };
                    let _ = crate::communication::write_audit_log(
                        "gui-user",
                        action_type,
                        &username,
                        &format!("User {} {}", username, if is_edit { "modified" } else { "created" })
                    );
                    info!("User {} saved", username);
                }
                Err(e) => {
                    error!("Failed to save users: {}", e);
                }
            }
            refresh_user_list(&store, &state);
        }
        dlg.close();
    }));

    dialog.run();
}

fn show_confirm_dialog(username: &str, state: &Arc<StdMutex<AppState>>, store: &ListStore) {
    let dialog = Dialog::with_buttons(
        Some("确认删除"),
        None::<&gtk::Window>,
        DialogFlags::MODAL,
        &[("取消", ResponseType::Cancel), ("删除", ResponseType::Ok)],
    );
    dialog.set_default_size(300, 100);

    let content = dialog.content_area();
    let label = Label::new(Some(&format!("确定要删除用户 \"{username}\" 吗？")));
    label.set_margin_top(20);
    label.set_margin_bottom(20);
    label.set_margin_start(20);
    label.set_margin_end(20);
    content.add(&label);
    content.show_all();

    let state_clone = Arc::clone(state);
    let store_clone = store.clone();
    let uname = username.to_string();

    dialog.connect_response(
        clone!(@strong state_clone, @strong store_clone, @strong uname => move |dlg, resp| {
            if resp == ResponseType::Ok {
                let store = store_clone.clone();
                let username = uname.clone();

                let users_json = {
                    if let Ok(s) = state_clone.try_lock() {
                        if let Ok(mut users) = s.user_manager.try_lock() {
                            let _ = users.remove_user(&username);
                            serde_json::to_string(&*users).unwrap_or_default()
                        } else { return; }
                    } else { return; }
                };

                match crate::communication::write_users(&users_json) {
                    Ok(_) => {
                        let _ = crate::communication::write_audit_log(
                            "gui-user",
                            "USER_DELETE",
                            &username,
                            &format!("User {username} deleted")
                        );
                        info!("User {} deleted", username);
                    }
                    Err(e) => {
                        error!("Failed to save users: {}", e);
                    }
                }
                refresh_user_list(&store, &state_clone);
            }
            dlg.close();
        }),
    );

    dialog.run();
}

fn refresh_user_list(store: &ListStore, state: &Arc<StdMutex<AppState>>) {
    store.clear();
    if let Ok(s) = state.try_lock()
        && let Ok(users) = s.user_manager.try_lock()
    {
        for (username, user) in users.list_users() {
            let quota_str = user
                .permissions
                .quota_mb
                .map_or_else(|| "无限制".to_string(), |q| format!("{q} MB"));
            let iter = store.append();
            store.set_value(&iter, 0, &username.to_value());
            store.set_value(&iter, 1, &user.home_dir.to_value());
            store.set_value(&iter, 2, &user.enabled.to_value());
            store.set_value(&iter, 3, &user.permissions.to_string().to_value());
            store.set_value(&iter, 4, &quota_str.to_value());
        }
    }
}

fn create_spin_button(min: f64, max: f64, step: f64) -> SpinButton {
    let adjustment = Adjustment::new(min, min, max, step, step * 10.0, 0.0);
    SpinButton::builder()
        .adjustment(&adjustment)
        .digits(0)
        .width_chars(8)
        .build()
}

fn show_suggested_directories_dialog(home_entry: &Entry) {
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

    dialog.connect_response(
        clone!(@strong home_entry_clone, @strong tree_clone => move |dlg, resp| {
            if resp == ResponseType::Ok {
                let selection = tree_clone.selection();
                if let Some((model, iter)) = selection.selected() {
                    let path: String = model.value(&iter, 0).get().unwrap_or_default();
                    home_entry_clone.set_text(&path);
                }
            }
            dlg.close();
        }),
    );

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

    let var_share = std::path::Path::new("/var/lib/wftpg/share");
    if var_share.exists() && check_dir_permission(&var_share.to_string_lossy()) {
        dirs.push((
            var_share.to_string_lossy().to_string(),
            "系统共享目录".to_string(),
            "读写".to_string(),
        ));
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

fn setup_shared_directory_permissions(path: &str) {
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
