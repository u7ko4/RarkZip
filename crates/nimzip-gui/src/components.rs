use gpui_kit::component::Icon;
use gpui_kit::*;

use crate::theme::Palette;

pub use gpui_kit::assets::IconName;

/// 所有按钮、下拉框与输入框共用的控件高度。
pub const CONTROL_HEIGHT: f32 = 36.0;
/// 工具栏图标按钮的点击区域。
pub const ICON_BUTTON_SIZE: f32 = 36.0;
/// 工具栏图标按钮的高度。
pub const ICON_BUTTON_HEIGHT: f32 = 36.0;

/// 通过 `Styled` 设置高度；`Input` 自带的 `h` 只对多行输入生效，
/// 单行输入若直接调用会被忽略，因此统一走这个辅助函数。
pub fn control_height<T: Styled>(element: T) -> T {
    element.h(px(CONTROL_HEIGHT))
}

/// Run a native file dialog away from the GPUI main thread.
///
/// GPUI holds a mutable `App` borrow while dispatching input. A blocking
/// `rfd::FileDialog` starts a nested Win32 message loop on that thread, which
/// can re-enter GPUI and panic with `RefCell already borrowed`. Keeping the
/// dialog on a dedicated thread avoids that re-entrancy; the modal dialog stays
/// responsive while the UI thread waits for the picked path.
pub fn file_dialog<T, F>(dialog: F) -> T
where
    T: Send + Default + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};

        let previous = unsafe { GetForegroundWindow() };
        let result = spawn_dialog(None, dialog);
        if !previous.is_invalid() {
            let _ = unsafe { SetForegroundWindow(previous) };
        }
        result
    }
    #[cfg(not(target_os = "windows"))]
    {
        dialog()
    }
}

/// 与 [`file_dialog`] 相同，并在对话框关闭后重新激活 `window`。
/// 独立功能窗口（新建压缩包/文件哈希/密码恢复）必须使用这个版本，否则
/// 对话框关闭后 Windows 会把焦点交还给主窗口。
///
/// 注意：不能在对话框线程把 `window` 设为 owner。GPUI 主线程在等待对话框
/// 结果时是阻塞的，owner 线程无响应会让文件对话框无法显示/卡死。
pub fn file_dialog_in<T, F>(window: &Window, dialog: F) -> T
where
    T: Send + Default + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;

        let hwnd = window_hwnd(window).map(|hwnd| hwnd as isize);
        // 对话框期间禁用同进程的其他顶层窗口（主要是主窗口），这样对话框
        // 关闭后 Windows 不会先激活主窗口再让我们抢回，避免可见的焦点闪动。
        let disabled = hwnd.map(disable_other_windows).unwrap_or_default();
        // 对话框线程在 Show 返回后立刻激活功能窗口。
        let result = spawn_dialog(hwnd, dialog);
        window.activate_window();
        if !disabled.is_empty() {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(200));
                for hwnd in disabled {
                    let _ = unsafe {
                        EnableWindow(
                            windows::Win32::Foundation::HWND(hwnd as *mut std::ffi::c_void),
                            true,
                        )
                    };
                }
            });
        }
        result
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
        dialog()
    }
}

#[cfg(target_os = "windows")]
fn window_hwnd(window: &Window) -> Option<*mut std::ffi::c_void> {
    use raw_window_handle::RawWindowHandle;

    match raw_window_handle::HasWindowHandle::window_handle(window)
        .ok()?
        .as_raw()
    {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as *mut std::ffi::c_void),
        _ => None,
    }
}

/// 禁用同进程的其他可见顶层窗口（保留功能窗口自身），返回被禁用的句柄。
/// 调用发生在 GPUI 线程阻塞前，同线程 `EnableWindow` 不会死锁。
#[cfg(target_os = "windows")]
fn disable_other_windows(feature: isize) -> Vec<isize> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetTopWindow, GetWindow, GetWindowThreadProcessId, IsWindowVisible, GW_HWNDNEXT,
    };

    let mut disabled = Vec::new();
    let pid = std::process::id();
    let mut hwnd = unsafe { GetTopWindow(None) }.unwrap_or_default();
    while !hwnd.is_invalid() {
        let mut window_pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut window_pid)) };
        if window_pid == pid
            && hwnd.0 as isize != feature
            && unsafe { IsWindowVisible(hwnd).as_bool() }
            && unsafe { IsWindowEnabled(hwnd).as_bool() }
        {
            let _ = unsafe { EnableWindow(hwnd, false) };
            disabled.push(hwnd.0 as isize);
        }
        hwnd = unsafe { GetWindow(hwnd, GW_HWNDNEXT) }.unwrap_or_default();
    }
    disabled
}

#[cfg(target_os = "windows")]
fn spawn_dialog<T, F>(activate_after: Option<isize>, dialog: F) -> T
where
    T: Send + Default + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (sender, receiver) = std::sync::mpsc::channel();
    let _ = std::thread::Builder::new()
        .name("nimzip-file-dialog".to_string())
        .spawn(move || {
            let result = dialog();
            if let Some(hwnd) = activate_after {
                use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
                let _ = unsafe {
                    SetForegroundWindow(windows::Win32::Foundation::HWND(
                        hwnd as *mut std::ffi::c_void,
                    ))
                };
            }
            let _ = sender.send(result);
        });
    receiver.recv().unwrap_or_default()
}

pub fn progress_bar(percent: f32, palette: &Palette, color: Hsla) -> impl IntoElement {
    let width = percent.clamp(0.0, 100.0);
    div()
        .h(px(4.0))
        .w_full()
        .rounded(px(99.0))
        .bg(palette.panel)
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .w(relative(width / 100.0))
                .rounded(px(99.0))
                .bg(color),
        )
}

pub fn empty_panel(text: impl Into<SharedString>, palette: &Palette) -> Div {
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_size(crate::theme::font_px(12.5))
        .text_color(palette.sub)
        .child(text.into())
}

pub fn chip(text: impl Into<SharedString>, palette: &Palette) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(5.0))
        .h(px(30.0))
        .px(px(10.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(palette.line)
        .text_size(crate::theme::font_px(12.5))
        .text_color(palette.sub)
        .child(text.into())
}

pub fn icon(icon: IconName, color: Hsla, size: f32) -> Icon {
    Icon::new(icon).text_color(color).size(px(size))
}
