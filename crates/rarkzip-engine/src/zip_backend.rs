use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use zip::result::ZipError;
use zip::{CompressionMethod, ZipArchive};

use crate::archive::{ArchiveEntry, ArchiveInfo, VerifyReport};
use crate::error::{EngineError, Result};
use crate::format::{ArchiveFormat, Codepage};
use crate::paths::{normalize_entry_name, safe_join, to_extended};
use crate::progress::{check_cancelled, Phase, Progress, ProgressSink};
use crate::security::{apply_motw, set_mtime};
use crate::util::{copy_with_progress, trim_comment, COPY_BUFFER_SIZE};

pub struct ExtractOutcome {
    pub files: usize,
    pub dirs: usize,
    pub bytes: u64,
    pub written: Vec<PathBuf>,
    pub skipped: Vec<String>,
}

pub(crate) fn map_zip_error(error: ZipError) -> EngineError {
    match error {
        ZipError::Io(err) => EngineError::Io(err),
        ZipError::InvalidPassword => EngineError::WrongPassword,
        ZipError::UnsupportedArchive(message) if message == ZipError::PASSWORD_REQUIRED => {
            EngineError::PasswordRequired
        }
        ZipError::UnsupportedArchive(message) => {
            EngineError::UnsupportedFormat(message.to_string())
        }
        ZipError::InvalidArchive(message) => EngineError::DataError(message.to_string()),
        ZipError::FileNotFound => EngineError::DataError("归档条目不存在".to_string()),
        other => EngineError::DataError(format!("ZIP 错误：{other}")),
    }
}

fn open_zip(path: &Path) -> Result<ZipArchive<File>> {
    let file = File::open(to_extended(path))?;
    ZipArchive::new(file).map_err(map_zip_error)
}

pub(crate) fn list(path: &Path, codepage: Codepage) -> Result<ArchiveInfo> {
    let mut archive = open_zip(path)?;
    let comment = trim_comment(archive.comment());
    let mut entries = Vec::with_capacity(archive.len());
    let mut any_encrypted = false;

    for index in 0..archive.len() {
        let file = archive.by_index_raw(index).map_err(map_zip_error)?;
        let decoded = decode_name(file.name_raw(), file.name(), codepage);
        let path_name = normalize_entry_name(&decoded);
        if path_name.is_empty() {
            continue;
        }
        let is_dir = file.is_dir() || path_name.ends_with('/');
        let encrypted = file.encrypted();
        any_encrypted |= encrypted;
        let modified = file.last_modified().and_then(zip_datetime_unix);
        let entry_comment = file.comment().trim().to_string();
        entries.push(ArchiveEntry {
            index,
            path: path_name,
            is_dir,
            size: file.size(),
            compressed_size: file.compressed_size(),
            modified,
            method: method_label(file.compression()),
            encrypted,
            crc32: Some(file.crc32() as u64),
            is_symlink: file.is_symlink(),
            comment: (!entry_comment.is_empty()).then_some(entry_comment),
        });
    }

    Ok(ArchiveInfo {
        path: path.to_path_buf(),
        payload_path: path.to_path_buf(),
        disguised: false,
        format: ArchiveFormat::Zip,
        entries,
        comment: (!comment.is_empty()).then_some(comment),
        encrypted: any_encrypted,
        protected_container: false,
        payload_offset: 0,
        note: None,
    })
}

