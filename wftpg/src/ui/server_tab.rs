//! 服务器配置页：FTP/SFTP 参数编辑。
//!
//! 配置文件位于用户 XDG 目录（`~/.config/wftpd/config.toml`），GUI 与后端同属
//! 桌面用户，具备读写权限。保存经后端 gRPC `SaveConfig` 完成校验与落盘，
//! 启用标志即时生效；绑定地址、端口等变更需重启对应协议服务后生效。

use crate::AppState;
use gtk::glib::clone;
use gtk::prelude::*;
use gtk::{
    Adjustment, Box, Button, CheckButton, ComboBoxText, Entry, Frame, Label, Orientation,
    SpinButton,
};
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};
use tracing::{error, info};

pub fn create(state: &Arc<StdMutex<AppState>>) -> Box {
    let container = Box::new(Orientation::Vertical, 10);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);

    create_ftp_config_frame(&container, state);
    create_sftp_config_frame(&container, state);

    let hint = Label::new(None);
    hint.set_markup(
        "<i>提示: 启用/禁用保存后立即生效；绑定地址、端口等变更需在“系统服务”页重启对应服务后生效</i>",
    );
    hint.set_halign(gtk::Align::Start);
    container.pack_start(&hint, false, false, 0);

    container
}

fn create_spin_button(min: f64, max: f64, step: f64) -> SpinButton {
    let adjustment = Adjustment::new(min, min, max, step, step * 10.0, 0.0);
    SpinButton::builder()
        .adjustment(&adjustment)
        .digits(0)
        .width_chars(6)
        .build()
}

/// 锁不可获取时返回 `None`（GUI 单线程使用锁，仅防御性处理）
fn with_config<R>(
    state: &Arc<StdMutex<AppState>>,
    f: impl FnOnce(&mut wftpd_common::Config) -> R,
) -> Option<R> {
    let state_guard = state.try_lock().ok()?;
    let mut cfg = state_guard.config.try_lock().ok()?;
    Some(f(&mut cfg))
}

// ---- FTP 配置 ----

/// FTP 配置区控件集合；GTK 控件为引用计数对象，clone 后仍指向同一控件
#[derive(Clone)]
struct FtpWidgets {
    enabled: CheckButton,
    anon: CheckButton,
    bind_ip: Entry,
    port: SpinButton,
    passive_start: SpinButton,
    passive_end: SpinButton,
    welcome: Entry,
    anon_home: Entry,
    browse: Button,
    anon_status: Label,
    max_speed: SpinButton,
    encoding: ComboBoxText,
    masquerade_ip: Entry,
    save: Button,
    status: Label,
}

