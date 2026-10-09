//! 用户管理 Tab：用户列表、操作按钮与启停切换。
//!
//! 对话框逻辑见 [`super::user_dialog`]，推荐目录与共享目录权限见
//! [`super::suggested_dirs`]。

use crate::AppState;
use gtk::glib::clone;
use gtk::prelude::*;
use gtk::{
    Box, Button, CellRendererText, CellRendererToggle, ListStore, Orientation, ScrolledWindow,
    TreeView, TreeViewColumn,
};
use std::sync::{Arc, Mutex as StdMutex};
use tracing::{error, info};

use super::user_dialog::{show_confirm_dialog, show_user_dialog};

pub fn create(state: &Arc<StdMutex<AppState>>) -> Box {
    let container = Box::new(Orientation::Vertical, 10);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);

    let list_frame = gtk::Frame::new(Some("用户列表"));
    let list_box = Box::new(Orientation::Vertical, 5);
    list_box.set_margin_top(10);
    list_box.set_margin_bottom(10);
    list_box.set_margin_start(10);
    list_box.set_margin_end(10);

    let store = ListStore::new(&[
        gtk::glib::Type::STRING,
        gtk::glib::Type::STRING,
        gtk::glib::Type::BOOL,
        gtk::glib::Type::STRING,
        gtk::glib::Type::STRING,
    ]);

    refresh_user_list(&store, state);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(300)
        .build();

    let tree = build_user_tree(&store);
    scrolled.add(&tree);
    list_box.pack_start(&scrolled, true, true, 0);

    let (add_btn, edit_btn, delete_btn, toggle_btn, refresh_btn) = user_action_buttons();
    list_box.pack_start(
        &btn_box_from(&add_btn, &edit_btn, &delete_btn, &toggle_btn, &refresh_btn),
        false,
        false,
        0,
    );

    list_frame.add(&list_box);
    container.pack_start(&list_frame, true, true, 0);

    let state_clone = Arc::clone(state);
    let store_clone = store.clone();
    add_btn.connect_clicked(
        clone!(#[strong] state_clone, #[strong] store_clone, move |_| {
            show_user_dialog(None, &state_clone, &store_clone);
        }),
    );

    let state_clone = Arc::clone(state);
    let store_clone = store.clone();
    let tree_clone = tree.clone();
    edit_btn.connect_clicked(
        clone!(#[strong] state_clone, #[strong] store_clone, #[strong] tree_clone, move |_| {
            if let Some((username, _)) = selected_user(&tree_clone) {
                show_user_dialog(Some(&username), &state_clone, &store_clone);
            }
        }),
    );

    let state_clone = Arc::clone(state);
    let store_clone = store.clone();
    let tree_clone = tree.clone();
    delete_btn.connect_clicked(
        clone!(#[strong] state_clone, #[strong] store_clone, #[strong] tree_clone, move |_| {
            if let Some((username, _)) = selected_user(&tree_clone) {
                show_confirm_dialog(&username, &state_clone, &store_clone);
            }
        }),
    );

    let state_clone = Arc::clone(state);
    let store_clone = store.clone();
    let tree_clone = tree.clone();
    toggle_btn.connect_clicked(
        clone!(#[strong] state_clone, #[strong] store_clone, #[strong] tree_clone, move |_| {
            if let Some((username, current_enabled)) = selected_user(&tree_clone) {
                toggle_selected_user(&state_clone, &store_clone, &username, current_enabled);
            }
        }),
    );

    let state_clone = Arc::clone(state);
    let store_clone = store.clone();
    refresh_btn.connect_clicked(
        clone!(#[strong] state_clone, #[strong] store_clone, move |_| {
            refresh_user_list(&store_clone, &state_clone);
        }),
    );

    container
}

