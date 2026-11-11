//! 自解压 EXE（SFX）封装与运行时检测。
//!
//! 结构：`[stub exe][zip payload][config json][payload_len u64][config_len u32][magic 8]`
//! stub 就是 NimZip 本体；运行时发现尾部标记后，把内嵌 ZIP 解到配置目录。

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{EngineError, Result};
use crate::paths::to_extended;
use crate::progress::ProgressSink;

pub const SFX_MAGIC: &[u8; 8] = b"NIMZSFX1";
const FOOTER_SIZE: u64 = 20;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SfxConfig {
    pub title: String,
    pub prompt: String,
    pub destination: String,
    pub run_after: String,
    pub auto_close: bool,
}

#[derive(Debug, Clone)]
pub struct SfxInfo {
    pub payload_offset: u64,
    pub payload_len: u64,
    pub config: SfxConfig,
}

pub fn create_sfx(payload: &Path, output: &Path, stub: &Path, config: &SfxConfig) -> Result<u64> {
    let payload_len = std::fs::metadata(to_extended(payload))?.len();
    let config_bytes =
        serde_json::to_vec(config).map_err(|error| EngineError::Other(error.to_string()))?;
    let mut stub_file = File::open(to_extended(stub))?;
    // 若 stub 本身带 SFX 尾（误把 SFX 再封装），只复制到 payload 起点。
    let stub_len = detect_sfx_from_file(&mut stub_file)?
        .map(|info| info.payload_offset)
        .unwrap_or_else(|| stub_file.metadata().map(|meta| meta.len()).unwrap_or(0));
    stub_file.seek(SeekFrom::Start(0))?;

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(to_extended(parent))?;
    }
    let mut out = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(to_extended(output))?;
    std::io::copy(&mut stub_file.take(stub_len), &mut out)?;
    let mut payload_file = File::open(to_extended(payload))?;
    std::io::copy(&mut payload_file, &mut out)?;
    // 标准 SFX 的 ZIP 偏移以整个 EXE 起点为基准：把内嵌 ZIP 的
    // 中央目录与条目偏移前移 stub 长度，外部解压工具才能正确定位。
    adjust_zip_offsets(&mut out, stub_len, payload_len, stub_len as i64)?;
    out.seek(SeekFrom::End(0))?;
    out.write_all(&config_bytes)?;
    out.write_all(&payload_len.to_le_bytes())?;
    out.write_all(&(config_bytes.len() as u32).to_le_bytes())?;
    out.write_all(SFX_MAGIC)?;
    out.flush()?;
    Ok(stub_len + payload_len + config_bytes.len() as u64 + FOOTER_SIZE)
}