fn create_ftp_config_frame(container: &Box, state: &Arc<StdMutex<AppState>>) {
    let frame = Frame::new(Some("FTP 配置"));
    let box_ = Box::new(Orientation::Vertical, 5);
    box_.set_margin_top(10);
    box_.set_margin_bottom(10);
    box_.set_margin_start(10);
    box_.set_margin_end(10);

    let w = build_ftp_widgets(state);

    let row1 = Box::new(Orientation::Horizontal, 5);
    row1.pack_start(&w.enabled, false, false, 0);
    row1.pack_start(&Label::new(Some("绑定地址:")), false, false, 0);
    row1.pack_start(&w.bind_ip, false, false, 0);
    row1.pack_start(&Label::new(Some("端口:")), false, false, 0);
    row1.pack_start(&w.port, false, false, 0);
    row1.pack_end(&w.save, false, false, 0);
    box_.pack_start(&row1, false, false, 0);

    let row2 = Box::new(Orientation::Horizontal, 5);
    row2.pack_start(&Label::new(Some("被动端口范围:")), false, false, 0);
    row2.pack_start(&w.passive_start, false, false, 0);
    row2.pack_start(&Label::new(Some("-")), false, false, 0);
    row2.pack_start(&w.passive_end, false, false, 0);
    row2.pack_start(&Label::new(Some("对外公开IP:")), false, false, 0);
    row2.pack_start(&w.masquerade_ip, false, false, 0);
    box_.pack_start(&row2, false, false, 0);

    let row3 = Box::new(Orientation::Horizontal, 5);
    row3.pack_start(&Label::new(Some("欢迎消息:")), false, false, 0);
    row3.pack_start(&w.welcome, true, true, 0);
    box_.pack_start(&row3, false, false, 0);

    let row4 = Box::new(Orientation::Horizontal, 5);
    row4.pack_start(&w.anon, false, false, 0);
    row4.pack_start(&Label::new(Some("匿名用户目录:")), false, false, 0);
    row4.pack_start(&w.anon_home, true, true, 0);
    row4.pack_start(&w.browse, false, false, 0);
    box_.pack_start(&row4, false, false, 0);

    box_.pack_start(&w.anon_status, false, false, 0);

    let row5 = Box::new(Orientation::Horizontal, 5);
    row5.pack_start(
        &Label::new(Some("最大传输速度(KB/s, 0为不限):")),
        false,
        false,
        0,
    );
    row5.pack_start(&w.max_speed, false, false, 0);
    row5.pack_start(&Label::new(Some("编码:")), false, false, 0);
    row5.pack_start(&w.encoding, false, false, 0);
    box_.pack_start(&row5, false, false, 0);

    box_.pack_start(&w.status, false, false, 0);

    frame.add(&box_);
    container.pack_start(&frame, false, false, 0);

    validate_anonymous_home(&w);
    w.anon.connect_toggled(clone!(@strong w => move |_| {
        validate_anonymous_home(&w);
    }));
    w.anon_home.connect_changed(clone!(@strong w => move |_| {
        validate_anonymous_home(&w);
    }));
    setup_anonymous_browse(&w);
    setup_ftp_save_button(state, &w);
}

/// 构造 FTP 配置控件并从用户目录配置回填初始值
fn build_ftp_widgets(state: &Arc<StdMutex<AppState>>) -> FtpWidgets {
    let enabled = CheckButton::with_label("启用FTP服务");
    let anon = CheckButton::with_label("允许匿名访问");
    let bind_ip = Entry::new();
    bind_ip.set_width_chars(15);
    bind_ip.set_placeholder_text(Some("0.0.0.0"));
    let port = create_spin_button(1.0, 65535.0, 1.0);
    let passive_start = create_spin_button(1024.0, 65535.0, 1.0);
    let passive_end = create_spin_button(1024.0, 65535.0, 1.0);
    let welcome = Entry::new();
    welcome.set_hexpand(true);
    let anon_home = Entry::new();
    anon_home.set_hexpand(true);
    let anon_status = Label::new(None);
    let max_speed = create_spin_button(0.0, 1_024_000.0, 100.0);
    let encoding = ComboBoxText::new();
    for item in ["UTF-8", "GBK", "GB2312"] {
        encoding.append_text(item);
    }
    let masquerade_ip = Entry::new();
    masquerade_ip.set_width_chars(15);
    masquerade_ip.set_placeholder_text(Some("如: 192.168.1.100"));

    // 从 GUI 启动时加载的用户目录配置回填
    let _ = with_config(state, |cfg| {
        let ftp = &cfg.ftp;
        enabled.set_active(ftp.enabled);
        anon.set_active(ftp.allow_anonymous);
        bind_ip.set_text(&ftp.bind_ip);
        port.set_value(super::utils::spin_f64_from(u64::from(ftp.port)));
        passive_start.set_value(super::utils::spin_f64_from(u64::from(ftp.passive_ports.0)));
        passive_end.set_value(super::utils::spin_f64_from(u64::from(ftp.passive_ports.1)));
        welcome.set_text(&ftp.welcome_message);
        if let Some(home) = &ftp.anonymous_home {
            anon_home.set_text(home);
        }
        max_speed.set_value(super::utils::spin_f64_from(ftp.max_speed_kbps));
        let enc_lower = ftp.encoding.to_lowercase();
        for (idx, item) in ["utf-8", "gbk", "gb2312"].into_iter().enumerate() {
            if enc_lower == item
                && let Ok(idx) = u32::try_from(idx)
            {
                encoding.set_active(Some(idx));
            }
        }
        if let Some(ip) = &ftp.masquerade_ip {
            masquerade_ip.set_text(ip);
        }
    });

    FtpWidgets {
        enabled,
        anon,
        bind_ip,
        port,
        passive_start,
        passive_end,
        welcome,
        anon_home,
        browse: Button::with_label("浏览..."),
        anon_status,
        max_speed,
        encoding,
        masquerade_ip,
        save: Button::with_label("保存配置"),
        status: Label::new(None),
    }
}

