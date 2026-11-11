//! 原生 ZPAQ 后端：使用 zpaq_rs 内置的 libzpaq 读取/校验。
//!
//! libzpaq 的 JIDAC 引擎通过命令接口解包；这里先解到隔离临时目录，再按
//! 安全路径复制到目标位置，以支持选中条目、覆盖策略与 ZIP-slip 防护。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::archive::{expand_selection, ArchiveEntry, ArchiveInfo, VerifyReport};
use crate::error::{EngineError, Result};
use crate::format::ArchiveFormat;
use crate::paths::{safe_join, to_extended};
use crate::progress::{check_cancelled, Phase, Progress, ProgressSink};
use crate::security::{apply_motw, set_mtime};
use crate::zip_backend::ExtractOutcome;

fn map_err(error: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("读取 ZPAQ 失败：{error}"))
}

/// libzpaq 命令引擎与进程当前目录有全局状态，创建/列表/解包统一串行。
pub(crate) static ZPAQ_LOCK: Mutex<()> = Mutex::new(());

/// 解析 `zpaq list` 的条目行：
/// `- 2026-11-07 08:50:34    11 A     alpha.txt`
fn parse_list_line(line: &str) -> Option<ArchiveEntry> {
    let rest = line.trim_start().strip_prefix("- ")?;
    let bytes = rest.as_bytes();
    let mut cursor = 0usize;
    let mut tokens = [""; 4];
    for token in &mut tokens {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let start = cursor;
        while cursor < bytes.len() && !bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        *token = rest.get(start..cursor)?;
    }
    while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    let path = rest.get(cursor..)?.trim();
    if path.is_empty() {
        return None;
    }
    let size = tokens[2].parse::<u64>().ok()?;
    let is_dir = tokens[3].eq_ignore_ascii_case("D") || path.ends_with('/');
    let modified = chrono::NaiveDateTime::parse_from_str(
        &format!("{} {}", tokens[0], tokens[1]),
        "%Y-%m-%d %H:%M:%S",
    )
    .ok()
    .map(|value| value.and_utc().timestamp());
    Some(ArchiveEntry {
        index: 0,
        path: path.trim_end_matches('/').to_string(),
        is_dir,
        size: if is_dir { 0 } else { size },
        // libzpaq 的列表不提供逐条压缩后大小。
        compressed_size: 0,
        modified,
        method: "ZPAQ".to_string(),
        encrypted: false,
        crc32: None,
        is_symlink: false,
        comment: None,
    })
}

pub(crate) fn list(path: &Path, _password: Option<&str>) -> Result<ArchiveInfo> {
    let archive = path.to_string_lossy().to_string();
    let output = {
        let _guard = ZPAQ_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        zpaq_rs::zpaq_list(&archive, &[]).map_err(map_err)?
    };
    let mut order: Vec<String> = Vec::new();
    let mut by_path: HashMap<String, ArchiveEntry> = HashMap::new();
    for line in output.stdout.lines() {
        let Some(entry) = parse_list_line(line) else {
            continue;
        };
        if !by_path.contains_key(&entry.path) {
            order.push(entry.path.clone());
        }
        by_path.insert(entry.path.clone(), entry);
    }
    let entries = order
        .into_iter()
        .filter_map(|key| by_path.remove(&key))
        .enumerate()
        .map(|(index, mut entry)| {
            entry.index = index;
            entry
        })
        .collect();
    Ok(ArchiveInfo {
        path: path.to_path_buf(),
        payload_path: path.to_path_buf(),
        disguised: false,
        format: ArchiveFormat::Zpaq,
        entries,
        comment: None,
        encrypted: false,
        protected_container: false,
        payload_offset: 0,
        note: None,
    })
}

fn temp_dir(tag: &str, path: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    std::env::temp_dir()
        .join("NimZip")
        .join(tag)
        .join(format!("{}-{stamp}-{name}", std::process::id()))
}

