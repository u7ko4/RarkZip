use std::collections::HashSet;
use std::fs::{File, Metadata};
use std::io::{self, BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, Timelike};
use libdeflater::{CompressionLvl, Compressor};
use lzma_rust2::{LzmaOptions, LzmaWriter};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{AesMode, CompressionMethod, ZipWriter};

use crate::archive::verify_archive;
use crate::error::{EngineError, Result};
use crate::format::ArchiveFormat;
use crate::paths::{entry_segments, normalize_entry_name, to_extended};
use crate::progress::{check_cancelled, Phase, Progress, ProgressSink};
use crate::security::delete_to_recycle;
use crate::util::{
    copy_with_progress, format_ratio, format_size, system_time_to_unix, COPY_BUFFER_SIZE,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ZipAlgorithm {
    #[default]
    Deflate,
    Bzip2,
    Lzma,
    Zstd,
}

impl ZipAlgorithm {
    pub fn label(self) -> &'static str {
        match self {
            Self::Deflate => "Deflate",
            Self::Bzip2 => "BZip2",
            Self::Lzma => "LZMA",
            Self::Zstd => "Zstd",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CreateOptions {
    pub format: ArchiveFormat,
    pub level: u8,
    pub password: Option<String>,
    pub zip_algorithm: ZipAlgorithm,
    pub include_root: bool,
    pub exclude: Vec<String>,
    pub overwrite: bool,
    pub comment: Option<String>,
    pub test_after: bool,
    pub delete_sources: bool,
    /// 将 ZIP 载荷封装为自解压 EXE。
    pub sfx: bool,
}

impl Default for CreateOptions {
    fn default() -> Self {
        Self {
            format: ArchiveFormat::Zip,
            level: 5,
            password: None,
            zip_algorithm: ZipAlgorithm::Deflate,
            include_root: true,
            exclude: Vec::new(),
            overwrite: false,
            comment: None,
            test_after: false,
            delete_sources: false,
            sfx: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CreateOutcome {
    pub dest: PathBuf,
    pub format: ArchiveFormat,
    pub entries: usize,
    pub source_bytes: u64,
    pub compressed_bytes: u64,
    pub ratio: String,
    pub deleted_sources: usize,
}

#[derive(Debug, Clone)]
struct Collected {
    disk: PathBuf,
    name: String,
    is_dir: bool,
    size: u64,
    mode: u32,
    mtime: Option<i64>,
}

pub fn create_archive(
    sources: &[PathBuf],
    dest: &Path,
    options: &CreateOptions,
    sink: &dyn ProgressSink,
) -> Result<CreateOutcome> {
    if sources.is_empty() {
        return Err(EngineError::Other("没有选择要压缩的文件".to_string()));
    }
    if sources.iter().any(|source| !source.exists()) {
        let missing = sources
            .iter()
            .find(|source| !source.exists())
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        return Err(EngineError::NotFound(PathBuf::from(missing)));
    }
    if dest.exists() && !options.overwrite {
        return Err(EngineError::AlreadyExists(dest.to_path_buf()));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(to_extended(parent))?;
    }

    if options.sfx {
        if options.format != ArchiveFormat::Zip || options.password.is_some() {
            return Err(EngineError::Other("SFX 仅支持无密码 ZIP 载荷".to_string()));
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or_default();
        let temp = std::env::temp_dir()
            .join("NimZip")
            .join(format!("sfx-payload-{}-{stamp}.zip", std::process::id()));
        if let Some(parent) = temp.parent() {
            std::fs::create_dir_all(to_extended(parent))?;
        }
        let mut inner = options.clone();
        inner.sfx = false;
        inner.format = ArchiveFormat::Zip;
        inner.overwrite = true;
        inner.test_after = false;
        inner.delete_sources = false;
        let payload = create_archive(sources, &temp, &inner, sink)?;
        let result = crate::sfx::create_sfx(
            &temp,
            dest,
            &sfx_stub()?,
            &crate::sfx::SfxConfig {
                title: "NimZip 自解压".to_string(),
                ..Default::default()
            },
        );
        let _ = std::fs::remove_file(to_extended(&temp));
        result?;
        if options.test_after || options.delete_sources {
            verify_archive(dest, None, sink)?;
        }
        let deleted_sources = if options.delete_sources {
            delete_sources_after_create(dest, sources, sink)
        } else {
            0
        };
        let compressed_bytes = payload.compressed_bytes;
        return Ok(CreateOutcome {
            dest: dest.to_path_buf(),
            format: ArchiveFormat::Zip,
            entries: payload.entries,
            source_bytes: payload.source_bytes,
            compressed_bytes,
            ratio: format_ratio(compressed_bytes, payload.source_bytes),
            deleted_sources,
        });
    }

    let dest_canonical = std::fs::canonicalize(dest).unwrap_or_else(|_| dest.to_path_buf());
    if sources
        .iter()
        .any(|source| std::fs::canonicalize(source).is_ok_and(|path| path == dest_canonical))
    {
        return Err(EngineError::Other("输出归档不能与输入文件相同".to_string()));
    }

    check_cancelled(sink)?;
    let entries = collect_entries(sources, options)?;
    if entries.is_empty() {
        return Err(EngineError::Other(
            "排除规则过滤后没有可压缩的文件".to_string(),
        ));
    }
    validate_names(&entries)?;
    let total: u64 = entries
        .iter()
        .filter(|entry| !entry.is_dir)
        .map(|entry| entry.size)
        .sum();

    sink.update(&Progress::new(
        Phase::Compressing,
        0,
        total,
        format!("准备写入 {} 个条目", entries.len()),
    ));

    match options.format {
        ArchiveFormat::Zip | ArchiveFormat::Zipx => {
            create_zip(&entries, dest, options, sink, total)?
        }
        ArchiveFormat::SevenZ => create_7z(&entries, dest, options, sink, total)?,
        ArchiveFormat::Tar
        | ArchiveFormat::TarGz
        | ArchiveFormat::TarBz2
        | ArchiveFormat::TarXz
        | ArchiveFormat::TarZst => create_tar(&entries, dest, options, sink, total)?,
        ArchiveFormat::Gz | ArchiveFormat::Bz2 | ArchiveFormat::Xz | ArchiveFormat::Zst => {
            create_single(sources, dest, options, sink)?
        }
        ArchiveFormat::Lzh => create_lzh(&entries, dest, options, sink)?,
        ArchiveFormat::Iso => create_iso(&entries, dest, options, sink, total)?,
        ArchiveFormat::Zpaq => create_zpaq(&entries, dest, options, sink, total)?,
        other => {
            return Err(EngineError::UnsupportedFormat(format!(
                "{} 不支持创建",
                other.label()
            )))
        }
    }

    sink.update(&Progress::new(
        Phase::Compressing,
        total,
        total,
        "写入完成".to_string(),
    ));

    let compressed_bytes = std::fs::metadata(to_extended(dest))
        .map(|meta| meta.len())
        .unwrap_or(0);

    if options.test_after || options.delete_sources {
        sink.update(&Progress::new(
            Phase::Testing,
            0,
            1,
            "校验新建归档".to_string(),
        ));
        verify_archive(dest, options.password.as_deref(), sink)?;
    }

    let deleted_sources = if options.delete_sources {
        delete_sources_after_create(dest, sources, sink)
    } else {
        0
    };

    Ok(CreateOutcome {
        dest: dest.to_path_buf(),
        format: options.format,
        entries: entries.iter().filter(|entry| !entry.is_dir).count(),
        source_bytes: total,
        compressed_bytes,
        ratio: format_ratio(compressed_bytes, total),
        deleted_sources,
    })
}

fn sfx_stub() -> Result<PathBuf> {
    let current = std::env::current_exe()?;
    let stem = current
        .file_stem()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if stem == "nimzip" {
        return Ok(current);
    }
    if let Some(parent) = current.parent() {
        let candidate = parent.join("nimzip.exe");
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(EngineError::Other(
        "找不到 NimZip 主程序作为 SFX stub".to_string(),
    ))
}

fn delete_sources_after_create(dest: &Path, sources: &[PathBuf], sink: &dyn ProgressSink) -> usize {
    let dest_parent = dest
        .parent()
        .map(|parent| std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf()))
        .unwrap_or_default();
    let mut deleted = 0usize;
    for source in sources {
        let canonical = std::fs::canonicalize(source).unwrap_or_else(|_| source.clone());
        if dest_parent.starts_with(&canonical) {
            sink.warn(&format!(
                "归档位于源目录内，已跳过删除：{}",
                source.display()
            ));
            continue;
        }
        match delete_to_recycle(source) {
            Ok(()) => deleted += 1,
            Err(error) => sink.warn(&format!("删除源文件失败：{error}")),
        }
    }
    deleted
}

fn collect_entries(sources: &[PathBuf], options: &CreateOptions) -> Result<Vec<Collected>> {
    let matcher = ExcludeMatcher::new(&options.exclude)?;
    let mut entries = Vec::new();

    for source in sources {
        let metadata = std::fs::metadata(to_extended(source))?;
        if metadata.is_file() {
            let name = source
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .ok_or_else(|| EngineError::Other("无法取得文件名".to_string()))?;
            if matcher.is_match(&name) {
                continue;
            }
            entries.push(collected_from_metadata(
                source.to_path_buf(),
                name,
                false,
                &metadata,
            ));
            continue;
        }
        if !metadata.is_dir() {
            continue;
        }

        let root_name = source
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| "archive".to_string());
        if options.include_root {
            entries.push(collected_from_metadata(
                source.to_path_buf(),
                root_name.clone(),
                true,
                &metadata,
            ));
        }

        for item in WalkDir::new(source).follow_links(false).into_iter() {
            let item = item.map_err(|error| EngineError::Other(error.to_string()))?;
            if item.path() == source {
                continue;
            }
            let file_type = item.file_type();
            if file_type.is_symlink() {
                continue;
            }
            let relative = item
                .path()
                .strip_prefix(source)
                .map_err(|error| EngineError::Other(error.to_string()))?;
            let relative_name = normalize_entry_name(&relative.to_string_lossy());
            let name = if options.include_root && !root_name.is_empty() {
                format!("{root_name}/{relative_name}")
            } else {
                relative_name
            };
            if name.is_empty() || matcher.is_match(&name) {
                continue;
            }
            let meta = std::fs::metadata(to_extended(item.path()))?;
            entries.push(collected_from_metadata(
                item.path().to_path_buf(),
                name,
                file_type.is_dir(),
                &meta,
            ));
        }
    }

    Ok(entries)
}

fn collected_from_metadata(
    disk: PathBuf,
    name: String,
    is_dir: bool,
    meta: &Metadata,
) -> Collected {
    let size = if is_dir { 0 } else { meta.len() };
    let mtime = meta.modified().ok().and_then(system_time_to_unix);
    let mode = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            (meta.permissions().mode() & 0o777).max(0o400)
        }
        #[cfg(not(unix))]
        {
            if is_dir {
                0o755
            } else {
                0o644
            }
        }
    };
    Collected {
        disk,
        name: normalize_entry_name(&name),
        is_dir,
        size,
        mode,
        mtime,
    }
}

fn validate_names(entries: &[Collected]) -> Result<()> {
    let mut seen = HashSet::new();
    for entry in entries {
        entry_segments(&entry.name)?;
        if !seen.insert(entry.name.clone()) {
            return Err(EngineError::Other(format!(
                "归档内存在重名条目：{}",
                entry.name
            )));
        }
    }
    Ok(())
}

fn create_zip(
    entries: &[Collected],
    dest: &Path,
    options: &CreateOptions,
    sink: &dyn ProgressSink,
    total: u64,
) -> Result<()> {
    let level = options.level.min(9);
    let (method, zip_level) = zip_method(options, level);
    // zip crate 只实现 LZMA 解压，不提供写入；ZIP LZMA 使用本文件的 ZIP 记录写入器。
    if method == CompressionMethod::Lzma {
        if options.password.is_some() {
            return Err(EngineError::UnsupportedFormat(
                "ZIP LZMA 暂不支持密码保护".to_string(),
            ));
        }
        return create_zip_lzma(entries, dest, options, level, total, sink);
    }
    // Deflate 条目彼此独立：用 libdeflate 并行压缩各文件，再顺序写本地头/中央目录。
    // 加密、Zipx、SFX、超大档案仍走 zip crate，保证兼容性。
    if options.format == ArchiveFormat::Zip
        && options.password.is_none()
        && !options.sfx
        && method == CompressionMethod::Deflated
        && entries.len() <= u16::MAX as usize
        && total < 3 * 1024 * 1024 * 1024
        && entries
            .iter()
            .all(|entry| entry.size <= 512 * 1024 * 1024 && entry.name.len() <= u16::MAX as usize)
    {
        return create_zip_fast(entries, dest, options, level, total, sink);
    }
    let file = File::create(to_extended(dest))?;
    let mut writer = ZipWriter::new(file).set_auto_large_file();
    if let Some(comment) = &options.comment {
        writer.set_comment(comment.as_str());
    }
    let mut done = 0u64;

    for entry in entries {
        check_cancelled(sink)?;
        let base = zip_options(entry, method, zip_level);
        if entry.is_dir {
            writer.add_directory(&entry.name, base).map_err(map_zip)?;
            continue;
        }
        let mut source = ProgressReader::new(
            File::open(to_extended(&entry.disk))?,
            sink,
            &mut done,
            total,
            &entry.name,
        );
        if let Some(password) = &options.password {
            writer
                .start_file(
                    &entry.name,
                    base.with_aes_encryption(AesMode::Aes256, password),
                )
                .map_err(map_zip)?;
        } else {
            writer.start_file(&entry.name, base).map_err(map_zip)?;
        }
        io::copy(&mut source, &mut writer)?;
    }

    writer.finish().map_err(map_zip)?;
    Ok(())
}

struct FastZipEntry {
    name: Vec<u8>,
    method: u16,
    crc32: u32,
    compressed_size: u32,
    uncompressed_size: u32,
    offset: u32,
    time: u16,
    date: u16,
    external_attrs: u32,
}

struct PreparedZipFile {
    data: Vec<u8>,
    method: u16,
    crc32: u32,
    uncompressed_size: u32,
}

struct CountingWriter<W> {
    inner: W,
    written: u64,
}

impl<W: Write> CountingWriter<W> {
    fn new(inner: W) -> Self {
        Self { inner, written: 0 }
    }
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buffer)?;
        self.written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// 快速抽样判断数据是否基本不可压缩。随机数据、已压缩媒体直接 Store，
/// 避免为 100 MB 级随机文件白跑一遍 Deflate。
fn looks_incompressible(data: &[u8]) -> bool {
    const SAMPLE: usize = 64 * 1024;
    if data.len() < SAMPLE * 2 {
        return false;
    }
    let mut offsets = vec![0usize, data.len() / 2, data.len() - SAMPLE];
    offsets.sort_unstable();
    offsets.dedup();
    let mut compressor = Compressor::new(CompressionLvl::new(1).unwrap_or_default());
    let mut raw_total = 0usize;
    let mut compressed_total = 0usize;
    for offset in offsets {
        let slice = &data[offset..offset + SAMPLE];
        let bound = compressor.deflate_compress_bound(slice.len());
        let mut output = vec![0u8; bound];
        if let Ok(written) = compressor.deflate_compress(slice, &mut output) {
            raw_total += slice.len();
            compressed_total += written;
        }
    }
    raw_total > 0 && compressed_total * 100 >= raw_total * 98
}

fn prepare_zip_entry(
    entry: &Collected,
    level: u8,
    sink: &dyn ProgressSink,
) -> Result<PreparedZipFile> {
    check_cancelled(sink)?;
    if entry.is_dir {
        return Ok(PreparedZipFile {
            data: Vec::new(),
            method: 0,
            crc32: 0,
            uncompressed_size: 0,
        });
    }
    let data = std::fs::read(to_extended(&entry.disk))?;
    if data.len() > u32::MAX as usize {
        return Err(EngineError::Other("单个文件超过 ZIP32 限制".to_string()));
    }
    let crc32 = crc32fast::hash(&data);
    let uncompressed_size = data.len() as u32;
    if looks_incompressible(&data) {
        return Ok(PreparedZipFile {
            data,
            method: 0,
            crc32,
            uncompressed_size,
        });
    }
    let compression_level = CompressionLvl::new(level as i32).unwrap_or_default();
    let mut compressor = Compressor::new(compression_level);
    let bound = compressor.deflate_compress_bound(data.len());
    let mut compressed = vec![0u8; bound];
    let written = compressor
        .deflate_compress(&data, &mut compressed)
        .map_err(|error| EngineError::Other(format!("Deflate 压缩失败：{error:?}")))?;
    compressed.truncate(written);
    if compressed.len() >= data.len() {
        Ok(PreparedZipFile {
            data,
            method: 0,
            crc32,
            uncompressed_size,
        })
    } else {
        Ok(PreparedZipFile {
            data: compressed,
            method: 8,
            crc32,
            uncompressed_size,
        })
    }
}

fn fast_entry_name(entry: &Collected) -> Vec<u8> {
    let mut name = entry.name.clone();
    if entry.is_dir && !name.ends_with('/') {
        name.push('/');
    }
    name.into_bytes()
}

fn fast_external_attrs(entry: &Collected) -> u32 {
    let mode = if entry.mode == 0 {
        if entry.is_dir {
            0o755
        } else {
            0o644
        }
    } else {
        entry.mode & 0xffff
    };
    (mode << 16) | if entry.is_dir { 0x10 } else { 0 }
}

fn dos_time_date(timestamp: Option<i64>) -> (u16, u16) {
    let Some(timestamp) = timestamp else {
        return (0, 0x21);
    };
    let Some(value) = chrono::DateTime::from_timestamp(timestamp, 0) else {
        return (0, 0x21);
    };
    let local = value.with_timezone(&Local);
    let year = local.year().clamp(1980, 2107) as u16;
    let date = ((year - 1980) << 9) | ((local.month() as u16) << 5) | local.day() as u16;
    let time = ((local.hour() as u16) << 11)
        | ((local.minute() as u16) << 5)
        | (local.second() as u16 / 2);
    (time, date)
}

fn write_u16<W: Write>(writer: &mut W, value: u16) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn write_u32<W: Write>(writer: &mut W, value: u32) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

#[allow(clippy::too_many_arguments)]
fn write_local_header<W: Write>(
    writer: &mut W,
    name: &[u8],
    method: u16,
    time: u16,
    date: u16,
    crc32: u32,
    compressed_size: u32,
    uncompressed_size: u32,
    version_needed: u16,
) -> io::Result<()> {
    write_u32(writer, 0x0403_4b50)?;
    write_u16(writer, version_needed)?;
    write_u16(writer, 0x0800)?;
    write_u16(writer, method)?;
    write_u16(writer, time)?;
    write_u16(writer, date)?;
    write_u32(writer, crc32)?;
    write_u32(writer, compressed_size)?;
    write_u32(writer, uncompressed_size)?;
    write_u16(writer, name.len() as u16)?;
    write_u16(writer, 0)?;
    writer.write_all(name)
}

fn write_central_header<W: Write>(
    writer: &mut W,
    entry: &FastZipEntry,
    version_needed: u16,
) -> io::Result<()> {
    write_u32(writer, 0x0201_4b50)?;
    write_u16(writer, 0x0314)?;
    write_u16(writer, version_needed)?;
    write_u16(writer, 0x0800)?;
    write_u16(writer, entry.method)?;
    write_u16(writer, entry.time)?;
    write_u16(writer, entry.date)?;
    write_u32(writer, entry.crc32)?;
    write_u32(writer, entry.compressed_size)?;
    write_u32(writer, entry.uncompressed_size)?;
    write_u16(writer, entry.name.len() as u16)?;
    write_u16(writer, 0)?;
    write_u16(writer, 0)?;
    write_u16(writer, 0)?;
    write_u16(writer, 0)?;
    write_u32(writer, entry.external_attrs)?;
    write_u32(writer, entry.offset)?;
    writer.write_all(&entry.name)
}

fn write_end_of_central_directory<W: Write>(
    writer: &mut W,
    count: u16,
    central_size: u32,
    central_offset: u32,
    comment: &[u8],
) -> io::Result<()> {
    let comment = &comment[..comment.len().min(u16::MAX as usize)];
    write_u32(writer, 0x0605_4b50)?;
    write_u16(writer, 0)?;
    write_u16(writer, 0)?;
    write_u16(writer, count)?;
    write_u16(writer, count)?;
    write_u32(writer, central_size)?;
    write_u32(writer, central_offset)?;
    write_u16(writer, comment.len() as u16)?;
    writer.write_all(comment)
}

fn create_zip_fast(
    entries: &[Collected],
    dest: &Path,
    options: &CreateOptions,
    level: u8,
    total: u64,
    sink: &dyn ProgressSink,
) -> Result<()> {
    let file = File::create(to_extended(dest))?;
    let mut writer = CountingWriter::new(BufWriter::with_capacity(1024 * 1024, file));
    let workers = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1)
        .clamp(1, 16);
    const CHUNK_BUDGET: u64 = 256 * 1024 * 1024;
    let mut central = Vec::with_capacity(entries.len());
    let mut done = 0u64;
    let mut index = 0usize;
    while index < entries.len() {
        let start = index;
        let mut chunk_bytes = 0u64;
        while index < entries.len()
            && index - start < workers
            && (index == start || chunk_bytes + entries[index].size <= CHUNK_BUDGET)
        {
            chunk_bytes += entries[index].size;
            index += 1;
        }
        let chunk = &entries[start..index];
        let prepared = std::thread::scope(|scope| -> Result<Vec<PreparedZipFile>> {
            let mut handles = Vec::with_capacity(chunk.len());
            for entry in chunk {
                handles.push(scope.spawn(move || prepare_zip_entry(entry, level, sink)));
            }
            let mut prepared = Vec::with_capacity(handles.len());
            for handle in handles {
                match handle.join() {
                    Ok(Ok(file)) => prepared.push(file),
                    Ok(Err(error)) => return Err(error),
                    Err(_) => return Err(EngineError::Other("ZIP 压缩线程异常".to_string())),
                }
            }
            Ok(prepared)
        })?;
        for (entry, prepared) in chunk.iter().zip(prepared) {
            check_cancelled(sink)?;
            let name = fast_entry_name(entry);
            let (time, date) = dos_time_date(entry.mtime);
            let external_attrs = fast_external_attrs(entry);
            let offset = writer.written as u32;
            write_local_header(
                &mut writer,
                &name,
                prepared.method,
                time,
                date,
                prepared.crc32,
                prepared.data.len() as u32,
                prepared.uncompressed_size,
                20,
            )?;
            writer.write_all(&prepared.data)?;
            central.push(FastZipEntry {
                name,
                method: prepared.method,
                crc32: prepared.crc32,
                compressed_size: prepared.data.len() as u32,
                uncompressed_size: prepared.uncompressed_size,
                offset,
                time,
                date,
                external_attrs,
            });
            if !entry.is_dir {
                done += prepared.uncompressed_size as u64;
                sink.update(&Progress::new(
                    Phase::Compressing,
                    done,
                    total,
                    entry.name.clone(),
                ));
            }
        }
    }
    let central_offset = writer.written as u32;
    for entry in &central {
        write_central_header(&mut writer, entry, 20)?;
    }
    let central_size = writer.written as u32 - central_offset;
    write_end_of_central_directory(
        &mut writer,
        central.len() as u16,
        central_size,
        central_offset,
        options.comment.as_deref().unwrap_or("").as_bytes(),
    )?;
    writer.flush()?;
    Ok(())
}

/// 写入 ZIP 的 LZMA 条目（方法 14）。
///
/// `zip` crate 只支持读取 LZMA，压缩方法需要自己写本地头/中央目录；每个条目
/// 先压缩到临时文件，避免把大文件整体读入内存。
fn create_zip_lzma(
    entries: &[Collected],
    dest: &Path,
    options: &CreateOptions,
    level: u8,
    total: u64,
    sink: &dyn ProgressSink,
) -> Result<()> {
    const METHOD_LZMA: u16 = 14;
    const VERSION_NEEDED: u16 = 63;
    if entries.len() > u16::MAX as usize {
        return Err(EngineError::Other("ZIP 条目数超过 65535".to_string()));
    }
    let file = File::create(to_extended(dest))?;
    let mut writer = CountingWriter::new(BufWriter::with_capacity(1024 * 1024, file));
    let mut central = Vec::with_capacity(entries.len());
    let mut done = 0u64;

    for entry in entries {
        check_cancelled(sink)?;
        let name = fast_entry_name(entry);
        let (time, date) = dos_time_date(entry.mtime);
        let external_attrs = fast_external_attrs(entry);
        let offset = writer.written;
        if offset > u32::MAX as u64 {
            return Err(EngineError::Other(
                "ZIP LZMA 暂不支持超过 4 GB 的归档".to_string(),
            ));
        }

        let mut temp = tempfile::tempfile()?;
        let (method, crc32, compressed_size, uncompressed_size) = if entry.is_dir {
            (0, 0, 0, 0)
        } else {
            let lzma_options = LzmaOptions::with_preset(level as u32);
            let props = lzma_options.get_props();
            let dict_size = lzma_options.dict_size;
            // ZIP LZMA 载荷：SDK 版本(2) + 属性长度(2) + 属性(5)。
            temp.write_all(&[0x1A, 0x05])?;
            temp.write_all(&5u16.to_le_bytes())?;
            temp.write_all(&[props])?;
            temp.write_all(&dict_size.to_le_bytes())?;

            let mut encoder = LzmaWriter::new_no_header(temp, &lzma_options, false)
                .map_err(|error| EngineError::Other(format!("LZMA 压缩失败：{error:?}")))?;
            let mut input = File::open(to_extended(&entry.disk))?;
            let mut buffer = vec![0u8; COPY_BUFFER_SIZE];
            let mut hasher = crc32fast::Hasher::new();
            let mut entry_done = 0u64;
            loop {
                check_cancelled(sink)?;
                let read = input.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
                encoder
                    .write_all(&buffer[..read])
                    .map_err(|error| EngineError::Other(format!("LZMA 写入失败：{error}")))?;
                entry_done += read as u64;
                sink.update(&Progress::new(
                    Phase::Compressing,
                    done + entry_done,
                    total,
                    entry.name.clone(),
                ));
            }
            temp = encoder
                .finish()
                .map_err(|error| EngineError::Other(format!("完成 LZMA 失败：{error:?}")))?;
            let compressed_size = temp.metadata()?.len();
            (METHOD_LZMA, hasher.finalize(), compressed_size, entry_done)
        };
        if compressed_size > u32::MAX as u64 || uncompressed_size > u32::MAX as u64 {
            return Err(EngineError::Other(
                "ZIP LZMA 单个文件超过 4 GB 限制".to_string(),
            ));
        }
        write_local_header(
            &mut writer,
            &name,
            method,
            time,
            date,
            crc32,
            compressed_size as u32,
            uncompressed_size as u32,
            VERSION_NEEDED,
        )?;
        if !entry.is_dir {
            temp.rewind()?;
            io::copy(&mut temp, &mut writer)?;
            done += uncompressed_size;
        }
        central.push(FastZipEntry {
            name,
            method,
            crc32,
            compressed_size: compressed_size as u32,
            uncompressed_size: uncompressed_size as u32,
            offset: offset as u32,
            time,
            date,
            external_attrs,
        });
    }

    if writer.written > u32::MAX as u64 {
        return Err(EngineError::Other(
            "ZIP LZMA 暂不支持超过 4 GB 的归档".to_string(),
        ));
    }
    let central_offset = writer.written as u32;
    for entry in &central {
        write_central_header(&mut writer, entry, VERSION_NEEDED)?;
    }
    let central_size = writer.written as u32 - central_offset;
    write_end_of_central_directory(
        &mut writer,
        central.len() as u16,
        central_size,
        central_offset,
        options.comment.as_deref().unwrap_or("").as_bytes(),
    )?;
    writer.flush()?;
    Ok(())
}

fn zip_options(
    entry: &Collected,
    method: CompressionMethod,
    level: Option<i64>,
) -> SimpleFileOptions {
    let mut options = SimpleFileOptions::default()
        .compression_method(method)
        .compression_level(level)
        .unix_permissions(entry.mode);
    if let Some(timestamp) = entry.mtime {
        options = options.last_modified_time(zip_datetime(timestamp));
    }
    options
}

fn zip_method(options: &CreateOptions, level: u8) -> (CompressionMethod, Option<i64>) {
    if level == 0 {
        return (CompressionMethod::Stored, None);
    }
    let algorithm = if options.format == ArchiveFormat::Zipx
        && options.zip_algorithm == ZipAlgorithm::Deflate
    {
        // ZIPX 需要非 Deflate 方法；未显式选择时使用 Zstd。
        ZipAlgorithm::Zstd
    } else {
        options.zip_algorithm
    };
    match algorithm {
        ZipAlgorithm::Deflate => (CompressionMethod::Deflated, Some(level as i64)),
        ZipAlgorithm::Bzip2 => (CompressionMethod::Bzip2, Some(level as i64)),
        ZipAlgorithm::Zstd => (
            CompressionMethod::Zstd,
            Some((level as i64 * 2).clamp(1, 22)),
        ),
        ZipAlgorithm::Lzma => (CompressionMethod::Lzma, None),
    }
}

fn zip_datetime(timestamp: i64) -> zip::DateTime {
    let Some(value) = chrono::DateTime::from_timestamp(timestamp, 0) else {
        return zip::DateTime::default();
    };
    let local = value.with_timezone(&Local);
    zip::DateTime::from_date_and_time(
        local.year() as u16,
        local.month() as u8,
        local.day() as u8,
        local.hour() as u8,
        local.minute() as u8,
        local.second() as u8,
    )
    .unwrap_or_default()
}

fn create_7z(
    entries: &[Collected],
    dest: &Path,
    options: &CreateOptions,
    sink: &dyn ProgressSink,
    total: u64,
) -> Result<()> {
    let mut writer = sevenz_rust2::ArchiveWriter::create(to_extended(dest))
        .map_err(|error| EngineError::Other(format!("创建 7z 失败：{error:?}")))?;
    // sevenz-rust2 默认单线程 LZMA2，启用多线程可明显提升大归档压缩速度。
    // 按 CPU 数启用多线程分块压缩；块大小取 16 MiB（会被字典大小下限钳制）。
    let threads = std::thread::available_parallelism()
        .map(|value| value.get() as u32)
        .unwrap_or(1)
        .clamp(1, 16);
    let level = options.level.min(9) as u32;
    let lzma = sevenz_rust2::encoder_options::Lzma2Options::from_level_mt(
        level,
        threads,
        16 * 1024 * 1024,
    );
    let mut methods = Vec::new();
    if let Some(password) = &options.password {
        writer.set_encrypt_header(true);
        methods.push(
            sevenz_rust2::encoder_options::AesEncoderOptions::new(sevenz_rust2::Password::new(
                password,
            ))
            .into(),
        );
    }
    methods.push(lzma.into());
    writer.set_content_methods(methods);
    let mut done = 0u64;

    for entry in entries {
        check_cancelled(sink)?;
        let archive_entry =
            sevenz_rust2::ArchiveEntry::from_path(to_extended(&entry.disk), entry.name.clone());
        if entry.is_dir {
            writer
                .push_archive_entry::<File>(archive_entry, None)
                .map_err(|error| EngineError::Other(format!("写入 7z 失败：{error:?}")))?;
        } else {
            let source = ProgressReader::new(
                File::open(to_extended(&entry.disk))?,
                sink,
                &mut done,
                total,
                &entry.name,
            );
            writer
                .push_archive_entry(archive_entry, Some(source))
                .map_err(|error| EngineError::Other(format!("写入 7z 失败：{error:?}")))?;
        }
    }
    writer
        .finish()
        .map_err(|error| EngineError::Io(io::Error::other(format!("完成 7z 失败：{error}"))))?;
    Ok(())
}

fn create_tar(
    entries: &[Collected],
    dest: &Path,
    options: &CreateOptions,
    sink: &dyn ProgressSink,
    total: u64,
) -> Result<()> {
    let level = options.level.min(9);
    let file = File::create(to_extended(dest))?;
    match options.format {
        ArchiveFormat::Tar => {
            let mut builder = tar::Builder::new(file);
            append_tar_entries(&mut builder, entries, sink, total)?;
            builder.finish()?;
        }
        ArchiveFormat::TarGz => {
            let encoder =
                flate2::write::GzEncoder::new(file, flate2::Compression::new(level as u32));
            let mut builder = tar::Builder::new(encoder);
            append_tar_entries(&mut builder, entries, sink, total)?;
            let encoder = builder.into_inner()?;
            encoder.finish()?;
        }
        ArchiveFormat::TarBz2 => {
            let encoder = bzip2::write::BzEncoder::new(file, bzip2::Compression::new(level as u32));
            let mut builder = tar::Builder::new(encoder);
            append_tar_entries(&mut builder, entries, sink, total)?;
            let encoder = builder.into_inner()?;
            encoder.finish()?;
        }
        ArchiveFormat::TarXz => {
            let encoder = xz2::write::XzEncoder::new(file, level as u32);
            let mut builder = tar::Builder::new(encoder);
            append_tar_entries(&mut builder, entries, sink, total)?;
            let encoder = builder.into_inner()?;
            encoder.finish()?;
        }
        ArchiveFormat::TarZst => {
            let encoder = zstd::stream::write::Encoder::new(file, level as i32)?;
            let mut builder = tar::Builder::new(encoder);
            append_tar_entries(&mut builder, entries, sink, total)?;
            let encoder = builder.into_inner()?;
            encoder.finish()?;
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn append_tar_entries<W: Write>(
    builder: &mut tar::Builder<W>,
    entries: &[Collected],
    sink: &dyn ProgressSink,
    total: u64,
) -> Result<()> {
    let mut done = 0u64;
    for entry in entries {
        check_cancelled(sink)?;
        if entry.is_dir {
            builder.append_dir(&entry.name, &entry.disk)?;
            continue;
        }
        let metadata = std::fs::metadata(to_extended(&entry.disk))?;
        let mut header = tar::Header::new_gnu();
        header.set_metadata(&metadata);
        header.set_size(entry.size);
        header.set_mode(entry.mode);
        header.set_entry_type(tar::EntryType::Regular);
        if let Some(timestamp) = entry.mtime {
            header.set_mtime(timestamp as u64);
        }
        header.set_cksum();
        let source = ProgressReader::new(
            File::open(to_extended(&entry.disk))?,
            sink,
            &mut done,
            total,
            &entry.name,
        );
        builder.append_data(&mut header, &entry.name, source)?;
    }
    Ok(())
}

/// 把待压缩条目按归档内路径落到临时目录，供只接受目录输入的写入器使用。
/// 优先硬链接，跨卷时退回复制。
fn stage_tree(entries: &[Collected]) -> Result<(TempDir, PathBuf)> {
    let temp = tempfile::Builder::new()
        .prefix("nimzip-stage-")
        .tempdir()
        .map_err(|error| EngineError::Other(format!("无法创建临时目录：{error}")))?;
    let root = temp.path().to_path_buf();
    for entry in entries {
        let target = root.join(&entry.name);
        if entry.is_dir {
            std::fs::create_dir_all(to_extended(&target))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(to_extended(parent))?;
        }
        if std::fs::hard_link(to_extended(&entry.disk), to_extended(&target)).is_err() {
            std::fs::copy(to_extended(&entry.disk), to_extended(&target))?;
        }
    }
    Ok((temp, root))
}

fn create_lzh(
    entries: &[Collected],
    dest: &Path,
    options: &CreateOptions,
    sink: &dyn ProgressSink,
) -> Result<()> {
    use oxiarc_archive::lzh::{LzhCompressionLevel, LzhWriter};

    if options.password.is_some() {
        return Err(EngineError::UnsupportedFormat(
            "LZH 不支持密码保护".to_string(),
        ));
    }
    let map =
        |error: Box<dyn std::error::Error>| EngineError::Other(format!("写入 LZH 失败：{error}"));
    let compression = if options.level == 0 {
        LzhCompressionLevel::Store
    } else {
        LzhCompressionLevel::Lh5
    };
    let file = File::create(to_extended(dest))?;
    let mut writer = LzhWriter::new(file).with_header_level(2);
    for entry in entries {
        check_cancelled(sink)?;
        if entry.is_dir {
            writer
                .add_directory(&entry.name)
                .map_err(|error| map(Box::new(error)))?;
            continue;
        }
        let mut data = Vec::with_capacity(entry.size as usize);
        File::open(to_extended(&entry.disk))?.read_to_end(&mut data)?;
        writer
            .add_file_with_options(&entry.name, &data, compression)
            .map_err(|error| map(Box::new(error)))?;
        sink.update(&Progress::new(
            Phase::Compressing,
            entry.size,
            0,
            entry.name.clone(),
        ));
    }
    writer.finish().map_err(|error| map(Box::new(error)))?;
    Ok(())
}

fn create_iso(
    entries: &[Collected],
    dest: &Path,
    options: &CreateOptions,
    sink: &dyn ProgressSink,
    total: u64,
) -> Result<()> {
    use libmkiso::{FilenamePolicy, IsoOptions, IsoTimestamp};

    if options.password.is_some() {
        return Err(EngineError::UnsupportedFormat(
            "ISO 不支持密码保护".to_string(),
        ));
    }
    let (_staging, root) = stage_tree(entries)?;
    if dest.exists() {
        std::fs::remove_file(to_extended(dest))?;
    }
    let now = chrono::Utc::now();
    let label = iso_volume_label(dest);
    let iso_options = IsoOptions {
        volume_label: label,
        timestamp: IsoTimestamp {
            year: now.year() as u16,
            month: now.month() as u8,
            day: now.day() as u8,
            hour: now.hour() as u8,
            minute: now.minute() as u8,
            second: now.second() as u8,
        },
        // Joliet 保留 Unicode/长文件名；主命名空间为兼容性生成别名。
        joliet: true,
        filename_policy: FilenamePolicy::Mangle,
        ..IsoOptions::default()
    };
    check_cancelled(sink)?;
    libmkiso::write_iso9660_with_options(&root, dest, &iso_options)
        .map_err(|error| EngineError::Other(format!("写入 ISO 失败：{error}")))?;
    sink.update(&Progress::new(
        Phase::Compressing,
        total,
        total,
        "写入 ISO 完成".to_string(),
    ));
    Ok(())
}

fn iso_volume_label(dest: &Path) -> String {
    let stem = dest
        .file_stem()
        .map(|value| value.to_string_lossy().to_ascii_uppercase())
        .unwrap_or_else(|| "NIMZIP".to_string());
    let mut label: String = stem
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .take(32)
        .collect();
    if label.is_empty() {
        label.push_str("NIMZIP");
    }
    label
}

fn create_zpaq(
    entries: &[Collected],
    dest: &Path,
    options: &CreateOptions,
    sink: &dyn ProgressSink,
    total: u64,
) -> Result<()> {
    if options.password.is_some() {
        return Err(EngineError::UnsupportedFormat(
            "ZPAQ 不支持密码保护".to_string(),
        ));
    }
    let (_staging, root) = stage_tree(entries)?;
    let inputs: Vec<String> = std::fs::read_dir(to_extended(&root))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    if inputs.is_empty() {
        return Err(EngineError::Other("没有可压缩的文件".to_string()));
    }
    let input_refs: Vec<&str> = inputs.iter().map(String::as_str).collect();
    let absolute = std::path::absolute(dest).unwrap_or_else(|_| dest.to_path_buf());
    let dest_text = absolute.to_string_lossy().to_string();
    if dest.exists() {
        std::fs::remove_file(to_extended(dest))?;
    }
    let method = match options.level {
        0 => "0".to_string(),
        1..=5 => options.level.to_string(),
        _ => "5".to_string(),
    };
    let threads = std::thread::available_parallelism()
        .map(|value| value.get().min(8))
        .unwrap_or(1);
    sink.update(&Progress::new(
        Phase::Compressing,
        0,
        total,
        "写入 ZPAQ 归档".to_string(),
    ));
    let _guard = crate::zpaq_backend::ZPAQ_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let previous = std::env::current_dir()?;
    std::env::set_current_dir(to_extended(&root))?;
    let result = zpaq_rs::zpaq_add(&dest_text, &input_refs, &method, threads);
    let _ = std::env::set_current_dir(&previous);
    result.map_err(|error| EngineError::Other(format!("写入 ZPAQ 失败：{error}")))?;
    sink.update(&Progress::new(
        Phase::Compressing,
        total,
        total,
        "写入完成".to_string(),
    ));
    Ok(())
}

fn create_single(
    sources: &[PathBuf],
    dest: &Path,
    options: &CreateOptions,
    sink: &dyn ProgressSink,
) -> Result<()> {
    if sources.len() != 1 {
        return Err(EngineError::Other(
            "单文件压缩格式一次只能选择一个文件".to_string(),
        ));
    }
    let source = &sources[0];
    if !source.is_file() {
        return Err(EngineError::Other("单文件压缩格式不支持文件夹".to_string()));
    }
    if options.password.is_some() {
        return Err(EngineError::UnsupportedFormat(
            "该格式不支持密码保护".to_string(),
        ));
    }
    let level = options.level.min(9);
    let file = File::create(to_extended(dest))?;
    let total = std::fs::metadata(to_extended(source))
        .map(|meta| meta.len())
        .unwrap_or(0);
    let name = source
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut done = 0u64;
    let mut input = File::open(to_extended(source))?;

    match options.format {
        ArchiveFormat::Gz => {
            let mut encoder =
                flate2::write::GzEncoder::new(file, flate2::Compression::new(level as u32));
            copy_with_progress(
                &mut input,
                &mut encoder,
                sink,
                Phase::Compressing,
                &mut done,
                total,
                &name,
            )?;
            encoder.finish()?;
        }
        ArchiveFormat::Bz2 => {
            let mut encoder =
                bzip2::write::BzEncoder::new(file, bzip2::Compression::new(level as u32));
            copy_with_progress(
                &mut input,
                &mut encoder,
                sink,
                Phase::Compressing,
                &mut done,
                total,
                &name,
            )?;
            encoder.finish()?;
        }
        ArchiveFormat::Xz => {
            let mut encoder = xz2::write::XzEncoder::new(file, level as u32);
            copy_with_progress(
                &mut input,
                &mut encoder,
                sink,
                Phase::Compressing,
                &mut done,
                total,
                &name,
            )?;
            encoder.finish()?;
        }
        ArchiveFormat::Zst => {
            let mut encoder = zstd::stream::write::Encoder::new(file, level as i32)?;
            copy_with_progress(
                &mut input,
                &mut encoder,
                sink,
                Phase::Compressing,
                &mut done,
                total,
                &name,
            )?;
            encoder.finish()?;
        }
        _ => unreachable!(),
    }
    Ok(())
}

pub fn create_summary(outcome: &CreateOutcome) -> String {
    format!(
        "{} · {} 个文件 · {} → {} ({})",
        outcome.format.label(),
        outcome.entries,
        format_size(outcome.source_bytes),
        format_size(outcome.compressed_bytes),
        outcome.ratio
    )
}

fn map_zip(error: zip::result::ZipError) -> EngineError {
    match error {
        zip::result::ZipError::Io(error) => EngineError::Io(error),
        zip::result::ZipError::UnsupportedArchive(message) => {
            EngineError::UnsupportedFormat(message.to_string())
        }
        zip::result::ZipError::InvalidArchive(message) => {
            EngineError::DataError(message.to_string())
        }
        other => EngineError::Other(other.to_string()),
    }
}

struct ProgressReader<'a, R> {
    inner: R,
    sink: &'a dyn ProgressSink,
    done: &'a mut u64,
    total: u64,
    name: String,
}

impl<'a, R> ProgressReader<'a, R> {
    fn new(
        inner: R,
        sink: &'a dyn ProgressSink,
        done: &'a mut u64,
        total: u64,
        name: &str,
    ) -> Self {
        Self {
            inner,
            sink,
            done,
            total,
            name: name.to_string(),
        }
    }
}

impl<R: Read> Read for ProgressReader<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        if read > 0 {
            *self.done += read as u64;
            self.sink.update(&Progress::new(
                Phase::Compressing,
                *self.done,
                self.total,
                self.name.clone(),
            ));
        }
        Ok(read)
    }
}

struct ExcludeMatcher {
    set: globset::GlobSet,
}

impl ExcludeMatcher {
    fn new(patterns: &[String]) -> Result<Self> {
        let mut builder = globset::GlobSetBuilder::new();
        for raw in patterns {
            for pattern in raw.split([';', ',', '\n', '\r']) {
                let pattern = pattern.trim();
                if pattern.is_empty() {
                    continue;
                }
                let glob = globset::GlobBuilder::new(pattern)
                    .case_insensitive(true)
                    .literal_separator(false)
                    .build()
                    .map_err(|error| EngineError::Other(format!("排除规则无效：{error}")))?;
                builder.add(glob);
            }
        }
        let set = builder
            .build()
            .map_err(|error| EngineError::Other(format!("排除规则无效：{error}")))?;
        Ok(Self { set })
    }

    fn is_match(&self, name: &str) -> bool {
        let normalized = name.replace('\\', "/");
        self.set.is_match(&normalized)
            || self
                .set
                .is_match(normalized.rsplit('/').next().unwrap_or(&normalized))
    }
}

#[cfg(test)]
mod perf_tests {
    use super::*;

    #[test]
    fn sampled_text_is_compressible() {
        let data = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. "
            .repeat(20_000)
            .into_bytes();
        assert!(!looks_incompressible(&data));
    }

    #[test]
    fn sampled_random_is_incompressible() {
        let mut state = 0x1234_5678u32;
        let mut data = Vec::with_capacity(200_000);
        for _ in 0..200_000 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            data.push((state & 0xff) as u8);
        }
        assert!(looks_incompressible(&data));
    }
}
