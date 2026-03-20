#![windows_subsystem = "windows"]

use eframe::{App, Frame};
use egui::{CentralPanel, RichText, Color32, IconData};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use wftpg::core::ipc::IpcClient;
use wftpg::core::server_manager::ServerManager;
use wftpg::gui_egui::{server_tab, user_tab, security_tab, service_tab, log_tab, file_log_tab, styles};

const INIT_TIMEOUT_SECS: u64 = 10;
const SERVICE_INSTALL_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, Copy, PartialEq)]
enum InitState {
    Loading,
    Ready,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
enum ServiceInstallStatus {
    None,
    Installing,
    Success(String),
    Failed(String),
}

struct InitResult {
    show_service_dialog: bool,
    ftp_running: bool,
    sftp_running: bool,
    server_running: bool,
}

struct StatusCheckResult {
    ftp_running: bool,
    sftp_running: bool,
    server_running: bool,
}

struct CachedStyles {
    loading_frame: egui::Frame,
    error_frame: egui::Frame,
    main_frame: egui::Frame,
    tab_frame: egui::Frame,
}

impl CachedStyles {
    fn new() -> Self {
        Self {
            loading_frame: egui::Frame::new().fill(styles::BG_PRIMARY),
            error_frame: egui::Frame::new().fill(styles::BG_PRIMARY),
            main_frame: egui::Frame::new()
                .fill(styles::BG_PRIMARY)
                .inner_margin(egui::Margin::same(16)),
            tab_frame: egui::Frame::new()
                .fill(styles::BG_CARD)
                .stroke(egui::Stroke::new(1.0, styles::BORDER_COLOR))
                .inner_margin(egui::Margin::symmetric(20, 12))
                .corner_radius(egui::CornerRadius::same(8)),
        }
    }
}

#[cfg(windows)]
mod admin {
    use std::process::Command;

    pub fn is_admin() -> bool {
        let output = Command::new("net")
            .args(["session"])
            .output();
        
        match output {
            Ok(o) => o.status.success(),
            Err(_) => false,
        }
    }

    pub fn request_admin() -> ! {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWDEFAULT;

        let exe_path = match std::env::current_exe() {
            Ok(path) => path,
            Err(_) => {
                eprintln!("无法获取当前程序路径");
                std::process::exit(1);
            }
        };
        
        fn to_wide(s: &str) -> Vec<u16> {
            OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
        }

        let result = unsafe {
            let operation = to_wide("runas");
            let file = to_wide(exe_path.to_string_lossy().as_ref());
            let params = to_wide("--elevated");

            ShellExecuteW(
                None,
                windows::core::PCWSTR(operation.as_ptr()),
                windows::core::PCWSTR(file.as_ptr()),
                windows::core::PCWSTR(params.as_ptr()),
                None,
                SW_SHOWDEFAULT,
            )
        };

        if result.0 as usize > 32 {
            std::process::exit(0);
        } else {
            eprintln!("请求管理员权限失败，请右键程序选择\"以管理员身份运行\"");
            std::process::exit(1);
        }
    }
}

struct WftpgApp {
    current_tab:    usize,
    server_tab:     server_tab::ServerTab,
    user_tab:       user_tab::UserTab,
    security_tab:   security_tab::SecurityTab,
    service_tab:    service_tab::ServiceTab,
    log_tab:        log_tab::LogTab,
    file_log_tab:   file_log_tab::FileLogTab,
    ftp_running:    bool,
    sftp_running:   bool,
    server_running: bool,
    show_service_install_dialog: bool,
    service_install_status: ServiceInstallStatus,
    service_install_receiver: Option<mpsc::Receiver<Result<(), String>>>,
    service_install_start_time: Option<Instant>,
    last_refresh:   Instant,
    init_state:     InitState,
    init_error:     Option<String>,
    init_receiver:  Option<mpsc::Receiver<Result<InitResult, String>>>,
    init_start_time: Instant,
    status_check_receiver: Option<mpsc::Receiver<StatusCheckResult>>,
    cached_styles:  CachedStyles,
}

impl WftpgApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_style(styles::get_custom_style());
        
        let (init_tx, init_rx) = mpsc::channel();
        let ctx_clone = cc.egui_ctx.clone();
        
