//! 系统剪贴板文件列表读写。
//!
//! Windows 使用 `CF_HDROP` 与资源管理器互通；Linux/macOS 通过 GPUI 的
//! `ExternalPaths`（Wayland 的 `text/uri-list` 等）传递文件列表，并在
//! 只有纯文本时按行/URI 解析路径。
//!
//! 复制归档条目时先解压到临时目录，再把实际文件路径放入剪贴板；粘贴时读取
//! 文件管理器复制的文件/文件夹路径，交给 ZIP 编辑接口追加。

use std::path::PathBuf;

#[cfg(not(windows))]
use gpui_kit::App;

#[cfg(windows)]
mod platform {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::thread;
    use std::time::Duration;

    use windows::Win32::Foundation::{GlobalFree, HANDLE, TRUE};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
        OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::UI::Shell::{DragQueryFileW, DROPFILES, HDROP};

    const CF_HDROP_FORMAT: u32 = 15;

    fn open_clipboard() -> Result<(), String> {
        for _ in 0..10 {
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(20));
        }
        Err("剪贴板正被其他程序占用，请稍后重试".to_string())
    }

    fn absolute(path: &Path) -> PathBuf {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        // Explorer 的文件列表不接受 \\?\ 前缀；还原为普通绝对路径。
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            PathBuf::from(format!(r"\\{rest}"))
        } else if let Some(rest) = text.strip_prefix(r"\\?\") {
            PathBuf::from(rest.to_string())
        } else {
            path
        }
    }

    pub fn set_file_list(paths: &[PathBuf]) -> Result<(), String> {
        if paths.is_empty() {
            return Err("没有可复制的文件".to_string());
        }
        open_clipboard()?;
        let result = (|| -> Result<(), String> {
            unsafe { EmptyClipboard() }.map_err(|error| error.to_string())?;
            let mut wide = Vec::<u16>::new();
            for path in paths {
                wide.extend(absolute(path).as_os_str().encode_wide());
                wide.push(0);
            }
            if wide.is_empty() {
                return Err("没有可复制的文件".to_string());
            }
            // 末尾再补一个 NUL，形成 Explorer 需要的双 NUL 结尾。
            wide.push(0);

            let header_size = std::mem::size_of::<DROPFILES>();
            let total = header_size + wide.len() * 2;
            let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, total) }
                .map_err(|error| format!("分配剪贴板内存失败：{error}"))?;
            let pointer = unsafe { GlobalLock(handle) } as *mut u8;
            if pointer.is_null() {
                unsafe {
                    let _ = GlobalFree(Some(handle));
                }
                return Err("锁定剪贴板内存失败".to_string());
            }
            let header = DROPFILES {
                pFiles: header_size as u32,
                fWide: TRUE,
                ..Default::default()
            };
            unsafe {
                std::ptr::copy_nonoverlapping(
                    &header as *const DROPFILES as *const u8,
                    pointer,
                    header_size,
                );
                std::ptr::copy_nonoverlapping(
                    wide.as_ptr() as *const u8,
                    pointer.add(header_size),
                    wide.len() * 2,
                );
                let _ = GlobalUnlock(handle);
            }
            match unsafe { SetClipboardData(CF_HDROP_FORMAT, Some(HANDLE(handle.0))) } {
                Ok(_) => Ok(()),
                Err(error) => {
                    // SetClipboardData 失败时所有权仍在调用方，需释放。
                    unsafe {
                        let _ = GlobalFree(Some(handle));
                    }
                    Err(format!("写入剪贴板失败：{error}"))
                }
            }
        })();
        unsafe { CloseClipboard() }.map_err(|error| error.to_string())?;
        result
    }

    pub fn file_list() -> Result<Vec<PathBuf>, String> {
        if unsafe { IsClipboardFormatAvailable(CF_HDROP_FORMAT) }.is_err() {
            return Ok(Vec::new());
        }
        open_clipboard()?;
        let result = (|| -> Result<Vec<PathBuf>, String> {
            let handle =
                unsafe { GetClipboardData(CF_HDROP_FORMAT) }.map_err(|error| error.to_string())?;
            let drop = HDROP(handle.0);
            let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
            let mut paths = Vec::new();
            for index in 0..count {
                let length = unsafe { DragQueryFileW(drop, index, None) } as usize;
                if length == 0 {
                    continue;
                }
                let mut buffer = vec![0u16; length + 1];
                unsafe { DragQueryFileW(drop, index, Some(&mut buffer)) };
                let end = buffer.iter().position(|ch| *ch == 0).unwrap_or(length);
                if end > 0 {
                    paths.push(PathBuf::from(String::from_utf16_lossy(&buffer[..end])));
                }
            }
            Ok(paths)
        })();
        unsafe { CloseClipboard() }.map_err(|error| error.to_string())?;
        result
    }
}

#[cfg(windows)]
pub fn set_file_list(_cx: &mut gpui_kit::App, paths: &[PathBuf]) -> Result<(), String> {
    platform::set_file_list(paths)
}

#[cfg(windows)]
pub fn file_list(_cx: &mut gpui_kit::App) -> Result<Vec<PathBuf>, String> {
    platform::file_list()
}

#[cfg(not(windows))]
pub fn set_file_list(cx: &mut App, paths: &[PathBuf]) -> Result<(), String> {
    use gpui_kit::{ClipboardEntry, ClipboardItem, ExternalPaths};

    if paths.is_empty() {
        return Err("没有可复制的文件".to_string());
    }
    cx.write_to_clipboard(ClipboardItem {
        entries: vec![ClipboardEntry::ExternalPaths(ExternalPaths(
            paths.iter().cloned().collect(),
        ))],
    });
    Ok(())
}

#[cfg(not(windows))]
pub fn file_list(cx: &mut App) -> Result<Vec<PathBuf>, String> {
    use gpui_kit::ClipboardEntry;

    let Some(item) = cx.read_from_clipboard() else {
        return Ok(Vec::new());
    };
    let mut paths = Vec::new();
    for entry in &item.entries {
        if let ClipboardEntry::ExternalPaths(external) = entry {
            paths.extend(external.0.iter().cloned());
        }
    }
    if !paths.is_empty() {
        return Ok(paths);
    }
    // 回退：部分文件管理器只提供 text/plain（file:// URI 或每行一个路径）。
    if let Some(text) = item.text() {
        for line in text.lines() {
            let line = line.trim().trim_start_matches('\u{feff}');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let path = if let Some(rest) = line.strip_prefix("file://") {
                let rest = rest.strip_prefix("localhost").unwrap_or(rest);
                PathBuf::from(percent_decode(rest))
            } else {
                PathBuf::from(line)
            };
            if path.is_absolute() && path.exists() {
                paths.push(path);
            }
        }
    }
    Ok(paths)
}

/// 解析 `text/uri-list` 中的百分号编码（例如 `%E4%B8%AD`）。
#[cfg(not(windows))]
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && bytes[index + 1].is_ascii_hexdigit()
            && bytes[index + 2].is_ascii_hexdigit()
        {
            let hex = &input[index + 1..index + 3];
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
