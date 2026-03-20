use egui::RichText;
use crate::core::ipc::{IpcClient, LogEntryDto};
use crate::gui_egui::styles;
use egui_extras::TableBuilder;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const PAGE_SIZE: usize = 100;
const LOG_THRESHOLD: usize = 500;

enum RefreshCommand {
    Fetch(usize),
}

enum RefreshResult {
    Logs(Vec<LogEntryDto>),
    Error(String),
}

pub struct LogTab {
    logs: Vec<LogEntryDto>,
    auto_refresh: bool,
    fetch_count: usize,
    fetch_count_buf: String,
    last_error: Option<String>,
    loading: bool,
    last_refresh_time: Option<Instant>,
    refresh_sender: mpsc::Sender<RefreshCommand>,
    refresh_receiver: mpsc::Receiver<RefreshResult>,
    current_page: usize,
    total_pages: usize,
}

impl Default for LogTab {
    fn default() -> Self {
        let (tx, rx_cmd) = mpsc::channel();
        let (tx_result, rx_result) = mpsc::channel();
        
        std::thread::spawn(move || {
            while let Ok(cmd) = rx_cmd.recv() {
                match cmd {
                    RefreshCommand::Fetch(count) => {
                        let result = if !IpcClient::is_server_running() {
                            RefreshResult::Error("后台服务未运行，无法获取日志".to_string())
                        } else {
                            match IpcClient::get_logs(count) {
                                Ok(resp) => {
                                    if let Some(logs) = resp.logs {
                                        RefreshResult::Logs(logs)
                                    } else {
                                        RefreshResult::Logs(Vec::new())
                                    }
                                }
                                Err(e) => RefreshResult::Error(format!("获取日志失败：{}", e)),
                            }
                        };
                        let _ = tx_result.send(result);
                    }
                }
            }
        });
        
        Self {
            logs: Vec::new(),
            auto_refresh: true,
            fetch_count: 200,
            fetch_count_buf: "200".to_string(),
            last_error: None,
            loading: false,
            last_refresh_time: None,
            refresh_sender: tx,
            refresh_receiver: rx_result,
            current_page: 1,
            total_pages: 1,
        }
    }
}

impl LogTab {
    pub fn new() -> Self {
        let mut tab = Self::default();
        tab.request_refresh();
        tab
    }