/// 构建用户列表的五个列（用户名/主目录/启用/配额/权限）
fn build_user_tree(store: &ListStore) -> TreeView {
    let tree = TreeView::with_model(store);

    for (title, kind, column, min_width) in [
        ("用户名", CellKind::Text, 0, 100),
        ("主目录", CellKind::Text, 1, 200),
        ("启用", CellKind::Toggle, 2, 0),
        ("配额", CellKind::Text, 4, 80),
        ("权限", CellKind::Text, 3, 0),
    ] {
        let col = TreeViewColumn::new();
        col.set_title(title);
        col.set_resizable(kind == CellKind::Text);
        if min_width > 0 {
            col.set_min_width(min_width);
        }
        match kind {
            CellKind::Text => {
                let renderer = CellRendererText::new();
                gtk::prelude::CellLayoutExt::pack_start(&col, &renderer, true);
                gtk::prelude::CellLayoutExt::add_attribute(&col, &renderer, "text", column);
            }
            CellKind::Toggle => {
                let renderer = CellRendererToggle::new();
                gtk::prelude::CellLayoutExt::pack_start(&col, &renderer, true);
                gtk::prelude::CellLayoutExt::add_attribute(&col, &renderer, "active", column);
            }
        }
        tree.append_column(&col);
    }

    tree
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CellKind {
    Text,
    Toggle,
}

/// 工具栏的五个操作按钮
fn user_action_buttons() -> (Button, Button, Button, Button, Button) {
    (
        Button::with_label("新建用户"),
        Button::with_label("编辑用户"),
        Button::with_label("删除用户"),
        Button::with_label("启用/禁用"),
        Button::with_label("刷新列表"),
    )
}

fn btn_box_from(
    add_btn: &Button,
    edit_btn: &Button,
    delete_btn: &Button,
    toggle_btn: &Button,
    refresh_btn: &Button,
) -> Box {
    let btn_box = Box::new(Orientation::Horizontal, 10);
    btn_box.pack_start(add_btn, false, false, 0);
    btn_box.pack_start(edit_btn, false, false, 0);
    btn_box.pack_start(delete_btn, false, false, 0);
    btn_box.pack_start(toggle_btn, false, false, 0);
    btn_box.pack_start(refresh_btn, false, false, 0);
    btn_box
}

/// 返回当前选中行的 (用户名, 是否启用)；未选中时返回 `None`
fn selected_user(tree: &TreeView) -> Option<(String, bool)> {
    let selection = tree.selection();
    let (model, iter) = selection.selected()?;
    let username: String = model.value(&iter, 0).get().unwrap_or_default();
    let enabled: bool = model.value(&iter, 2).get().unwrap_or(true);
    Some((username, enabled))
}

/// 切换用户启用状态并落盘、刷新列表
fn toggle_selected_user(
    state: &Arc<StdMutex<AppState>>,
    store: &ListStore,
    username: &str,
    current_enabled: bool,
) {
    let new_enabled = !current_enabled;

    let users_json = {
        if let Ok(s) = state.try_lock() {
            if let Ok(mut users) = s.user_manager.try_lock() {
                if users.set_user_enabled(username, new_enabled).is_ok() {
                    serde_json::to_string(&*users).unwrap_or_default()
                } else {
                    return;
                }
            } else {
                return;
            }
        } else {
            return;
        }
    };

    match crate::communication::write_users(&users_json) {
        Ok(_) => {
            let action = if new_enabled { "enabled" } else { "disabled" };
            let _ = crate::communication::write_audit_log(
                "gui-user",
                "USER_TOGGLE",
                username,
                &format!("User {username} status changed to {action}"),
            );
            info!(
                "User {} enabled status toggled to {}",
                username, new_enabled
            );
        }
        Err(e) => {
            error!("Failed to save users: {}", e);
        }
    }
    refresh_user_list(store, state);
}

/// 用当前用户库重建列表内容（供本模块与用户对话框共用）
pub(super) fn refresh_user_list(store: &ListStore, state: &Arc<StdMutex<AppState>>) {
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
