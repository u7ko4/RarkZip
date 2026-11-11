//! ZIP 归档编辑：删除条目、重命名条目、修改注释。
//!
//! 通过重写整个 ZIP 实现，保留条目修改时间与压缩方法；加密条目会用已知密码
//! 重新以 AES-256 加密。输出先写临时文件，成功后再替换目标，避免半成品。

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use zip::{AesMode, ZipArchive, ZipWriter};

use crate::error::{EngineError, Result};
use crate::format::ArchiveFormat;
use crate::paths::{entry_segments, normalize_entry_name, to_extended};

#[derive(Debug, Clone, Default)]
pub struct ZipEditOptions {
    /// 要删除的条目索引。
    pub delete: Vec<usize>,
    /// 条目索引 -> 新路径。
    pub rename: HashMap<usize, String>,
    /// 新注释；None 保留原注释，Some("") 清空。
    pub comment: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ZipEditReport {
    pub format: ArchiveFormat,
    pub entries: usize,
    pub written: usize,
    pub output: PathBuf,
}

pub fn edit_zip(
    input: &Path,
    output: &Path,
    password: Option<&str>,
    options: &ZipEditOptions,
) -> Result<ZipEditReport> {
    let file = File::open(to_extended(input))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| EngineError::DataError(format!("读取 ZIP 失败：{error}")))?;
    let original_comment = String::from_utf8_lossy(archive.comment()).to_string();
    let delete: HashSet<usize> = options.delete.iter().copied().collect();
    let temporary = output.with_extension(format!(
        "{}.nimzip-tmp",
        output
            .extension()
            .map(|value| value.to_string_lossy())
            .unwrap_or_default()
    ));
    let target = if output == input { &temporary } else { output };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(to_extended(parent))?;
    }
    let out_file = File::create(to_extended(target))?;
    let mut writer = ZipWriter::new(out_file);
    writer.set_comment(if let Some(comment) = &options.comment {
        comment.clone()
    } else {
        original_comment
    });

    let mut seen = HashSet::new();
    let mut written = 0usize;
    for index in 0..archive.len() {
        if delete.contains(&index) {
            continue;
        }
        let encrypted = {
            let file = archive
                .by_index_raw(index)
                .map_err(|error| EngineError::DataError(error.to_string()))?;
            file.encrypted()
        };
        if encrypted && password.is_none() {
            return Err(EngineError::PasswordRequired);
        }
        let name = {
            let file = archive
                .by_index_raw(index)
                .map_err(|error| EngineError::DataError(error.to_string()))?;
            let decoded = file.name().to_string();
            drop(file);
            options.rename.get(&index).cloned().unwrap_or(decoded)
        };
        let name = normalize_entry_name(&name);
        if name.is_empty() {
            continue;
        }
        if !seen.insert(name.clone()) {
            return Err(EngineError::Other(format!("重命名后路径重复：{name}")));
        }
        let mut entry = if encrypted {
            archive
                .by_index_decrypt(index, password.unwrap_or_default().as_bytes())
                .map_err(|error| EngineError::DataError(error.to_string()))?
        } else {
            archive
                .by_index(index)
                .map_err(|error| EngineError::DataError(error.to_string()))?
        };
        let modified = entry.last_modified();
        let method = match entry.compression() {
            zip::CompressionMethod::Stored => zip::CompressionMethod::Stored,
            zip::CompressionMethod::Deflated => zip::CompressionMethod::Deflated,
            zip::CompressionMethod::Bzip2 => zip::CompressionMethod::Bzip2,
            zip::CompressionMethod::Zstd => zip::CompressionMethod::Zstd,
            zip::CompressionMethod::Lzma => zip::CompressionMethod::Lzma,
            _ => zip::CompressionMethod::Deflated,
        };
        let mut simple = zip::write::SimpleFileOptions::default().compression_method(method);
        if let Some(modified) = modified {
            simple = simple.last_modified_time(modified);
        }
        if entry.is_dir() {
            writer
                .add_directory(name.trim_end_matches('/'), simple)
                .map_err(|error| EngineError::DataError(error.to_string()))?;
            written += 1;
            continue;
        }
        if encrypted {
            let aes = simple.with_aes_encryption(AesMode::Aes256, password.unwrap_or_default());
            writer
                .start_file(name.clone(), aes)
                .map_err(|error| EngineError::DataError(error.to_string()))?;
        } else {
            writer
                .start_file(name.clone(), simple)
                .map_err(|error| EngineError::DataError(error.to_string()))?;
        }
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let read = entry.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            writer.write_all(&buffer[..read])?;
        }
        written += 1;
    }
    writer
        .finish()
        .map_err(|error| EngineError::DataError(error.to_string()))?;
    if target == temporary {
        std::fs::rename(to_extended(&temporary), to_extended(output))?;
    }
    Ok(ZipEditReport {
        format: ArchiveFormat::Zip,
        entries: archive.len(),
        written,
        output: output.to_path_buf(),
    })
}

#[derive(Debug, Clone)]
pub struct ZipAddReport {
    pub format: ArchiveFormat,
    pub added: usize,
    pub output: PathBuf,
}

