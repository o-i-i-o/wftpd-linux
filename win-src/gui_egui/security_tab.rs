use egui::RichText;
use crate::core::config::Config;
use crate::core::ipc::IpcClient;
use crate::gui_egui::styles;
use egui_file_dialog::FileDialog;
use std::sync::mpsc;
use std::time::Instant;

#[derive(Debug, Clone)]
struct ValidationError {
    field: String,
    message: String,
}

fn validate_ip_cidr(input: &str) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    
    for (line_num, line) in input.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        
        if !is_valid_ip_or_cidr(trimmed) {
            errors.push(ValidationError {
                field: format!("第 {} 行", line_num + 1),
                message: format!("无效的 IP/CIDR 格式: {}", trimmed),
            });
        }
    }
    
    errors
}

fn is_valid_ip_or_cidr(input: &str) -> bool {
    if input.contains('/') {
        let parts: Vec<&str> = input.split('/').collect();
        if parts.len() != 2 {
            return false;
        }
        let ip = parts[0];
        let prefix = match parts[1].parse::<u8>() {
            Ok(p) => p,
            Err(_) => return false,
        };
        
        if ip.contains(':') {
            if prefix > 128 {
                return false;
            }
            is_valid_ipv6(ip)
        } else {
            if prefix > 32 {
                return false;
            }
            is_valid_ipv4(ip)
        }
    } else {
        if input.contains(':') {
            is_valid_ipv6(input)
        } else {
            is_valid_ipv4(input)
        }
    }
}

fn is_valid_ipv4(ip: &str) -> bool {
    let parts: Vec<&str> = ip.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    parts.iter().all(|p| {
        p.parse::<u8>().is_ok()
    })
}

fn is_valid_ipv6(ip: &str) -> bool {
    if ip.is_empty() || ip.len() > 45 {
        return false;
    }
    
    let parts: Vec<&str> = ip.split("::").collect();
    if parts.len() > 2 {
        return false;
    }
    
    let total_groups: usize = if parts.len() == 2 {
        let left = if parts[0].is_empty() { 0 } else { parts[0].split(':').count() };
        let right = if parts[1].is_empty() { 0 } else { parts[1].split(':').count() };
        left + right
    } else {
        ip.split(':').count()
    };
    
    total_groups <= 8
}

enum SaveResult {
    Success,
    Error(String),
}

pub struct SecurityTab {
    config: Config,
    max_login_attempts_buf: String,
    ban_duration_buf: String,
    host_key_path_buf: String,
    allowed_ips_text: String,
    denied_ips_text: String,
    status_message: Option<(String, bool)>,
    validation_errors: Vec<ValidationError>,
    max_login_attempts_error: Option<String>,
    ban_duration_error: Option<String>,
    host_key_path_error: Option<String>,
    file_dialog: FileDialog,
    save_sender: mpsc::Sender<SaveResult>,
    save_receiver: Option<mpsc::Receiver<SaveResult>>,
    is_saving: bool,
    last_save_time: Option<Instant>,
}

impl Default for SecurityTab {
    fn default() -> Self {
        let config = Config::load(&Config::get_config_path()).unwrap_or_default();
        let max_login_attempts_buf = config.security.max_login_attempts.to_string();
        let ban_duration_buf = config.security.ban_duration.to_string();
        let host_key_path_buf = config.sftp.host_key_path.clone();
        let allowed_ips_text = config.security.allowed_ips.join("\n");
        let denied_ips_text = config.security.denied_ips.join("\n");
        
        let (tx, rx) = mpsc::channel();
        
        Self {
            config,
            max_login_attempts_buf,
            ban_duration_buf,
            host_key_path_buf,
            allowed_ips_text,
            denied_ips_text,
            status_message: None,
            validation_errors: Vec::new(),
            max_login_attempts_error: None,
            ban_duration_error: None,
            host_key_path_error: None,
            file_dialog: FileDialog::new().title("选择主机密钥文件"),
            save_sender: tx,
            save_receiver: Some(rx),
            is_saving: false,
            last_save_time: None,
        }
    }
}

impl SecurityTab {
    pub fn new() -> Self { 
        Self::default() 
    }

    fn validate_all(&mut self) -> bool {
        self.validation_errors.clear();
        self.max_login_attempts_error = None;
        self.ban_duration_error = None;
        self.host_key_path_error = None;
        
        let mut valid = true;
        
        if let Ok(v) = self.max_login_attempts_buf.parse::<u32>() {
            if v == 0 {
                self.max_login_attempts_error = Some("最大登录失败次数必须大于 0".to_string());
                valid = false;
            }
        } else {
            self.max_login_attempts_error = Some("请输入有效的数字".to_string());
            valid = false;
        }
        
        if self.ban_duration_buf.parse::<u64>().is_err() {
            self.ban_duration_error = Some("请输入有效的数字".to_string());
            valid = false;
        }
        
        if self.host_key_path_buf.trim().is_empty() {
            self.host_key_path_error = Some("主机密钥路径不能为空".to_string());
            valid = false;
        }
        
        let allowed_errors = validate_ip_cidr(&self.allowed_ips_text);
        let denied_errors = validate_ip_cidr(&self.denied_ips_text);
        
        if !allowed_errors.is_empty() || !denied_errors.is_empty() {
            self.validation_errors = allowed_errors;
            self.validation_errors.extend(denied_errors);
            valid = false;
        }
        
        valid
    }

