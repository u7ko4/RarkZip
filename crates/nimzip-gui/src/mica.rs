//! 采样当前桌面壁纸，生成接近 Windows 11 Mica 的底色。
//!
//! GPUI 的 Windows 后端使用 DirectComposition，系统 Mica 背景无法稳定透传到
//! 自绘内容；这里按 WinUI Gallery 的视觉规则，用壁纸平均色生成不透明材质底色，
//! 卡片与导航面板再叠加半透明表面，达到接近 Mica 的效果。

#[cfg(windows)]
pub fn refresh_wallpaper_tint() {
    let Some(path) = wallpaper_path() else {
        crate::theme::set_mica_tint((243, 243, 243));
        return;
    };
    let Some(tint) = average_color(&path) else {
        crate::theme::set_mica_tint((243, 243, 243));
        return;
    };
    crate::theme::set_mica_tint(tint);
}

#[cfg(windows)]
fn wallpaper_path() -> Option<std::path::PathBuf> {
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };

    let mut buffer = vec![0u16; 32768];
    unsafe {
        SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            buffer.len() as u32,
            Some(buffer.as_mut_ptr() as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .ok()?;
    }
    let len = buffer.iter().position(|value| *value == 0)?;
    if len == 0 {
        return None;
    }
    let path = std::path::PathBuf::from(String::from_utf16_lossy(&buffer[..len]));
    path.is_file().then_some(path)
}

#[cfg(windows)]
fn average_color(path: &std::path::Path) -> Option<(u8, u8, u8)> {
    let image = image::open(path).ok()?;
    let thumb = image.thumbnail(16, 16).to_rgb8();
    let mut total = [0u64; 3];
    let mut count = 0u64;
    for pixel in thumb.pixels() {
        total[0] += pixel[0] as u64;
        total[1] += pixel[1] as u64;
        total[2] += pixel[2] as u64;
        count += 1;
    }
    if count == 0 {
        return None;
    }
    Some((
        (total[0] / count) as u8,
        (total[1] / count) as u8,
        (total[2] / count) as u8,
    ))
}

#[cfg(not(windows))]
pub fn refresh_wallpaper_tint() {
    crate::theme::set_mica_tint((243, 243, 243));
}

/// 让系统标题栏跟随应用明暗主题（Mica 弹窗使用原生标题栏）。
#[cfg(windows)]
pub fn apply_titlebar(window: &gpui_kit::Window, dark: bool) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = HWND(win32.hwnd.get() as *mut _);
    let value: u32 = u32::from(dark);
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &value as *const u32 as *const _,
            std::mem::size_of::<u32>() as u32,
        );
    }
}

#[cfg(not(windows))]
pub fn apply_titlebar(_window: &gpui_kit::Window, _dark: bool) {}
