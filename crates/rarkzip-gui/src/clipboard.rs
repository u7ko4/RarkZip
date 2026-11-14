//! 系统剪贴板文件列表（CF_HDROP）读写。
//!
//! 复制归档条目时先解压到临时目录，再把实际文件路径放入剪贴板；粘贴时读取
//! Explorer 复制的文件/文件夹路径，交给 ZIP 编辑接口追加。

#[cfg(not(windows))]
use std::path::PathBuf;

#[cfg(windows)]
mod platform {
    use crate::i18n::{tr, tr_fmt, Language};
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

    fn open_clipboard(language: Language) -> Result<(), String> {
        for _ in 0..10 {
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(20));
        }
        Err(tr(language, "剪贴板正被其他程序占用，请稍后重试").to_string())
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

    pub fn set_file_list(paths: &[PathBuf], language: Language) -> Result<(), String> {
        if paths.is_empty() {
            return Err(tr(language, "没有可复制的文件").to_string());
        }
        open_clipboard(language)?;
        let result = (|| -> Result<(), String> {
            unsafe { EmptyClipboard() }.map_err(|error| error.to_string())?;
            let mut wide = Vec::<u16>::new();
            for path in paths {
                wide.extend(absolute(path).as_os_str().encode_wide());
                wide.push(0);
            }
            if wide.is_empty() {
                return Err(tr(language, "没有可复制的文件").to_string());
            }
            // 末尾再补一个 NUL，形成 Explorer 需要的双 NUL 结尾。
            wide.push(0);

            let header_size = std::mem::size_of::<DROPFILES>();
            let total = header_size + wide.len() * 2;
            let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, total) }
                .map_err(|error| tr_fmt(language, "分配剪贴板内存失败：{}", &[&error]))?;
            let pointer = unsafe { GlobalLock(handle) } as *mut u8;
            if pointer.is_null() {
                unsafe {
                    let _ = GlobalFree(Some(handle));
                }
                return Err(tr(language, "锁定剪贴板内存失败").to_string());
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
                    Err(tr_fmt(language, "写入剪贴板失败：{}", &[&error]))
                }
            }
        })();
        unsafe { CloseClipboard() }.map_err(|error| error.to_string())?;
        result
    }

    pub fn file_list(language: Language) -> Result<Vec<PathBuf>, String> {
        if unsafe { IsClipboardFormatAvailable(CF_HDROP_FORMAT) }.is_err() {
            return Ok(Vec::new());
        }
        open_clipboard(language)?;
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
pub use platform::{file_list, set_file_list};

#[cfg(not(windows))]
use crate::i18n::{tr, Language};

#[cfg(not(windows))]
pub fn set_file_list(_paths: &[PathBuf], language: Language) -> Result<(), String> {
    Err(tr(language, "当前平台不支持文件剪贴板").to_string())
}

#[cfg(not(windows))]
pub fn file_list(language: Language) -> Result<Vec<PathBuf>, String> {
    Err(tr(language, "当前平台不支持文件剪贴板").to_string())
}