    fn apply_buffers_to_config(&mut self) {
        if let Ok(v) = self.max_login_attempts_buf.parse::<u32>() {
            self.config.security.max_login_attempts = v;
        }
        if let Ok(v) = self.ban_duration_buf.parse::<u64>() {
            self.config.security.ban_duration = v;
        }
        self.config.sftp.host_key_path = self.host_key_path_buf.trim().to_string();
        
        self.config.security.allowed_ips = self
            .allowed_ips_text
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        self.config.security.denied_ips = self
            .denied_ips_text
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }

    fn save_async(&mut self, ctx: &egui::Context) {
        if self.is_saving {
            return;
        }
        
        if !self.validate_all() {
            self.status_message = Some(("输入验证失败，请检查红色标记的字段".to_string(), false));
            return;
        }
        
        self.apply_buffers_to_config();
        
        self.is_saving = true;
        let config = self.config.clone();
        let tx = self.save_sender.clone();
        let ctx_clone = ctx.clone();
        
        std::thread::spawn(move || {
            let _result = match config.save(&Config::get_config_path()) {
                Ok(_) => {
                    if IpcClient::is_server_running() {
                        let _ = IpcClient::send_command(crate::core::ipc::Command {
                            action: "reload".to_string(),
                            service: None,
                            data: None,
                        });
                    }
                    SaveResult::Success
                }
                Err(e) => SaveResult::Error(format!("保存失败：{}", e)),
            };
            let _ = tx.send(_result);
            ctx_clone.request_repaint();
        });
    }

    fn check_save_result(&mut self) {
        if let Some(rx) = &self.save_receiver
            && let Ok(result) = rx.try_recv()
        {
            self.is_saving = false;
            match result {
                SaveResult::Success => {
                    self.last_save_time = Some(Instant::now());
                    self.status_message = Some(("安全配置已保存并已通知服务器重新加载".to_string(), true));
                }
                SaveResult::Error(e) => {
                    self.status_message = Some((e, false));
                }
            }
        }
    }

    fn section_header(ui: &mut egui::Ui, icon: &str, title: &str) {
        styles::section_header(ui, icon, title);
    }

