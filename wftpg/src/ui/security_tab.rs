use crate::AppState;
use gtk::glib::clone;
use gtk::prelude::*;
use gtk::{
    Adjustment, Box, Button, CellRendererText, Entry, Frame, Label, ListStore, Orientation,
    ScrolledWindow, SpinButton, TreeView, TreeViewColumn,
};
use std::sync::{Arc, Mutex as StdMutex};
use tracing::{error, info, warn};

pub fn create(state: &Arc<StdMutex<AppState>>) -> Box {
    let container = Box::new(Orientation::Vertical, 10);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);

    create_login_security_frame(&container, state);
    create_ip_whitelist_frame(&container, state);
    create_ip_blacklist_frame(&container, state);
    create_security_mode_frame(&container);

    container
}

fn create_login_security_frame(container: &Box, state: &Arc<StdMutex<AppState>>) {
    let frame = Frame::new(Some("登录安全"));
    let box_ = Box::new(Orientation::Vertical, 5);
    box_.set_margin_top(10);
    box_.set_margin_bottom(10);
    box_.set_margin_start(10);
    box_.set_margin_end(10);

    let row1 = Box::new(Orientation::Horizontal, 5);
    row1.pack_start(&Label::new(Some("最大登录尝试次数:")), false, false, 0);
    let max_attempts_spin = create_spin_button(1.0, 20.0, 1.0);
    if let Ok(s) = state.try_lock()
        && let Ok(config) = s.config.try_lock()
    {
        max_attempts_spin.set_value(f64::from(config.security.max_login_attempts));
    }
    row1.pack_start(&max_attempts_spin, false, false, 0);

    row1.pack_start(&Label::new(Some("封禁时长(秒):")), false, false, 0);
    let ban_duration_spin = create_spin_button(60.0, 86400.0, 60.0);
    if let Ok(s) = state.try_lock()
        && let Ok(config) = s.config.try_lock()
    {
        ban_duration_spin.set_value(super::utils::spin_f64_from(config.security.ban_duration));
    }
    row1.pack_start(&ban_duration_spin, false, false, 0);
    box_.pack_start(&row1, false, false, 0);

    let hint = Label::new(None);
    hint.set_markup("<i>提示: 超过最大登录尝试次数后，IP将被临时封禁指定时长</i>");
    box_.pack_start(&hint, false, false, 0);

    let save_btn = Button::with_label("保存设置");
    let state_clone = Arc::clone(state);
    let max_attempts_clone = max_attempts_spin.clone();
    let ban_duration_clone = ban_duration_spin.clone();
    save_btn.connect_clicked(
        clone!(@strong state_clone, @strong max_attempts_clone, @strong ban_duration_clone => move |_| {
            let max_attempts = super::utils::spin_u32(max_attempts_clone.value());
            let ban_duration = super::utils::spin_u64(ban_duration_clone.value());

            let config_str = {
                if let Ok(s) = state_clone.try_lock() {
                    if let Ok(mut config) = s.config.try_lock() {
                        config.security.max_login_attempts = max_attempts;
                        config.security.ban_duration = ban_duration;
                        toml::to_string_pretty(&*config).unwrap_or_default()
                    } else { return; }
                } else { return; }
            };

            match crate::communication::write_config(&config_str) {
                Ok(saved_content) => {
                    if let Ok(s) = state_clone.try_lock()
                        && let Ok(mut cfg) = s.config.try_lock()
                            && let Ok(new_config) = toml::from_str(&saved_content) {
                                *cfg = new_config;
                            }
                    let _ = crate::communication::write_audit_log(
                        "gui-security",
                        "SECURITY_CONFIG",
                        "login_settings",
                        &format!("Login security updated: max_attempts={max_attempts}, ban_duration={ban_duration}s")
                    );
                    info!("Login security settings saved");
                }
                Err(e) => {
                    error!("Failed to save security settings: {}", e);
                }
            }
        }),
    );
    box_.pack_start(&save_btn, false, false, 0);

    frame.add(&box_);
    container.pack_start(&frame, false, false, 0);
}

struct WhitelistWidgets<'a> {
    ip_entry: &'a Entry,
    store: &'a ListStore,
    tree: &'a TreeView,
    add_btn: &'a Button,
    delete_btn: &'a Button,
    clear_btn: &'a Button,
    allow_all_btn: &'a Button,
}

