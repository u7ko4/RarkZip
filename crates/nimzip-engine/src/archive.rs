use std::path::{Path, PathBuf};

use crate::error::{EngineError, Result};
use crate::format::{ArchiveFormat, Codepage};
use crate::progress::ProgressSink;
use crate::sniff;

pub use crate::extract::expand_selection;
pub use crate::format::Codepage as ArchiveCodepage;

#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    /// 在归档中的序号（zip 为 ZipArchive 索引）。
    pub index: usize,
    /// 规范化的归档内路径，使用 `/` 分隔。
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub compressed_size: u64,
    /// Unix 秒。
    pub modified: Option<i64>,
    pub method: String,
    pub encrypted: bool,
    pub crc32: Option<u64>,
    pub is_symlink: bool,
    pub comment: Option<String>,
}

impl ArchiveEntry {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    pub fn parent_path(&self) -> Option<&str> {
        self.path.rfind('/').map(|index| &self.path[..index])
    }
}

#[derive(Debug, Clone)]
pub struct ArchiveInfo {
    /// 用户打开的原始路径（可能是伪装文件）。
    pub path: PathBuf,
    /// 实际读取的归档路径；apate 伪装还原后指向临时副本。
    pub payload_path: PathBuf,
    /// 是否检测到 apate 结构化伪装。
    pub disguised: bool,
    pub format: ArchiveFormat,
    pub entries: Vec<ArchiveEntry>,
    pub comment: Option<String>,
    pub encrypted: bool,
    pub protected_container: bool,
    pub payload_offset: u64,
    pub note: Option<String>,
}

impl ArchiveInfo {
    pub fn total_size(&self) -> u64 {
        self.entries
            .iter()
            .filter(|e| !e.is_dir)
            .map(|e| e.size)
            .sum()
    }

    pub fn total_compressed(&self) -> u64 {
        self.entries
            .iter()
            .filter(|e| !e.is_dir)
            .map(|e| e.compressed_size)
            .sum()
    }

    pub fn file_count(&self) -> usize {
        self.entries.iter().filter(|e| !e.is_dir).count()
    }

    pub fn encrypted_entry_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| !e.is_dir && e.encrypted)
            .count()
    }

    pub fn find(&self, index: usize) -> Option<&ArchiveEntry> {
        self.entries.iter().find(|entry| entry.index == index)
    }
}

#[derive(Debug, Clone, Default)]
pub struct OpenOptions {
    pub password: Option<String>,
    pub codepage: Codepage,
}

#[derive(Debug, Clone)]
pub struct VerifyReport {
    pub format: ArchiveFormat,
    pub entries: usize,
    pub bytes: u64,
}

/// 识别并打开归档，返回条目清单。列表阶段通常不需要密码（7z 头加密除外）。
pub fn open_archive(path: &Path, options: &OpenOptions) -> Result<ArchiveInfo> {
    if !path.exists() {
        return Err(EngineError::NotFound(path.to_path_buf()));
    }
    let volume = crate::volumes::resolve(path)?;
    let payload = if volume.is_none() {
        crate::disguise::resolve_payload(path)?
    } else {
        None
    };
    let read_path = volume
        .as_ref()
        .map(|set| set.payload.as_path())
        .or(payload.as_deref())
        .unwrap_or(path);
    let mut sniffed = sniff::sniff(read_path)?
        .ok_or_else(|| EngineError::UnsupportedFormat("无法识别的归档内容".to_string()))?;
    sniff::ensure_supported(&sniffed)?;
    // ZIPX 与 ZIP 共用 PK 结构；按原文件扩展名保留 ZIPX 显示。
    if sniffed.format == ArchiveFormat::Zip
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("zipx"))
    {
        sniffed.format = ArchiveFormat::Zipx;
    }
    let actual_path = if sniffed.format == ArchiveFormat::Rar {
        crate::rar_backend::first_part(read_path)
    } else {
        read_path.to_path_buf()
    };
    // 7z 内部偏移以归档起点计算；SFX 前缀需要先剥掉，写入临时缓存副本。
    let sfx_payload = if sniffed.format == ArchiveFormat::SevenZ && sniffed.payload_offset > 0 {
        Some(extract_7z_sfx_payload(
            actual_path.as_path(),
            sniffed.payload_offset,
        )?)
    } else {
        None
    };
    let read_path = sfx_payload.as_deref().unwrap_or(actual_path.as_path());

    let mut info = match sniffed.format {
        ArchiveFormat::Zip | ArchiveFormat::Zipx => {
            crate::zip_backend::list(read_path, options.codepage)?
        }
        ArchiveFormat::SevenZ => {
            crate::sevenz_backend::list(read_path, options.password.as_deref())?
        }
        ArchiveFormat::Tar
        | ArchiveFormat::TarGz
        | ArchiveFormat::TarBz2
        | ArchiveFormat::TarXz
        | ArchiveFormat::TarZst => crate::tar_backend::list(read_path, sniffed.format)?,
        ArchiveFormat::Gz | ArchiveFormat::Bz2 | ArchiveFormat::Xz | ArchiveFormat::Zst => {
            crate::tar_backend::list_single(read_path, sniffed.format)?
        }
        ArchiveFormat::Rar => {
            crate::rar_backend::list(read_path, options.codepage, options.password.as_deref())?
        }
        ArchiveFormat::Iso => crate::iso_backend::list(read_path, options.password.as_deref())?,
        ArchiveFormat::Lzh => crate::lzh_backend::list(read_path, options.password.as_deref())?,
        ArchiveFormat::Zpaq => crate::zpaq_backend::list(read_path, options.password.as_deref())?,
        ArchiveFormat::Wim | ArchiveFormat::Cab | ArchiveFormat::Arj | ArchiveFormat::Asar => {
            crate::external7z::list(read_path, sniffed.format, options.password.as_deref())?
        }
        ArchiveFormat::Unknown => {
            return Err(EngineError::UnsupportedFormat(
                "无法识别的归档内容".to_string(),
            ))
        }
    };

    info.path = path.to_path_buf();
    info.payload_path = sfx_payload
        .clone()
        .unwrap_or_else(|| read_path.to_path_buf());
    info.disguised = payload.is_some();
    info.format = sniffed.format;
    info.protected_container = sniffed.protected_container || sniff::is_protected_container(path);
    info.payload_offset = sniffed.payload_offset;
    if let Some(volume) = &volume {
        info.note = Some(volume.note());
    } else if info.disguised {
        info.note = Some("已去除 apate 伪装，使用临时还原副本".to_string());
    } else if info.note.is_none() {
        info.note = sniffed.note;
    }
    Ok(info)
}

