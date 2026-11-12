use std::collections::HashSet;
use std::fs::File;
use std::io;
use std::path::Path;
use std::time::SystemTime;

use sevenz_rust2::{ArchiveReader, EncoderMethod, Error as SevenZError, Password};

use crate::archive::{ArchiveEntry, ArchiveInfo, VerifyReport};
use crate::error::{EngineError, Result};
use crate::format::ArchiveFormat;
use crate::paths::{normalize_entry_name, safe_join, to_extended};
use crate::progress::{Phase, Progress, ProgressSink};
use crate::security::{apply_motw, set_mtime};
use crate::util::{copy_with_progress, preallocate, system_time_to_unix, COPY_BUFFER_SIZE};
use crate::zip_backend::ExtractOutcome;

fn open_reader(path: &Path, password: Option<&str>) -> Result<ArchiveReader<File>> {
    let provided = password.is_some();
    let password = Password::from(password.unwrap_or_default());
    ArchiveReader::open(to_extended(path), password).map_err(|error| map_7z(error, provided))
}

fn map_7z(error: SevenZError, password_provided: bool) -> EngineError {
    match error {
        SevenZError::PasswordRequired => EngineError::PasswordRequired,
        SevenZError::MaybeBadPassword(_) => EngineError::WrongPassword,
        SevenZError::NextHeaderCrcMismatch => {
            if password_provided {
                EngineError::WrongPassword
            } else {
                EngineError::PasswordRequired
            }
        }
        SevenZError::ChecksumVerificationFailed => {
            EngineError::DataError("7z 数据校验失败".to_string())
        }
        SevenZError::BadSignature(_) => EngineError::DataError("7z 签名无效".to_string()),
        SevenZError::UnsupportedVersion { major, minor } => {
            EngineError::UnsupportedFormat(format!("不支持的 7z 版本 {major}.{minor}"))
        }
        SevenZError::UnsupportedCompressionMethod(method) => {
            EngineError::UnsupportedFormat(format!("不支持的 7z 压缩方法：{method}"))
        }
        SevenZError::Unsupported(message) => {
            EngineError::UnsupportedFormat(format!("不支持的 7z 特性：{message}"))
        }
        SevenZError::ExternalUnsupported => {
            EngineError::UnsupportedFormat("7z 使用了外部编码器".to_string())
        }
        SevenZError::MaxMemLimited { max_kb, actaul_kb } => EngineError::Other(format!(
            "7z 解压内存超限：需要 {} MB，上限 {} MB",
            actaul_kb / 1024,
            max_kb / 1024
        )),
        SevenZError::FileNotFound => EngineError::DataError("7z 条目不存在".to_string()),
        SevenZError::Io(error, message) => {
            if message.is_empty() {
                EngineError::Io(error)
            } else {
                EngineError::Io(io::Error::new(error.kind(), format!("{message}: {error}")))
            }
        }
        SevenZError::FileOpen(error, path) => {
            EngineError::Io(io::Error::new(error.kind(), format!("{path}: {error}")))
        }
        other => EngineError::DataError(format!("7z 错误：{other:?}")),
    }
}

pub(crate) fn list(path: &Path, password: Option<&str>) -> Result<ArchiveInfo> {
    let provided = password.is_some();
    let password = Password::from(password.unwrap_or_default());
    let reader = ArchiveReader::open(to_extended(path), password)
        .map_err(|error| map_7z(error, provided))?;
    let archive = reader.archive();
    let encrypted = archive.blocks.iter().any(|block| {
        block
            .coders
            .iter()
            .any(|coder| coder.encoder_method_id() == EncoderMethod::ID_AES256_SHA256)
    });

    let mut entries = Vec::with_capacity(archive.files.len());
    for (index, file) in archive.files.iter().enumerate() {
        let name = normalize_entry_name(file.name());
        if name.is_empty() {
            continue;
        }
        let modified = if file.has_last_modified_date {
            system_time_to_unix(SystemTime::from(file.last_modified_date))
        } else {
            None
        };
        entries.push(ArchiveEntry {
            index,
            path: name,
            is_dir: file.is_directory(),
            size: file.size(),
            compressed_size: file.compressed_size,
            modified,
            method: "7z".to_string(),
            encrypted,
            crc32: file.has_crc.then_some(file.crc),
            is_symlink: false,
            comment: None,
        });
    }

    Ok(ArchiveInfo {
        path: path.to_path_buf(),
        payload_path: path.to_path_buf(),
        disguised: false,
        format: ArchiveFormat::SevenZ,
        entries,
        comment: None,
        encrypted,
        protected_container: false,
        payload_offset: 0,
        note: archive.is_solid.then(|| "固实压缩包".to_string()),
    })
}