pub(crate) fn decode_name(raw: &[u8], fallback: &str, codepage: Codepage) -> String {
    if raw.is_empty() {
        return fallback.to_string();
    }
    let decoded = match codepage {
        Codepage::Default => return decode_name(raw, fallback, Codepage::system()),
        Codepage::Auto => {
            if let Ok(value) = std::str::from_utf8(raw) {
                return value.to_string();
            }
            let (value, _, had_errors) = encoding_rs::GBK.decode(raw);
            if had_errors {
                fallback.to_string()
            } else {
                value.into_owned()
            }
        }
        Codepage::Utf8 => String::from_utf8_lossy(raw).into_owned(),
        Codepage::Gbk => {
            let (value, _, _) = encoding_rs::GBK.decode(raw);
            value.into_owned()
        }
        Codepage::Big5 => {
            let (value, _, _) = encoding_rs::BIG5.decode(raw);
            value.into_owned()
        }
        Codepage::ShiftJis => {
            let (value, _, _) = encoding_rs::SHIFT_JIS.decode(raw);
            value.into_owned()
        }
        Codepage::Korean => {
            let (value, _, _) = encoding_rs::EUC_KR.decode(raw);
            value.into_owned()
        }
        Codepage::Windows1252 => {
            let (value, _, _) = encoding_rs::WINDOWS_1252.decode(raw);
            value.into_owned()
        }
        Codepage::Windows1250 => {
            let (value, _, _) = encoding_rs::WINDOWS_1250.decode(raw);
            value.into_owned()
        }
        Codepage::Windows1251 => {
            let (value, _, _) = encoding_rs::WINDOWS_1251.decode(raw);
            value.into_owned()
        }
        Codepage::Windows1254 => {
            let (value, _, _) = encoding_rs::WINDOWS_1254.decode(raw);
            value.into_owned()
        }
        Codepage::Windows1253 => {
            let (value, _, _) = encoding_rs::WINDOWS_1253.decode(raw);
            value.into_owned()
        }
        Codepage::Windows1256 => {
            let (value, _, _) = encoding_rs::WINDOWS_1256.decode(raw);
            value.into_owned()
        }
        Codepage::Windows1255 => {
            let (value, _, _) = encoding_rs::WINDOWS_1255.decode(raw);
            value.into_owned()
        }
        Codepage::Windows1257 => {
            let (value, _, _) = encoding_rs::WINDOWS_1257.decode(raw);
            value.into_owned()
        }
        Codepage::Windows874 => {
            let (value, _, _) = encoding_rs::WINDOWS_874.decode(raw);
            value.into_owned()
        }
        Codepage::Cp437 => fallback.to_string(),
    };
    decoded
}

pub(crate) fn method_label(method: CompressionMethod) -> String {
    match method {
        CompressionMethod::Stored => "Store",
        CompressionMethod::Deflated => "Deflate",
        CompressionMethod::Bzip2 => "BZip2",
        CompressionMethod::Zstd => "Zstd",
        CompressionMethod::Lzma => "LZMA",
        CompressionMethod::Aes => "AES",
        CompressionMethod::Deflate64 => "Deflate64",
        CompressionMethod::Ppmd => "PPMd",
        CompressionMethod::Xz => "XZ",
        _ => "其他",
    }
    .to_string()
}

