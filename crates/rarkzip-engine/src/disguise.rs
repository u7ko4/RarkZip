//! apate 伪装识别与还原。
//!
//! apate（https://github.com/rippod/apate）把原文件头替换为任意面具，并在文件尾部
//! 追加“倒序原头 + 4 字节小端面具长度”。本模块只读原文件，把还原结果写入临时目录。

use std::collections::HashMap;
use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use crate::error::{EngineError, Result};
use crate::format::ArchiveFormat;
use crate::paths::to_extended;

const MAX_MASK_LENGTH: u64 = 1 << 31;
const COPY_CHUNK: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct ApateInfo {
    pub found: bool,
    pub validated: bool,
    pub mask_length: u64,
    pub original_length: u64,
    /// 校验通过时，倒序原头还原后的归档格式。
    pub format: Option<ArchiveFormat>,
}

#[derive(Clone)]
struct CachedReveal {
    size: u64,
    modified: Option<SystemTime>,
    revealed: PathBuf,
}

static REVEAL_CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedReveal>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<PathBuf, CachedReveal>> {
    REVEAL_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 读取文件尾部的长度标记；`validate_head` 为真时额外校验倒序头部签名。
pub fn detect_apate(path: &Path, validate_head: bool) -> Result<Option<ApateInfo>> {
    let mut file = File::open(to_extended(path))?;
    let size = file.metadata()?.len();
    if size < 16 {
        return Ok(None);
    }
    file.seek(SeekFrom::End(-4))?;
    let mut marker = [0u8; 4];
    file.read_exact(&mut marker)?;
    let mask_length = u32::from_le_bytes(marker) as u64;
    if !(4..=MAX_MASK_LENGTH).contains(&mask_length) || mask_length > size - 4 {
        return Ok(None);
    }
    let original_length = size - 4 - mask_length;
    let mut info = ApateInfo {
        found: true,
        validated: false,
        mask_length,
        original_length,
        format: None,
    };
    if validate_head {
        let probe = mask_length.min(16) as usize;
        if probe < 4 {
            return Ok(None);
        }
        file.seek(SeekFrom::Start(size - 4 - probe as u64))?;
        let mut tail = vec![0u8; probe];
        file.read_exact(&mut tail)?;
        tail.reverse();
        info.format = signature_kind(&tail);
        info.validated = info.format.is_some();
        if !info.validated {
            return Ok(None);
        }
    }
    Ok(Some(info))
}

/// 快速判断是否是经过 apate 处理且头部可还原的文件。
pub fn is_disguised(path: &Path) -> bool {
    detect_apate(path, true).ok().flatten().is_some()
}

/// 把还原结果写到 `destination`，返回还原后的长度。
pub fn reveal_to_file(source: &Path, destination: &Path) -> Result<u64> {
    let Some(info) = detect_apate(source, false)? else {
        return Err(EngineError::Other(
            "文件末尾没有 apate 伪装标记".to_string(),
        ));
    };
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(to_extended(parent))?;
    }
    let mut output = File::create(to_extended(destination))?;
    let size = std::fs::metadata(to_extended(source))?.len();
    reveal_into(source, &info, size, &mut output)?;
    output.flush()?;
    Ok(info.original_length)
}

/// 把一组 apate 伪装的连续分卷还原并按顺序拼接成一个归档文件。
///
/// zip/7z/tar 的分卷本质上是字节切片，逐卷还原后直接接起来就是完整归档。
pub fn reveal_merged_to_file(members: &[PathBuf], destination: &Path) -> Result<u64> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(to_extended(parent))?;
    }
    let mut output = File::create(to_extended(destination))?;
    let mut total = 0u64;
    for member in members {
        let Some(info) = detect_apate(member, false)? else {
            return Err(EngineError::Other(format!(
                "文件末尾没有 apate 伪装标记：{}",
                member.display()
            )));
        };
        let size = std::fs::metadata(to_extended(member))?.len();
        reveal_into(member, &info, size, &mut output)?;
        total += info.original_length;
    }
    output.flush()?;
    Ok(total)
}

