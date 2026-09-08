use gtk::ApplicationWindow;
use gtk::prelude::*;

/// `u64` -> `f64`（GTK `SpinButton` 数值域），小值无精度问题；集中放置以通过 pedantic cast 检查
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn spin_f64_from(v: u64) -> f64 {
    v as f64
}

/// `SpinButton` 值（已被控件范围钳制为非负）取整为 `u64`。
/// `f64` 无 `TryFrom` 到整型，只能 `as` 转换；先 round 并钳制到 9e15
/// （< 2^53，`f64` 可精确表示，亦小于 `u64::MAX`），故转换不可能截断或丢符号。
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn spin_u64(value: f64) -> u64 {
    value.round().clamp(0.0, 9.0e15) as u64
}

/// 同 [`spin_u64`]，目标类型为 `u32`（钳制上限 4e9 < `u32::MAX`）
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn spin_u32(value: f64) -> u32 {
    value.round().clamp(0.0, 4.0e9) as u32
}

/// 同 [`spin_u64`]，目标类型为 `usize`
#[must_use]
pub fn spin_usize(value: f64) -> usize {
    usize::try_from(spin_u64(value)).unwrap_or(usize::MAX)
}

/// `f64` -> `i32`（窗口尺寸），四舍五入并钳制到非负 `i32` 范围（理由同 [`spin_u64`]）
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn f64_to_i32(value: f64) -> i32 {
    value.round().clamp(0.0, 2.0e9) as i32
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