/// 校验匿名访问目录：启用匿名时必须为已存在的目录
fn validate_anonymous_home(w: &FtpWidgets) {
    let home = w.anon_home.text().to_string();

    if !w.anon.is_active() {
        w.anon_status
            .set_markup("<span foreground='gray' size='small'>匿名访问未启用</span>");
        return;
    }

    if home.trim().is_empty() {
        w.anon_status.set_markup(
            "<span foreground='red' size='small'>⚠ 启用匿名访问必须配置匿名用户目录</span>",
        );
        return;
    }

    let path = Path::new(&home);
    if !path.exists() {
        w.anon_status.set_markup(&format!(
            "<span foreground='red' size='small'>⚠ 目录不存在: {home}</span>"
        ));
        return;
    }
    if !path.is_dir() {
        w.anon_status.set_markup(&format!(
            "<span foreground='red' size='small'>⚠ 路径不是目录: {home}</span>"
        ));
        return;
    }

    w.anon_status
        .set_markup("<span foreground='green' size='small'>✓ 目录有效</span>");
}

/// 匿名用户目录选择对话框
fn setup_anonymous_browse(w: &FtpWidgets) {
    w.browse.connect_clicked(clone!(@strong w => move |_| {
        let dialog = gtk::FileChooserDialog::new(
            Some("选择匿名用户主目录"),
            None::<&gtk::Window>,
            gtk::FileChooserAction::SelectFolder,
        );
        dialog.add_button("取消", gtk::ResponseType::Cancel);
        dialog.add_button("选择", gtk::ResponseType::Accept);
        dialog.connect_response(clone!(@strong w, @strong dialog => move |dlg, resp| {
            if resp == gtk::ResponseType::Accept
                && let Some(path) = dlg.file().and_then(|f| f.path())
            {
                w.anon_home.set_text(&path.to_string_lossy());
                validate_anonymous_home(&w);
            }
            dialog.close();
        }));
        dialog.run();
        dialog.close();
    }));
}

fn setup_ftp_save_button(state: &Arc<StdMutex<AppState>>, w: &FtpWidgets) {
    w.save.connect_clicked(clone!(@strong state, @strong w => move |_| {
        let enabled = w.enabled.is_active();
        let anon = w.anon.is_active();

        // 启用匿名访问时先做客户端校验，后端 SaveConfig 还会再校验一次
        if anon {
            let home = w.anon_home.text().to_string();
            let path = Path::new(home.trim());
            if home.trim().is_empty() || !path.is_dir() {
                validate_anonymous_home(&w);
                error!("保存失败：匿名用户目录未配置或无效");
                return;
            }
        }

        let bind_ip = w.bind_ip.text().to_string();
        let port = spin_u16(w.port.value());
        let start = spin_u16(w.passive_start.value());
        let end = spin_u16(w.passive_end.value());
        let welcome = w.welcome.text().to_string();
        let anon_home = w.anon_home.text().to_string();
        let max_speed = super::utils::spin_u64(w.max_speed.value());
        let encoding = w
            .encoding
            .active_text()
            .map_or_else(|| "UTF-8".to_string(), |s| s.to_string());
        let masquerade_ip = w.masquerade_ip.text().to_string();

        let summary = format!(
            "ftp: enabled={enabled}, bind={bind_ip}:{port}, passive={start}-{end}, anon={anon}"
        );
        let saved = with_config(&state, |cfg| {
            cfg.ftp.enabled = enabled;
            cfg.ftp.allow_anonymous = anon;
            cfg.ftp.bind_ip = if bind_ip.trim().is_empty() {
                "0.0.0.0".to_string()
            } else {
                bind_ip.clone()
            };
            cfg.ftp.port = port;
            cfg.ftp.passive_ports = (start.min(end), start.max(end));
            cfg.ftp.welcome_message = welcome;
            cfg.ftp.anonymous_home = if anon_home.trim().is_empty() {
                None
            } else {
                Some(anon_home.clone())
            };
            cfg.ftp.max_speed_kbps = max_speed;
            cfg.ftp.encoding = encoding;
            cfg.ftp.masquerade_ip = if masquerade_ip.trim().is_empty() {
                None
            } else {
                Some(masquerade_ip.trim().to_string())
            };
            toml::to_string_pretty(&*cfg).unwrap_or_default()
        });

        let Some(config_str) = saved else {
            return;
        };
        match crate::communication::write_config(&config_str) {
            Ok(saved_content) => {
                let _ = with_config(&state, |cfg| {
                    if let Ok(new_config) = toml::from_str(&saved_content) {
                        *cfg = new_config;
                    }
                });
                w.status.set_markup(
                    "<span foreground='green' size='small'>✓ FTP 配置已保存（启用状态已生效）</span>",
                );
                let _ = crate::communication::write_audit_log(
                    "gui-server",
                    "SERVER_CONFIG",
                    "ftp",
                    &summary,
                );
                info!("{summary} saved");
            }
            Err(e) => {
                w.status.set_markup(&format!(
                    "<span foreground='red' size='small'>✗ 保存失败: {e}</span>"
                ));
                error!("保存 FTP 配置失败: {e}");
            }
        }
    }));
}

