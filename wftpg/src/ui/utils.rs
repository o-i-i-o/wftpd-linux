use gtk::ApplicationWindow;
use gtk::prelude::*;

/// `u64` -> `f64`，用于设置 `SpinButton` 值。
/// GUI 数值域（端口/速率/时长等）远小于 `u32::MAX`，经 `u32` 饱和后由
/// `From` 无损转换，不存在精度损失。
#[must_use]
pub fn spin_f64_from(v: u64) -> f64 {
    f64::from(u32::try_from(v).unwrap_or(u32::MAX))
}

/// `SpinButton` 值（已被控件范围钳制为非负）取整为 `u64`。
/// `f64` 到整型没有 `TryFrom`，为避免 `as` 的截断语义，经十进制字符串
/// 取整转换：先 round 并钳制到 9e15（< 2^53，`f64` 可精确表示），解析
/// 必然成功；`NaN` 等异常值解析失败时回退为 0（与 `as` 的饱和语义一致）。
#[must_use]
pub fn spin_u64(value: f64) -> u64 {
    value
        .round()
        .clamp(0.0, 9.0e15)
        .to_string()
        .parse()
        .unwrap_or_default()
}

/// 同 [`spin_u64`]，目标类型为 `u32`
#[must_use]
pub fn spin_u32(value: f64) -> u32 {
    u32::try_from(spin_u64(value)).unwrap_or(u32::MAX)
}

/// 同 [`spin_u64`]，目标类型为 `usize`
#[must_use]
pub fn spin_usize(value: f64) -> usize {
    usize::try_from(spin_u64(value)).unwrap_or(usize::MAX)
}

/// `f64` -> `i32`（窗口尺寸），四舍五入并钳制到非负 `i32` 范围（理由同 [`spin_u64`]）
#[must_use]
pub fn f64_to_i32(value: f64) -> i32 {
    value
        .round()
        .clamp(0.0, 2.0e9)
        .to_string()
        .parse()
        .unwrap_or(i32::MAX)
}

pub fn setup_window_for_uos(window: &ApplicationWindow) {
    set_window_icon(window);

    let display = gtk::gdk::Display::default();
    if let Some(display) = display {
        let monitor = display.primary_monitor();
        if let Some(monitor) = monitor {
            let geometry = monitor.geometry();
            let scale_factor = monitor.scale_factor();

            if scale_factor > 1 {
                let width = f64_to_i32(f64::from(geometry.width()) * 0.7 / f64::from(scale_factor));
                let height =
                    f64_to_i32(f64::from(geometry.height()) * 0.7 / f64::from(scale_factor));
                window.set_default_size(width.min(1200), height.min(900));
            }
        }
    }
}

pub fn set_window_icon(window: &ApplicationWindow) {
    let icon_paths = [
        "/usr/share/icons/hicolor/scalable/apps/wftpg.svg",
        "/usr/share/icons/hicolor/256x256/apps/wftpg.png",
        "/usr/share/icons/hicolor/48x48/apps/wftpg.png",
        "/usr/share/pixmaps/wftpg.svg",
        "/usr/share/pixmaps/wftpg.png",
    ];

    for path in &icon_paths {
        let icon_path = std::path::Path::new(path);
        if icon_path.exists()
            && let Ok(pixbuf) = gtk::gdk_pixbuf::Pixbuf::from_file(icon_path)
        {
            window.set_icon(Some(&pixbuf));
            return;
        }
    }

    if let Some(icon_theme) = gtk::IconTheme::default() {
        for size in [48, 64, 128, 256] {
            if let Ok(Some(pixbuf)) =
                icon_theme.load_icon("wftpg", size, gtk::IconLookupFlags::empty())
            {
                window.set_icon(Some(&pixbuf));
                return;
            }
        }
    }
}
