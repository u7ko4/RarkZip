use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::archive::{ArchiveEntry, ArchiveInfo, VerifyReport};
use crate::error::{EngineError, Result};
use crate::format::ArchiveFormat;
use crate::paths::{archive_stem, normalize_entry_name, safe_join, to_extended};
use crate::progress::{check_cancelled, Phase, Progress, ProgressSink};
use crate::security::{apply_motw, modified_unix, set_mtime};
use crate::util::{copy_with_progress, preallocate, COPY_BUFFER_SIZE};
use crate::zip_backend::ExtractOutcome;

fn open_reader(path: &Path, format: ArchiveFormat) -> Result<Box<dyn Read>> {
    let file = File::open(to_extended(path))?;
    Ok(match format {
        ArchiveFormat::Tar => Box::new(file),
        ArchiveFormat::TarGz | ArchiveFormat::Gz => Box::new(flate2::read::GzDecoder::new(file)),
        ArchiveFormat::TarBz2 | ArchiveFormat::Bz2 => Box::new(bzip2::read::BzDecoder::new(file)),
        ArchiveFormat::TarXz | ArchiveFormat::Xz => Box::new(xz2::read::XzDecoder::new(file)),
        ArchiveFormat::TarZst | ArchiveFormat::Zst => {
            Box::new(zstd::stream::read::Decoder::new(file)?)
        }
        other => {
            return Err(EngineError::UnsupportedFormat(format!(
                "不是 tar 系格式：{other}"
            )))
        }
    })
}

pub(crate) fn list(path: &Path, format: ArchiveFormat) -> Result<ArchiveInfo> {
    let reader = open_reader(path, format)?;
    let mut archive = tar::Archive::new(reader);
    let mut entries = Vec::new();

    for (index, item) in archive.entries().map_err(tar_error)?.enumerate() {
        let item = item.map_err(tar_error)?;
        let entry_path = item.path().map_err(tar_error)?;
        let raw_name = entry_path.to_string_lossy().replace('\\', "/");
        let path_name = normalize_entry_name(&raw_name);
        if path_name.is_empty() {
            continue;
        }
        let header = item.header();
        let entry_type = header.entry_type();
        let is_dir = entry_type.is_dir();
        let is_symlink = entry_type.is_symlink() || entry_type.is_hard_link();
        let size = header.size().unwrap_or(0);
        let modified = header.mtime().ok().map(|value| value as i64);
        let mode = header.mode().ok().unwrap_or(0o644);
        let method = match format {
            ArchiveFormat::TarGz => "GZip",
            ArchiveFormat::TarBz2 => "BZip2",
            ArchiveFormat::TarXz => "XZ",
            ArchiveFormat::TarZst => "Zstd",
            _ => "Tar",
        };
        entries.push(ArchiveEntry {
            index,
            path: path_name,
            is_dir,
            size,
            compressed_size: 0,
            modified,
            method: method.to_string(),
            encrypted: false,
            crc32: None,
            is_symlink,
            comment: (mode != 0).then(|| format!("{mode:o}")),
        });
    }

    Ok(ArchiveInfo {
        path: path.to_path_buf(),
        payload_path: path.to_path_buf(),
        disguised: false,
        format,
        entries,
        comment: None,
        encrypted: false,
        protected_container: false,
        payload_offset: 0,
        note: None,
    })
}

pub(crate) fn list_single(path: &Path, format: ArchiveFormat) -> Result<ArchiveInfo> {
    let size = match format {
        ArchiveFormat::Gz => gzip_isize(path).unwrap_or(0),
        // BZ2/XZ/ZST 没有可靠的尾部大小字段；流式解码统计一次，避免列表显示 0 B。
        ArchiveFormat::Bz2 | ArchiveFormat::Xz | ArchiveFormat::Zst => {
            decoded_size(path, format).unwrap_or(0)
        }
        _ => 0,
    };
    let compressed_size = std::fs::metadata(to_extended(path))
        .map(|meta| meta.len())
        .unwrap_or(0);
    let method = match format {
        ArchiveFormat::Gz => "GZip",
        ArchiveFormat::Bz2 => "BZip2",
        ArchiveFormat::Xz => "XZ",
        ArchiveFormat::Zst => "Zstd",
        _ => "未知",
    };
    let entry = ArchiveEntry {
        index: 0,
        path: archive_stem(path),
        is_dir: false,
        size,
        compressed_size,
        modified: modified_unix(path),
        method: method.to_string(),
        encrypted: false,
        crc32: None,
        is_symlink: false,
        comment: None,
    };
    Ok(ArchiveInfo {
        path: path.to_path_buf(),
        payload_path: path.to_path_buf(),
        disguised: false,
        format,
        entries: vec![entry],
        comment: None,
        encrypted: false,
        protected_container: false,
        payload_offset: 0,
        note: None,
    })
}