/// 把 apate 伪装的分卷逐卷还原到 `directory`，使用给定文件名。
pub fn reveal_set_to_directory(
    members: &[PathBuf],
    directory: &Path,
    names: &[String],
) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(to_extended(directory))?;
    let mut revealed = Vec::with_capacity(members.len());
    for (member, name) in members.iter().zip(names.iter()) {
        let target = directory.join(name);
        reveal_to_file(member, &target)?;
        revealed.push(target);
    }
    Ok(revealed)
}

/// 还原结果的缓存路径；键包含所有分卷的路径、大小和修改时间。
pub fn set_cache_path(members: &[PathBuf], suffix: &str) -> Result<PathBuf> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    for member in members {
        let metadata = std::fs::metadata(to_extended(member))?;
        std::fs::canonicalize(to_extended(member))
            .unwrap_or_else(|_| member.clone())
            .hash(&mut hasher);
        metadata.len().hash(&mut hasher);
        metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .hash(&mut hasher);
    }
    let directory = revealed_directory()
        .unwrap_or_else(|| std::env::temp_dir().join("RarkZip").join("revealed"))
        .join("sets");
    std::fs::create_dir_all(to_extended(&directory))?;
    Ok(directory.join(format!("{:016x}{suffix}", hasher.finish())))
}

/// 还原一个 apate 分卷集合，返回可直接交给后端的归档路径。
///
/// zip/7z/tar 会被拼接成一个临时文件；RAR 必须保留分卷结构，因此逐卷还原
/// 成 `volume.partN.rar` 后交给 libunrar。
pub fn reveal_set(members: &[PathBuf], format: ArchiveFormat) -> Result<PathBuf> {
    if format == ArchiveFormat::Rar {
        let directory = set_cache_path(members, ".rar")?;
        std::fs::create_dir_all(to_extended(&directory))?;
        let mut names = Vec::with_capacity(members.len());
        for index in 0..members.len() {
            names.push(if members.len() == 1 {
                "volume.rar".to_string()
            } else {
                format!("volume.part{}.rar", index + 1)
            });
        }
        let expected: Vec<u64> = members
            .iter()
            .map(|member| {
                detect_apate(member, false)
                    .ok()
                    .flatten()
                    .map(|info| info.original_length)
                    .unwrap_or(0)
            })
            .collect();
        let complete = names.iter().zip(expected.iter()).all(|(name, length)| {
            directory
                .join(name)
                .metadata()
                .map(|metadata| metadata.len() == *length)
                .unwrap_or(false)
        });
        if !complete {
            let _ = std::fs::remove_dir_all(to_extended(&directory));
            reveal_set_to_directory(members, &directory, &names)?;
        }
        return Ok(directory.join(names[0].clone()));
    }

    let expected: u64 = members
        .iter()
        .map(|member| {
            detect_apate(member, false)
                .ok()
                .flatten()
                .map(|info| info.original_length)
                .unwrap_or(0)
        })
        .sum();
    let cache = set_cache_path(members, ".archive")?;
    if cache
        .metadata()
        .map(|metadata| metadata.len() == expected)
        .unwrap_or(false)
    {
        return Ok(cache);
    }
    let temporary = cache.with_extension("archive.part");
    reveal_merged_to_file(members, &temporary)?;
    std::fs::rename(to_extended(&temporary), to_extended(&cache))?;
    Ok(cache)
}

fn reveal_into(source: &Path, info: &ApateInfo, size: u64, output: &mut File) -> Result<()> {
    let mut input = File::open(to_extended(source))?;
    let mut buffer = vec![0u8; COPY_CHUNK];

    // 1. 倒序存放的原头：从块尾向前读出、翻转后写到目标开头。
    let block_start = size - 4 - info.mask_length;
    let mut remaining = info.mask_length;
    while remaining > 0 {
        let chunk = remaining.min(COPY_CHUNK as u64) as usize;
        let chunk_start = block_start + remaining - chunk as u64;
        read_at(&mut input, chunk_start, &mut buffer[..chunk])?;
        buffer[..chunk].reverse();
        output.write_all(&buffer[..chunk])?;
        remaining -= chunk as u64;
    }

    // 2. 载荷：面具之后到原头之前的所有字节。
    let mut position = info.mask_length;
    let mut payload = info.original_length.saturating_sub(info.mask_length);
    while payload > 0 {
        let chunk = payload.min(COPY_CHUNK as u64) as usize;
        read_at(&mut input, position, &mut buffer[..chunk])?;
        output.write_all(&buffer[..chunk])?;
        position += chunk as u64;
        payload -= chunk as u64;
    }
    Ok(())
}