fn create_ip_whitelist_frame(container: &Box, state: &Arc<StdMutex<AppState>>) {
    let ip_frame = Frame::new(Some("IP白名单"));
    let ip_box = Box::new(Orientation::Vertical, 5);
    ip_box.set_margin_top(10);
    ip_box.set_margin_bottom(10);
    ip_box.set_margin_start(10);
    ip_box.set_margin_end(10);

    let add_box = Box::new(Orientation::Horizontal, 5);
    add_box.pack_start(&Label::new(Some("IP/CIDR:")), false, false, 0);
    let ip_entry = Entry::new();
    ip_entry.set_width_chars(25);
    ip_entry.set_placeholder_text(Some("例如: 192.168.1.0/24 或 192.168.1.100"));
    add_box.pack_start(&ip_entry, false, false, 0);

    let add_btn = Button::with_label("添加");
    add_box.pack_start(&add_btn, false, false, 0);

    let delete_btn = Button::with_label("删除选中IP");
    add_box.pack_start(&delete_btn, false, false, 0);

    let clear_btn = Button::with_label("清空白名单");
    add_box.pack_start(&clear_btn, false, false, 0);

    let allow_all_btn = Button::with_label("允许所有IP");
    add_box.pack_start(&allow_all_btn, false, false, 0);

    ip_box.pack_start(&add_box, false, false, 0);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(100)
        .build();

    let store = ListStore::new(&[gtk::glib::Type::STRING]);

    refresh_whitelist(&store, state);

    let tree = TreeView::with_model(&store);
    let renderer = CellRendererText::new();
    let column = TreeViewColumn::new();
    column.set_title("白名单IP/CIDR");
    gtk::prelude::CellLayoutExt::pack_start(&column, &renderer, true);
    gtk::prelude::CellLayoutExt::add_attribute(&column, &renderer, "text", 0);
    tree.append_column(&column);

    scrolled.add(&tree);
    ip_box.pack_start(&scrolled, true, true, 0);

    ip_frame.add(&ip_box);
    container.pack_start(&ip_frame, true, true, 0);

    let widgets = WhitelistWidgets {
        ip_entry: &ip_entry,
        store: &store,
        tree: &tree,
        add_btn: &add_btn,
        delete_btn: &delete_btn,
        clear_btn: &clear_btn,
        allow_all_btn: &allow_all_btn,
    };
    setup_whitelist_buttons(state, &widgets);
}

struct BlacklistWidgets<'a> {
    ip_entry: &'a Entry,
    store: &'a ListStore,
    tree: &'a TreeView,
    add_btn: &'a Button,
    delete_btn: &'a Button,
    clear_btn: &'a Button,
}

fn create_ip_blacklist_frame(container: &Box, state: &Arc<StdMutex<AppState>>) {
    let ip_frame = Frame::new(Some("IP黑名单"));
    let ip_box = Box::new(Orientation::Vertical, 5);
    ip_box.set_margin_top(10);
    ip_box.set_margin_bottom(10);
    ip_box.set_margin_start(10);
    ip_box.set_margin_end(10);

    let add_box = Box::new(Orientation::Horizontal, 5);
    add_box.pack_start(&Label::new(Some("IP/CIDR:")), false, false, 0);
    let ip_entry = Entry::new();
    ip_entry.set_width_chars(25);
    ip_entry.set_placeholder_text(Some("例如: 192.168.1.100"));
    add_box.pack_start(&ip_entry, false, false, 0);

    let add_btn = Button::with_label("添加");
    add_box.pack_start(&add_btn, false, false, 0);

    let delete_btn = Button::with_label("删除选中IP");
    add_box.pack_start(&delete_btn, false, false, 0);

    let clear_btn = Button::with_label("清空黑名单");
    add_box.pack_start(&clear_btn, false, false, 0);

    ip_box.pack_start(&add_box, false, false, 0);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(100)
        .build();

    let store = ListStore::new(&[gtk::glib::Type::STRING]);

    refresh_blacklist(&store, state);

    let tree = TreeView::with_model(&store);
    let renderer = CellRendererText::new();
    let column = TreeViewColumn::new();
    column.set_title("黑名单IP/CIDR");
    gtk::prelude::CellLayoutExt::pack_start(&column, &renderer, true);
    gtk::prelude::CellLayoutExt::add_attribute(&column, &renderer, "text", 0);
    tree.append_column(&column);

    scrolled.add(&tree);
    ip_box.pack_start(&scrolled, true, true, 0);

    ip_frame.add(&ip_box);
    container.pack_start(&ip_frame, true, true, 0);

    let widgets = BlacklistWidgets {
        ip_entry: &ip_entry,
        store: &store,
        tree: &tree,
        add_btn: &add_btn,
        delete_btn: &delete_btn,
        clear_btn: &clear_btn,
    };
    setup_blacklist_buttons(state, &widgets);
}