pub(crate) fn extract(
    path: &Path,
    info: &ArchiveInfo,
    dest: &Path,
    strip_prefix: Option<&str>,
    selected: Option<&[usize]>,
    sink: &dyn ProgressSink,
    motw: Option<&[u8]>,
) -> Result<ExtractOutcome> {
    let selected_set: Option<HashSet<usize>> =
        selected.map(|indices| crate::archive::expand_selection(info, indices));
    let total: u64 = info
        .entries
        .iter()
        .filter(|entry| {
            !entry.is_dir
                && !entry.is_symlink
                && selected_set
                    .as_ref()
                    .is_none_or(|set| set.contains(&entry.index))
        })
        .map(|entry| entry.size)
        .sum();

    let reader = open_reader(path, info.format)?;
    let mut archive = tar::Archive::new(reader);
    let mut outcome = ExtractOutcome {
        files: 0,
        dirs: 0,
        bytes: 0,
        written: Vec::new(),
        skipped: Vec::new(),
    };
    let mut done = 0u64;

    for (index, item) in archive.entries().map_err(tar_error)?.enumerate() {
        check_cancelled(sink)?;
        if let Some(set) = &selected_set {
            if !set.contains(&index) {
                continue;
            }
        }
        let mut item = item.map_err(tar_error)?;
        let entry_path = item.path().map_err(tar_error)?;
        let raw_name = entry_path.to_string_lossy().replace('\\', "/");
        let name = normalize_entry_name(&raw_name);
        if name.is_empty() {
            continue;
        }
        let header = item.header();
        let entry_type = header.entry_type();
        if entry_type.is_dir() {
            let relative = strip_entry_prefix(&name, strip_prefix);
            if relative.is_empty() {
                continue;
            }
            let target = safe_join(dest, relative)?;
            std::fs::create_dir_all(to_extended(&target))?;
            outcome.dirs += 1;
            sink.update(&Progress::new(Phase::Extracting, done, total, name.clone()));
            continue;
        }
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            outcome.skipped.push(format!("{name}（链接已跳过）"));
            sink.warn(&format!("跳过链接条目：{name}"));
            continue;
        }
        if !entry_type.is_file() {
            outcome.skipped.push(format!("{name}（特殊文件已跳过）"));
            continue;
        }

        let relative = strip_entry_prefix(&name, strip_prefix);
        if relative.is_empty() {
            continue;
        }
        let target = safe_join(dest, relative)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(to_extended(parent))?;
        }
        let modified = header.mtime().ok().map(|value| value as i64);
        let size = header.size().unwrap_or(0);

        let mut output = File::create(to_extended(&target))?;
        preallocate(&output, size)?;
        copy_with_progress(
            &mut item,
            &mut output,
            sink,
            Phase::Extracting,
            &mut done,
            total,
            &name,
        )?;
        set_mtime(&target, modified)?;
        if let Some(zone) = motw {
            if let Err(err) = apply_motw(&target, zone) {
                sink.warn(&format!("写入 MOTW 失败：{err}"));
            }
        }
        outcome.files += 1;
        outcome.bytes += size;
        outcome.written.push(target);
    }

    sink.update(&Progress::new(
        Phase::Extracting,
        total,
        total,
        "解压完成".to_string(),
    ));
    Ok(outcome)
}