/// 返回可直接打开的干净副本路径；普通文件返回 `None`。
///
/// 还原结果按源路径 + 大小 + 修改时间缓存，密码本逐条试密时不会反复复制大文件。
pub fn resolve_payload(path: &Path) -> Result<Option<PathBuf>> {
    static CLEANUP: OnceLock<()> = OnceLock::new();
    CLEANUP.get_or_init(cleanup_stale_reveals);
    let Some(_) = detect_apate(path, true)? else {
        return Ok(None);
    };
    let metadata = std::fs::metadata(to_extended(path))?;
    let key = std::fs::canonicalize(to_extended(path)).unwrap_or_else(|_| path.to_path_buf());
    let size = metadata.len();
    let modified = metadata.modified().ok();
    {
        let cache = cache()
            .lock()
            .map_err(|_| EngineError::Other("伪装缓存锁失败".to_string()))?;
        if let Some(entry) = cache.get(&key) {
            if entry.size == size && entry.modified == modified && entry.revealed.exists() {
                return Ok(Some(entry.revealed.clone()));
            }
        }
    }

    let revealed = reveal_path(&key, size, modified);
    let mut cache = cache()
        .lock()
        .map_err(|_| EngineError::Other("伪装缓存锁失败".to_string()))?;
    if let Some(entry) = cache.get(&key) {
        if entry.revealed.exists() {
            return Ok(Some(entry.revealed.clone()));
        }
    }
    reveal_to_file(path, &revealed)?;
    cache.insert(
        key,
        CachedReveal {
            size,
            modified,
            revealed: revealed.clone(),
        },
    );
    Ok(Some(revealed))
}

/// 清理超过 24 小时的旧还原文件。
pub fn cleanup_stale_reveals() {
    let Some(dir) = revealed_directory() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(to_extended(&dir)) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if now
            .duration_since(modified)
            .map(|age| age.as_secs() > 24 * 60 * 60)
            .unwrap_or(false)
        {
            if meta.is_dir() {
                let _ = std::fs::remove_dir_all(entry.path());
            } else {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

fn revealed_directory() -> Option<PathBuf> {
    Some(std::env::temp_dir().join("RarkZip").join("revealed"))
}

fn reveal_path(key: &Path, size: u64, modified: Option<SystemTime>) -> PathBuf {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    size.hash(&mut hasher);
    modified.hash(&mut hasher);
    let name = format!("{:016x}.archive", hasher.finish());
    revealed_directory()
        .unwrap_or_else(|| std::env::temp_dir().join("RarkZip").join("revealed"))
        .join(name)
}

fn read_at(file: &mut File, offset: u64, buffer: &mut [u8]) -> std::io::Result<()> {
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(buffer)
}

fn signature_kind(data: &[u8]) -> Option<ArchiveFormat> {
    if data.len() >= 4
        && data[0] == b'P'
        && data[1] == b'K'
        && ((data[2] == 3 && data[3] == 4)
            || (data[2] == 5 && data[3] == 6)
            || (data[2] == 7 && data[3] == 8))
    {
        return Some(ArchiveFormat::Zip);
    }
    if data.len() >= 6 && &data[..6] == b"7z\xBC\xAF\x27\x1C" {
        return Some(ArchiveFormat::SevenZ);
    }
    if data.len() >= 8 && &data[..8] == b"Rar!\x1A\x07\x01\x00" {
        return Some(ArchiveFormat::Rar);
    }
    if data.len() >= 7 && &data[..7] == b"Rar!\x1A\x07\x00" {
        return Some(ArchiveFormat::Rar);
    }
    None
}

#[allow(dead_code)]
fn modified_of(meta: &Metadata) -> Option<SystemTime> {
    meta.modified().ok()
}
