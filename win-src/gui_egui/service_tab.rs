use egui::{Color32, RichText, Ui};
use crate::core::server_manager::ServerManager;
use crate::gui_egui::styles;

pub struct ServiceTab {
    manager: ServerManager,
    status_message: Option<(String, bool)>,
    last_check: std::time::Instant,
    is_installed: bool,
    is_running: bool,
    confirming_uninstall: bool,
}

impl Default for ServiceTab {
    fn default() -> Self {
        let manager = ServerManager::new();
        let is_installed = manager.is_service_installed();
        let is_running = manager.is_service_running();
        Self {
            manager,
            status_message: None,
            last_check: std::time::Instant::now(),
            is_installed,
            is_running,
            confirming_uninstall: false,
        }
    }
}

impl ServiceTab {
    pub fn new() -> Self { Self::default() }

    fn refresh_status(&mut self) {
        self.is_installed = self.manager.is_service_installed();
        self.is_running = self.manager.is_service_running();
        self.last_check = std::time::Instant::now();
    }

    fn set_ok(&mut self, msg: &str) {
        self.status_message = Some((format!("✓ {}", msg), true));
        self.refresh_status();
    }

    fn set_err(&mut self, msg: String) {
        self.status_message = Some((format!("✗ {}", msg), false));
    }

    fn section_header(&self, ui: &mut Ui, icon: &str, title: &str) {
        styles::section_header(ui, icon, title);
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        if self.last_check.elapsed().as_secs() >= 2 {
            self.refresh_status();
        }

        styles::page_header(ui, "🖥", "系统服务管理");

        if let Some((msg, ok)) = &self.status_message.clone() {
            styles::status_message(ui, msg, *ok);
            ui.add_space(styles::SPACING_MD);
        }

        styles::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            self.section_header(ui, "📋", "服务信息");
            
            let available_width = ui.available_width();
            let label_width = (available_width * 0.25).clamp(80.0, 120.0);
            
            egui::Grid::new("svc_info")
                .num_columns(2)
                .spacing([20.0, 8.0])
                .min_col_width(label_width)
                .show(ui, |ui| {
                    ui.label(RichText::new("服务名称").size(styles::FONT_SIZE_MD).color(styles::TEXT_LABEL_COLOR));
                    ui.label(RichText::new("wftpd").size(styles::FONT_SIZE_MD).strong().color(styles::TEXT_PRIMARY_COLOR));
                    ui.end_row();

                    ui.label(RichText::new("显示名称").size(styles::FONT_SIZE_MD).color(styles::TEXT_LABEL_COLOR));
                    ui.label(RichText::new("WFTPD SFTP/FTP Server").size(styles::FONT_SIZE_MD).strong().color(styles::TEXT_PRIMARY_COLOR));
                    ui.end_row();

                    ui.label(RichText::new("安装状态").size(styles::FONT_SIZE_MD).color(styles::TEXT_LABEL_COLOR));
                    let (inst_txt, inst_col) = if self.is_installed {
                        ("● 已安装", styles::SUCCESS_DARK)
                    } else {
                        ("● 未安装", styles::DANGER_DARK)
                    };
                    ui.label(RichText::new(inst_txt).size(styles::FONT_SIZE_MD).color(inst_col).strong());
                    ui.end_row();

                    ui.label(RichText::new("运行状态").size(styles::FONT_SIZE_MD).color(styles::TEXT_LABEL_COLOR));
                    let (run_txt, run_col) = if self.is_running {
                        ("● 运行中", styles::SUCCESS_DARK)
                    } else {
                        ("● 已停止", styles::DANGER_DARK)
                    };
                    ui.label(RichText::new(run_txt).size(styles::FONT_SIZE_MD).color(run_col).strong());
                    ui.end_row();
                });
        });

        ui.add_space(styles::SPACING_MD);

        styles::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            self.section_header(ui, "⚙", "服务操作");

            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;

                if ui.button("🔄 刷新状态").clicked() {
                    self.refresh_status();
                }

                ui.separator();

                if !self.is_installed {
                    let btn = egui::Button::new(
                        RichText::new("📦 安装服务").color(Color32::WHITE).size(styles::FONT_SIZE_MD)
                    ).fill(styles::INFO_COLOR)
                     .corner_radius(egui::CornerRadius::same(6));
                    if ui.add(btn).clicked() {
                        match self.manager.install_service() {
                            Ok(_) => self.set_ok("服务安装成功，开机将自动启动"),
                            Err(e) => self.set_err(format!("安装失败：{}（需要管理员权限）", e)),
                        }
                    }
                } else {
                    if !self.is_running {
                        let btn = egui::Button::new(
                            RichText::new("▶ 启动服务").color(Color32::WHITE).size(styles::FONT_SIZE_MD)
                        ).fill(styles::SUCCESS_DARK)
                         .corner_radius(egui::CornerRadius::same(6));
                        if ui.add(btn).clicked() {
                            match self.manager.start_service() {
                                Ok(_) => self.set_ok("服务已启动"),
                                Err(e) => self.set_err(format!("启动失败：{}", e)),
                            }
                        }
                    } else {
                        let btn = egui::Button::new(
                            RichText::new("⏹ 停止服务").color(Color32::WHITE).size(styles::FONT_SIZE_MD)
                        ).fill(Color32::from_rgb(230, 126, 34))
                         .corner_radius(egui::CornerRadius::same(6));
                        if ui.add(btn).clicked() {
                            match self.manager.stop_service() {
                                Ok(_) => self.set_ok("服务已停止"),
                                Err(e) => self.set_err(format!("停止失败：{}", e)),
                            }
                        }
                    }

                    ui.separator();

                    if self.confirming_uninstall {
                        ui.label(RichText::new("确认卸载？").size(styles::FONT_SIZE_MD).color(styles::DANGER_DARK));
                        let yes = egui::Button::new(
                            RichText::new("确认").color(Color32::WHITE).size(styles::FONT_SIZE_MD)
                        ).fill(styles::DANGER_DARK)
                         .corner_radius(egui::CornerRadius::same(6));
                        if ui.add(yes).clicked() {
                            self.confirming_uninstall = false;
                            match self.manager.uninstall_service() {
                                Ok(_) => self.set_ok("服务已卸载"),
                                Err(e) => self.set_err(format!("卸载失败：{}", e)),
                            }
                        }
                        if ui.button("取消").clicked() {
                            self.confirming_uninstall = false;
                        }
                    } else {
                        let uninstall_btn = egui::Button::new(
                            RichText::new("🗑 卸载服务").size(styles::FONT_SIZE_MD).color(styles::DANGER_DARK)
                        ).fill(styles::DANGER_LIGHT)
                         .corner_radius(egui::CornerRadius::same(6));
                        if ui.add(uninstall_btn).clicked() {
                            self.confirming_uninstall = true;
                        }
                    }
                }
            });
        });

        ui.add_space(styles::SPACING_MD);

        styles::warning_box(ui, "注意事项", &[
            "安装/卸载服务需要以管理员身份运行本程序",
            "服务安装后将设为开机自动启动（AutoStart）",
            "服务以 SYSTEM 账户运行，配置文件位于 ProgramData\\wftpg\\",
            "停止服务会断开所有当前活动的 FTP/SFTP 连接",
        ]);
    }
}