/// 把 7z SFX 的归档载荷复制到临时缓存文件，后续列表/解压/校验共用同一副本。
fn extract_7z_sfx_payload(path: &Path, offset: u64) -> Result<PathBuf> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::io::{Seek, SeekFrom};

    let metadata = std::fs::metadata(crate::paths::to_extended(path))?;
    let expected = metadata.len().saturating_sub(offset);
    let mut hasher = DefaultHasher::new();
    std::fs::canonicalize(crate::paths::to_extended(path))
        .unwrap_or_else(|_| path.to_path_buf())
        .hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .hash(&mut hasher);
    offset.hash(&mut hasher);

    let dir = std::env::temp_dir().join("NimZip").join("sfx-7z");
    std::fs::create_dir_all(crate::paths::to_extended(&dir))?;
    let dest = dir.join(format!("{:016x}.7z", hasher.finish()));
    if std::fs::metadata(crate::paths::to_extended(&dest))
        .map(|meta| meta.len() == expected)
        .unwrap_or(false)
    {
        return Ok(dest);
    }
    let mut input = std::fs::File::open(crate::paths::to_extended(path))?;
    input.seek(SeekFrom::Start(offset))?;
    let temporary = dest.with_extension("7z.tmp");
    let mut output = std::fs::File::create(crate::paths::to_extended(&temporary))?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    drop(output);
    std::fs::rename(
        crate::paths::to_extended(&temporary),
        crate::paths::to_extended(&dest),
    )?;
    Ok(dest)
}

/// 扫描解压产物时使用的内容探测；包含 apate 伪装识别，但不返回完整清单。
pub fn probe_archive(path: &Path) -> Result<Option<ArchiveFormat>> {
    if let Some(sniffed) = sniff::sniff_with_window(path, 64 * 1024)? {
        if sniffed.format != ArchiveFormat::Unknown {
            return Ok(Some(sniffed.format));
        }
    }
    if let Some(payload) = crate::disguise::resolve_payload(path)? {
        if let Some(sniffed) = sniff::sniff(&payload)? {
            return Ok(Some(sniffed.format));
        }
    }
    Ok(None)
}

