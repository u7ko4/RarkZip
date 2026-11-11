//! Windows AMSI 恶意软件扫描桥。
//!
//! 扫描失败（AMSI 未安装、Provider 不可用、非 Windows）不会阻塞解压，
//! 由调用方决定提示方式。扫描只读取配置的上限字节，避免大文件卡住前台。

use std::path::Path;

use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AmsiVerdict {
    Clean,
    Detected,
    Unavailable(String),
}

impl AmsiVerdict {
    pub fn is_detected(&self) -> bool {
        matches!(self, Self::Detected)
    }
}

pub fn scan_bytes(content_name: &str, data: &[u8]) -> AmsiVerdict {
    #[cfg(windows)]
    {
        windows_scan(content_name, data)
    }
    #[cfg(not(windows))]
    {
        let _ = (content_name, data);
        AmsiVerdict::Unavailable("AMSI 仅支持 Windows".to_string())
    }
}

pub fn scan_file(path: &Path, max_bytes: u64) -> Result<AmsiVerdict> {
    use std::io::Read;
    let file = std::fs::File::open(crate::paths::to_extended(path))?;
    let limit = max_bytes.clamp(64 * 1024, 256 * 1024 * 1024) as usize;
    let mut data = Vec::with_capacity(limit.min(4 * 1024 * 1024));
    file.take(limit as u64).read_to_end(&mut data)?;
    Ok(scan_bytes(&path.to_string_lossy(), &data))
}

#[cfg(windows)]
fn windows_scan(content_name: &str, data: &[u8]) -> AmsiVerdict {
    use windows::core::HSTRING;
    use windows::Win32::System::Antimalware::{
        AmsiCloseSession, AmsiInitialize, AmsiOpenSession, AmsiScanBuffer, AmsiUninitialize,
        AMSI_RESULT_BLOCKED_BY_ADMIN_END, AMSI_RESULT_BLOCKED_BY_ADMIN_START, AMSI_RESULT_DETECTED,
    };

    if data.is_empty() {
        return AmsiVerdict::Clean;
    }
    let app = HSTRING::from("NimZip");
    let name = HSTRING::from(content_name);
    unsafe {
        let context = match AmsiInitialize(&app) {
            Ok(context) => context,
            Err(error) => {
                return AmsiVerdict::Unavailable(format!("AMSI 初始化失败：{error}"));
            }
        };
        let mut session = AmsiOpenSession(context).ok();
        let result = AmsiScanBuffer(
            context,
            data.as_ptr() as *const core::ffi::c_void,
            data.len().min(u32::MAX as usize) as u32,
            &name,
            session,
        );
        if let Some(session) = session.take() {
            AmsiCloseSession(context, session);
        }
        AmsiUninitialize(context);
        match result {
            Ok(value) => {
                if value.0 >= AMSI_RESULT_DETECTED.0
                    || (value.0 >= AMSI_RESULT_BLOCKED_BY_ADMIN_START.0
                        && value.0 <= AMSI_RESULT_BLOCKED_BY_ADMIN_END.0)
                {
                    AmsiVerdict::Detected
                } else {
                    AmsiVerdict::Clean
                }
            }
            Err(error) => AmsiVerdict::Unavailable(error.to_string()),
        }
    }
}
