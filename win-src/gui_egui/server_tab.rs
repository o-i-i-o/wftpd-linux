use egui::{RichText, Ui};
use crate::core::config::Config;
use crate::core::ipc::IpcClient;
use crate::gui_egui::styles;
use egui_file_dialog::FileDialog;
use std::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum FileDialogTarget {
    #[default]
    None,
    AnonymousHome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigLoadState {
    Loading,
    Loaded,
    Error,
}

#[derive(Debug)]
pub struct ServerTab {
    pub config: Option<Config>,
    pub ftp_running: bool,
    pub sftp_running: bool,
    pub status_message: Option<(String, bool)>,
    file_dialog: FileDialog,
    file_dialog_target: FileDialogTarget,
    config_load_state: ConfigLoadState,
    config_load_error: Option<String>,
    save_receiver: Option<mpsc::Receiver<Result<(), String>>>,
    is_saving: bool,
}

impl Default for ServerTab {
    fn default() -> Self {
        Self {
            config: None,
            ftp_running: false,
            sftp_running: false,
            status_message: None,
            file_dialog: FileDialog::new().title("选择匿名用户目录"),
            file_dialog_target: FileDialogTarget::None,
            config_load_state: ConfigLoadState::Loading,
            config_load_error: None,
            save_receiver: None,
            is_saving: false,
        }
    }
}

impl ServerTab {
    pub fn new() -> Self { 
        Self::default() 
    }

    pub fn load_config(&mut self) {
        match Config::load(&Config::get_config_path()) {
            Ok(config) => {
                log::info!("服务器配置加载成功");
                self.config = Some(config);
                self.config_load_state = ConfigLoadState::Loaded;
            }
            Err(e) => {
                log::warn!("加载服务器配置失败，使用默认配置: {}", e);
                self.config = Some(Config::default());
                self.config_load_state = ConfigLoadState::Loaded;
                self.status_message = Some((format!("配置加载失败，使用默认配置: {}", e), false));
            }
        }
    }

    pub fn update_status(&mut self, ftp_running: bool, sftp_running: bool) {
        self.ftp_running = ftp_running;
        self.sftp_running = sftp_running;
    }

    pub fn save_config_async(&mut self, ctx: &egui::Context) {
        if self.is_saving {
            return;
        }
        
        let config = match &self.config {
            Some(c) => c.clone(),
            None => {
                self.status_message = Some(("配置未加载，无法保存".to_string(), false));
                return;
            }
        };
        
        self.is_saving = true;
        let (tx, rx) = mpsc::channel();
        self.save_receiver = Some(rx);
        
        let ctx_clone = ctx.clone();
        std::thread::spawn(move || {
            let result = match config.save(&Config::get_config_path()) {
                Ok(_) => {
                    log::info!("服务器配置保存成功");
                    Ok(())
                }
                Err(e) => {
                    log::error!("保存服务器配置失败: {}", e);
                    Err(format!("保存失败: {}", e))
                }
            };
            
            if IpcClient::is_server_running() {
                match IpcClient::send_command(crate::core::ipc::Command {
                    action: "reload".to_string(),
                    service: None,
                    data: None,
                }) {
                    Ok(_) => {
                        log::info!("已通知服务器重新加载配置");
                    }
                    Err(e) => {
                        log::warn!("通知服务器重新加载配置失败: {}。建议手动重启服务。", e);
                    }
                }
            }
            
            let _ = tx.send(result);
            ctx_clone.request_repaint();
        });
    }

    fn check_save_result(&mut self) {
        if let Some(rx) = &self.save_receiver
            && let Ok(result) = rx.try_recv()
        {
            self.save_receiver = None;
            self.is_saving = false;
            
            match result {
                Ok(_) => {
                    self.status_message = Some(("配置已保存并已通知服务器重新加载".to_string(), true));
                }
                Err(e) => {
                    self.status_message = Some((e, false));
                }
            }
        }
    }

    fn section_header(ui: &mut Ui, icon: &str, title: &str) {
        styles::section_header(ui, icon, title);
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        self.check_save_result();
        
        match self.config_load_state {
            ConfigLoadState::Loading => {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() / 2.0 - 50.0);
                    ui.spinner();
                    ui.add_space(styles::SPACING_MD);
                    ui.label(RichText::new("正在加载配置...").size(styles::FONT_SIZE_LG).color(styles::TEXT_SECONDARY_COLOR));
                });
                return;
            }
            ConfigLoadState::Error => {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() / 2.0 - 80.0);
                    ui.label(RichText::new("⚠ 配置加载失败").size(styles::FONT_SIZE_LG).strong().color(styles::DANGER_COLOR));
                    ui.add_space(styles::SPACING_MD);
                    if let Some(error) = &self.config_load_error {
                        ui.label(RichText::new(error).size(styles::FONT_SIZE_MD).color(styles::TEXT_SECONDARY_COLOR));
                    }
                });
                return;
            }
            ConfigLoadState::Loaded => {}
        }
        
        let mut config = match self.config.take() {
            Some(c) => c,
            None => return,
        };

        let mut save_clicked = false;
        let is_saving = self.is_saving;
        let status_message = self.status_message.clone();
        
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("⚙").size(styles::FONT_SIZE_XL));
            ui.label(RichText::new("服务器配置").size(styles::FONT_SIZE_XL).strong().color(styles::TEXT_PRIMARY_COLOR));
            
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let save_btn = if is_saving {
                    egui::Button::new(RichText::new("💾 保存中...").color(egui::Color32::GRAY).size(styles::FONT_SIZE_MD))
                        .fill(styles::BG_SECONDARY)
                        .corner_radius(egui::CornerRadius::same(6))
                } else {
                    styles::primary_button("💾 保存配置")
                };
                
                if ui.add(save_btn).clicked() && !is_saving {
                    save_clicked = true;
                }
                
                if let Some((msg, success)) = &status_message {
                    let msg_text = if *success {
                        RichText::new(format!("✓ {}", msg)).color(styles::SUCCESS_COLOR).size(styles::FONT_SIZE_SM)
                    } else {
                        RichText::new(format!("✗ {}", msg)).color(styles::DANGER_COLOR).size(styles::FONT_SIZE_SM)
                    };
                    ui.label(msg_text);
                }
            });
        });
        
        if save_clicked {
            self.config = Some(config);
            self.save_config_async(ui.ctx());
            config = match self.config.take() {
                Some(c) => c,
                None => return,
            };
        }
        
        ui.add_space(styles::SPACING_MD);

        let mut open_file_dialog = false;
        
        styles::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            Self::section_header(ui, "📡", "FTP 设置");
            
            ui.checkbox(&mut config.ftp.enabled, 
                RichText::new("启用 FTP 服务").size(styles::FONT_SIZE_MD));
            ui.add_space(styles::SPACING_MD);

            let available_width = ui.available_width();
            let label_width = (available_width * 0.15).clamp(100.0, 160.0);
            
            styles::form_row(ui, "绑定 IP", label_width, |ui| {
                styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut config.server.bind_ip)
                        .desired_width(ui.available_width())
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)));
                });
            });
            
            styles::form_row(ui, "FTP 端口", label_width, |ui| {
                let mut port_str = config.server.ftp_port.to_string();
                styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut port_str)
                        .desired_width(80.0)
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)));
                });
                if let Ok(p) = port_str.parse::<u16>() {
                    config.server.ftp_port = p;
                }
            });
            
            styles::form_row(ui, "欢迎消息", label_width, |ui| {
                styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut config.ftp.welcome_message)
                        .desired_width(ui.available_width())
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)));
                });
            });
            
            styles::form_row(ui, "编码", label_width, |ui| {
                styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut config.ftp.encoding)
                        .desired_width(100.0)
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)));
                });
            });
            
            styles::form_row(ui, "默认传输模式", label_width, |ui| {
                let modes = ["binary", "ascii"];
                egui::ComboBox::from_id_salt("transfer_mode")
                    .selected_text(&config.ftp.default_transfer_mode)
                    .width(100.0)
                    .show_ui(ui, |ui| {
                        for mode in modes {
                            ui.selectable_value(
                                &mut config.ftp.default_transfer_mode,
                                mode.to_string(),
                                mode
                            );
                        }
                    });
                ui.label(RichText::new("(binary: 二进制，ascii: 文本)").size(styles::FONT_SIZE_SM).color(styles::TEXT_MUTED_COLOR));
            });
            
            styles::form_row(ui, "默认连接模式", label_width, |ui| {
                let passive_label = if config.ftp.default_passive_mode { "被动模式 (PASV)" } else { "主动模式 (PORT)" };
                egui::ComboBox::from_id_salt("connection_mode")
                    .selected_text(passive_label)
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut config.ftp.default_passive_mode, true, "被动模式 (PASV)");
                        ui.selectable_value(&mut config.ftp.default_passive_mode, false, "主动模式 (PORT)");
                    });
                ui.label(RichText::new("(被动模式兼容性更好)").size(styles::FONT_SIZE_SM).color(styles::TEXT_MUTED_COLOR));
            });
            
            styles::form_row(ui, "允许匿名访问", label_width, |ui| {
                ui.checkbox(&mut config.ftp.allow_anonymous, "");
            });
            
            if config.ftp.allow_anonymous {
                styles::form_row(ui, "匿名目录", label_width, |ui| {
                    let mut anon_home = config.ftp.anonymous_home.clone().unwrap_or_default();
                    styles::input_frame().show(ui, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut anon_home)
                            .desired_width(ui.available_width() - 80.0)
                            .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)));
                    });
                    if ui.button("浏览...").clicked() {
                        open_file_dialog = true;
                    }
                    config.ftp.anonymous_home = if anon_home.is_empty() { None } else { Some(anon_home) };
                });
                
                if config.ftp.anonymous_home.as_ref().is_none_or(|s| s.trim().is_empty()) {
                    ui.horizontal(|ui| {
                        ui.add_sized([label_width, 24.0], egui::Label::new(""));
                        ui.label(RichText::new("⚠ 匿名用户目录未配置，匿名访问将无法使用").size(styles::FONT_SIZE_SM).color(styles::WARNING_COLOR));
                    });
                }
            }
            
            styles::form_row(ui, "被动端口范围", label_width, |ui| {
                let mut min_str = config.ftp.passive_ports.0.to_string();
                let mut max_str = config.ftp.passive_ports.1.to_string();
                
                ui.label(RichText::new("从").size(styles::FONT_SIZE_MD).color(styles::TEXT_MUTED_COLOR));
                styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut min_str)
                        .desired_width(60.0)
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)));
                });
                if let Ok(p) = min_str.parse::<u16>() {
                    config.ftp.passive_ports.0 = p;
                }
                
                ui.label(RichText::new("到").size(styles::FONT_SIZE_MD).color(styles::TEXT_MUTED_COLOR));
                styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut max_str)
                        .desired_width(60.0)
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)));
                });
                if let Ok(p) = max_str.parse::<u16>() {
                    config.ftp.passive_ports.1 = p;
                }
                
                if config.ftp.passive_ports.0 > config.ftp.passive_ports.1 {
                    ui.label(RichText::new("⚠ 起始端口不能大于结束端口").color(styles::DANGER_COLOR).size(styles::FONT_SIZE_SM));
                }
            });
            
            styles::form_row_with_suffix(ui, "最大传输速度", label_width, |ui| {
                let mut speed_str = config.ftp.max_speed_kbps.to_string();
                styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut speed_str)
                        .desired_width(80.0)
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)));
                });
                if let Ok(v) = speed_str.parse::<u64>() {
                    config.ftp.max_speed_kbps = v;
                }
            }, "KB/s (0 表示不限制)");
        });

        ui.add_space(styles::SPACING_MD);

        styles::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            Self::section_header(ui, "🔐", "SFTP 设置");
            
            ui.checkbox(&mut config.sftp.enabled, 
                RichText::new("启用 SFTP 服务").size(styles::FONT_SIZE_MD));
            ui.add_space(styles::SPACING_MD);

            let available_width = ui.available_width();
            let label_width = (available_width * 0.15).clamp(100.0, 160.0);
            
            styles::form_row(ui, "绑定 IP", label_width, |ui| {
                styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut config.sftp.bind_ip)
                        .desired_width(ui.available_width())
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)));
                });
            });
            
            styles::form_row_with_suffix(ui, "SFTP 端口", label_width, |ui| {
                let mut port_str = config.server.sftp_port.to_string();
                styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut port_str)
                        .desired_width(80.0)
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)));
                });
                if let Ok(p) = port_str.parse::<u16>() {
                    config.server.sftp_port = p;
                }
            }, "(建议 2222)");
            
            styles::form_row(ui, "主机密钥路径", label_width, |ui| {
                styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut config.sftp.host_key_path)
                        .desired_width(ui.available_width())
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)));
                });
            });
            
            styles::form_row(ui, "最大认证次数", label_width, |ui| {
                let mut val_str = config.sftp.max_auth_attempts.to_string();
                styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut val_str)
                        .desired_width(80.0)
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)));
                });
                if let Ok(v) = val_str.parse::<u32>() {
                    config.sftp.max_auth_attempts = v;
                }
            });
            
            styles::form_row_with_suffix(ui, "认证超时", label_width, |ui| {
                let mut val_str = config.sftp.auth_timeout.to_string();
                styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut val_str)
                        .desired_width(100.0)
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)));
                });
                if let Ok(v) = val_str.parse::<u64>() {
                    config.sftp.auth_timeout = v;
                }
            }, "秒");
            
            styles::form_row(ui, "日志级别", label_width, |ui| {
                styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut config.sftp.log_level)
                        .desired_width(120.0)
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)));
                });
            });
        });

        ui.add_space(styles::SPACING_MD);

        styles::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            Self::section_header(ui, "⚙", "通用设置");
            
            let available_width = ui.available_width();
            let label_width = (available_width * 0.15).clamp(100.0, 160.0);
            
            styles::form_row(ui, "最大连接数", label_width, |ui| {
                let mut val_str = config.server.max_connections.to_string();
                styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut val_str)
                        .desired_width(80.0)
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)));
                });
                if let Ok(v) = val_str.parse::<usize>() {
                    config.server.max_connections = v;
                }
            });
            
            styles::form_row_with_suffix(ui, "连接超时", label_width, |ui| {
                let mut val_str = config.server.connection_timeout.to_string();
                styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut val_str)
                        .desired_width(80.0)
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)));
                });
                if let Ok(v) = val_str.parse::<u64>() {
                    config.server.connection_timeout = v;
                }
            }, "秒");
            
            styles::form_row_with_suffix(ui, "空闲超时", label_width, |ui| {
                let mut val_str = config.server.idle_timeout.to_string();
                styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut val_str)
                        .desired_width(80.0)
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)));
                });
                if let Ok(v) = val_str.parse::<u64>() {
                    config.server.idle_timeout = v;
                }
            }, "(0 表示不限制)");
        });

        self.config = Some(config);

        if open_file_dialog {
            self.file_dialog_target = FileDialogTarget::AnonymousHome;
            self.file_dialog.pick_directory();
        }

        self.file_dialog.update(ui.ctx());
        if let Some(path) = self.file_dialog.take_picked()
            && let Some(config) = &mut self.config
        {
            match self.file_dialog_target {
                FileDialogTarget::None => {}
                FileDialogTarget::AnonymousHome => {
                    config.ftp.anonymous_home = Some(path.to_string_lossy().to_string());
                }
            }
        }
        if !matches!(self.file_dialog.state(), egui_file_dialog::DialogState::Open) {
            self.file_dialog_target = FileDialogTarget::None;
        }
    }
}