pub(crate) fn extract_single(
    path: &Path,
    info: &ArchiveInfo,
    dest: &Path,
    overwrite: bool,
    sink: &dyn ProgressSink,
    motw: Option<&[u8]>,
) -> Result<ExtractOutcome> {
    let entry = info
        .entries
        .first()
        .ok_or_else(|| EngineError::DataError("空压缩流".to_string()))?;
    let target = dest.join(&entry.path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(to_extended(parent))?;
    }
    if target.exists() && !overwrite {
        return Ok(ExtractOutcome {
            files: 0,
            dirs: 0,
            bytes: 0,
            written: Vec::new(),
            skipped: vec![format!("{}（目标已存在）", entry.path)],
        });
    }

    let mut reader = open_reader(path, info.format)?;
    let total = std::fs::metadata(to_extended(path))
        .map(|meta| meta.len())
        .unwrap_or(0);
    let mut output = File::create(to_extended(&target))?;
    let mut done = 0u64;
    let bytes = copy_with_progress(
        &mut reader,
        &mut output,
        sink,
        Phase::Extracting,
        &mut done,
        total,
        &entry.path,
    )?;
    set_mtime(&target, entry.modified)?;
    if let Some(zone) = motw {
        if let Err(err) = apply_motw(&target, zone) {
            sink.warn(&format!("写入 MOTW 失败：{err}"));
        }
    }
    Ok(ExtractOutcome {
        files: 1,
        dirs: 0,
        bytes,
        written: vec![target],
        skipped: Vec::new(),
    })
}

pub(crate) fn verify(
    path: &Path,
    format: ArchiveFormat,
    sink: &dyn ProgressSink,
) -> Result<VerifyReport> {
    let info = list(path, format)?;
    let total = info.total_size();
    let reader = open_reader(path, format)?;
    let mut archive = tar::Archive::new(reader);
    let mut done = 0u64;
    let mut bytes = 0u64;
    let mut count = 0usize;
    let mut buffer = vec![0u8; COPY_BUFFER_SIZE];
    for item in archive.entries().map_err(tar_error)? {
        check_cancelled(sink)?;
        let mut item = item.map_err(tar_error)?;
        if !item.header().entry_type().is_file() {
            continue;
        }
        count += 1;
        let name = item
            .path()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default();
        loop {
            let read = item.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            done += read as u64;
            bytes += read as u64;
            sink.update(&Progress::new(Phase::Testing, done, total, name.clone()));
        }
    }
    Ok(VerifyReport {
        format,
        entries: count,
        bytes,
    })
}

pub(crate) fn verify_single(
    path: &Path,
    format: ArchiveFormat,
    sink: &dyn ProgressSink,
) -> Result<VerifyReport> {
    let total = std::fs::metadata(to_extended(path))
        .map(|meta| meta.len())
        .unwrap_or(0);
    let mut reader = open_reader(path, format)?;
    let mut buffer = vec![0u8; COPY_BUFFER_SIZE];
    let mut done = 0u64;
    let mut bytes = 0u64;
    loop {
        check_cancelled(sink)?;
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        done += read as u64;
        bytes += read as u64;
        sink.update(&Progress::new(
            Phase::Testing,
            done,
            total,
            archive_stem(path),
        ));
    }
    Ok(VerifyReport {
        format,
        entries: 1,
        bytes,
    })
}

fn gzip_isize(path: &Path) -> Option<u64> {
    let mut file = File::open(to_extended(path)).ok()?;
    let len = file.metadata().ok()?.len();
    if len < 4 {
        return None;
    }
    file.seek(SeekFrom::End(-4)).ok()?;
    let mut bytes = [0u8; 4];
    file.read_exact(&mut bytes).ok()?;
    Some(u32::from_le_bytes(bytes) as u64)
}

/// 解压流并统计原始字节数；仅用于缺少大小元数据的单文件压缩流。
fn decoded_size(path: &Path, format: ArchiveFormat) -> Option<u64> {
    let mut reader = open_reader(path, format).ok()?;
    let mut buffer = vec![0u8; COPY_BUFFER_SIZE];
    let mut total = 0u64;
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Some(total),
            Ok(read) => total += read as u64,
            Err(_) => return None,
        }
    }
}

fn strip_entry_prefix<'a>(name: &'a str, prefix: Option<&str>) -> &'a str {
    match prefix {
        Some(prefix) if !prefix.is_empty() => {
            let with_slash = format!("{prefix}/");
            name.strip_prefix(&with_slash)
                .or_else(|| (name == prefix).then_some(""))
                .unwrap_or(name)
        }
        _ => name,
    }
}

fn tar_error(error: std::io::Error) -> EngineError {
    if error.kind() == std::io::ErrorKind::InvalidData {
        EngineError::DataError(error.to_string())
    } else {
        EngineError::Io(error)
    }
}