/// 平移 ZIP 的中央目录、ZIP64 定位器与条目本地头偏移。
///
/// `archive_start` 是 ZIP 数据在文件中的起点，`delta` 是期望的偏移变化量
/// （封装 SFX 时为正、从 SFX 剥离载荷时为负）。
fn adjust_zip_offsets(
    file: &mut File,
    archive_start: u64,
    archive_len: u64,
    delta: i64,
) -> Result<()> {
    if archive_len < 22 {
        return Err(EngineError::DataError("ZIP 载荷过小".to_string()));
    }
    let tail_len = archive_len.min(22 + u16::MAX as u64);
    let mut tail = vec![0u8; tail_len as usize];
    read_at(file, archive_start + archive_len - tail_len, &mut tail)?;

    let eocd_rel = (0..=tail.len() - 22)
        .rev()
        .find(|&index| {
            let comment_len = u16::from_le_bytes([tail[index + 20], tail[index + 21]]) as usize;
            u32::from_le_bytes(tail[index..index + 4].try_into().unwrap_or_default())
                == EOCD_SIGNATURE
                && index + 22 + comment_len <= tail.len()
        })
        .ok_or_else(|| EngineError::DataError("ZIP 结束记录缺失".to_string()))?;
    let eocd_offset = archive_start + archive_len - tail_len + eocd_rel as u64;
    let eocd = &tail[eocd_rel..];

    let mut cd_offset = u32::from_le_bytes(eocd[16..20].try_into().unwrap_or_default()) as u64;
    let mut entry_count = u16::from_le_bytes(eocd[10..12].try_into().unwrap_or_default()) as u64;
    let mut cd_size = u32::from_le_bytes(eocd[12..16].try_into().unwrap_or_default()) as u64;
    let eocd_cd_field = u32::from_le_bytes(eocd[16..20].try_into().unwrap_or_default());
    let mut central_start = eocd_offset.saturating_sub(cd_size);

    if eocd_cd_field == u32::MAX || entry_count == u16::MAX as u64 || cd_size == u32::MAX as u64 {
        let locator_offset = eocd_offset
            .checked_sub(20)
            .ok_or_else(|| EngineError::DataError("ZIP64 定位器缺失".to_string()))?;
        let mut locator = [0u8; 20];
        read_at(file, locator_offset, &mut locator)?;
        if u32::from_le_bytes(locator[0..4].try_into().unwrap_or_default())
            != ZIP64_LOCATOR_SIGNATURE
        {
            return Err(EngineError::DataError("ZIP64 定位器损坏".to_string()));
        }
        let zip64_offset = u64::from_le_bytes(locator[8..16].try_into().unwrap_or_default());
        // 定位器中的偏移可能仍是相对 ZIP 起点的旧值（封装时），
        // 也可能是已按 EXE 起点修正的值（解包还原时）。
        let zip64_abs = shift_offset(zip64_offset, delta)?;
        let mut zip64 = [0u8; 56];
        read_at(file, zip64_abs, &mut zip64)?;
        if u32::from_le_bytes(zip64[0..4].try_into().unwrap_or_default()) != ZIP64_EOCD_SIGNATURE {
            return Err(EngineError::DataError("ZIP64 结束记录损坏".to_string()));
        }
        cd_offset = u64::from_le_bytes(zip64[48..56].try_into().unwrap_or_default());
        cd_size = u64::from_le_bytes(zip64[40..48].try_into().unwrap_or_default());
        entry_count = u64::from_le_bytes(zip64[32..40].try_into().unwrap_or_default());
        central_start = zip64_abs.saturating_sub(cd_size);

        let new_cd = shift_offset(cd_offset, delta)?;
        write_at(file, zip64_abs + 48, &new_cd.to_le_bytes())?;
        let new_zip64 = shift_offset(zip64_offset, delta)?;
        write_at(file, locator_offset + 8, &new_zip64.to_le_bytes())?;
    }

    if eocd_cd_field != u32::MAX {
        let new_cd = shift_offset(cd_offset, delta)?;
        let new_cd = u32::try_from(new_cd)
            .map_err(|_| EngineError::DataError("SFX 载荷过大，超出 ZIP32 偏移范围".to_string()))?;
        write_at(file, eocd_offset + 16, &new_cd.to_le_bytes())?;
    }

    let mut cursor = central_start;
    for _ in 0..entry_count {
        let mut header = [0u8; 46];
        read_at(file, cursor, &mut header)?;
        if u32::from_le_bytes(header[0..4].try_into().unwrap_or_default())
            != CENTRAL_HEADER_SIGNATURE
        {
            return Err(EngineError::DataError("ZIP 中央目录条目损坏".to_string()));
        }
        let name_len = u16::from_le_bytes(header[28..30].try_into().unwrap_or_default()) as u64;
        let extra_len = u16::from_le_bytes(header[30..32].try_into().unwrap_or_default()) as u64;
        let comment_len = u16::from_le_bytes(header[32..34].try_into().unwrap_or_default()) as u64;
        let local_offset = u32::from_le_bytes(header[42..46].try_into().unwrap_or_default());

        if local_offset != u32::MAX {
            let new_local = shift_offset(local_offset as u64, delta)?;
            let new_local = u32::try_from(new_local).map_err(|_| {
                EngineError::DataError("SFX 载荷过大，超出 ZIP32 偏移范围".to_string())
            })?;
            write_at(file, cursor + 42, &new_local.to_le_bytes())?;
        } else {
            let extra_start = cursor + 46 + name_len;
            let mut extra = vec![0u8; extra_len as usize];
            read_at(file, extra_start, &mut extra)?;
            let Some(position) = zip64_local_offset_position(&header, &extra) else {
                return Err(EngineError::DataError(
                    "ZIP64 条目缺少本地头偏移字段".to_string(),
                ));
            };
            let field = extra_start + position as u64;
            let mut value = [0u8; 8];
            read_at(file, field, &mut value)?;
            let new_local = shift_offset(u64::from_le_bytes(value), delta)?;
            write_at(file, field, &new_local.to_le_bytes())?;
        }

        cursor = cursor
            .checked_add(46 + name_len + extra_len + comment_len)
            .ok_or_else(|| EngineError::DataError("ZIP 中央目录偏移溢出".to_string()))?;
    }
    Ok(())
}

const EOCD_SIGNATURE: u32 = 0x0605_4b50;
const ZIP64_EOCD_SIGNATURE: u32 = 0x0606_4b50;
const ZIP64_LOCATOR_SIGNATURE: u32 = 0x0706_4b50;
const CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;

/// 返回 ZIP64 扩展字段中“本地头相对偏移”值在其数据区内的偏移。
fn zip64_local_offset_position(header: &[u8; 46], extra: &[u8]) -> Option<usize> {
    let mut position = 0usize;
    while position + 4 <= extra.len() {
        let id = u16::from_le_bytes(extra[position..position + 2].try_into().ok()?);
        let length =
            u16::from_le_bytes(extra[position + 2..position + 4].try_into().ok()?) as usize;
        let data_start = position + 4;
        if data_start + length > extra.len() {
            return None;
        }
        if id == 0x0001 {
            let mut inner = 0usize;
            if u32::from_le_bytes(header[24..28].try_into().ok()?) == u32::MAX {
                inner += 8;
            }
            if u32::from_le_bytes(header[20..24].try_into().ok()?) == u32::MAX {
                inner += 8;
            }
            if inner + 8 <= length {
                return Some(data_start + inner);
            }
            return None;
        }
        position = data_start + length;
    }
    None
}