fn create_security_mode_frame(container: &Box) {
    let mode_frame = Frame::new(Some("安全模式说明"));
    let mode_box = Box::new(Orientation::Vertical, 5);
    mode_box.set_margin_top(10);
    mode_box.set_margin_bottom(10);
    mode_box.set_margin_start(10);
    mode_box.set_margin_end(10);

    let mode_label = Label::new(None);
    mode_label.set_markup(
        "<b>白名单模式:</b> 仅允许列表中的IP访问服务器\n\
         <b>黑名单模式:</b> 禁止列表中的IP访问服务器\n\
         <b>优先级:</b> 黑名单优先于白名单",
    );
    mode_box.pack_start(&mode_label, false, false, 0);

    let hint_label = Label::new(None);
    hint_label.set_markup("<i>提示: 如果白名单为空或包含 0.0.0.0/0, 则允许所有IP访问</i>");
    mode_box.pack_start(&hint_label, false, false, 0);

    mode_frame.add(&mode_box);
    container.pack_start(&mode_frame, false, false, 0);
}

/// 对安全 IP 列表应用 `mutate` 并落盘；成功后回读配置并刷新 UI 列表
///
/// 返回 `true` 表示保存成功；锁不可获取或保存失败（记录日志）时返回 `false`。
fn push_security_config(
    state: &Arc<StdMutex<AppState>>,
    store: &ListStore,
    refresh: fn(&ListStore, &Arc<StdMutex<AppState>>),
    context: &str,
    mutate: impl FnOnce(&mut wftpd_common::SecurityConfig),
) -> bool {
    let config_str = {
        if let Ok(s) = state.try_lock()
            && let Ok(mut config) = s.config.try_lock()
        {
            mutate(&mut config.security);
            toml::to_string_pretty(&*config).unwrap_or_default()
        } else {
            return false;
        }
    };

    match crate::communication::write_config(&config_str) {
        Ok(saved_content) => {
            if let Ok(s) = state.try_lock()
                && let Ok(mut cfg) = s.config.try_lock()
                && let Ok(new_config) = toml::from_str(&saved_content)
            {
                *cfg = new_config;
            }
            refresh(store, state);
            true
        }
        Err(e) => {
            error!("Failed to {context}: {e}");
            false
        }
    }
}

fn setup_whitelist_buttons(state: &Arc<StdMutex<AppState>>, widgets: &WhitelistWidgets) {
    let state_clone = Arc::clone(state);
    let store_clone = widgets.store.clone();
    let ip_entry_clone = widgets.ip_entry.clone();
    widgets.add_btn.connect_clicked(
        clone!(@strong state_clone, @strong store_clone, @strong ip_entry_clone => move |_| {
            let ip = ip_entry_clone.text().to_string();
            if ip.is_empty() {
                return;
            }
            if !validate_ip_or_cidr(&ip) {
                warn!("Invalid IP or CIDR format: {ip}");
                return;
            }
            let added = push_security_config(
                &state_clone,
                &store_clone,
                refresh_whitelist,
                "add IP to whitelist",
                |sec| {
                    if !sec.allowed_ips.contains(&ip) {
                        sec.allowed_ips.push(ip.clone());
                    }
                },
            );
            if added {
                ip_entry_clone.set_text("");
                info!("IP {ip} added to whitelist");
            }
        }),
    );

    let state_clone = Arc::clone(state);
    let store_clone = widgets.store.clone();
    let tree_clone = widgets.tree.clone();
    widgets.delete_btn.connect_clicked(
        clone!(@strong state_clone, @strong store_clone, @strong tree_clone => move |_| {
            let selection = tree_clone.selection();
            if let Some((model, iter)) = selection.selected() {
                let ip: String = model.value(&iter, 0).get().unwrap_or_default();
                let removed = push_security_config(
                    &state_clone,
                    &store_clone,
                    refresh_whitelist,
                    "remove IP from whitelist",
                    |sec| sec.allowed_ips.retain(|x| x != &ip),
                );
                if removed {
                    info!("IP {ip} removed from whitelist");
                }
            }
        }),
    );

    let state_clone = Arc::clone(state);
    let store_clone = widgets.store.clone();
    widgets.clear_btn.connect_clicked(
        clone!(@strong state_clone, @strong store_clone => move |_| {
            if push_security_config(
                &state_clone,
                &store_clone,
                refresh_whitelist,
                "clear whitelist",
                |sec| sec.allowed_ips.clear(),
            ) {
                info!("Whitelist cleared");
            }
        }),
    );

    let state_clone = Arc::clone(state);
    let store_clone = widgets.store.clone();
    widgets.allow_all_btn.connect_clicked(
        clone!(@strong state_clone, @strong store_clone => move |_| {
            if push_security_config(
                &state_clone,
                &store_clone,
                refresh_whitelist,
                "set allow all IPs",
                |sec| sec.allowed_ips = vec!["0.0.0.0/0".to_string()],
            ) {
                info!("Allow all IPs set");
            }
        }),
    );
}