        std::thread::spawn(move || {
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Self::do_initialization()
            }));
            
            let init_result = match res {
                Ok(Ok(result)) => Ok(result),
                Ok(Err(e)) => Err(e),
                Err(panic_info) => {
                    let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                        s.to_string()
                    } else if let Some(s) = panic_info.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "未知错误".to_string()
                    };
                    Err(format!("初始化时发生 panic: {}", msg))
                }
            };
            
            let _ = init_tx.send(init_result);
            ctx_clone.request_repaint();
        });
        
        Self {
            current_tab:    0,
            server_tab:     server_tab::ServerTab::new(),
            user_tab:       user_tab::UserTab::new(),
            security_tab:   security_tab::SecurityTab::new(),
            service_tab:    service_tab::ServiceTab::new(),
            log_tab:        log_tab::LogTab::new(),
            file_log_tab:   file_log_tab::FileLogTab::new(),
            ftp_running:    false,
            sftp_running:   false,
            server_running: false,
            show_service_install_dialog: false,
            service_install_status: ServiceInstallStatus::None,
            service_install_receiver: None,
            service_install_start_time: None,
            last_refresh:   Instant::now(),
            init_state:     InitState::Loading,
            init_error:     None,
            init_receiver:  Some(init_rx),
            init_start_time: Instant::now(),
            status_check_receiver: None,
            cached_styles:  CachedStyles::new(),
        }
    }
    
    fn do_initialization() -> Result<InitResult, String> {
        let manager = ServerManager::new();
        let is_service_installed = manager.is_service_installed();
        let mut show_service_dialog = false;

        if !is_service_installed
            && let Ok(current_exe) = std::env::current_exe()
            && let Some(exe_dir) = current_exe.parent() {
                let wftpd_exe = exe_dir.join("wftpd.exe");
                if wftpd_exe.exists() {
                    show_service_dialog = true;
                }
            }

        let (ftp_running, sftp_running, server_running) = if IpcClient::is_server_running() {
            match IpcClient::get_status() {
                Ok(resp) => (resp.ftp_running, resp.sftp_running, true),
                Err(e) => {
                    log::warn!("获取服务器状态失败: {}", e);
                    (false, false, false)
                }
            }
        } else {
            (false, false, false)
        };

        Ok(InitResult {
            show_service_dialog,
            ftp_running,
            sftp_running,
            server_running,
        })
    }
    
    fn check_init_result(&mut self, ctx: &egui::Context) {
        if self.init_start_time.elapsed() >= Duration::from_secs(INIT_TIMEOUT_SECS) {
            self.init_receiver = None;
            self.init_error = Some(format!(
                "初始化超时（{}秒），请检查程序权限或查看日志。",
                INIT_TIMEOUT_SECS
            ));
            self.init_state = InitState::Error;
            log::error!("应用初始化超时");
            return;
        }
        
        if let Some(rx) = &self.init_receiver
            && let Ok(result) = rx.try_recv() {
                self.init_receiver = None;
                
                match result {
                    Ok(init_result) => {
                        self.show_service_install_dialog = init_result.show_service_dialog;
                        self.ftp_running = init_result.ftp_running;
                        self.sftp_running = init_result.sftp_running;
                        self.server_running = init_result.server_running;
                        self.server_tab.update_status(init_result.ftp_running, init_result.sftp_running);
                        self.server_tab.load_config();
                        self.init_state = InitState::Ready;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    }
                    Err(e) => {
                        self.init_error = Some(e);
                        self.init_state = InitState::Error;
                        log::error!("应用初始化失败");
                    }
                }
            }
    }
    
    fn check_service_install_result(&mut self) {
        if let Some(start_time) = self.service_install_start_time
            && start_time.elapsed() >= Duration::from_secs(SERVICE_INSTALL_TIMEOUT_SECS)
        {
            self.service_install_receiver = None;
            self.service_install_start_time = None;
            self.service_install_status = ServiceInstallStatus::Failed(
                format!("服务安装超时（{}秒），请检查服务状态或手动安装。", SERVICE_INSTALL_TIMEOUT_SECS)
            );
            return;
        }
        
        if let Some(rx) = &self.service_install_receiver
            && let Ok(result) = rx.try_recv() {
                self.service_install_receiver = None;
                self.service_install_start_time = None;
                
                match result {
                    Ok(_) => {
                        self.service_install_status = ServiceInstallStatus::Success(
                            "服务安装并启动成功！".to_string()
                        );
                    }
                    Err(e) => {
                        self.service_install_status = ServiceInstallStatus::Failed(e);
                    }
                }
            }
    }
}

