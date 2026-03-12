use gtk::prelude::*;
use gtk::{
    Box, Orientation, Label, Button, Entry, Frame, ScrolledWindow, TreeView, ListStore,
    CellRendererText, TreeViewColumn, CellRendererToggle, CheckButton, glib,
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

    let title_label = Label::new(Some("<b>用户管理</b>"));
    title_label.set_use_markup(true);
    container.pack_start(&title_label, false, false, 0);

    let add_frame = Frame::new(Some("添加用户"));
    let add_box = Box::new(Orientation::Vertical, 5);
    add_box.set_margin_top(10);
    add_box.set_margin_bottom(10);
    add_box.set_margin_start(10);
    add_box.set_margin_end(10);

    let row1 = Box::new(Orientation::Horizontal, 5);
    row1.pack_start(&Label::new(Some("用户名:")), false, false, 0);
    let username_entry = Entry::new();
    username_entry.set_width_chars(15);
    row1.pack_start(&username_entry, false, false, 0);

    row1.pack_start(&Label::new(Some("密码:")), false, false, 0);
    let password_entry = Entry::new();
    password_entry.set_width_chars(15);
    password_entry.set_visibility(false);
    row1.pack_start(&password_entry, false, false, 0);
    add_box.pack_start(&row1, false, false, 0);

    let row2 = Box::new(Orientation::Horizontal, 5);
    row2.pack_start(&Label::new(Some("主目录:")), false, false, 0);
    let home_entry = Entry::new();
    home_entry.set_hexpand(true);
    home_entry.set_placeholder_text(Some("留空则使用默认: /home/用户名/Desktop/文件共享"));
    row2.pack_start(&home_entry, true, true, 0);
    add_box.pack_start(&row2, false, false, 0);

    let perm_frame = Frame::new(Some("权限设置"));
    let perm_box = Box::new(Orientation::Horizontal, 10);
    perm_box.set_margin_top(5);
    perm_box.set_margin_bottom(5);
    perm_box.set_margin_start(5);
    perm_box.set_margin_end(5);

    let read_cb = CheckButton::with_label("读取");
    read_cb.set_active(true);
    let write_cb = CheckButton::with_label("写入");
    write_cb.set_active(true);
    let delete_cb = CheckButton::with_label("删除");
    delete_cb.set_active(true);
    let list_cb = CheckButton::with_label("列表");
    list_cb.set_active(true);
    let mkdir_cb = CheckButton::with_label("建目录");
    mkdir_cb.set_active(true);
    let rmdir_cb = CheckButton::with_label("删目录");
    rmdir_cb.set_active(true);
    let rename_cb = CheckButton::with_label("重命名");
    rename_cb.set_active(true);
    let append_cb = CheckButton::with_label("追加");
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
    add_box.pack_start(&perm_frame, false, false, 0);

    let add_btn = Button::with_label("添加用户");
    add_box.pack_start(&add_btn, false, false, 0);

    add_frame.add(&add_box);
    container.pack_start(&add_frame, false, false, 0);

    let list_frame = Frame::new(Some("用户列表"));
    let list_box = Box::new(Orientation::Vertical, 5);
    list_box.set_margin_top(10);
    list_box.set_margin_bottom(10);
    list_box.set_margin_start(10);
    list_box.set_margin_end(10);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(200)
        .build();

    let store = ListStore::new(&[
        gtk::glib::Type::STRING,
        gtk::glib::Type::STRING,
        gtk::glib::Type::BOOL,
        gtk::glib::Type::STRING,
    ]);

    refresh_user_list(&store, state);

    let tree = TreeView::with_model(&store);

    let col_username = TreeViewColumn::new();
    col_username.set_title("用户名");
    let renderer_username = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_username, &renderer_username, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_username, &renderer_username, "text", 0);
    tree.append_column(&col_username);

    let col_home = TreeViewColumn::new();
    col_home.set_title("主目录");
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

    let col_perms = TreeViewColumn::new();
    col_perms.set_title("权限");
    let renderer_perms = CellRendererText::new();
    gtk::prelude::CellLayoutExt::pack_start(&col_perms, &renderer_perms, true);
    gtk::prelude::CellLayoutExt::add_attribute(&col_perms, &renderer_perms, "text", 3);
    tree.append_column(&col_perms);

    scrolled.add(&tree);
    list_box.pack_start(&scrolled, true, true, 0);

    let btn_box = Box::new(Orientation::Horizontal, 10);
    let delete_btn = Button::with_label("删除选中用户");
    let toggle_btn = Button::with_label("启用/禁用切换");
    let refresh_btn = Button::with_label("刷新列表");
    btn_box.pack_start(&delete_btn, false, false, 0);
    btn_box.pack_start(&toggle_btn, false, false, 0);
    btn_box.pack_start(&refresh_btn, false, false, 0);
    list_box.pack_start(&btn_box, false, false, 0);

    list_frame.add(&list_box);
    container.pack_start(&list_frame, true, true, 0);

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
    add_btn.connect_clicked(clone!(@strong state_clone, @strong store_clone, @strong username_entry_clone, @strong password_entry_clone, @strong home_entry_clone,
           @strong read_cb_clone, @strong write_cb_clone, @strong delete_cb_clone, @strong list_cb_clone,
           @strong mkdir_cb_clone, @strong rmdir_cb_clone, @strong rename_cb_clone, @strong append_cb_clone => move |_| {
        let username = username_entry_clone.text().to_string();
        let password = password_entry_clone.text().to_string();
        let home = home_entry_clone.text().to_string();
        
        if username.is_empty() || password.is_empty() {
            return;
        }

        let home_str = if home.is_empty() {
            get_default_home_dir(&username)
        } else {
            home
        };

        let perms = wftpg::users::Permissions {
            can_read: read_cb_clone.is_active(),
            can_write: write_cb_clone.is_active(),
            can_delete: delete_cb_clone.is_active(),
            can_list: list_cb_clone.is_active(),
            can_mkdir: mkdir_cb_clone.is_active(),
            can_rmdir: rmdir_cb_clone.is_active(),
            can_rename: rename_cb_clone.is_active(),
            can_append: append_cb_clone.is_active(),
            quota_mb: None,
            speed_limit_kbps: None,
        };

        let state = Arc::clone(&state_clone);
        let store = store_clone.clone();
        let username_clear = username_entry_clone.clone();
        let password_clear = password_entry_clone.clone();
        let home_clear = home_entry_clone.clone();
        let home_dir = home_str.clone();
        let uname = username.clone();

        glib::MainContext::ref_thread_default().spawn_local(async move {
            if let Ok(s) = state.try_lock() {
                if let Ok(mut users) = s.user_manager.try_lock() {
                    if users.add_user(&uname, &password, &home_dir, false).is_ok() {
                        if let Err(e) = users.update_permissions(&uname, perms) {
                            log::error!("Failed to set permissions: {}", e);
                        }
                        let _ = users.save(&s.users_path);
                        
                        if let Err(e) = std::fs::create_dir_all(&home_dir) {
                            log::warn!("Failed to create home directory: {}", e);
                        }

                        refresh_user_list(&store, &state);
                        username_clear.set_text("");
                        password_clear.set_text("");
                        home_clear.set_text("");
                        log::info!("User {} added with home {}", uname, home_dir);
                    }
                }
            }
        });
    }));

    let state_clone = Arc::clone(state);
    let store_clone = store.clone();
    let tree_clone = tree.clone();
    delete_btn.connect_clicked(clone!(@strong state_clone, @strong store_clone, @strong tree_clone => move |_| {
        let selection = tree_clone.selection();
        if let Some((model, iter)) = selection.selected() {
            let username: String = model.value(&iter, 0).get().unwrap_or_default();
            let state = Arc::clone(&state_clone);
            let store = store_clone.clone();
            let uname = username.clone();
            
            glib::MainContext::ref_thread_default().spawn_local(async move {
                if let Ok(s) = state.try_lock() {
                    if let Ok(mut users) = s.user_manager.try_lock() {
                        let _ = users.remove_user(&uname);
                        let _ = users.save(&s.users_path);
                    }
                }
                refresh_user_list(&store, &state);
                log::info!("User {} deleted", uname);
            });
        }
    }));

    let state_clone = Arc::clone(state);
    let store_clone = store.clone();
    let tree_clone = tree.clone();
    toggle_btn.connect_clicked(clone!(@strong state_clone, @strong store_clone, @strong tree_clone => move |_| {
        let selection = tree_clone.selection();
        if let Some((model, iter)) = selection.selected() {
            let username: String = model.value(&iter, 0).get().unwrap_or_default();
            let current_enabled: bool = model.value(&iter, 2).get().unwrap_or(true);
            
            let state = Arc::clone(&state_clone);
            let store = store_clone.clone();
            let uname = username.clone();
            let new_enabled = !current_enabled;
            
            glib::MainContext::ref_thread_default().spawn_local(async move {
                if let Ok(s) = state.try_lock() {
                    if let Ok(mut users) = s.user_manager.try_lock() {
                        if users.set_user_enabled(&uname, new_enabled).is_ok() {
                            let _ = users.save(&s.users_path);
                        }
                    }
                }
                refresh_user_list(&store, &state);
                log::info!("User {} enabled status toggled to {}", uname, new_enabled);
            });
        }
    }));

    let state_clone = Arc::clone(state);
    let store_clone = store.clone();
    refresh_btn.connect_clicked(clone!(@strong state_clone, @strong store_clone => move |_| {
        refresh_user_list(&store_clone, &state_clone);
    }));

    container
}

fn get_default_home_dir(username: &str) -> String {
    if let Ok(home) = std::env::var("HOME") {
        format!("{}/Desktop/文件共享", home)
    } else {
        format!("/home/{}/Desktop/文件共享", username)
    }
}

fn refresh_user_list(store: &ListStore, state: &Arc<StdMutex<AppState>>) {
    store.clear();
    if let Ok(s) = state.try_lock() {
        if let Ok(users) = s.user_manager.try_lock() {
            for (username, user) in users.list_users() {
                let iter = store.append();
                store.set_value(&iter, 0, &username.to_value());
                store.set_value(&iter, 1, &user.home_dir.to_value());
                store.set_value(&iter, 2, &user.enabled.to_value());
                store.set_value(&iter, 3, &user.permissions.to_string().to_value());
            }
        }
    }
}
