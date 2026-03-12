use gtk::prelude::*;
use gtk::{Box, Orientation, Label, Button, TextView, ScrolledWindow};
use gtk::glib::clone;
use std::sync::{Arc, Mutex as StdMutex};
use wftpg::AppState;

pub fn create(state: &Arc<StdMutex<AppState>>) -> Box {
    let container = Box::new(Orientation::Vertical, 10);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);

    let title_label = Label::new(Some("<b>日志查看</b>"));
    title_label.set_use_markup(true);
    container.pack_start(&title_label, false, false, 0);

    let (refresh_btn, clear_btn) = create_control_buttons(&container);
    let text_view = create_log_view(&container, state);

    setup_button_handlers(state, &text_view, &refresh_btn, &clear_btn);

    container
}

fn create_control_buttons(container: &Box) -> (Button, Button) {
    let control_box = Box::new(Orientation::Horizontal, 10);

    let refresh_btn = Button::with_label("刷新日志");
    let clear_btn = Button::with_label("清空显示");

    control_box.pack_start(&refresh_btn, false, false, 0);
    control_box.pack_start(&clear_btn, false, false, 0);
    container.pack_start(&control_box, false, false, 0);

    (refresh_btn, clear_btn)
}

fn create_log_view(container: &Box, state: &Arc<StdMutex<AppState>>) -> TextView {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();

    let text_view = TextView::new();
    text_view.set_editable(false);
    text_view.set_monospace(true);
    text_view.set_left_margin(10);
    text_view.set_right_margin(10);
    text_view.set_top_margin(10);
    text_view.set_bottom_margin(10);

    populate_log_view(&text_view, state);

    scrolled.add(&text_view);
    container.pack_start(&scrolled, true, true, 0);

    text_view
}

fn format_log_entries(state: &Arc<StdMutex<AppState>>) -> String {
    let state = state.lock().unwrap();
    let logger = state.logger.lock().unwrap();
    let entries = logger.get_recent_logs(100);
    
    if entries.is_empty() {
        return "暂无日志记录".to_string();
    }
    
    let mut text = String::with_capacity(entries.len() * 100);
    for entry in entries {
        text.push_str(&format!(
            "[{}] {} - {} - {} - {}\n",
            entry.timestamp.format("%Y-%m-%d %H:%M:%S"),
            entry.level,
            entry.source,
            entry.message,
            entry.action.unwrap_or_default()
        ));
    }
    text
}

fn populate_log_view(text_view: &TextView, state: &Arc<StdMutex<AppState>>) {
    let text = format_log_entries(state);
    if let Some(buffer) = text_view.buffer() {
        buffer.set_text(&text);
    }
}

fn setup_button_handlers(
    state: &Arc<StdMutex<AppState>>,
    text_view: &TextView,
    refresh_btn: &Button,
    clear_btn: &Button,
) {
    let state_clone = Arc::clone(state);
    let text_view_clone = text_view.clone();
    refresh_btn.connect_clicked(clone!(@strong state_clone, @strong text_view_clone => move |_| {
        let text = format_log_entries(&state_clone);
        if let Some(buffer) = text_view_clone.buffer() {
            buffer.set_text(&text);
        }
    }));

    let text_view_clone = text_view.clone();
    clear_btn.connect_clicked(clone!(@strong text_view_clone => move |_| {
        if let Some(buffer) = text_view_clone.buffer() {
            buffer.set_text("");
        }
    }));
}
