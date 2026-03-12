use gtk::prelude::*;
use gtk::{
    Box, Orientation, Label, Button, Entry, Frame, ScrolledWindow, TreeView, ListStore,
    CellRendererText, TreeViewColumn,
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
    let add_box = Box::new(Orientation::Horizontal, 5);
    add_box.set_margin_top(10);
    add_box.set_margin_bottom(10);
    add_box.set_margin_start(10);
    add_box.set_margin_end(10);

    add_box.pack_start(&Label::new(Some("用户名:")), false, false, 0);
    let username_entry = Entry::new();
    username_entry.set_width_chars(15);
    add_box.pack_start(&username_entry, false, false, 0);

    add_box.pack_start(&Label::new(Some("密码:")), false, false, 0);
    let password_entry = Entry::new();
    password_entry.set_width_chars(15);
    password_entry.set_visibility(false);
    add_box.pack_start(&password_entry, false, false, 0);

    add_box.pack_start(&Label::new(Some("主目录:")), false, false, 0);
    let home_entry = Entry::new();
    home_entry.set_width_chars(20);
    home_entry.set_placeholder_text(Some("/home/user"));
    add_box.pack_start(&home_entry, false, false, 0);

    let add_btn = Button::with_label("添加");
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

    {
        let state = state.lock().unwrap();
        let users = state.user_manager.lock().unwrap();
        for (username, user) in users.list_users() {
            let iter = store.append();
            store.set_value(&iter, 0, &username.to_value());
            store.set_value(&iter, 1, &user.home_dir.to_value());
            store.set_value(&iter, 2, &user.enabled.to_value());
            store.set_value(&iter, 3, &user.permissions.to_string().to_value());
        }
    }

    let tree = TreeView::with_model(&store);

    for (i, title) in ["用户名", "主目录", "启用", "权限"].iter().enumerate() {
        let renderer = CellRendererText::new();
        let column = TreeViewColumn::new();
        column.set_title(title);
        gtk::prelude::CellLayoutExt::pack_start(&column, &renderer, true);
        gtk::prelude::CellLayoutExt::add_attribute(&column, &renderer, "text", i as i32);
        tree.append_column(&column);
    }

    scrolled.add(&tree);
    list_box.pack_start(&scrolled, true, true, 0);

    let delete_btn = Button::with_label("删除选中用户");
    list_box.pack_start(&delete_btn, false, false, 0);

    list_frame.add(&list_box);
    container.pack_start(&list_frame, true, true, 0);

    let state_clone = Arc::clone(state);
    let store_clone = store.clone();
    let username_entry_clone = username_entry.clone();
    let password_entry_clone = password_entry.clone();
    let home_entry_clone = home_entry.clone();
    add_btn.connect_clicked(
        clone!(@strong state_clone, @strong store_clone, @strong username_entry_clone, @strong password_entry_clone, @strong home_entry_clone => move |_| {
            let username = username_entry_clone.text();
            let password = password_entry_clone.text();
            let home = home_entry_clone.text();
            if !username.is_empty() && !password.is_empty() {
                let username_str = username.to_string();
                let password_str = password.to_string();
                let home_str = if home.is_empty() {
                    format!("/home/{}", username_str)
                } else {
                    home.to_string()
                };

                let state = state_clone.lock().unwrap();
                let _ = state.user_manager.lock().unwrap().add_user(&username_str, &password_str, &home_str, false);
                let _ = state.save_users();

                let iter = store_clone.append();
                store_clone.set_value(&iter, 0, &username_str.to_value());
                store_clone.set_value(&iter, 1, &home_str.to_value());
                store_clone.set_value(&iter, 2, &true.to_value());
                store_clone.set_value(&iter, 3, &"读写".to_value());

                username_entry_clone.set_text("");
                password_entry_clone.set_text("");
                home_entry_clone.set_text("");
                log::info!("User {} added", username_str);
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
                let state = state_clone.lock().unwrap();
                let _ = state.user_manager.lock().unwrap().remove_user(&username);
                let _ = state.save_users();
                store_clone.remove(&iter);
                log::info!("User {} deleted", username);
            }
        }),
    );

    container
}
