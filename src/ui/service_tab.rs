use gtk::prelude::*;
use gtk::{Box, Orientation, Label};
use std::sync::{Arc, Mutex as StdMutex};
use crate::AppState;

pub fn create(_state: &Arc<StdMutex<AppState>>) -> Box {
    let container = Box::new(Orientation::Vertical, 10);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);

    // 服务管理功能已移除，wftpd 服务由 systemd 统一管理
    // 前端将通过 systemctl 命令直接管理服务
    
    let info_label = Label::new(None);
    info_label.set_markup(
        "<b>服务管理说明</b>\n\n\
         wftpd 服务已由 systemd 统一管理。\n\n\
         <b>使用方式:</b>\n\
         • 启动服务：<tt>sudo systemctl start wftpd</tt>\n\
         • 停止服务：<tt>sudo systemctl stop wftpd</tt>\n\
         • 重启服务：<tt>sudo systemctl restart wftpd</tt>\n\
         • 查看状态：<tt>systemctl status wftpd</tt>\n\
         • 开机自启：<tt>sudo systemctl enable wftpd</tt>\n\
         • 禁用自启：<tt>sudo systemctl disable wftpd</tt>\n\n\
         <i>注意：服务配置文件位于 /etc/wftpg/config.toml</i>"
    );
    container.pack_start(&info_label, false, false, 0);

    container
}
