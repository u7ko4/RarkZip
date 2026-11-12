//! 分卷归档识别与合并。
//!
//! `.zip.001` / `.7z.001` / `.tar.gz.001` 等格式本质上是把单个归档按字节切块。
//! 库后端通常只接受完整文件，因此这里把连续分卷按顺序合并到临时目录，并把
//! 原始分卷保持只读、不改名。RAR 的 `.part1.rar` / `.r00` 由 libunrar 原生处理，
//! 不走本模块。

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::format::ArchiveFormat;
use crate::paths::to_extended;

#[derive(Debug, Clone)]
pub struct VolumeSet {
    /// 合并后的临时归档路径。
    pub payload: PathBuf,
    /// 参与合并的分卷数量。
    pub parts: usize,
    pub format: ArchiveFormat,
}

/// 如果 `path` 是连续的 `.001` 分卷之一，则返回合并后的临时归档。
pub fn resolve(path: &Path) -> Result<Option<VolumeSet>> {
    let Some(parts) = collect_parts(path)? else {
        return Ok(None);
    };
    let format = header_format(&parts[0])?;
    let Some(format) = format else {
        return Ok(None);
    };
    if format == ArchiveFormat::Rar {
        // RAR 分卷由 unrar 负责首卷查找，不能简单字节拼接。
        return Ok(None);
    }
    let total = parts
        .iter()
        .map(|part| part.metadata().map(|meta| meta.len()))
        .collect::<std::io::Result<Vec<_>>>()?
        .iter()
        .sum::<u64>();
    let cache = cache_path(&parts, format, total)?;
    if cache
        .metadata()
        .map(|meta| meta.len() == total)
        .unwrap_or(false)
    {
        return Ok(Some(VolumeSet {
            payload: cache,
            parts: parts.len(),
            format,
        }));
    }
    assemble(&parts, &cache)?;
    Ok(Some(VolumeSet {
        payload: cache,
        parts: parts.len(),
        format,
    }))
}

fn collect_parts(path: &Path) -> Result<Option<Vec<PathBuf>>> {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(None);
    };
    let Some(dot) = name.rfind('.') else {
        return Ok(None);
    };
    let number = &name[dot + 1..];
    if !(2..=4).contains(&number.len()) || !number.chars().all(|ch| ch.is_ascii_digit()) {
        return Ok(None);
    }
    let width = number.len();
    let base = &name[..dot];
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let first = parent.join(format!("{base}.{:0width$}", 1, width = width));
    if !first.is_file() {
        return Ok(None);
    }
    let mut parts = Vec::new();
    let mut index = 1usize;
    loop {
        let candidate = parent.join(format!("{base}.{index:0width$}"));
        if !candidate.is_file() {
            break;
        }
        parts.push(candidate);
        index += 1;
    }
    if parts.len() < 2 {
        return Ok(None);
    }
    Ok(Some(parts))
}

fn header_format(path: &Path) -> Result<Option<ArchiveFormat>> {
    let mut file = File::open(to_extended(path))?;
    let mut head = vec![0u8; 512];
    let read = file.read(&mut head)?;
    head.truncate(read);
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        return Ok(Some(ArchiveFormat::Zip));
    }
    if head.starts_with(b"7z\xBC\xAF\x27\x1C") {
        return Ok(Some(ArchiveFormat::SevenZ));
    }
    if head.starts_with(&[0x1F, 0x8B]) {
        return Ok(Some(ArchiveFormat::TarGz));
    }
    if head.starts_with(b"BZh") {
        return Ok(Some(ArchiveFormat::TarBz2));
    }
    if head.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0x00]) {
        return Ok(Some(ArchiveFormat::TarXz));
    }
    if head.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        return Ok(Some(ArchiveFormat::TarZst));
    }
    if head.len() >= 262 && &head[257..262] == b"ustar" {
        return Ok(Some(ArchiveFormat::Tar));
    }
    Ok(None)
}

fn cache_path(parts: &[PathBuf], format: ArchiveFormat, total: u64) -> Result<PathBuf> {
    let mut hasher = DefaultHasher::new();
    for part in parts {
        let metadata = part.metadata()?;
        part.canonicalize()
            .unwrap_or_else(|_| part.clone())
            .hash(&mut hasher);
        metadata.len().hash(&mut hasher);
        metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .hash(&mut hasher);
    }
    total.hash(&mut hasher);
    let dir = std::env::temp_dir().join("RarkZip").join("volumes");
    fs::create_dir_all(to_extended(&dir))?;
    Ok(dir.join(format!("{:016x}.{}", hasher.finish(), format.extension())))
}

fn assemble(parts: &[PathBuf], cache: &Path) -> Result<()> {
    let temp = cache.with_extension(format!(
        "{}.part",
        cache
            .extension()
            .map(|ext| ext.to_string_lossy())
            .unwrap_or_default()
    ));
    {
        let mut output = File::create(to_extended(&temp))?;
        for part in parts {
            let mut input = File::open(to_extended(part))?;
            std::io::copy(&mut input, &mut output)?;
        }
        output.flush()?;
    }
    fs::rename(to_extended(&temp), to_extended(cache))?;
    Ok(())
}

/// 删除一个分卷归档时，同时删除其余分卷（仍由调用方决定是否走回收站）。
pub fn part_paths(path: &Path) -> Result<Vec<PathBuf>> {
    Ok(collect_parts(path)?.unwrap_or_else(|| vec![path.to_path_buf()]))
}

/// `.002` 及后续分卷不作为独立嵌套归档处理，避免重复解压。
pub fn is_continuation(path: &Path) -> Result<bool> {
    let Some(parts) = collect_parts(path)? else {
        return Ok(false);
    };
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    Ok(parts
        .first()
        .map(|first| first.canonicalize().unwrap_or_else(|_| first.clone()) != canonical)
        .unwrap_or(false))
}

impl VolumeSet {
    pub fn note(&self) -> String {
        format!(
            "已将 {} 个分卷合并为临时归档（原始分卷保持只读）",
            self.parts
        )
    }
}