    fn format_last_save(&self) -> String {
        match self.last_save_time {
            Some(t) => {
                let elapsed = t.elapsed();
                if elapsed.as_secs() < 60 {
                    format!("{} 秒前保存", elapsed.as_secs())
                } else if elapsed.as_secs() < 3600 {
                    format!("{} 分钟前保存", elapsed.as_secs() / 60)
                } else {
                    format!("{} 小时前保存", elapsed.as_secs() / 3600)
                }
            }
            None => "未保存".to_string(),
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.check_save_result();
        
        styles::page_header(ui, "🔒", "安全设置");

        if let Some((msg, success)) = &self.status_message {
            styles::status_message(ui, msg, *success);
            ui.add_space(styles::SPACING_MD);
        }

        styles::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            Self::section_header(ui, "🔐", "登录安全");
            
            let available_width = ui.available_width();
            let label_width = (available_width * 0.2).clamp(100.0, 160.0);
            
            styles::form_row(ui, "最大登录失败次数", label_width, |ui| {
                let response = styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.max_login_attempts_buf)
                        .desired_width(100.0)
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)))
                });
                
                if response.response.lost_focus() {
                    if let Ok(v) = self.max_login_attempts_buf.parse::<u32>() {
                        if v == 0 {
                            self.max_login_attempts_error = Some("必须大于 0".to_string());
                        } else {
                            self.max_login_attempts_error = None;
                        }
                    } else {
                        self.max_login_attempts_error = Some("请输入有效数字".to_string());
                    }
                }
            });
            
            if let Some(err) = &self.max_login_attempts_error {
                ui.horizontal(|ui| {
                    ui.add_sized([label_width, 24.0], egui::Label::new(""));
                    ui.label(RichText::new(format!("⚠ {}", err))
                        .size(styles::FONT_SIZE_SM)
                        .color(styles::DANGER_COLOR));
                });
            }
            
            styles::form_row_with_suffix(ui, "封禁时间", label_width, |ui| {
                let response = styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.ban_duration_buf)
                        .desired_width(100.0)
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)))
                });
                
                if response.response.lost_focus() {
                    if self.ban_duration_buf.parse::<u64>().is_err() {
                        self.ban_duration_error = Some("请输入有效数字".to_string());
                    } else {
                        self.ban_duration_error = None;
                    }
                }
            }, "秒");
            
            if let Some(err) = &self.ban_duration_error {
                ui.horizontal(|ui| {
                    ui.add_sized([label_width, 24.0], egui::Label::new(""));
                    ui.label(RichText::new(format!("⚠ {}", err))
                        .size(styles::FONT_SIZE_SM)
                        .color(styles::DANGER_COLOR));
                });
            }
        });

        ui.add_space(styles::SPACING_MD);

        styles::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            Self::section_header(ui, "🔑", "SFTP 主机密钥");
            
            let available_width = ui.available_width();
            let label_width = (available_width * 0.2).clamp(100.0, 160.0);
            
            styles::form_row(ui, "密钥文件路径", label_width, |ui| {
                styles::input_frame().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.host_key_path_buf)
                        .desired_width(ui.available_width() - 80.0)
                        .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)));
                });
                
                if ui.button("浏览...").clicked() {
                    self.file_dialog.pick_file();
                }
            });
            
            if let Some(err) = &self.host_key_path_error {
                ui.horizontal(|ui| {
                    ui.add_sized([label_width, 24.0], egui::Label::new(""));
                    ui.label(RichText::new(format!("⚠ {}", err))
                        .size(styles::FONT_SIZE_SM)
                        .color(styles::DANGER_COLOR));
                });
            }
            
            let path = std::path::Path::new(self.host_key_path_buf.trim());
            let path_exists = path.exists();
            let path_status = if path_exists {
                ("✓ 文件已存在", styles::SUCCESS_COLOR)
            } else {
                ("ℹ 文件不存在，启动时将自动生成", styles::TEXT_MUTED_COLOR)
            };
            
            ui.horizontal(|ui| {
                ui.add_sized([label_width, 24.0], egui::Label::new(""));
                ui.label(RichText::new(path_status.0)
                    .size(styles::FONT_SIZE_SM)
                    .color(path_status.1)
                    .italics());
            });
        });

        ui.add_space(styles::SPACING_MD);

        styles::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            Self::section_header(ui, "🌐", "IP 访问控制");
            
            ui.label(RichText::new("允许的 IP/CIDR").size(styles::FONT_SIZE_MD).color(styles::TEXT_SECONDARY_COLOR));
            ui.label(RichText::new("每行一个，0.0.0.0/0 表示允许全部")
                .size(styles::FONT_SIZE_SM)
                .color(styles::TEXT_MUTED_COLOR));
            
            let allowed_response = styles::input_frame().show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut self.allowed_ips_text)
                    .desired_rows(4)
                    .desired_width(ui.available_width())
                    .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)))
            });
            
            if allowed_response.response.lost_focus() {
                let errors = validate_ip_cidr(&self.allowed_ips_text);
                self.validation_errors.retain(|e| !e.message.contains("允许"));
                self.validation_errors.extend(errors.into_iter().map(|mut e| {
                    e.message = format!("[允许列表] {}", e.message);
                    e
                }));
            }

            ui.add_space(styles::SPACING_MD);
            
            ui.label(RichText::new("拒绝的 IP/CIDR").size(styles::FONT_SIZE_MD).color(styles::TEXT_SECONDARY_COLOR));
            ui.label(RichText::new("每行一个，优先级高于允许列表")
                .size(styles::FONT_SIZE_SM)
                .color(styles::TEXT_MUTED_COLOR));
            
            let denied_response = styles::input_frame().show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut self.denied_ips_text)
                    .desired_rows(4)
                    .desired_width(ui.available_width())
                    .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)))
            });
            
            if denied_response.response.lost_focus() {
                let errors = validate_ip_cidr(&self.denied_ips_text);
                self.validation_errors.retain(|e| !e.message.contains("拒绝"));
                self.validation_errors.extend(errors.into_iter().map(|mut e| {
                    e.message = format!("[拒绝列表] {}", e.message);
                    e
                }));
            }
            
            if !self.validation_errors.is_empty() {
                ui.add_space(styles::SPACING_SM);
                ui.label(RichText::new("⚠ IP/CIDR 验证错误：")
                    .size(styles::FONT_SIZE_SM)
                    .color(styles::DANGER_COLOR)
                    .strong());
                
                egui::ScrollArea::vertical()
                    .max_height(100.0)
                    .show(ui, |ui| {
                        for err in &self.validation_errors {
                            ui.label(RichText::new(format!("  • {}: {}", err.field, err.message))
                                .size(styles::FONT_SIZE_SM)
                                .color(styles::WARNING_COLOR));
                        }
                    });
            }
        });

        ui.add_space(styles::SPACING_MD);

        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let save_btn = if self.is_saving {
                    egui::Button::new(RichText::new("💾 保存中...").color(egui::Color32::GRAY).size(styles::FONT_SIZE_MD))
                        .fill(styles::BG_SECONDARY)
                        .corner_radius(egui::CornerRadius::same(6))
                } else {
                    styles::primary_button("💾 保存安全配置")
                };
                
                if ui.add(save_btn).clicked() && !self.is_saving {
                    self.save_async(ui.ctx());
                }
                
                ui.label(RichText::new(self.format_last_save())
                    .size(styles::FONT_SIZE_SM)
                    .color(styles::TEXT_MUTED_COLOR));
            });
        });

        self.file_dialog.update(ui.ctx());
        if let Some(path) = self.file_dialog.take_picked() {
            self.host_key_path_buf = path.to_string_lossy().to_string();
            self.host_key_path_error = None;
        }
    }
}