pub(crate) fn test_password(path: &Path, password: &str) -> Result<bool> {
    let password_value = Password::from(password);
    let mut reader = match ArchiveReader::open(to_extended(path), password_value) {
        Ok(reader) => reader,
        Err(error) => {
            return match map_7z(error, true) {
                EngineError::WrongPassword | EngineError::PasswordRequired => Ok(false),
                other => Err(other),
            };
        }
    };

    let target = reader
        .archive()
        .files
        .iter()
        .filter(|file| file.has_stream && !file.is_directory())
        .min_by_key(|file| file.size)
        .map(|file| file.name().to_string());
    let Some(name) = target else {
        return Ok(true);
    };
    match reader.read_file(&name) {
        Ok(_) => Ok(true),
        Err(error) => match map_7z(error, true) {
            EngineError::WrongPassword => Ok(false),
            other => Err(other),
        },
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn extract(
    path: &Path,
    info: &ArchiveInfo,
    dest: &Path,
    strip_prefix: Option<&str>,
    selected: Option<&[usize]>,
    password: Option<&str>,
    sink: &dyn ProgressSink,
    motw: Option<&[u8]>,
) -> Result<ExtractOutcome> {
    let selected_names: Option<HashSet<String>> = selected.map(|indices| {
        let selected_indices = crate::archive::expand_selection(info, indices);
        info.entries
            .iter()
            .filter(|entry| selected_indices.contains(&entry.index))
            .map(|entry| entry.path.clone())
            .collect()
    });
    let total: u64 = info
        .entries
        .iter()
        .filter(|entry| {
            !entry.is_dir
                && selected_names
                    .as_ref()
                    .is_none_or(|set| set.contains(&entry.path))
        })
        .map(|entry| entry.size)
        .sum();

    let mut reader = open_reader(path, password)?;
    let mut outcome = ExtractOutcome {
        files: 0,
        dirs: 0,
        bytes: 0,
        written: Vec::new(),
        skipped: Vec::new(),
    };
    let mut done = 0u64;
    let mut side_error: Option<EngineError> = None;
    let mut cancelled = false;

    let result = reader.for_each_entries(|entry, data| {
        if sink.is_cancelled() {
            cancelled = true;
            return Ok(false);
        }
        let name = normalize_entry_name(entry.name());
        let wanted = selected_names
            .as_ref()
            .is_none_or(|set| set.contains(&name));
        if !wanted {
            // 固实块仍然需要顺序读取，丢弃未选中的数据。
            let _ = io::copy(data, &mut io::sink());
            return Ok(true);
        }
        if entry.is_directory() {
            if let Err(error) = create_dir(dest, &name, strip_prefix) {
                side_error = Some(error);
                return Ok(false);
            }
            outcome.dirs += 1;
            sink.update(&Progress::new(Phase::Extracting, done, total, name.clone()));
            return Ok(true);
        }
        if !entry.has_stream() {
            return Ok(true);
        }
        let relative = strip_entry_prefix(&name, strip_prefix);
        if relative.is_empty() {
            return Ok(true);
        }
        let target = match safe_join(dest, relative) {
            Ok(path) => path,
            Err(error) => {
                side_error = Some(error);
                return Ok(false);
            }
        };
        if let Some(parent) = target.parent() {
            if let Err(error) = std::fs::create_dir_all(to_extended(parent)) {
                side_error = Some(EngineError::Io(error));
                return Ok(false);
            }
        }
        let mut output = match File::create(to_extended(&target)) {
            Ok(file) => file,
            Err(error) => {
                side_error = Some(EngineError::Io(error));
                return Ok(false);
            }
        };
        if let Err(error) = preallocate(&output, entry.size()) {
            side_error = Some(EngineError::Io(error));
            return Ok(false);
        }
        if let Err(error) = copy_with_progress(
            data,
            &mut output,
            sink,
            Phase::Extracting,
            &mut done,
            total,
            &name,
        ) {
            side_error = Some(error);
            return Ok(false);
        }
        let modified = entry
            .has_last_modified_date
            .then(|| system_time_to_unix(SystemTime::from(entry.last_modified_date)))
            .flatten();
        let _ = set_mtime(&target, modified);
        if let Some(zone) = motw {
            if let Err(error) = apply_motw(&target, zone) {
                sink.warn(&format!("写入 MOTW 失败：{error}"));
            }
        }
        outcome.files += 1;
        outcome.bytes += entry.size();
        outcome.written.push(target);
        Ok(true)
    });

    if let Some(error) = side_error {
        return Err(error);
    }
    if cancelled {
        return Err(EngineError::Cancelled);
    }
    result.map_err(|error| map_7z(error, true))?;
    sink.update(&Progress::new(
        Phase::Extracting,
        total,
        total,
        "解压完成".to_string(),
    ));
    Ok(outcome)
}

fn create_dir(dest: &Path, name: &str, strip_prefix: Option<&str>) -> Result<()> {
    let relative = strip_entry_prefix(name, strip_prefix);
    if relative.is_empty() {
        return Ok(());
    }
    let target = safe_join(dest, relative)?;
    std::fs::create_dir_all(to_extended(&target))?;
    Ok(())
}

pub(crate) fn verify(
    path: &Path,
    password: Option<&str>,
    sink: &dyn ProgressSink,
) -> Result<VerifyReport> {
    let mut reader = open_reader(path, password)?;
    let total: u64 = reader
        .archive()
        .files
        .iter()
        .filter(|file| file.has_stream && !file.is_directory())
        .map(|file| file.size)
        .sum();
    let mut done = 0u64;
    let mut bytes = 0u64;
    let mut entries = 0usize;
    let mut side_error: Option<EngineError> = None;
    let mut cancelled = false;
    let mut buffer = vec![0u8; COPY_BUFFER_SIZE];

    let result = reader.for_each_entries(|entry, data| {
        if sink.is_cancelled() {
            cancelled = true;
            return Ok(false);
        }
        if !entry.has_stream() || entry.is_directory() {
            return Ok(true);
        }
        entries += 1;
        let name = entry.name().to_string();
        loop {
            match data.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    done += read as u64;
                    bytes += read as u64;
                    sink.update(&Progress::new(Phase::Testing, done, total, name.clone()));
                }
                Err(error) => {
                    side_error = Some(EngineError::Io(error));
                    return Ok(false);
                }
            }
        }
        Ok(true)
    });
    if let Some(error) = side_error {
        return Err(error);
    }
    if cancelled {
        return Err(EngineError::Cancelled);
    }
    result.map_err(|error| map_7z(error, true))?;
    Ok(VerifyReport {
        format: ArchiveFormat::SevenZ,
        entries,
        bytes,
    })
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

/// 读取 7z 归档中某个条目到内存（供哈希工具等使用）。
pub fn read_entry_bytes(path: &Path, name: &str, password: Option<&str>) -> Result<Vec<u8>> {
    let mut reader = open_reader(path, password)?;
    reader.read_file(name).map_err(|error| map_7z(error, true))
}