/// 向现有 ZIP 追加文件或文件夹。
///
/// 追加的加密条目沿用传入密码（AES-256）；若归档本身已加密但未提供密码，
/// 返回 `PasswordRequired`，避免写出混合安全级别的归档。
pub fn add_zip_entries(
    archive: &Path,
    sources: &[PathBuf],
    password: Option<&str>,
) -> Result<ZipAddReport> {
    let info = crate::archive::open_archive(
        archive,
        &crate::archive::OpenOptions {
            password: password.map(ToString::to_string),
            ..Default::default()
        },
    )?;
    if !info.format.is_zip() {
        return Err(EngineError::UnsupportedFormat(
            "当前仅支持编辑 ZIP 归档".to_string(),
        ));
    }
    if info.encrypted && password.is_none() {
        return Err(EngineError::PasswordRequired);
    }
    let existing: HashSet<String> = info
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    let mut entries = Vec::new();
    for source in sources {
        collect_source(source, &mut entries)?;
    }
    if entries.is_empty() {
        return Err(EngineError::Other("没有可添加的文件".to_string()));
    }
    for entry in &entries {
        if existing.contains(&entry.name) {
            return Err(EngineError::AlreadyExists(PathBuf::from(&entry.name)));
        }
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(to_extended(archive))?;
    let mut writer = ZipWriter::new_append(file)
        .map_err(|error| EngineError::DataError(format!("打开 ZIP 失败：{error}")))?;
    let mut added = 0usize;
    for entry in &entries {
        let mut options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(entry.mode);
        if let Some(modified) = entry.modified {
            options = options.last_modified_time(modified);
        }
        if entry.is_dir {
            writer
                .add_directory(entry.name.trim_end_matches('/'), options)
                .map_err(|error| EngineError::DataError(error.to_string()))?;
            added += 1;
            continue;
        }
        if let Some(password) = password {
            writer
                .start_file(
                    entry.name.clone(),
                    options.with_aes_encryption(AesMode::Aes256, password),
                )
                .map_err(|error| EngineError::DataError(error.to_string()))?;
        } else {
            writer
                .start_file(entry.name.clone(), options)
                .map_err(|error| EngineError::DataError(error.to_string()))?;
        }
        let mut input = File::open(to_extended(&entry.disk))?;
        std::io::copy(&mut input, &mut writer)?;
        added += 1;
    }
    writer
        .finish()
        .map_err(|error| EngineError::DataError(error.to_string()))?;
    Ok(ZipAddReport {
        format: ArchiveFormat::Zip,
        added,
        output: archive.to_path_buf(),
    })
}

/// 在现有 ZIP 中创建一个空文件夹条目。
pub fn add_zip_directory(
    archive: &Path,
    name: &str,
    password: Option<&str>,
) -> Result<ZipAddReport> {
    let info = crate::archive::open_archive(
        archive,
        &crate::archive::OpenOptions {
            password: password.map(ToString::to_string),
            ..Default::default()
        },
    )?;
    if !info.format.is_zip() {
        return Err(EngineError::UnsupportedFormat(
            "当前仅支持编辑 ZIP 归档".to_string(),
        ));
    }
    if info.encrypted && password.is_none() {
        return Err(EngineError::PasswordRequired);
    }
    let segments = entry_segments(name)?;
    if segments.is_empty() {
        return Err(EngineError::Other("文件夹名称不能为空".to_string()));
    }
    let name = segments.join("/");
    if info.entries.iter().any(|entry| entry.path == name) {
        return Err(EngineError::AlreadyExists(PathBuf::from(name)));
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(to_extended(archive))?;
    let mut writer = ZipWriter::new_append(file)
        .map_err(|error| EngineError::DataError(format!("打开 ZIP 失败：{error}")))?;
    writer
        .add_directory(
            name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o755),
        )
        .map_err(|error| EngineError::DataError(error.to_string()))?;
    writer
        .finish()
        .map_err(|error| EngineError::DataError(error.to_string()))?;
    Ok(ZipAddReport {
        format: ArchiveFormat::Zip,
        added: 1,
        output: archive.to_path_buf(),
    })
}

struct AddEntry {
    disk: PathBuf,
    name: String,
    is_dir: bool,
    mode: u32,
    modified: Option<zip::DateTime>,
}

fn collect_source(source: &Path, out: &mut Vec<AddEntry>) -> Result<()> {
    if source.is_file() {
        let name = source
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .ok_or_else(|| EngineError::Other("非法文件名".to_string()))?;
        out.push(add_entry(source, &name)?);
        return Ok(());
    }
    if source.is_dir() {
        let root = source
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .ok_or_else(|| EngineError::Other("非法目录名".to_string()))?;
        for item in walkdir::WalkDir::new(source).follow_links(false) {
            let item = item.map_err(|error| EngineError::Other(error.to_string()))?;
            let relative = item
                .path()
                .strip_prefix(source)
                .unwrap_or_else(|_| item.path())
                .to_string_lossy()
                .replace('\\', "/");
            if relative.is_empty() {
                continue;
            }
            out.push(add_entry(item.path(), &format!("{root}/{relative}"))?);
        }
        return Ok(());
    }
    Err(EngineError::NotFound(source.to_path_buf()))
}

fn add_entry(disk: &Path, name: &str) -> Result<AddEntry> {
    let meta = std::fs::metadata(to_extended(disk))?;
    let modified = meta.modified().ok().and_then(|value| {
        let seconds = value.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
        let date = chrono::DateTime::from_timestamp(seconds, 0)?;
        use chrono::{Datelike, Timelike};
        zip::DateTime::from_date_and_time(
            date.year() as u16,
            date.month() as u8,
            date.day() as u8,
            date.hour() as u8,
            date.minute() as u8,
            date.second() as u8,
        )
        .ok()
    });
    Ok(AddEntry {
        disk: disk.to_path_buf(),
        name: normalize_entry_name(name),
        is_dir: meta.is_dir(),
        mode: if meta.is_dir() { 0o755 } else { 0o644 },
        modified,
    })
}