pub fn verify_archive(
    path: &Path,
    password: Option<&str>,
    sink: &dyn ProgressSink,
) -> Result<VerifyReport> {
    let volume = crate::volumes::resolve(path)?;
    let payload = if volume.is_none() {
        crate::disguise::resolve_payload(path)?
    } else {
        None
    };
    let read_path = volume
        .as_ref()
        .map(|set| set.payload.as_path())
        .or(payload.as_deref())
        .unwrap_or(path);
    let sniffed = sniff::sniff(read_path)?
        .ok_or_else(|| EngineError::UnsupportedFormat("无法识别的归档内容".to_string()))?;
    sniff::ensure_supported(&sniffed)?;
    let actual_path = if sniffed.format == ArchiveFormat::Rar {
        crate::rar_backend::first_part(read_path)
    } else {
        read_path.to_path_buf()
    };
    let sfx_payload = if sniffed.format == ArchiveFormat::SevenZ && sniffed.payload_offset > 0 {
        Some(extract_7z_sfx_payload(
            actual_path.as_path(),
            sniffed.payload_offset,
        )?)
    } else {
        None
    };
    let read_path = sfx_payload.as_deref().unwrap_or(actual_path.as_path());
    match sniffed.format {
        ArchiveFormat::Zip | ArchiveFormat::Zipx => {
            crate::zip_backend::verify(read_path, password, sink)
        }
        ArchiveFormat::SevenZ => crate::sevenz_backend::verify(read_path, password, sink),
        ArchiveFormat::Tar
        | ArchiveFormat::TarGz
        | ArchiveFormat::TarBz2
        | ArchiveFormat::TarXz
        | ArchiveFormat::TarZst => crate::tar_backend::verify(read_path, sniffed.format, sink),
        ArchiveFormat::Gz | ArchiveFormat::Bz2 | ArchiveFormat::Xz | ArchiveFormat::Zst => {
            crate::tar_backend::verify_single(read_path, sniffed.format, sink)
        }
        ArchiveFormat::Rar => crate::rar_backend::verify(read_path, password, sink),
        ArchiveFormat::Iso => crate::iso_backend::verify(read_path, password, sink),
        ArchiveFormat::Lzh => crate::lzh_backend::verify(read_path, password, sink),
        ArchiveFormat::Zpaq => crate::zpaq_backend::verify(read_path, password, sink),
        ArchiveFormat::Wim | ArchiveFormat::Cab | ArchiveFormat::Arj | ArchiveFormat::Asar => {
            crate::external7z::verify(read_path, password, sink)
        }
        ArchiveFormat::Unknown => Err(EngineError::UnsupportedFormat(
            "无法识别的归档内容".to_string(),
        )),
    }
}

/// 该文件能否快速校验密码；不能时应直接请求用户输入，避免逐个试密造成长时间等待。
pub fn supports_quick_password_test(path: &Path) -> bool {
    let format = probe_archive(path)
        .ok()
        .flatten()
        .or_else(|| ArchiveFormat::from_extension(path));
    format.is_some_and(ArchiveFormat::supports_quick_password_test)
}

/// 快速判断密码是否正确；只解最小加密条目，避免整包试跑。
pub fn test_password(path: &Path, password: &str) -> Result<bool> {
    let volume = crate::volumes::resolve(path)?;
    let payload = if volume.is_none() {
        crate::disguise::resolve_payload(path)?
    } else {
        None
    };
    let read_path = volume
        .as_ref()
        .map(|set| set.payload.as_path())
        .or(payload.as_deref())
        .unwrap_or(path);
    let sniffed = sniff::sniff(read_path)?
        .ok_or_else(|| EngineError::UnsupportedFormat("无法识别的归档内容".to_string()))?;
    sniff::ensure_supported(&sniffed)?;
    let actual_path = if sniffed.format == ArchiveFormat::Rar {
        crate::rar_backend::first_part(read_path)
    } else {
        read_path.to_path_buf()
    };
    let sfx_payload = if sniffed.format == ArchiveFormat::SevenZ && sniffed.payload_offset > 0 {
        Some(extract_7z_sfx_payload(
            actual_path.as_path(),
            sniffed.payload_offset,
        )?)
    } else {
        None
    };
    let read_path = sfx_payload.as_deref().unwrap_or(actual_path.as_path());
    match sniffed.format {
        ArchiveFormat::Zip | ArchiveFormat::Zipx => {
            crate::zip_backend::test_password(read_path, password)
        }
        ArchiveFormat::SevenZ => crate::sevenz_backend::test_password(read_path, password),
        ArchiveFormat::Rar => crate::rar_backend::test_password(read_path, password),
        ArchiveFormat::Wim | ArchiveFormat::Cab | ArchiveFormat::Arj | ArchiveFormat::Asar => {
            crate::external7z::test_password(read_path, password)
        }
        // tar 系、ISO、LZH、ZPAQ 与单流格式没有密码保护。
        _ => Ok(true),
    }
}

/// 生成概要信息，供任务卡与状态栏使用。
pub fn archive_summary(info: &ArchiveInfo) -> String {
    format!(
        "{} · {} 项 · 原始 {} · 压缩后 {}",
        info.format.label(),
        info.file_count(),
        crate::util::format_size(info.total_size()),
        crate::util::format_size(info.total_compressed())
    )
}
