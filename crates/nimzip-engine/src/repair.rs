//! ZIP 修复：绕过损坏的中央目录，顺序扫描本地文件头并重建可读归档。
//!
//! 支持 Store / Deflate / BZip2 条目；加密、ZIP64 数据描述符或无法通过 CRC
//! 校验的条目不写入结果，但会在报告中计数并给出警告。原文件始终只读。

use std::fs::File;
use std::io::{Read, Write};

use crate::error::{EngineError, Result};
use crate::format::ArchiveFormat;
use crate::paths::to_extended;
use crate::progress::{check_cancelled, Phase, Progress, ProgressSink};
use crate::zip_backend::decode_name;

#[derive(Debug, Clone)]
pub struct RepairReport {
    pub format: ArchiveFormat,
    pub entries: usize,
    pub recovered: usize,
    pub skipped: usize,
    pub bytes: u64,
    pub output: std::path::PathBuf,
}

const LOCAL_HEADER: &[u8; 4] = b"PK\x03\x04";
const CENTRAL_HEADER: &[u8; 4] = b"PK\x01\x02";

pub fn repair_zip(
    input: &std::path::Path,
    output: &std::path::Path,
    sink: &dyn ProgressSink,
) -> Result<RepairReport> {
    let bytes = std::fs::read(to_extended(input))?;
    let total = bytes.len() as u64;
    let file = File::create(to_extended(output))?;
    let mut writer = zip::ZipWriter::new(file);
    let mut pos = 0usize;
    let mut entries = 0usize;
    let mut recovered = 0usize;
    let mut skipped = 0usize;
    let mut bytes_out = 0u64;

    while pos + 30 <= bytes.len() {
        check_cancelled(sink)?;
        if &bytes[pos..pos + 4] == CENTRAL_HEADER {
            break;
        }
        if &bytes[pos..pos + 4] != LOCAL_HEADER {
            pos += 1;
            continue;
        }
        let Some(parsed) = parse_local(&bytes, pos) else {
            pos += 4;
            skipped += 1;
            continue;
        };
        entries += 1;
        sink.update(&Progress::new(
            Phase::Testing,
            pos as u64,
            total,
            parsed.name.clone(),
        ));
        pos = parsed.next;
        if parsed.encrypted || parsed.zip64 || parsed.descriptor {
            skipped += 1;
            sink.warn(&format!(
                "{}：加密/ZIP64/数据描述符条目无法安全修复，已跳过",
                parsed.name
            ));
            continue;
        }
        let Some(data) = parsed.data else {
            skipped += 1;
            continue;
        };
        let mut plain = Vec::new();
        match parsed.method {
            0 => plain = data,
            8 => {
                let mut decoder = flate2::read::DeflateDecoder::new(data.as_slice());
                if decoder.read_to_end(&mut plain).is_err() {
                    skipped += 1;
                    sink.warn(&format!("{}：Deflate 数据损坏，已跳过", parsed.name));
                    continue;
                }
            }
            12 => {
                let mut decoder = bzip2::read::BzDecoder::new(data.as_slice());
                if decoder.read_to_end(&mut plain).is_err() {
                    skipped += 1;
                    sink.warn(&format!("{}：BZip2 数据损坏，已跳过", parsed.name));
                    continue;
                }
            }
            14 | 93 => {
                skipped += 1;
                sink.warn(&format!("{}：LZMA/Zstd 条目暂不支持修复", parsed.name));
                continue;
            }
            _ => {
                skipped += 1;
                sink.warn(&format!("{}：未知压缩方法 {}", parsed.name, parsed.method));
                continue;
            }
        }
        let crc = crc32fast::hash(&plain);
        if parsed.crc != 0 && crc != parsed.crc {
            skipped += 1;
            sink.warn(&format!("{}：CRC 校验失败，已跳过", parsed.name));
            continue;
        }
        if parsed.name.ends_with('/') || (parsed.uncompressed == 0 && plain.is_empty()) {
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            writer
                .add_directory(parsed.name.trim_end_matches('/'), options)
                .map_err(|err| EngineError::DataError(format!("写入目录失败：{err}")))?;
        } else {
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(if parsed.method == 0 {
                    zip::CompressionMethod::Stored
                } else {
                    zip::CompressionMethod::Deflated
                })
                .last_modified_time(parsed.modified);
            writer
                .start_file(parsed.name.clone(), options)
                .map_err(|err| EngineError::DataError(format!("写入条目失败：{err}")))?;
            writer.write_all(&plain)?;
        }
        bytes_out += plain.len() as u64;
        recovered += 1;
    }
    writer
        .finish()
        .map_err(|err| EngineError::DataError(format!("完成修复失败：{err}")))?;
    if recovered == 0 {
        return Err(EngineError::DataError(
            "未找到可恢复的 ZIP 条目，原始文件可能不是 ZIP 或已严重损坏".to_string(),
        ));
    }
    Ok(RepairReport {
        format: ArchiveFormat::Zip,
        entries,
        recovered,
        skipped,
        bytes: bytes_out,
        output: output.to_path_buf(),
    })
}

