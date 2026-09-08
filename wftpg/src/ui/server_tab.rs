use crate::AppState;
use gtk::prelude::*;
use gtk::{Box, Frame, Label, Orientation};
use std::sync::{Arc, Mutex as StdMutex};

pub fn create(state: &Arc<StdMutex<AppState>>) -> Box {
    let container = Box::new(Orientation::Vertical, 10);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);

    create_server_control_frame(&container, state);

    container
}

fn create_server_control_frame(container: &Box, _state: &Arc<StdMutex<AppState>>) {
    let frame = Frame::new(Some("服务控制"));
    let main_box = Box::new(Orientation::Vertical, 10);
    main_box.set_margin_top(10);
    main_box.set_margin_bottom(10);
    main_box.set_margin_start(10);
    main_box.set_margin_end(10);

    // FTP/SFTP 服务启停由 systemd 管理，通过配置文件控制
    let info_label = Label::new(None);
    info_label.set_markup(
        "<b>FTP/SFTP 服务管理说明</b>\n\n\
         服务启停由 systemd 统一管理，通过配置文件 <tt>/etc/wftpg/config.toml</tt> 控制：\n\n\
         • <b>启用/禁用 FTP:</b> 修改配置文件中 <tt>[ftp]</tt> 部分的 <tt>enabled = true/false</tt>\n\
         • <b>启用/禁用 SFTP:</b> 修改配置文件中 <tt>[sftp]</tt> 部分的 <tt>enabled = true/false</tt>\n\n\
         <b>使用方式:</b>\n\
         1. 修改配置文件中的 enabled 设置\n\
         2. 保存配置文件\n\
         3. 重启服务：<tt>sudo systemctl restart wftpd</tt>\n\
         4. 查看状态：<tt>systemctl status wftpd</tt>\n\n\
         <i>注意：修改配置后必须重启服务才能生效</i>"
    );
    main_box.pack_start(&info_label, false, false, 0);

    frame.add(&main_box);
    container.pack_start(&frame, false, false, 0);
}