fn shift_offset(value: u64, delta: i64) -> Result<u64> {
    if delta >= 0 {
        value
            .checked_add(delta as u64)
            .ok_or_else(|| EngineError::DataError("ZIP 偏移溢出".to_string()))
    } else {
        value
            .checked_sub(delta.unsigned_abs())
            .ok_or_else(|| EngineError::DataError("ZIP 偏移损坏".to_string()))
    }
}

fn read_at(file: &mut File, offset: u64, buffer: &mut [u8]) -> Result<()> {
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(buffer)?;
    Ok(())
}

fn write_at(file: &mut File, offset: u64, bytes: &[u8]) -> Result<()> {
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(bytes)?;
    Ok(())
}

pub fn detect_sfx(path: &Path) -> Result<Option<SfxInfo>> {
    let mut file = File::open(to_extended(path))?;
    detect_sfx_from_file(&mut file)
}

pub fn detect_current_exe() -> Result<Option<(SfxInfo, PathBuf)>> {
    let exe = std::env::current_exe()?;
    Ok(detect_sfx(&exe)?.map(|info| (info, exe)))
}

fn detect_sfx_from_file(file: &mut File) -> Result<Option<SfxInfo>> {
    let len = file.metadata()?.len();
    if len < FOOTER_SIZE {
        return Ok(None);
    }
    file.seek(SeekFrom::End(-(FOOTER_SIZE as i64)))?;
    let mut footer = [0u8; 20];
    file.read_exact(&mut footer)?;
    if &footer[12..20] != SFX_MAGIC {
        return Ok(None);
    }
    let payload_len = u64::from_le_bytes(footer[0..8].try_into().unwrap_or_default());
    let config_len = u32::from_le_bytes(footer[8..12].try_into().unwrap_or_default()) as u64;
    let payload_end = len
        .checked_sub(FOOTER_SIZE)
        .and_then(|value| value.checked_sub(config_len));
    let Some(payload_end) = payload_end else {
        return Ok(None);
    };
    let payload_offset = payload_end
        .checked_sub(payload_len)
        .ok_or_else(|| EngineError::DataError("SFX 载荷长度损坏".to_string()))?;
    if payload_offset < 2 {
        return Ok(None);
    }
    let mut config_bytes = vec![0u8; config_len as usize];
    file.seek(SeekFrom::Start(payload_end))?;
    file.read_exact(&mut config_bytes)?;
    let config = serde_json::from_slice::<SfxConfig>(&config_bytes)
        .map_err(|error| EngineError::DataError(format!("SFX 配置损坏：{error}")))?;
    Ok(Some(SfxInfo {
        payload_offset,
        payload_len,
        config,
    }))
}

/// 提取内嵌 ZIP 到配置目录（默认 EXE 所在目录）。
pub fn extract_sfx(payload: &Path, exe: &Path, sink: &dyn ProgressSink) -> Result<PathBuf> {
    let info = detect_sfx(exe)?
        .ok_or_else(|| EngineError::DataError("不是 NimZip SFX 文件".to_string()))?;
    let destination = if info.config.destination.trim().is_empty() {
        exe.parent().unwrap_or_else(|| Path::new(".")).to_path_buf()
    } else {
        let configured = PathBuf::from(&info.config.destination);
        if configured.is_absolute() {
            configured
        } else {
            exe.parent()
                .unwrap_or_else(|| Path::new("."))
                .join(configured)
        }
    };
    std::fs::create_dir_all(to_extended(&destination))?;
    let temp = std::env::temp_dir()
        .join("NimZip")
        .join("sfx")
        .join(format!("{:016x}.zip", simple_hash(payload)));
    if let Some(parent) = temp.parent() {
        std::fs::create_dir_all(to_extended(parent))?;
    }
    copy_payload(exe, &info, &temp)?;
    // create_sfx 写入的是相对整个 EXE 的偏移；剥离出的临时 ZIP 需要还原为相对载荷起点。
    let payload_offset = i64::try_from(info.payload_offset)
        .map_err(|_| EngineError::DataError("SFX 载荷偏移过大".to_string()))?;
    {
        let mut temp_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(to_extended(&temp))?;
        adjust_zip_offsets(&mut temp_file, 0, info.payload_len, -payload_offset)?;
    }
    let options = crate::extract::ExtractOptions {
        mode: crate::extract::ExtractMode::Custom(destination.clone()),
        overwrite: true,
        strip_root: false,
        propagate_motw: true,
        nested_depth: 0,
        password: None,
        ..Default::default()
    };
    crate::extract::extract_archive(&temp, &options, sink)?;
    let _ = std::fs::remove_file(to_extended(&temp));
    Ok(destination)
}

fn copy_payload(exe: &Path, info: &SfxInfo, temp: &Path) -> Result<()> {
    let mut input = File::open(to_extended(exe))?;
    input.seek(SeekFrom::Start(info.payload_offset))?;
    let mut output = File::create(to_extended(temp))?;
    std::io::copy(&mut input.take(info.payload_len), &mut output)?;
    output.flush()?;
    Ok(())
}

fn simple_hash(path: &Path) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    std::fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .hash(&mut hasher);
    hasher.finish()
}