fn extract_to_temp(path: &Path, sink: &dyn ProgressSink) -> Result<PathBuf> {
    let temp = temp_dir("zpaq", path);
    if temp.exists() {
        let _ = std::fs::remove_dir_all(to_extended(&temp));
    }
    std::fs::create_dir_all(to_extended(&temp))?;
    let archive = path.to_string_lossy().to_string();
    let output_dir = temp.to_string_lossy().to_string();
    sink.update(&Progress::new(
        Phase::Extracting,
        0,
        1,
        "ZPAQ：解包到临时目录".to_string(),
    ));
    let result = {
        let _guard = ZPAQ_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        zpaq_rs::zpaq_command(&["extract", &archive, "-force", "-to", &output_dir])
    };
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(to_extended(&temp));
        return Err(map_err(error));
    }
    Ok(temp)
}

pub(crate) fn verify(
    path: &Path,
    _password: Option<&str>,
    sink: &dyn ProgressSink,
) -> Result<VerifyReport> {
    let info = list(path, None)?;
    let temp = extract_to_temp(path, sink)?;
    let mut bytes = 0u64;
    let mut files = 0usize;
    for entry in &info.entries {
        check_cancelled(sink)?;
        if entry.is_dir {
            continue;
        }
        let extracted = temp.join(entry.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let size = std::fs::metadata(to_extended(&extracted))
            .map(|meta| meta.len())
            .unwrap_or(0);
        bytes += size;
        files += 1;
        sink.update(&Progress::new(
            Phase::Testing,
            bytes,
            info.total_size(),
            entry.path.clone(),
        ));
    }
    let _ = std::fs::remove_dir_all(to_extended(&temp));
    Ok(VerifyReport {
        format: ArchiveFormat::Zpaq,
        entries: files,
        bytes,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn extract(
    path: &Path,
    info: &ArchiveInfo,
    dest: &Path,
    strip_prefix: Option<&str>,
    selected: Option<&[usize]>,
    _password: Option<&str>,
    overwrite: bool,
    sink: &dyn ProgressSink,
    motw: Option<&[u8]>,
) -> Result<ExtractOutcome> {
    let selected_set: Option<HashSet<usize>> =
        selected.map(|indices| expand_selection(info, indices));
    let temp = extract_to_temp(path, sink)?;
    let total = info.total_size();
    let mut done = 0u64;
    let mut outcome = ExtractOutcome {
        files: 0,
        dirs: 0,
        bytes: 0,
        written: Vec::new(),
        skipped: Vec::new(),
    };

    for entry in &info.entries {
        check_cancelled(sink)?;
        if let Some(set) = &selected_set {
            if !set.contains(&entry.index) {
                continue;
            }
        }
        let relative = strip_prefix
            .filter(|prefix| !prefix.is_empty())
            .and_then(|prefix| {
                entry
                    .path
                    .strip_prefix(&format!("{prefix}/"))
                    .or_else(|| (entry.path == prefix).then_some(""))
            })
            .unwrap_or(&entry.path);
        if relative.is_empty() {
            continue;
        }
        let target = safe_join(dest, relative)?;
        if entry.is_dir {
            std::fs::create_dir_all(to_extended(&target))?;
            outcome.dirs += 1;
            continue;
        }
        let source = temp.join(entry.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        if !source.exists() {
            outcome
                .skipped
                .push(format!("{}（ZPAQ 未输出该条目）", entry.path));
            continue;
        }
        if target.exists() && !overwrite {
            outcome
                .skipped
                .push(format!("{}（目标已存在）", entry.path));
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(to_extended(parent))?;
        }
        std::fs::copy(to_extended(&source), to_extended(&target))?;
        done += entry.size;
        outcome.files += 1;
        outcome.bytes += entry.size;
        outcome.written.push(target.clone());
        set_mtime(&target, entry.modified).ok();
        if let Some(zone) = motw {
            if let Err(error) = apply_motw(&target, zone) {
                sink.warn(&format!("写入 MOTW 失败：{error}"));
            }
        }
        sink.update(&Progress::new(
            Phase::Extracting,
            done,
            total,
            entry.path.clone(),
        ));
    }
    let _ = std::fs::remove_dir_all(to_extended(&temp));
    Ok(outcome)
}