impl WftpgApp {
    fn check_server_status_async(&mut self, ctx: &egui::Context) {
        if self.status_check_receiver.is_some() {
            return;
        }
        
        let (tx, rx) = mpsc::channel();
        self.status_check_receiver = Some(rx);
        
        let ctx_clone = ctx.clone();
        std::thread::spawn(move || {
            let result = if IpcClient::is_server_running() {
                match IpcClient::get_status() {
                    Ok(resp) => StatusCheckResult {
                        ftp_running: resp.ftp_running,
                        sftp_running: resp.sftp_running,
                        server_running: true,
                    },
                    Err(e) => {
                        log::warn!("异步获取服务器状态失败: {}", e);
                        StatusCheckResult {
                            ftp_running: false,
                            sftp_running: false,
                            server_running: false,
                        }
                    }
                }
            } else {
                StatusCheckResult {
                    ftp_running: false,
                    sftp_running: false,
                    server_running: false,
                }
            };
            
            let _ = tx.send(result);
            ctx_clone.request_repaint();
        });
    }

    fn check_status_result(&mut self) {
        if let Some(rx) = &self.status_check_receiver
            && let Ok(result) = rx.try_recv() {
                self.status_check_receiver = None;
                self.ftp_running = result.ftp_running;
                self.sftp_running = result.sftp_running;
                self.server_running = result.server_running;
                self.server_tab.update_status(result.ftp_running, result.sftp_running);
                self.last_refresh = Instant::now();
            }
    }

    fn auto_refresh(&mut self, ctx: &egui::Context) {
        if self.last_refresh.elapsed() >= Duration::from_secs(3) {
            self.check_server_status_async(ctx);
        }
        self.check_status_result();
    }

    fn install_service(&mut self, ctx: &egui::Context) {
        self.service_install_status = ServiceInstallStatus::Installing;
        self.service_install_start_time = Some(Instant::now());
        
        let (tx, rx) = mpsc::channel();
        self.service_install_receiver = Some(rx);
        
        let ctx_clone = ctx.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let manager = ServerManager::new();
                manager.install_service()
                    .and_then(|_| manager.start_service())
            }));
            
            let final_result = match result {
                Ok(Ok(_)) => Ok(()),
                Ok(Err(e)) => Err(format!("服务安装失败: {}。请以管理员身份运行程序。", e)),
                Err(panic_info) => {
                    let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                        s.to_string()
                    } else if let Some(s) = panic_info.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "未知错误".to_string()
                    };
                    Err(format!("服务安装时发生 panic: {}", msg))
                }
            };
            
            let _ = tx.send(final_result);
            ctx_clone.request_repaint();
        });
    }

    fn show_service_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_service_install_dialog {
            return;
        }

        egui::Window::new("安装后台服务")
            .collapsible(false)
            .resizable(false)
            .fixed_size([520.0, 0.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(styles::SPACING_LG);
                    ui.label(RichText::new("🔔 检测到 wftpd.exe").size(styles::FONT_SIZE_LG).strong());
                    ui.add_space(styles::SPACING_SM);
                    ui.label(RichText::new("WFTPG 后台服务尚未安装。").size(styles::FONT_SIZE_MD));
                    ui.label(RichText::new("是否将 wftpd.exe 注册为 Windows 服务并启动？").size(styles::FONT_SIZE_MD));
                    ui.add_space(styles::SPACING_LG);

                    match &self.service_install_status {
                        ServiceInstallStatus::Installing => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("正在安装服务...").size(styles::FONT_SIZE_MD));
                            });
                            ui.add_space(styles::SPACING_SM);
                            ui.label(RichText::new("这可能需要几秒钟，请稍候...").size(styles::FONT_SIZE_SM).color(styles::TEXT_MUTED_COLOR));
                        }
                        ServiceInstallStatus::Success(msg) => {
                            ui.label(RichText::new(format!("✓ {}", msg)).color(styles::SUCCESS_COLOR).size(styles::FONT_SIZE_MD));
                            ui.add_space(styles::SPACING_LG);
                            
                            if ui.add(styles::secondary_button("关闭")).clicked() {
                                self.show_service_install_dialog = false;
                                self.service_install_status = ServiceInstallStatus::None;
                            }
                        }
                        ServiceInstallStatus::Failed(msg) => {
                            ui.label(RichText::new(format!("✗ {}", msg)).color(styles::DANGER_COLOR).size(styles::FONT_SIZE_MD));
                            ui.add_space(styles::SPACING_SM);
                            
                            egui::Frame::new()
                                .fill(styles::BG_SECONDARY)
                                .inner_margin(egui::Margin::same(8))
                                .corner_radius(egui::CornerRadius::same(4))
                                .show(ui, |ui| {
                                    ui.label(RichText::new("手动安装命令:").size(styles::FONT_SIZE_SM).strong());
                                    ui.label(RichText::new("sc create wftpd binPath= \"<安装目录>\\wftpd.exe\" start= auto").size(styles::FONT_SIZE_SM).color(styles::TEXT_MUTED_COLOR));
                                    ui.add_space(styles::SPACING_XS);
                                    ui.label(RichText::new("sc start wftpd").size(styles::FONT_SIZE_SM).color(styles::TEXT_MUTED_COLOR));
                                });
                            
                            ui.add_space(styles::SPACING_LG);
                            
                            ui.horizontal(|ui| {
                                if ui.add(styles::secondary_button("关闭")).clicked() {
                                    self.show_service_install_dialog = false;
                                    self.service_install_status = ServiceInstallStatus::None;
                                }
                                if ui.add(styles::primary_button("重试")).clicked() {
                                    self.install_service(ctx);
                                }
                            });
                        }
                        ServiceInstallStatus::None => {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = styles::SPACING_MD;

                                if ui.add(styles::secondary_button("稍后手动安装")).clicked() {
                                    self.show_service_install_dialog = false;
                                }

                                if ui.add(styles::primary_button("安装并启动服务")).clicked() {
                                    self.install_service(ctx);
                                }
                            });
                        }
                    }
                    ui.add_space(styles::SPACING_MD);
                });
            });
    }
}

