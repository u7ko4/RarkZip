//! 原生 LZH/LHA 后端：列表、校验与安全解压。

use std::collections::HashSet;
use std::fs::File;
use std::path::Path;

use oxiarc_archive::lzh::LzhReader;

use crate::archive::{expand_selection, ArchiveEntry, ArchiveInfo, VerifyReport};
use crate::error::{EngineError, Result};
use crate::format::ArchiveFormat;
use crate::paths::{safe_join, to_extended};
use crate::progress::{check_cancelled, Phase, Progress, ProgressSink};
use crate::security::{apply_motw, set_mtime};
use crate::util::system_time_to_unix;
use crate::zip_backend::ExtractOutcome;

fn map_err(error: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("读取 LZH 失败：{error}"))
}

fn method_label(method: impl std::fmt::Debug) -> String {
    format!("{method:?}").to_ascii_uppercase()
}

fn entry_path(name: &str) -> String {
    name.replace('\\', "/").trim_matches('/').to_string()
}

pub(crate) fn list(path: &Path, _password: Option<&str>) -> Result<ArchiveInfo> {
    let file = File::open(to_extended(path))?;
    let reader = LzhReader::new(file).map_err(map_err)?;
    let entries = reader
        .entries()
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            let is_dir = entry.entry_type.is_dir();
            ArchiveEntry {
                index,
                path: entry_path(&entry.name),
                is_dir,
                size: if is_dir { 0 } else { entry.size },
                compressed_size: entry.compressed_size,
                modified: entry.modified.and_then(system_time_to_unix),
                method: method_label(entry.method),
                encrypted: false,
                crc32: entry.crc32.map(u64::from),
                is_symlink: entry.entry_type.is_symlink(),
                comment: entry.comment,
            }
        })
        .collect();
    Ok(ArchiveInfo {
        path: path.to_path_buf(),
        payload_path: path.to_path_buf(),
        disguised: false,
        format: ArchiveFormat::Lzh,
        entries,
        comment: None,
        encrypted: false,
        protected_container: false,
        payload_offset: 0,
        note: None,
    })
}

pub(crate) fn verify(
    path: &Path,
    _password: Option<&str>,
    sink: &dyn ProgressSink,
) -> Result<VerifyReport> {
    let file = File::open(to_extended(path))?;
    let mut reader = LzhReader::new(file).map_err(map_err)?;
    let entries = reader.entries();
    let total: u64 = entries.iter().map(|entry| entry.size).sum();
    let mut done = 0u64;
    let mut files = 0usize;
    for entry in &entries {
        check_cancelled(sink)?;
        if entry.entry_type.is_dir() {
            continue;
        }
        reader
            .extract(entry, &mut std::io::sink())
            .map_err(map_err)?;
        done += entry.size;
        files += 1;
        sink.update(&Progress::new(
            Phase::Testing,
            done,
            total,
            entry_path(&entry.name),
        ));
    }
    Ok(VerifyReport {
        format: ArchiveFormat::Lzh,
        entries: files,
        bytes: done,
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
    let file = File::open(to_extended(path))?;
    let mut reader = LzhReader::new(file).map_err(map_err)?;
    let source_entries = reader.entries();
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
        if entry.is_symlink {
            outcome.skipped.push(format!("{}（符号链接）", entry.path));
            continue;
        }
        if target.exists() && !overwrite {
            outcome
                .skipped
                .push(format!("{}（目标已存在）", entry.path));
            continue;
        }
        let Some(source) = source_entries.get(entry.index) else {
            outcome
                .skipped
                .push(format!("{}（LZH 中找不到条目）", entry.path));
            continue;
        };
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(to_extended(parent))?;
        }
        let mut output = File::create(to_extended(&target))?;
        reader.extract(source, &mut output).map_err(map_err)?;
        drop(output);
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
    Ok(outcome)
}
