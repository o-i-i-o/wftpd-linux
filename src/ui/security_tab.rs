use gtk::prelude::*;
use gtk::{
    Box, Orientation, Label, Button, Entry, Frame, ScrolledWindow, TreeView, ListStore,
    CellRendererText, TreeViewColumn, SpinButton, Adjustment, glib,
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

    let title_label = Label::new(Some("<b>安全设置</b>"));
    title_label.set_use_markup(true);
    container.pack_start(&title_label, false, false, 0);

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
    if let Ok(s) = state.try_lock() {
        if let Ok(config) = s.config.try_lock() {
            max_attempts_spin.set_value(config.security.max_login_attempts as f64);
        }
    }
    row1.pack_start(&max_attempts_spin, false, false, 0);

    row1.pack_start(&Label::new(Some("封禁时长(秒):")), false, false, 0);
    let ban_duration_spin = create_spin_button(60.0, 86400.0, 60.0);
    if let Ok(s) = state.try_lock() {
        if let Ok(config) = s.config.try_lock() {
            ban_duration_spin.set_value(config.security.ban_duration as f64);
        }
    }
    row1.pack_start(&ban_duration_spin, false, false, 0);
    box_.pack_start(&row1, false, false, 0);

    let hint = Label::new(None);
    hint.set_markup("<i>提示: 超过最大登录尝试次数后，IP将被临时封禁指定时长</i>");
    box_.pack_start(&hint, false, false, 0);

    let save_btn = Button::with_label("保存登录安全设置");
    let state_clone = Arc::clone(state);
    let max_attempts_clone = max_attempts_spin.clone();
    let ban_duration_clone = ban_duration_spin.clone();
    save_btn.connect_clicked(
        clone!(@strong state_clone, @strong max_attempts_clone, @strong ban_duration_clone => move |_| {
            let state = Arc::clone(&state_clone);
            let max_attempts = max_attempts_clone.value() as u32;
            let ban_duration = ban_duration_clone.value() as u64;
            
            glib::MainContext::ref_thread_default().spawn_local(async move {
                if let Ok(s) = state.try_lock() {
                    if let Ok(mut config) = s.config.try_lock() {
                        config.security.max_login_attempts = max_attempts;
                        config.security.ban_duration = ban_duration;
                        let _ = config.save(&s.config_path);
                        log::info!("Login security settings saved");
                    }
                }
            });
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

    let delete_btn = Button::with_label("删除选中IP");
    ip_box.pack_start(&delete_btn, false, false, 0);

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

    let clear_btn = Button::with_label("清空黑名单");
    add_box.pack_start(&clear_btn, false, false, 0);

    ip_box.pack_start(&add_box, false, false, 0);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(80)
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

    let delete_btn = Button::with_label("删除选中IP");
    ip_box.pack_start(&delete_btn, false, false, 0);

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
         <b>优先级:</b> 黑名单优先于白名单"
    );
    mode_box.pack_start(&mode_label, false, false, 0);

    let hint_label = Label::new(None);
    hint_label.set_markup("<i>提示: 如果白名单为空或包含 0.0.0.0/0, 则允许所有IP访问</i>");
    mode_box.pack_start(&hint_label, false, false, 0);

    mode_frame.add(&mode_box);
    container.pack_start(&mode_frame, false, false, 0);
}

fn setup_whitelist_buttons(state: &Arc<StdMutex<AppState>>, widgets: &WhitelistWidgets) {
    let state_clone = Arc::clone(state);
    let store_clone = widgets.store.clone();
    let ip_entry_clone = widgets.ip_entry.clone();
    widgets.add_btn.connect_clicked(clone!(@strong state_clone, @strong store_clone, @strong ip_entry_clone => move |_| {
        let ip = ip_entry_clone.text();
        if !ip.is_empty() {
            let ip_str = ip.to_string();
            if !validate_ip_or_cidr(&ip_str) {
                log::warn!("Invalid IP or CIDR format: {}", ip_str);
                return;
            }
            let state = Arc::clone(&state_clone);
            let store = store_clone.clone();
            let ip_entry = ip_entry_clone.clone();
            let ip_to_add = ip_str.clone();
            
            glib::MainContext::ref_thread_default().spawn_local(async move {
                if let Ok(s) = state.try_lock() {
                    if let Ok(mut config) = s.config.try_lock() {
                        if !config.security.allowed_ips.contains(&ip_to_add) {
                            config.security.allowed_ips.push(ip_to_add.clone());
                            let _ = config.save(&s.config_path);
                            refresh_whitelist(&store, &state);
                            ip_entry.set_text("");
                            log::info!("IP {} added to whitelist", ip_to_add);
                        }
                    }
                }
            });
        }
    }));

    let state_clone = Arc::clone(state);
    let store_clone = widgets.store.clone();
    let tree_clone = widgets.tree.clone();
    widgets.delete_btn.connect_clicked(clone!(@strong state_clone, @strong store_clone, @strong tree_clone => move |_| {
        let selection = tree_clone.selection();
        if let Some((model, iter)) = selection.selected() {
            let ip: String = model.value(&iter, 0).get().unwrap_or_default();
            let state = Arc::clone(&state_clone);
            let store = store_clone.clone();
            let ip_to_remove = ip.clone();
            
            glib::MainContext::ref_thread_default().spawn_local(async move {
                if let Ok(s) = state.try_lock() {
                    if let Ok(mut config) = s.config.try_lock() {
                        config.security.allowed_ips.retain(|x| x != &ip_to_remove);
                        let _ = config.save(&s.config_path);
                        refresh_whitelist(&store, &state);
                        log::info!("IP {} removed from whitelist", ip_to_remove);
                    }
                }
            });
        }
    }));

    let state_clone = Arc::clone(state);
    let store_clone = widgets.store.clone();
    widgets.clear_btn.connect_clicked(clone!(@strong state_clone, @strong store_clone => move |_| {
        let state = Arc::clone(&state_clone);
        let store = store_clone.clone();
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            if let Ok(s) = state.try_lock() {
                if let Ok(mut config) = s.config.try_lock() {
                    config.security.allowed_ips.clear();
                    let _ = config.save(&s.config_path);
                    refresh_whitelist(&store, &state);
                    log::info!("Whitelist cleared");
                }
            }
        });
    }));

    let state_clone = Arc::clone(state);
    let store_clone = widgets.store.clone();
    widgets.allow_all_btn.connect_clicked(clone!(@strong state_clone, @strong store_clone => move |_| {
        let state = Arc::clone(&state_clone);
        let store = store_clone.clone();
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            if let Ok(s) = state.try_lock() {
                if let Ok(mut config) = s.config.try_lock() {
                    config.security.allowed_ips = vec!["0.0.0.0/0".to_string()];
                    let _ = config.save(&s.config_path);
                    refresh_whitelist(&store, &state);
                    log::info!("Allow all IPs set");
                }
            }
        });
    }));
}