fn setup_blacklist_buttons(state: &Arc<StdMutex<AppState>>, widgets: &BlacklistWidgets) {
    let state_clone = Arc::clone(state);
    let store_clone = widgets.store.clone();
    let ip_entry_clone = widgets.ip_entry.clone();
    widgets.add_btn.connect_clicked(
        clone!(@strong state_clone, @strong store_clone, @strong ip_entry_clone => move |_| {
            let ip = ip_entry_clone.text().to_string();
            if ip.is_empty() {
                return;
            }
            if !validate_ip_or_cidr(&ip) {
                warn!("Invalid IP or CIDR format: {ip}");
                return;
            }
            let added = push_security_config(
                &state_clone,
                &store_clone,
                refresh_blacklist,
                "add IP to blacklist",
                |sec| {
                    if !sec.denied_ips.contains(&ip) {
                        sec.denied_ips.push(ip.clone());
                    }
                },
            );
            if added {
                ip_entry_clone.set_text("");
                info!("IP {ip} added to blacklist");
            }
        }),
    );

    let state_clone = Arc::clone(state);
    let store_clone = widgets.store.clone();
    let tree_clone = widgets.tree.clone();
    widgets.delete_btn.connect_clicked(
        clone!(@strong state_clone, @strong store_clone, @strong tree_clone => move |_| {
            let selection = tree_clone.selection();
            if let Some((model, iter)) = selection.selected() {
                let ip: String = model.value(&iter, 0).get().unwrap_or_default();
                let removed = push_security_config(
                    &state_clone,
                    &store_clone,
                    refresh_blacklist,
                    "remove IP from blacklist",
                    |sec| sec.denied_ips.retain(|x| x != &ip),
                );
                if removed {
                    info!("IP {ip} removed from blacklist");
                }
            }
        }),
    );

    let state_clone = Arc::clone(state);
    let store_clone = widgets.store.clone();
    widgets.clear_btn.connect_clicked(
        clone!(@strong state_clone, @strong store_clone => move |_| {
            if push_security_config(
                &state_clone,
                &store_clone,
                refresh_blacklist,
                "clear blacklist",
                |sec| sec.denied_ips.clear(),
            ) {
                info!("Blacklist cleared");
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

fn validate_ip_or_cidr(input: &str) -> bool {
    use std::net::{Ipv4Addr, Ipv6Addr};

    if input == "0.0.0.0/0" || input == "::/0" {
        return true;
    }

    if input.parse::<Ipv4Addr>().is_ok() {
        return true;
    }
    if input.parse::<Ipv6Addr>().is_ok() {
        return true;
    }
    if input.parse::<ipnet::Ipv4Net>().is_ok() {
        return true;
    }
    if input.parse::<ipnet::Ipv6Net>().is_ok() {
        return true;
    }

    false
}

fn refresh_whitelist(store: &ListStore, state: &Arc<StdMutex<AppState>>) {
    store.clear();
    if let Ok(s) = state.try_lock()
        && let Ok(config) = s.config.try_lock()
    {
        for ip in &config.security.allowed_ips {
            let iter = store.append();
            store.set_value(&iter, 0, &ip.to_value());
        }
    }
}

fn refresh_blacklist(store: &ListStore, state: &Arc<StdMutex<AppState>>) {
    store.clear();
    if let Ok(s) = state.try_lock()
        && let Ok(config) = s.config.try_lock()
    {
        for ip in &config.security.denied_ips {
            let iter = store.append();
            store.set_value(&iter, 0, &ip.to_value());
        }
    }
}