impl App for WftpgApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        match self.init_state {
            InitState::Loading => {
                self.check_init_result(ctx);
                
                CentralPanel::default()
                    .frame(self.cached_styles.loading_frame)
                    .show(ctx, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(ui.available_height() / 2.0 - 50.0);
                            ui.spinner();
                            ui.add_space(styles::SPACING_MD);
                            ui.label(RichText::new("正在初始化...").size(styles::FONT_SIZE_LG).color(styles::TEXT_SECONDARY_COLOR));
                        });
                    });
                return;
            }
            InitState::Error => {
                CentralPanel::default()
                    .frame(self.cached_styles.error_frame)
                    .show(ctx, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(ui.available_height() / 2.0 - 80.0);
                            ui.label(RichText::new("⚠ 初始化失败").size(styles::FONT_SIZE_LG).strong().color(styles::DANGER_COLOR));
                            ui.add_space(styles::SPACING_MD);
                            if let Some(error) = &self.init_error {
                                ui.label(RichText::new(error).size(styles::FONT_SIZE_MD).color(styles::TEXT_SECONDARY_COLOR));
                            }
                            ui.add_space(styles::SPACING_LG);
                            if ui.add(styles::primary_button("退出程序")).clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        });
                    });
                return;
            }
            InitState::Ready => {}
        }
        
        self.check_service_install_result();
        self.auto_refresh(ctx);
        self.show_service_dialog(ctx);

        CentralPanel::default()
            .frame(self.cached_styles.main_frame)
            .show(ctx, |ui| {
                ui.add_space(12.0);
                
                self.cached_styles.tab_frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        
                        let tabs = [
                            ("⚙", "服务器",   0usize),
                            ("👤", "用户管理", 1),
                            ("🔒", "安全设置", 2),
                            ("🖥", "系统服务", 3),
                            ("📋", "运行日志", 4),
                            ("📁", "文件日志", 5),
                        ];
                        
                        for (icon, label, idx) in &tabs {
                            let selected = self.current_tab == *idx;
                            
                            let bg_color = if selected {
                                Color32::from_rgb(243, 232, 255)
                            } else {
                                Color32::TRANSPARENT
                            };
                            
                            let text_color = if selected {
                                styles::PRIMARY_COLOR
                            } else {
                                styles::TEXT_SECONDARY_COLOR
                            };
                            
                            let text = RichText::new(format!("{icon}  {label}"))
                                .size(14.0)
                                .strong()
                                .color(text_color);
                            
                            let btn = egui::Button::new(text)
                                .fill(bg_color)
                                .stroke(if selected {
                                    egui::Stroke::new(1.5, styles::PRIMARY_COLOR)
                                } else {
                                    egui::Stroke::new(1.0, styles::BORDER_COLOR)
                                })
                                .corner_radius(egui::CornerRadius::same(6))
                                .min_size(egui::vec2(110.0, 42.0));
                            
                            let resp = ui.add(btn);
                            if resp.clicked() { 
                                self.current_tab = *idx; 
                            }
                        }
                    });
                });

                ui.add_space(12.0);

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        match self.current_tab {
                            0 => self.server_tab.ui(ui),
                            1 => self.user_tab.ui(ui),
                            2 => self.security_tab.ui(ui),
                            3 => self.service_tab.ui(ui),
                            4 => self.log_tab.ui(ui),
                            5 => self.file_log_tab.ui(ui),
                            _ => self.server_tab.ui(ui),
                        }
                    });
                    
                ui.add_space(12.0);
            });
    }
}