fn setup_blacklist_buttons(state: &Arc<StdMutex<AppState>>, widgets: &BlacklistWidgets) {
    let state_clone = Arc::clone(state);
    let store_clone = widgets.store.clone();
    let ip_entry_clone = widgets.ip_entry.clone();
    widgets.add_btn.connect_clicked(clone!(@strong state_clone, @strong store_clone, @strong ip_entry_clone => move |_| {
        let ip = ip_entry_clone.text();
        if !ip.is_empty() {
            let ip_str = ip.to_string();
            if !validate_ip_or_cidr(&ip_str) {
                log::warn!("Invalid IP or CIDR format: {}", ip_str);
                return;
            }
            let state = Arc::clone(&state_clone);
            let store = store_clone.clone();
            let ip_entry = ip_entry_clone.clone();
            let ip_to_add = ip_str.clone();
            
            glib::MainContext::ref_thread_default().spawn_local(async move {
                if let Ok(s) = state.try_lock() {
                    if let Ok(mut config) = s.config.try_lock() {
                        if !config.security.denied_ips.contains(&ip_to_add) {
                            config.security.denied_ips.push(ip_to_add.clone());
                            let _ = config.save(&s.config_path);
                            refresh_blacklist(&store, &state);
                            ip_entry.set_text("");
                            log::info!("IP {} added to blacklist", ip_to_add);
                        }
                    }
                }
            });
        }
    }));

    let state_clone = Arc::clone(state);
    let store_clone = widgets.store.clone();
    let tree_clone = widgets.tree.clone();
    widgets.delete_btn.connect_clicked(clone!(@strong state_clone, @strong store_clone, @strong tree_clone => move |_| {
        let selection = tree_clone.selection();
        if let Some((model, iter)) = selection.selected() {
            let ip: String = model.value(&iter, 0).get().unwrap_or_default();
            let state = Arc::clone(&state_clone);
            let store = store_clone.clone();
            let ip_to_remove = ip.clone();
            
            glib::MainContext::ref_thread_default().spawn_local(async move {
                if let Ok(s) = state.try_lock() {
                    if let Ok(mut config) = s.config.try_lock() {
                        config.security.denied_ips.retain(|x| x != &ip_to_remove);
                        let _ = config.save(&s.config_path);
                        refresh_blacklist(&store, &state);
                        log::info!("IP {} removed from blacklist", ip_to_remove);
                    }
                }
            });
        }
    }));

    let state_clone = Arc::clone(state);
    let store_clone = widgets.store.clone();
    widgets.clear_btn.connect_clicked(clone!(@strong state_clone, @strong store_clone => move |_| {
        let state = Arc::clone(&state_clone);
        let store = store_clone.clone();
        
        glib::MainContext::ref_thread_default().spawn_local(async move {
            if let Ok(s) = state.try_lock() {
                if let Ok(mut config) = s.config.try_lock() {
                    config.security.denied_ips.clear();
                    let _ = config.save(&s.config_path);
                    refresh_blacklist(&store, &state);
                    log::info!("Blacklist cleared");
                }
            }
        });
    }));
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
    if let Ok(s) = state.try_lock() {
        if let Ok(config) = s.config.try_lock() {
            for ip in &config.security.allowed_ips {
                let iter = store.append();
                store.set_value(&iter, 0, &ip.to_value());
            }
        }
    }
}

fn refresh_blacklist(store: &ListStore, state: &Arc<StdMutex<AppState>>) {
    store.clear();
    if let Ok(s) = state.try_lock() {
        if let Ok(config) = s.config.try_lock() {
            for ip in &config.security.denied_ips {
                let iter = store.append();
                store.set_value(&iter, 0, &ip.to_value());
            }
        }
    }
}