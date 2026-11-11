use std::collections::HashSet;
use std::path::{Path, PathBuf};

use unrar_ng::error::{Code, UnrarError};
use unrar_ng::{Archive, FileHeader};

use crate::archive::{ArchiveEntry, ArchiveInfo, VerifyReport};
use crate::error::{EngineError, Result};
use crate::format::{ArchiveFormat, Codepage};
use crate::paths::{normalize_entry_name, safe_join, to_extended};
use crate::progress::{check_cancelled, Phase, Progress, ProgressSink};
use crate::security::{apply_motw, set_mtime};
use crate::zip_backend::ExtractOutcome;

fn map_unrar(error: UnrarError) -> EngineError {
    match error.code {
        Code::MissingPassword => EngineError::PasswordRequired,
        Code::BadPassword => EngineError::WrongPassword,
        Code::BadArchive | Code::BadData | Code::UnknownFormat => {
            EngineError::DataError(error.to_string())
        }
        Code::LargeDict => EngineError::UnsupportedFormat("RAR 使用了过大的字典".to_string()),
        Code::EOpen | Code::ERead | Code::ECreate | Code::EWrite | Code::EClose => {
            EngineError::Other(error.to_string())
        }
        _ => EngineError::DataError(error.to_string()),
    }
}

pub(crate) fn first_part(path: &Path) -> PathBuf {
    let archive = Archive::new(path);
    if archive.is_multipart() {
        archive.first_part()
    } else {
        path.to_path_buf()
    }
}

fn header_path(header: &FileHeader) -> String {
    normalize_entry_name(&header.filename.to_string_lossy().replace('\\', "/"))
}

fn is_unix_symlink(header: &FileHeader) -> bool {
    header.file_attr & 0xF000 == 0xA000
}

fn method_label(method: u32) -> String {
    match method {
        0x30 => "Store",
        0x31 => "Fastest",
        0x32 => "Fast",
        0x33 => "Normal",
        0x34 => "Good",
        0x35 => "Best",
        _ => return format!("RAR {method:#x}"),
    }
    .to_string()
}

fn dos_time_unix(value: u32) -> Option<i64> {
    use chrono::{Local, TimeZone};
    if value == 0 {
        return None;
    }
    let date = value >> 16;
    let time = value & 0xffff;
    let year = 1980 + ((date >> 9) & 0x7f) as i32;
    let month = (date >> 5) & 0x0f;
    let day = date & 0x1f;
    let hour = (time >> 11) & 0x1f;
    let minute = (time >> 5) & 0x3f;
    let second = (time & 0x1f) * 2;
    Local
        .with_ymd_and_hms(year, month, day, hour, minute, second)
        .single()
        .map(|value| value.timestamp())
}

pub(crate) fn list(
    path: &Path,
    _codepage: Codepage,
    password: Option<&str>,
) -> Result<ArchiveInfo> {
    let listing = match password {
        Some(password) => Archive::with_password(path, password).open_for_listing(),
        None => Archive::new(path).open_for_listing(),
    }
    .map_err(map_unrar)?;

    let mut entries = Vec::new();
    let mut any_encrypted = false;
    for (index, header) in listing.enumerate() {
        let header = header.map_err(map_unrar)?;
        let path_name = header_path(&header);
        if path_name.is_empty() {
            continue;
        }
        let encrypted = header.is_encrypted();
        any_encrypted |= encrypted;
        entries.push(ArchiveEntry {
            index,
            path: path_name,
            is_dir: header.is_directory(),
            size: header.unpacked_size,
            compressed_size: 0,
            modified: dos_time_unix(header.file_time),
            method: method_label(header.method),
            encrypted,
            crc32: Some(header.file_crc as u64),
            is_symlink: is_unix_symlink(&header),
            comment: None,
        });
    }

    Ok(ArchiveInfo {
        path: path.to_path_buf(),
        payload_path: path.to_path_buf(),
        disguised: false,
        format: ArchiveFormat::Rar,
        entries,
        comment: None,
        encrypted: any_encrypted,
        protected_container: false,
        payload_offset: 0,
        note: None,
    })
}

