//! 原生 ISO9660 后端：优先 Joliet 名称，安全解压与校验。

use std::collections::HashSet;
use std::fs::File;
use std::path::Path;

use libmkiso::{IsoLimits, IsoNamespace, IsoReadOptions, IsoReader};

use crate::archive::{expand_selection, ArchiveEntry, ArchiveInfo, VerifyReport};
use crate::error::{EngineError, Result};
use crate::format::ArchiveFormat;
use crate::paths::{safe_join, to_extended};
use crate::progress::{check_cancelled, Phase, Progress, ProgressSink};
use crate::security::{apply_motw, set_mtime};
use crate::zip_backend::ExtractOutcome;

fn map_err(error: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("读取 ISO 失败：{error}"))
}

fn open(path: &Path) -> Result<IsoReader<File>> {
    let file = File::open(to_extended(path))?;
    IsoReader::open_with_options(
        file,
        IsoReadOptions {
            namespace: IsoNamespace::PreferJoliet,
            limits: IsoLimits::default(),
        },
    )
    .map_err(map_err)
}

fn entry_path(name: &str) -> String {
    name.replace('\\', "/").trim_start_matches('/').to_string()
}

pub(crate) fn list(path: &Path, _password: Option<&str>) -> Result<ArchiveInfo> {
    let reader = open(path)?;
    let entries = reader
        .entries()
        .iter()
        .enumerate()
        .map(|(index, entry)| ArchiveEntry {
            index,
            path: entry_path(&entry.name),
            is_dir: entry.directory,
            size: if entry.directory { 0 } else { entry.size },
            compressed_size: if entry.directory { 0 } else { entry.size },
            modified: None,
            method: "Store".to_string(),
            encrypted: false,
            crc32: None,
            is_symlink: entry.link_target.is_some(),
            comment: None,
        })
        .collect();
    Ok(ArchiveInfo {
        path: path.to_path_buf(),
        payload_path: path.to_path_buf(),
        disguised: false,
        format: ArchiveFormat::Iso,
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
    let mut reader = open(path)?;
    let snapshot: Vec<(usize, bool, u64, String)> = reader
        .entries()
        .iter()
        .enumerate()
        .map(|(index, entry)| (index, entry.directory, entry.size, entry_path(&entry.name)))
        .collect();
    let total: u64 = snapshot
        .iter()
        .map(|(_, dir, size, _)| if *dir { 0 } else { *size })
        .sum();
    let mut done = 0u64;
    let mut files = 0usize;
    for (index, is_dir, size, name) in snapshot {
        check_cancelled(sink)?;
        if is_dir {
            continue;
        }
        reader
            .extract(index, &mut std::io::sink())
            .map_err(map_err)?;
        done += size;
        files += 1;
        sink.update(&Progress::new(Phase::Testing, done, total, name));
    }
    Ok(VerifyReport {
        format: ArchiveFormat::Iso,
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
    let mut reader = open(path)?;
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
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(to_extended(parent))?;
        }
        let mut output = File::create(to_extended(&target))?;
        reader.extract(entry.index, &mut output).map_err(map_err)?;
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