fn setup_fonts(ctx: &egui::Context) {
    use egui::{FontData, FontDefinitions, FontFamily};
    let mut fonts = FontDefinitions::default();
    
    let candidates = [
        ("C:\\Windows\\Fonts\\msyh.ttc", "Microsoft YaHei"),
        ("C:\\Windows\\Fonts\\msyhbd.ttc", "Microsoft YaHei Bold"),
        ("C:\\Windows\\Fonts\\simsun.ttc", "SimSun"),
        ("C:\\Windows\\Fonts\\simhei.ttf", "SimHei"),
    ];
    
    let mut loaded = false;
    for (path, name) in &candidates {
        match std::fs::read(path) {
            Ok(data) => {
                fonts.font_data.insert((*name).into(), FontData::from_owned(data).into());
                if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
                    family.push((*name).into());
                }
                if let Some(family) = fonts.families.get_mut(&FontFamily::Monospace) {
                    family.push((*name).into());
                }
                loaded = true;
                log::info!("成功加载中文字体: {}", name);
                break;
            }
            Err(e) => {
                log::warn!("加载字体 {} 失败: {}", path, e);
            }
        }
    }
    
    if !loaded {
        log::warn!("未能加载任何中文字体，将使用系统默认字体");
    }
    
    ctx.set_fonts(fonts);
}

fn load_icon() -> IconData {
    let exe_dir = match std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf())) {
        Some(dir) => dir,
        None => {
            log::warn!("无法获取程序目录，使用默认图标");
            return create_default_icon();
        }
    };
    
    let icon_path = exe_dir.join("ui/wftpg.ico");
    if !icon_path.exists() {
        log::warn!("图标文件不存在: {:?}，使用默认图标", icon_path);
        return create_default_icon();
    }
    
    match std::fs::read(&icon_path) {
        Ok(data) => {
            match ico::IconDir::read(std::io::Cursor::new(&data)) {
                Ok(icon) => {
                    for entry in icon.entries() {
                        if let Ok(image) = entry.decode() {
                            let rgba = image.rgba_data().to_vec();
                            let width = entry.width();
                            let height = entry.height();
                            log::info!("成功加载图标: {}x{}", width, height);
                            return IconData { rgba, width, height };
                        }
                    }
                }
                Err(e) => {
                    log::warn!("解析图标文件失败: {}", e);
                }
            }
        }
        Err(e) => {
            log::warn!("读取图标文件失败: {}", e);
        }
    }
    
    create_default_icon()
}

fn create_default_icon() -> IconData {
    let size = 32u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    
    for y in 0..size {
        for x in 0..size {
            let dx = (x as i32 - 16).abs();
            let dy = (y as i32 - 16).abs();
            let dist = ((dx * dx + dy * dy) as f64).sqrt();
            
            let (r, g, b, a) = if dist < 14.0 {
                (108, 92, 231, 255)
            } else if dist < 16.0 {
                (80, 70, 200, 255)
            } else {
                (0, 0, 0, 0)
            };
            
            rgba.push(r);
            rgba.push(g);
            rgba.push(b);
            rgba.push(a);
        }
    }
    
    IconData {
        rgba,
        width: size,
        height: size,
    }
}

fn main() -> eframe::Result<()> {
    #[cfg(windows)]
    {
        if !admin::is_admin() {
            admin::request_admin();
        }
    }

    let icon = load_icon();
    
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 750.0])
            .with_min_inner_size([900.0, 650.0])
            .with_resizable(true)
            .with_visible(false)
            .with_icon(icon),
        ..Default::default()
    };
    
    eframe::run_native(
        "WFTPG - SFTP/FTP 管理工具",
        options,
        Box::new(|cc| {
            setup_fonts(&cc.egui_ctx);
            Ok(Box::new(WftpgApp::new(cc)))
        }),
    )
}