// ---- SFTP 配置 ----

/// SFTP 配置区控件集合
#[derive(Clone)]
struct SftpWidgets {
    enabled: CheckButton,
    bind_ip: Entry,
    port: SpinButton,
    host_key: Entry,
    max_auth: SpinButton,
    auth_timeout: SpinButton,
    log_level: ComboBoxText,
    save: Button,
    status: Label,
}

fn create_sftp_config_frame(container: &Box, state: &Arc<StdMutex<AppState>>) {
    let frame = Frame::new(Some("SFTP 配置"));
    let box_ = Box::new(Orientation::Vertical, 5);
    box_.set_margin_top(10);
    box_.set_margin_bottom(10);
    box_.set_margin_start(10);
    box_.set_margin_end(10);

    let enabled = CheckButton::with_label("启用SFTP服务");
    let bind_ip = Entry::new();
    bind_ip.set_width_chars(15);
    bind_ip.set_placeholder_text(Some("0.0.0.0"));
    let port = create_spin_button(1.0, 65535.0, 1.0);
    let host_key = Entry::new();
    host_key.set_hexpand(true);
    let max_auth = create_spin_button(1.0, 20.0, 1.0);
    let auth_timeout = create_spin_button(10.0, 600.0, 5.0);
    let log_level = ComboBoxText::new();
    for item in ["error", "warn", "info", "debug", "trace"] {
        log_level.append_text(item);
    }
    let save = Button::with_label("保存配置");
    let status = Label::new(None);

    let _ = with_config(state, |cfg| {
        let sftp = &cfg.sftp;
        enabled.set_active(sftp.enabled);
        bind_ip.set_text(&sftp.bind_ip);
        port.set_value(super::utils::spin_f64_from(u64::from(sftp.port)));
        host_key.set_text(&sftp.host_key_path);
        max_auth.set_value(f64::from(sftp.max_auth_attempts));
        auth_timeout.set_value(super::utils::spin_f64_from(sftp.auth_timeout));
        let level = sftp.log_level.to_lowercase();
        for (idx, item) in ["error", "warn", "info", "debug", "trace"]
            .into_iter()
            .enumerate()
        {
            if level == item
                && let Ok(idx) = u32::try_from(idx)
            {
                log_level.set_active(Some(idx));
            }
        }
    });

    let row1 = Box::new(Orientation::Horizontal, 5);
    row1.pack_start(&enabled, false, false, 0);
    row1.pack_start(&Label::new(Some("绑定地址:")), false, false, 0);
    row1.pack_start(&bind_ip, false, false, 0);
    row1.pack_start(&Label::new(Some("端口:")), false, false, 0);
    row1.pack_start(&port, false, false, 0);
    row1.pack_end(&save, false, false, 0);
    box_.pack_start(&row1, false, false, 0);

    let row2 = Box::new(Orientation::Horizontal, 5);
    row2.pack_start(&Label::new(Some("主机密钥路径:")), false, false, 0);
    row2.pack_start(&host_key, true, true, 0);
    box_.pack_start(&row2, false, false, 0);

    let row3 = Box::new(Orientation::Horizontal, 5);
    row3.pack_start(&Label::new(Some("最大认证尝试次数:")), false, false, 0);
    row3.pack_start(&max_auth, false, false, 0);
    row3.pack_start(&Label::new(Some("认证超时(秒):")), false, false, 0);
    row3.pack_start(&auth_timeout, false, false, 0);
    row3.pack_start(&Label::new(Some("日志级别:")), false, false, 0);
    row3.pack_start(&log_level, false, false, 0);
    box_.pack_start(&row3, false, false, 0);

    let key_hint = Label::new(None);
    key_hint.set_markup(
        "<span foreground='gray' size='small'>首次启动若密钥不存在会自动生成 Ed25519 密钥对</span>",
    );
    key_hint.set_halign(gtk::Align::Start);
    box_.pack_start(&key_hint, false, false, 0);

    box_.pack_start(&status, false, false, 0);

    frame.add(&box_);
    container.pack_start(&frame, false, false, 0);

    let widgets = SftpWidgets {
        enabled,
        bind_ip,
        port,
        host_key,
        max_auth,
        auth_timeout,
        log_level,
        save,
        status,
    };
    setup_sftp_save_button(state, &widgets);
}