pub(crate) fn test_password(path: &Path, password: &str) -> Result<bool> {
    let mut archive = Archive::with_password(path, password)
        .open_for_processing()
        .map_err(map_unrar)?;
    loop {
        let Some(header) = archive.read_header().map_err(map_unrar)? else {
            return Ok(true);
        };
        let encrypted = {
            let entry = header.entry();
            entry.is_file() && entry.is_encrypted()
        };
        if encrypted {
            return match header.test() {
                Ok(_) => Ok(true),
                Err(error)
                    if matches!(
                        error.code,
                        Code::BadPassword | Code::BadData | Code::UnknownFormat
                    ) =>
                {
                    Ok(false)
                }
                Err(error) => Err(map_unrar(error)),
            };
        }
        archive = header.skip().map_err(map_unrar)?;
    }
}

pub(crate) fn verify(
    path: &Path,
    password: Option<&str>,
    sink: &dyn ProgressSink,
) -> Result<VerifyReport> {
    let mut archive = match password {
        Some(password) => Archive::with_password(path, password).open_for_processing(),
        None => Archive::new(path).open_for_processing(),
    }
    .map_err(map_unrar)?;

    let mut entries = 0usize;
    let mut bytes = 0u64;
    let mut done = 0u64;
    loop {
        let Some(header) = archive.read_header().map_err(map_unrar)? else {
            break;
        };
        let (is_file, size, name) = {
            let entry = header.entry();
            (entry.is_file(), entry.unpacked_size, header_path(entry))
        };
        if is_file {
            entries += 1;
            bytes += size;
        }
        sink.update(&Progress::new(Phase::Testing, done, bytes.max(size), name));
        archive = match header.test() {
            Ok(next) => next,
            Err(error) if error.code == Code::BadPassword => {
                return Err(EngineError::WrongPassword)
            }
            Err(error) => return Err(map_unrar(error)),
        };
        done += size;
    }
    Ok(VerifyReport {
        format: ArchiveFormat::Rar,
        entries,
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
    password: Option<&str>,
    overwrite: bool,
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
    let mut outcome = ExtractOutcome {
        files: 0,
        dirs: 0,
        bytes: 0,
        written: Vec::new(),
        skipped: Vec::new(),
    };
    let mut done = 0u64;

    let mut archive = match password {
        Some(password) => Archive::with_password(path, password).open_for_processing(),
        None => Archive::new(path).open_for_processing(),
    }
    .map_err(map_unrar)?;

    let mut index = 0usize;
    loop {
        check_cancelled(sink)?;
        let Some(header) = archive.read_header().map_err(map_unrar)? else {
            break;
        };
        let (entry_path, is_dir, is_symlink, size, file_time) = {
            let entry = header.entry();
            (
                header_path(entry),
                entry.is_directory(),
                is_unix_symlink(entry),
                entry.unpacked_size,
                entry.file_time,
            )
        };
        let current = index;
        index += 1;

        if let Some(set) = &selected_set {
            if !set.contains(&current) {
                archive = header.skip().map_err(map_unrar)?;
                continue;
            }
        }
        if is_symlink {
            outcome
                .skipped
                .push(format!("{entry_path}（符号链接已跳过）"));
            sink.warn(&format!("跳过符号链接：{entry_path}"));
            archive = header.skip().map_err(map_unrar)?;
            continue;
        }
        let relative = strip_entry_prefix(&entry_path, strip_prefix);
        if relative.is_empty() {
            archive = header.skip().map_err(map_unrar)?;
            continue;
        }
        let target = safe_join(dest, relative)?;
        if is_dir {
            std::fs::create_dir_all(to_extended(&target))?;
            outcome.dirs += 1;
            sink.update(&Progress::new(Phase::Extracting, done, total, entry_path));
            archive = header.skip().map_err(map_unrar)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(to_extended(parent))?;
        }
        if target.exists() && !overwrite {
            outcome.skipped.push(format!("{entry_path}（目标已存在）"));
            archive = header.skip().map_err(map_unrar)?;
            continue;
        }
        archive = header.extract_to(to_extended(&target)).map_err(map_unrar)?;
        set_mtime(&target, dos_time_unix(file_time))?;
        if let Some(zone) = motw {
            if let Err(err) = apply_motw(&target, zone) {
                sink.warn(&format!("写入 MOTW 失败：{err}"));
            }
        }
        done += size;
        outcome.files += 1;
        outcome.bytes += size;
        outcome.written.push(target);
        sink.update(&Progress::new(
            Phase::Extracting,
            done,
            total.max(done),
            &entry_path,
        ));
    }

    sink.update(&Progress::new(
        Phase::Extracting,
        total,
        total.max(done),
        "解压完成".to_string(),
    ));
    Ok(outcome)
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
