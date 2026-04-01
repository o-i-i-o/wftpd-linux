use gtk::prelude::*;
use gtk::Application;

fn main() {
    let app = Application::builder()
        .application_id("com.wftpg")
        .build();

    app.connect_activate(|app| {
        wftpg::ui::build_ui(app);
    });

    app.run();
}