fn setup_sftp_save_button(state: &Arc<StdMutex<AppState>>, w: &SftpWidgets) {
    w.save.connect_clicked(clone!(@strong state, @strong w => move |_| {
        let enabled = w.enabled.is_active();
        let bind_ip = w.bind_ip.text().to_string();
        let port = spin_u16(w.port.value());
        let host_key = w.host_key.text().to_string();
        let max_auth = super::utils::spin_u32(w.max_auth.value());
        let auth_timeout = super::utils::spin_u64(w.auth_timeout.value());
        let log_level = w
            .log_level
            .active_text()
            .map_or_else(|| "info".to_string(), |s| s.to_string());

        if host_key.trim().is_empty() {
            w.status.set_markup(
                "<span foreground='red' size='small'>✗ 保存失败: 主机密钥路径不能为空</span>",
            );
            error!("保存失败：SFTP 主机密钥路径为空");
            return;
        }

        let summary =
            format!("sftp: enabled={enabled}, bind={bind_ip}:{port}, max_auth={max_auth}");
        let saved = with_config(&state, |cfg| {
            cfg.sftp.enabled = enabled;
            cfg.sftp.bind_ip = if bind_ip.trim().is_empty() {
                "0.0.0.0".to_string()
            } else {
                bind_ip.clone()
            };
            cfg.sftp.port = port;
            cfg.sftp.host_key_path = host_key;
            cfg.sftp.max_auth_attempts = max_auth;
            cfg.sftp.auth_timeout = auth_timeout;
            cfg.sftp.log_level = log_level;
            toml::to_string_pretty(&*cfg).unwrap_or_default()
        });

        let Some(config_str) = saved else {
            return;
        };
        match crate::communication::write_config(&config_str) {
            Ok(saved_content) => {
                let _ = with_config(&state, |cfg| {
                    if let Ok(new_config) = toml::from_str(&saved_content) {
                        *cfg = new_config;
                    }
                });
                w.status.set_markup(
                    "<span foreground='green' size='small'>✓ SFTP 配置已保存（启用状态已生效）</span>",
                );
                let _ = crate::communication::write_audit_log(
                    "gui-server",
                    "SERVER_CONFIG",
                    "sftp",
                    &summary,
                );
                info!("{summary} saved");
            }
            Err(e) => {
                w.status.set_markup(&format!(
                    "<span foreground='red' size='small'>✗ 保存失败: {e}</span>"
                ));
                error!("保存 SFTP 配置失败: {e}");
            }
        }
    }));
}

/// `SpinButton` 值转 u16（控件范围已钳制在 1..=65535）
fn spin_u16(value: f64) -> u16 {
    u16::try_from(super::utils::spin_u32(value)).unwrap_or(u16::MAX)
}
