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
use crate::paths::to_extended;

const MAX_MASK_LENGTH: u64 = 1 << 31;
const COPY_CHUNK: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct ApateInfo {
    pub found: bool,
    pub validated: bool,
    pub mask_length: u64,
    pub original_length: u64,
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
        info.validated = signature_kind(&tail).is_some();
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
    let size = std::fs::metadata(to_extended(source))?.len();
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(to_extended(parent))?;
    }
    let mut input = File::open(to_extended(source))?;
    let mut output = File::create(to_extended(destination))?;
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
    output.flush()?;
    Ok(info.original_length)
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
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn revealed_directory() -> Option<PathBuf> {
    Some(std::env::temp_dir().join("NimZip").join("revealed"))
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
        .unwrap_or_else(|| std::env::temp_dir().join("NimZip").join("revealed"))
        .join(name)
}

fn read_at(file: &mut File, offset: u64, buffer: &mut [u8]) -> std::io::Result<()> {
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(buffer)
}

fn signature_kind(data: &[u8]) -> Option<()> {
    if data.len() >= 4
        && data[0] == b'P'
        && data[1] == b'K'
        && ((data[2] == 3 && data[3] == 4)
            || (data[2] == 5 && data[3] == 6)
            || (data[2] == 7 && data[3] == 8))
    {
        return Some(());
    }
    if data.len() >= 6 && &data[..6] == b"7z\xBC\xAF\x27\x1C" {
        return Some(());
    }
    if data.len() >= 8 && &data[..8] == b"Rar!\x1A\x07\x01\x00" {
        return Some(());
    }
    if data.len() >= 7 && &data[..7] == b"Rar!\x1A\x07\x00" {
        return Some(());
    }
    None
}

#[allow(dead_code)]
fn modified_of(meta: &Metadata) -> Option<SystemTime> {
    meta.modified().ok()
}
