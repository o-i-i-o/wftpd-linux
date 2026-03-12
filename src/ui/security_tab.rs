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

    let title_label = Label::new(Some("<b>安全设置</b>"));
    title_label.set_use_markup(true);
    container.pack_start(&title_label, false, false, 0);

    create_ip_whitelist_frame(&container, state);
    create_security_mode_frame(&container);

    container
}

fn create_ip_whitelist_frame(container: &Box, state: &Arc<StdMutex<AppState>>) {
    let ip_frame = Frame::new(Some("IP白名单"));
    let ip_box = Box::new(Orientation::Vertical, 5);
    ip_box.set_margin_top(10);
    ip_box.set_margin_bottom(10);
    ip_box.set_margin_start(10);
    ip_box.set_margin_end(10);

    let add_box = Box::new(Orientation::Horizontal, 5);
    add_box.pack_start(&Label::new(Some("IP地址:")), false, false, 0);
    let ip_entry = Entry::new();
    ip_entry.set_width_chars(20);
    ip_entry.set_placeholder_text(Some("例如: 192.168.1.100"));
    add_box.pack_start(&ip_entry, false, false, 0);

    let add_btn = Button::with_label("添加");
    add_box.pack_start(&add_btn, false, false, 0);
    ip_box.pack_start(&add_box, false, false, 0);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(150)
        .build();

    let store = ListStore::new(&[gtk::glib::Type::STRING]);

    {
        let state = state.lock().unwrap();
        let config = state.config.lock().unwrap();
        for ip in &config.security.allowed_ips {
            let iter = store.append();
            store.set_value(&iter, 0, &ip.to_value());
        }
    }

    let tree = TreeView::with_model(&store);
    let renderer = CellRendererText::new();
    let column = TreeViewColumn::new();
    column.set_title("IP地址");
    gtk::prelude::CellLayoutExt::pack_start(&column, &renderer, true);
    gtk::prelude::CellLayoutExt::add_attribute(&column, &renderer, "text", 0);
    tree.append_column(&column);

    scrolled.add(&tree);
    ip_box.pack_start(&scrolled, true, true, 0);

    let delete_btn = Button::with_label("删除选中IP");
    ip_box.pack_start(&delete_btn, false, false, 0);

    ip_frame.add(&ip_box);
    container.pack_start(&ip_frame, true, true, 0);

    setup_ip_buttons(state, &ip_entry, &store, &tree, &add_btn, &delete_btn);
}

fn setup_ip_buttons(
    state: &Arc<StdMutex<AppState>>,
    ip_entry: &Entry,
    store: &ListStore,
    tree: &TreeView,
    add_btn: &Button,
    delete_btn: &Button,
) {
    let state_clone = Arc::clone(state);
    let store_clone = store.clone();
    let ip_entry_clone = ip_entry.clone();
    add_btn.connect_clicked(clone!(@strong state_clone, @strong store_clone, @strong ip_entry_clone => move |_| {
        let ip = ip_entry_clone.text();
        if !ip.is_empty() {
            let ip_str = ip.to_string();
            let state = state_clone.lock().unwrap();
            let mut config = state.config.lock().unwrap();
            if !config.security.allowed_ips.contains(&ip_str) {
                config.security.allowed_ips.push(ip_str.clone());
                let _ = state.save_config();

                let iter = store_clone.append();
                store_clone.set_value(&iter, 0, &ip_str.to_value());

                ip_entry_clone.set_text("");
                log::info!("IP {} added to whitelist", ip_str);
            }
        }
    }));

    let state_clone = Arc::clone(state);
    let store_clone = store.clone();
    let tree_clone = tree.clone();
    delete_btn.connect_clicked(clone!(@strong state_clone, @strong store_clone, @strong tree_clone => move |_| {
        let selection = tree_clone.selection();
        if let Some((model, iter)) = selection.selected() {
            let ip: String = model.value(&iter, 0).get().unwrap_or_default();
            let state = state_clone.lock().unwrap();
            let mut config = state.config.lock().unwrap();
            config.security.allowed_ips.retain(|x| x != &ip);
            let _ = state.save_config();
            store_clone.remove(&iter);
            log::info!("IP {} removed from whitelist", ip);
        }
    }));
}

fn create_security_mode_frame(container: &Box) {
    let mode_frame = Frame::new(Some("安全模式"));
    let mode_box = Box::new(Orientation::Vertical, 5);
    mode_box.set_margin_top(10);
    mode_box.set_margin_bottom(10);
    mode_box.set_margin_start(10);
    mode_box.set_margin_end(10);

    let mode_label = Label::new(Some("白名单模式: 仅允许列表中的IP访问服务器"));
    mode_box.pack_start(&mode_label, false, false, 0);

    let hint_label = Label::new(None);
    hint_label.set_markup("<i>提示: 如果白名单为空, 则允许所有IP访问</i>");
    mode_box.pack_start(&hint_label, false, false, 0);

    mode_frame.add(&mode_box);
    container.pack_start(&mode_frame, false, false, 0);
}
