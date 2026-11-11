use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::Result;
use crate::paths::to_extended;
use crate::progress::{check_cancelled, Phase, Progress, ProgressSink};

pub const COPY_BUFFER_SIZE: usize = 1024 * 1024;

/// 只读取文件头部若干字节，不把整个文件载入内存。
pub fn read_prefix(path: &Path, len: usize) -> std::io::Result<Vec<u8>> {
    let mut file = File::open(to_extended(path))?;
    let mut buf = vec![0u8; len];
    let mut filled = 0;
    while filled < len {
        match file.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    buf.truncate(filled);
    Ok(buf)
}

/// 带进度与取消检查的流复制。
#[allow(clippy::too_many_arguments)]
pub fn copy_with_progress(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
    sink: &dyn ProgressSink,
    phase: Phase,
    done: &mut u64,
    total: u64,
    current: &str,
) -> Result<u64> {
    let mut buffer = vec![0u8; COPY_BUFFER_SIZE];
    let mut written = 0u64;
    loop {
        check_cancelled(sink)?;
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read])?;
        *done += read as u64;
        written += read as u64;
        sink.update(&Progress::new(phase, *done, total, current));
    }
    Ok(written)
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn format_ratio(compressed: u64, original: u64) -> String {
    if original == 0 {
        return "-".to_string();
    }
    format!("{:.0}%", compressed as f64 / original as f64 * 100.0)
}

pub fn format_unix_time(timestamp: Option<i64>) -> String {
    match timestamp {
        Some(ts) if ts > 0 => chrono::DateTime::from_timestamp(ts, 0)
            .map(|dt| {
                dt.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|| "-".to_string()),
        _ => "-".to_string(),
    }
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn system_time_to_unix(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

pub fn unix_to_system_time(timestamp: i64) -> Option<SystemTime> {
    if timestamp <= 0 {
        None
    } else {
        Some(UNIX_EPOCH + std::time::Duration::from_secs(timestamp as u64))
    }
}

/// 预分配目标文件空间，减少碎片。
pub fn preallocate(file: &File, size: u64) -> std::io::Result<()> {
    if size > 0 {
        file.set_len(size)?;
    }
    Ok(())
}

pub fn trim_comment(comment: &[u8]) -> String {
    String::from_utf8_lossy(comment).trim().to_string()
}

pub fn basename(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}