    fn request_refresh(&mut self) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.last_error = None;
        let _ = self.refresh_sender.send(RefreshCommand::Fetch(self.fetch_count));
    }

    fn check_refresh_result(&mut self, ctx: &egui::Context) {
        if let Ok(result) = self.refresh_receiver.try_recv() {
            self.loading = false;
            match result {
                RefreshResult::Logs(logs) => {
                    self.logs = logs;
                    self.last_error = None;
                    self.last_refresh_time = Some(Instant::now());
                    self.update_pagination();
                }
                RefreshResult::Error(e) => {
                    self.last_error = Some(e);
                }
            }
            ctx.request_repaint();
        }
    }

    fn update_pagination(&mut self) {
        self.total_pages = if self.logs.is_empty() {
            1
        } else {
            self.logs.len().div_ceil(PAGE_SIZE)
        };
        if self.current_page > self.total_pages {
            self.current_page = self.total_pages;
        }
    }

    fn get_page_logs(&self) -> &[LogEntryDto] {
        let start = (self.current_page - 1) * PAGE_SIZE;
        let end = std::cmp::min(start + PAGE_SIZE, self.logs.len());
        if start < self.logs.len() {
            &self.logs[start..end]
        } else {
            &[]
        }
    }

    fn format_last_refresh(&self) -> String {
        match self.last_refresh_time {
            Some(t) => {
                let elapsed = t.elapsed();
                if elapsed < Duration::from_secs(60) {
                    format!("{} 秒前刷新", elapsed.as_secs())
                } else if elapsed < Duration::from_secs(3600) {
                    format!("{} 分钟前刷新", elapsed.as_secs() / 60)
                } else {
                    format!("{} 小时前刷新", elapsed.as_secs() / 3600)
                }
            }
            None => "未刷新".to_string(),
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.check_refresh_result(ui.ctx());

        styles::page_header(ui, "📋", "系统日志");

        ui.horizontal(|ui| {
            let refresh_btn = if self.loading {
                egui::Button::new(RichText::new("⏳ 刷新中...").color(egui::Color32::GRAY).size(styles::FONT_SIZE_MD))
                    .fill(styles::BG_SECONDARY)
                    .corner_radius(egui::CornerRadius::same(6))
            } else {
                styles::small_button("🔄 刷新")
            };
            
            if ui.add(refresh_btn).clicked() && !self.loading {
                self.request_refresh();
            }
            
            ui.checkbox(&mut self.auto_refresh, "自动刷新");
            
            ui.label(RichText::new("显示条数:").size(styles::FONT_SIZE_MD).color(styles::TEXT_SECONDARY_COLOR));
            
            let response = styles::input_frame().show(ui, |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.fetch_count_buf)
                    .desired_width(60.0)
                    .font(egui::FontId::new(styles::FONT_SIZE_MD, egui::FontFamily::Proportional)))
            });
            
            if response.response.lost_focus() || response.response.has_focus() {
                if let Ok(v) = self.fetch_count_buf.parse::<usize>() {
                    let new_count = v.clamp(1, 10_000);
                    if new_count != self.fetch_count {
                        self.fetch_count = new_count;
                        self.fetch_count_buf = new_count.to_string();
                    }
                } else {
                    self.fetch_count_buf = self.fetch_count.to_string();
                }
            }
            
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("共 {} 条日志 | {}", self.logs.len(), self.format_last_refresh()))
                    .size(styles::FONT_SIZE_SM).color(styles::TEXT_MUTED_COLOR));
            });
        });

        if self.auto_refresh {
            ui.ctx().request_repaint_after(Duration::from_secs(2));
            if !self.loading {
                self.request_refresh();
            }
        }

        if let Some(err) = &self.last_error {
            styles::status_message(ui, err, false);
            ui.add_space(styles::SPACING_MD);
        }

        styles::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            
            if self.loading && self.logs.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(50.0);
                    ui.spinner();
                    ui.add_space(styles::SPACING_MD);
                    ui.label(RichText::new("正在加载日志...").size(styles::FONT_SIZE_MD).color(styles::TEXT_SECONDARY_COLOR));
                });
                return;
            }
            
            if self.logs.is_empty() {
                styles::empty_state(ui, "📭", "暂无日志记录", "点击刷新按钮获取最新日志");
                return;
            }

            let available_width = ui.available_width();
            let table = TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(styles::table_column_percent(available_width, 0.16, 130.0))
                .column(styles::table_column_percent(available_width, 0.07, 65.0))
                .column(styles::table_column_percent(available_width, 0.10, 80.0))
                .column(styles::table_column_percent(available_width, 0.12, 100.0))
                .column(styles::table_column_remainder(280.0))
                .min_scrolled_height(0.0)
                .sense(egui::Sense::hover());

            let display_logs = if self.logs.len() > LOG_THRESHOLD {
                self.get_page_logs()
            } else {
                &self.logs
            };

            table
                .header(styles::FONT_SIZE_MD, |mut header| {
                    header.col(|ui| {
                        ui.label(RichText::new("时间").strong().color(styles::TEXT_PRIMARY_COLOR));
                    });
                    header.col(|ui| {
                        ui.label(RichText::new("级别").strong().color(styles::TEXT_PRIMARY_COLOR));
                    });
                    header.col(|ui| {
                        ui.label(RichText::new("来源").strong().color(styles::TEXT_PRIMARY_COLOR));
                    });
                    header.col(|ui| {
                        ui.label(RichText::new("客户端").strong().color(styles::TEXT_PRIMARY_COLOR));
                    });
                    header.col(|ui| {
                        ui.label(RichText::new("消息详情").strong().color(styles::TEXT_PRIMARY_COLOR));
                    });
                })
                .body(|mut body| {
                    for entry in display_logs {
                        body.row(styles::FONT_SIZE_MD, |mut row| {
                            row.col(|ui| {
                                ui.label(RichText::new(&entry.timestamp)
                                    .size(styles::FONT_SIZE_SM)
                                    .color(styles::TEXT_SECONDARY_COLOR));
                            });
                            row.col(|ui| {
                                let level_color = match entry.level.as_str() {
                                    "ERROR" => styles::DANGER_COLOR,
                                    "WARN" => styles::WARNING_COLOR,
                                    "DEBUG" => styles::TEXT_MUTED_COLOR,
                                    _ => styles::SUCCESS_COLOR,
                                };
                                ui.label(RichText::new(&entry.level)
                                    .size(styles::FONT_SIZE_SM)
                                    .strong()
                                    .color(level_color));
                            });
                            row.col(|ui| {
                                ui.label(RichText::new(&entry.source)
                                    .size(styles::FONT_SIZE_SM)
                                    .color(styles::TEXT_SECONDARY_COLOR));
                            });
                            row.col(|ui| {
                                ui.label(RichText::new(
                                    entry.client_ip.as_deref().unwrap_or("-"),
                                )
                                .size(styles::FONT_SIZE_SM)
                                .color(styles::TEXT_LABEL_COLOR));
                            });
                            row.col(|ui| {
                                let msg = if let Some(user) = &entry.username {
                                    format!("[{}] {}", user, entry.message)
                                } else {
                                    entry.message.clone()
                                };
                                ui.label(RichText::new(&msg).size(styles::FONT_SIZE_SM).color(styles::TEXT_PRIMARY_COLOR));
                            });
                        });
                    }
                });
        });

        if self.logs.len() > LOG_THRESHOLD {
            ui.add_space(styles::SPACING_SM);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("第 {} / {} 页", self.current_page, self.total_pages))
                        .size(styles::FONT_SIZE_SM).color(styles::TEXT_MUTED_COLOR));
                    
                    ui.add_space(styles::SPACING_SM);
                    
                    if ui.add(styles::small_button("末页")).clicked() {
                        self.current_page = self.total_pages;
                    }
                    if ui.add(styles::small_button("下一页")).clicked() && self.current_page < self.total_pages {
                        self.current_page += 1;
                    }
                    if ui.add(styles::small_button("上一页")).clicked() && self.current_page > 1 {
                        self.current_page -= 1;
                    }
                    if ui.add(styles::small_button("首页")).clicked() {
                        self.current_page = 1;
                    }
                });
            });
        }
    }
}
