use std::fs;
use std::path::Path;

use crate::error::{EngineError, Result};
use crate::paths::to_extended;
use crate::util::system_time_to_unix;

/// 读取归档文件上的 Mark-of-the-Web（Zone.Identifier）内容。
pub fn read_motw(path: &Path) -> Option<Vec<u8>> {
    let ads = format!("{}:Zone.Identifier", path.to_string_lossy());
    fs::read(ads).ok()
}

/// 把 MOTW 传播到解压产物，避免 SmartScreen 被绕过。
pub fn apply_motw(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let ads = format!("{}:Zone.Identifier", path.to_string_lossy());
    fs::write(ads, content)
}

/// 删除到回收站；失败时返回错误，绝不静默永久删除。
pub fn delete_to_recycle(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    trash::delete(path).map_err(|err| EngineError::Other(format!("移入回收站失败：{err}")))
}

pub fn set_mtime(path: &Path, timestamp: Option<i64>) -> std::io::Result<()> {
    if let Some(ts) = timestamp.and_then(crate::util::unix_to_system_time) {
        let ft = filetime::FileTime::from_system_time(ts);
        filetime::set_file_mtime(to_extended(path), ft)?;
    }
    Ok(())
}

pub fn modified_unix(path: &Path) -> Option<i64> {
    fs::metadata(to_extended(path))
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(system_time_to_unix)
}