pub(crate) fn test_password(path: &Path, password: &str) -> Result<bool> {
    let mut archive = open_zip(path)?;
    let mut candidates: Vec<(u64, usize)> = Vec::new();
    for index in 0..archive.len() {
        let file = archive.by_index_raw(index).map_err(map_zip_error)?;
        if file.encrypted() && !file.is_dir() {
            candidates.push((file.size(), index));
        }
    }
    if candidates.is_empty() {
        return Ok(true);
    }
    candidates.sort_unstable();
    let index = candidates[0].1;
    let matched = match archive.by_index_decrypt(index, password.as_bytes()) {
        Ok(mut file) => {
            let mut buffer = [0u8; 64];
            match file.read(&mut buffer) {
                Ok(_) => Ok(true),
                Err(err) if err.kind() == std::io::ErrorKind::InvalidInput => Ok(false),
                Err(err) => Err(EngineError::Io(err)),
            }
        }
        Err(ZipError::InvalidPassword) => Ok(false),
        Err(err) => Err(map_zip_error(err)),
    };
    matched
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

    // 多文件 ZIP 可按块并行解压：每个线程持有独立的 ZipArchive。
    // 若单个文件占一半以上，并行收益有限且会与主拷贝争抢磁盘，退回顺序解压。
    let largest_entry = info
        .entries
        .iter()
        .filter(|entry| !entry.is_dir)
        .map(|entry| entry.size)
        .max()
        .unwrap_or(0);
    if selected_set.is_none()
        && info.entries.len() >= 8
        && total >= 4 * 1024 * 1024
        && largest_entry * 2 <= total
    {
        return extract_zip_parallel(
            path,
            info,
            dest,
            strip_prefix,
            password,
            overwrite,
            sink,
            motw,
            total,
        );
    }
    let mut archive = open_zip(path)?;
    let mut outcome = ExtractOutcome {
        files: 0,
        dirs: 0,
        bytes: 0,
        written: Vec::new(),
        skipped: Vec::new(),
    };
    let mut done = 0u64;

    for entry in &info.entries {
        check_cancelled(sink)?;
        if let Some(set) = &selected_set {
            if !set.contains(&entry.index) {
                continue;
            }
        }
        if entry.is_symlink {
            outcome
                .skipped
                .push(format!("{}（符号链接已跳过）", entry.path));
            sink.warn(&format!("跳过符号链接：{}", entry.path));
            continue;
        }
        let relative = strip_entry_prefix(&entry.path, strip_prefix);
        if relative.is_empty() {
            continue;
        }
        let target = safe_join(dest, relative)?;
        if entry.is_dir {
            std::fs::create_dir_all(to_extended(&target))?;
            outcome.dirs += 1;
            sink.update(&Progress::new(
                Phase::Extracting,
                done,
                total,
                entry.path.clone(),
            ));
            continue;
        }

        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(to_extended(parent))?;
        }
        if target.exists() && !overwrite {
            outcome
                .skipped
                .push(format!("{}（目标已存在）", entry.path));
            continue;
        }

        let encrypted = entry.encrypted;
        if encrypted && password.is_none() {
            return Err(EngineError::PasswordRequired);
        }
        let mut file = if encrypted {
            archive
                .by_index_decrypt(entry.index, password.unwrap_or_default().as_bytes())
                .map_err(map_zip_error)?
        } else {
            archive.by_index(entry.index).map_err(map_zip_error)?
        };

        let mut output = File::create(to_extended(&target))?;
        copy_with_progress(
            &mut file,
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
        outcome.files += 1;
        outcome.bytes += file.size();
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

pub(crate) fn verify(
    path: &Path,
    password: Option<&str>,
    sink: &dyn ProgressSink,
) -> Result<VerifyReport> {
    let mut archive = open_zip(path)?;
    let mut total = 0u64;
    let mut count = 0usize;
    for index in 0..archive.len() {
        let file = archive.by_index_raw(index).map_err(map_zip_error)?;
        if !file.is_dir() {
            total += file.size();
            count += 1;
        }
    }
    let mut done = 0u64;
    let mut bytes = 0u64;
    for index in 0..archive.len() {
        let meta = archive.by_index_raw(index).map_err(map_zip_error)?;
        if meta.is_dir() {
            continue;
        }
        let encrypted = meta.encrypted();
        let name = meta.name().to_string();
        drop(meta);
        if encrypted && password.is_none() {
            return Err(EngineError::PasswordRequired);
        }
        let mut file = if encrypted {
            archive
                .by_index_decrypt(index, password.unwrap_or_default().as_bytes())
                .map_err(map_zip_error)?
        } else {
            archive.by_index(index).map_err(map_zip_error)?
        };
        let mut buffer = vec![0u8; COPY_BUFFER_SIZE];
        loop {
            check_cancelled(sink)?;
            let read = file.read(&mut buffer).map_err(|err| {
                if err.kind() == std::io::ErrorKind::InvalidInput {
                    EngineError::WrongPassword
                } else {
                    EngineError::Io(err)
                }
            })?;
            if read == 0 {
                break;
            }
            done += read as u64;
            bytes += read as u64;
            sink.update(&Progress::new(Phase::Testing, done, total, name.clone()));
        }
    }
    Ok(VerifyReport {
        format: ArchiveFormat::Zip,
        entries: count,
        bytes,
    })
}

#[allow(clippy::too_many_arguments)]
fn extract_zip_entry(
    archive: &mut ZipArchive<File>,
    entry: &ArchiveEntry,
    dest: &Path,
    strip_prefix: Option<&str>,
    password: Option<&str>,
    overwrite: bool,
    sink: &dyn ProgressSink,
    done: &AtomicU64,
    total: u64,
    motw: Option<&[u8]>,
    outcome: &mut ExtractOutcome,
) -> Result<()> {
    check_cancelled(sink)?;
    if entry.is_symlink {
        outcome
            .skipped
            .push(format!("{}（符号链接已跳过）", entry.path));
        sink.warn(&format!("跳过符号链接：{}", entry.path));
        return Ok(());
    }
    let relative = strip_entry_prefix(&entry.path, strip_prefix);
    if relative.is_empty() {
        return Ok(());
    }
    let target = safe_join(dest, relative)?;
    if entry.is_dir {
        std::fs::create_dir_all(to_extended(&target))?;
        outcome.dirs += 1;
        sink.update(&Progress::new(
            Phase::Extracting,
            done.load(Ordering::Relaxed),
            total,
            entry.path.clone(),
        ));
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(to_extended(parent))?;
    }
    if target.exists() && !overwrite {
        outcome
            .skipped
            .push(format!("{}（目标已存在）", entry.path));
        return Ok(());
    }
    if entry.encrypted && password.is_none() {
        return Err(EngineError::PasswordRequired);
    }
    let mut file = if entry.encrypted {
        archive
            .by_index_decrypt(entry.index, password.unwrap_or_default().as_bytes())
            .map_err(map_zip_error)?
    } else {
        archive.by_index(entry.index).map_err(map_zip_error)?
    };
    let mut output = File::create(to_extended(&target))?;
    let mut buffer = vec![0u8; COPY_BUFFER_SIZE];
    loop {
        check_cancelled(sink)?;
        let read = file.read(&mut buffer).map_err(|error| {
            if error.kind() == std::io::ErrorKind::InvalidInput {
                EngineError::WrongPassword
            } else {
                EngineError::Io(error)
            }
        })?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        let current = done.fetch_add(read as u64, Ordering::Relaxed) + read as u64;
        sink.update(&Progress::new(
            Phase::Extracting,
            current,
            total,
            entry.path.clone(),
        ));
    }
    set_mtime(&target, entry.modified)?;
    if let Some(zone) = motw {
        if let Err(error) = apply_motw(&target, zone) {
            sink.warn(&format!("写入 MOTW 失败：{error}"));
        }
    }
    outcome.files += 1;
    outcome.bytes += file.size();
    outcome.written.push(target);
    Ok(())
}

fn split_extract_groups(entries: &[ArchiveEntry], workers: usize) -> Vec<&[ArchiveEntry]> {
    if workers <= 1 || entries.len() <= 1 {
        return vec![entries];
    }
    let total: u64 = entries
        .iter()
        .filter(|entry| !entry.is_dir)
        .map(|entry| entry.size)
        .sum();
    let target = (total / workers as u64).max(1);
    let mut groups = Vec::new();
    let mut start = 0usize;
    let mut accumulated = 0u64;
    for (index, entry) in entries.iter().enumerate() {
        if !entry.is_dir {
            accumulated += entry.size;
        }
        if accumulated >= target && groups.len() + 1 < workers && index + 1 < entries.len() {
            groups.push(&entries[start..=index]);
            start = index + 1;
            accumulated = 0;
        }
    }
    groups.push(&entries[start..]);
    groups
}

#[allow(clippy::too_many_arguments)]
fn extract_zip_parallel(
    path: &Path,
    info: &ArchiveInfo,
    dest: &Path,
    strip_prefix: Option<&str>,
    password: Option<&str>,
    overwrite: bool,
    sink: &dyn ProgressSink,
    motw: Option<&[u8]>,
    total: u64,
) -> Result<ExtractOutcome> {
    let workers = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1)
        .clamp(1, 16)
        .min(info.entries.len());
    let groups = split_extract_groups(&info.entries, workers);
    let done = AtomicU64::new(0);
    let outcomes = std::thread::scope(|scope| -> Result<Vec<ExtractOutcome>> {
        let mut handles = Vec::with_capacity(groups.len());
        for group in groups {
            let done = &done;
            handles.push(scope.spawn(move || -> Result<ExtractOutcome> {
                let mut archive = open_zip(path)?;
                let mut outcome = ExtractOutcome {
                    files: 0,
                    dirs: 0,
                    bytes: 0,
                    written: Vec::new(),
                    skipped: Vec::new(),
                };
                for entry in group {
                    extract_zip_entry(
                        &mut archive,
                        entry,
                        dest,
                        strip_prefix,
                        password,
                        overwrite,
                        sink,
                        done,
                        total,
                        motw,
                        &mut outcome,
                    )?;
                }
                Ok(outcome)
            }));
        }
        let mut outcomes = Vec::with_capacity(handles.len());
        for handle in handles {
            match handle.join() {
                Ok(Ok(outcome)) => outcomes.push(outcome),
                Ok(Err(error)) => return Err(error),
                Err(_) => {
                    return Err(EngineError::Other("ZIP 解压线程异常".to_string()));
                }
            }
        }
        Ok(outcomes)
    })?;
    let mut outcome = ExtractOutcome {
        files: 0,
        dirs: 0,
        bytes: 0,
        written: Vec::new(),
        skipped: Vec::new(),
    };
    for part in outcomes {
        outcome.files += part.files;
        outcome.dirs += part.dirs;
        outcome.bytes += part.bytes;
        outcome.written.extend(part.written);
        outcome.skipped.extend(part.skipped);
    }
    sink.update(&Progress::new(
        Phase::Extracting,
        total,
        total,
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

/// ZIP 的 DOS 时间没有时区，按本地时间解释。
fn zip_datetime_unix(value: zip::DateTime) -> Option<i64> {
    use chrono::{Local, TimeZone};
    Local
        .with_ymd_and_hms(
            value.year() as i32,
            value.month() as u32,
            value.day() as u32,
            value.hour() as u32,
            value.minute() as u32,
            value.second() as u32,
        )
        .single()
        .map(|date| date.timestamp())
}