struct LocalEntry {
    name: String,
    method: u16,
    crc: u32,
    uncompressed: u64,
    encrypted: bool,
    zip64: bool,
    descriptor: bool,
    modified: zip::DateTime,
    data: Option<Vec<u8>>,
    next: usize,
}

fn parse_local(bytes: &[u8], offset: usize) -> Option<LocalEntry> {
    if offset + 30 > bytes.len() {
        return None;
    }
    let header = &bytes[offset..offset + 30];
    let version = u16::from_le_bytes([header[4], header[5]]);
    let flags = u16::from_le_bytes([header[6], header[7]]);
    let method = u16::from_le_bytes([header[8], header[9]]);
    let time = u16::from_le_bytes([header[10], header[11]]);
    let date = u16::from_le_bytes([header[12], header[13]]);
    let crc = u32::from_le_bytes([header[14], header[15], header[16], header[17]]);
    let compressed = u32::from_le_bytes([header[18], header[19], header[20], header[21]]) as u64;
    let uncompressed = u32::from_le_bytes([header[22], header[23], header[24], header[25]]) as u64;
    let name_len = u16::from_le_bytes([header[26], header[27]]) as usize;
    let extra_len = u16::from_le_bytes([header[28], header[29]]) as usize;
    let name_start = offset + 30;
    let extra_start = name_start.checked_add(name_len)?;
    let data_start = extra_start.checked_add(extra_len)?;
    if data_start > bytes.len() {
        return None;
    }
    let raw_name = &bytes[name_start..extra_start];
    let fallback = String::from_utf8_lossy(raw_name);
    let name = decode_name(raw_name, &fallback, crate::format::Codepage::Auto).replace('\\', "/");
    if name.is_empty() {
        return None;
    }
    let zip64 = compressed == u32::MAX as u64
        || uncompressed == u32::MAX as u64
        || extra_has_zip64(&bytes[extra_start..data_start]);
    let descriptor = flags & 0x0008 != 0;
    let (data, next) = if descriptor || zip64 {
        (None, data_start)
    } else {
        let data_end = data_start.checked_add(compressed as usize)?;
        if data_end > bytes.len() {
            (None, data_start)
        } else {
            (Some(bytes[data_start..data_end].to_vec()), data_end)
        }
    };
    let modified = dos_datetime(date, time).unwrap_or_default();
    let _ = version;
    Some(LocalEntry {
        name,
        method,
        crc,
        uncompressed,
        encrypted: flags & 0x0001 != 0,
        zip64,
        descriptor,
        modified,
        data,
        next: next.max(offset + 30),
    })
}

fn extra_has_zip64(extra: &[u8]) -> bool {
    let mut pos = 0usize;
    while pos + 4 <= extra.len() {
        let id = u16::from_le_bytes([extra[pos], extra[pos + 1]]);
        let size = u16::from_le_bytes([extra[pos + 2], extra[pos + 3]]) as usize;
        if id == 0x0001 {
            return true;
        }
        pos = pos.saturating_add(4 + size);
    }
    false
}

fn dos_datetime(date: u16, time: u16) -> Option<zip::DateTime> {
    let year = 1980 + (date >> 9);
    let month = ((date >> 5) & 0x0f) as u8;
    let day = (date & 0x1f) as u8;
    let hour = ((time >> 11) & 0x1f) as u8;
    let minute = ((time >> 5) & 0x3f) as u8;
    let second = ((time & 0x1f) * 2) as u8;
    zip::DateTime::from_date_and_time(year, month, day, hour, minute, second).ok()
}
