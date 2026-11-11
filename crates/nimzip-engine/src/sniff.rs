use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::error::{EngineError, Result};
use crate::format::ArchiveFormat;
use crate::paths::to_extended;
use crate::util::read_prefix;

pub const HEAD_SNIFF_WINDOW: usize = 4 * 1024 * 1024;
const TAIL_WINDOW: u64 = 128 * 1024;

const ZIP_LOCAL: &[u8; 4] = b"PK\x03\x04";
const ZIP_EOCD: &[u8; 4] = b"PK\x05\x06";
const ZIP_SPANNED: &[u8; 4] = b"PK\x07\x08";
const ZIP64_EOCD: &[u8; 4] = b"PK\x06\x06";
const SEVEN_Z: &[u8; 6] = b"7z\xBC\xAF\x27\x1C";
const RAR4: &[u8; 7] = b"Rar!\x1A\x07\x00";
const RAR5: &[u8; 8] = b"Rar!\x1A\x07\x01\x00";

#[derive(Debug, Clone)]
pub struct SniffResult {
    pub format: ArchiveFormat,
    /// 归档结构在文件中的起始偏移；SFX 或前缀伪装时大于 0。
    pub payload_offset: u64,
    /// docx/xlsx 等本质是 zip 的容器，嵌套解压时不应拆开。
    pub protected_container: bool,
    pub note: Option<String>,
}

impl SniffResult {
    fn new(format: ArchiveFormat, payload_offset: u64) -> Self {
        Self {
            format,
            payload_offset,
            protected_container: false,
            note: None,
        }
    }
}

/// 按内容识别归档格式；扩展名只用于容器保护标记。
pub fn sniff(path: &Path) -> Result<Option<SniffResult>> {
    sniff_with_window(path, HEAD_SNIFF_WINDOW)
}

/// 使用指定头部窗口识别；嵌套解压扫描使用 64 KiB，避免大量小文件造成大读放大。
pub fn sniff_with_window(path: &Path, head_window: usize) -> Result<Option<SniffResult>> {
    let metadata = std::fs::metadata(to_extended(path))?;
    let file_len = metadata.len();
    if file_len == 0 {
        return Ok(None);
    }

    let head = read_prefix(path, head_window.min(file_len as usize))?;
    let protected = is_protected_container(path);

    // 1. ZIP：先按头部签名，再做 EOCD 尾部回扫，支持任意前缀。
    if head.starts_with(ZIP_LOCAL) || head.starts_with(ZIP_EOCD) || head.starts_with(ZIP_SPANNED) {
        let mut result = SniffResult::new(ArchiveFormat::Zip, 0);
        result.protected_container = protected;
        return Ok(Some(result));
    }

    if file_len >= 22 {
        if let Some(offset) = find_zip_payload(path, file_len)? {
            let mut result = SniffResult::new(ArchiveFormat::Zip, offset);
            result.protected_container = protected;
            if offset > 0 {
                result.note = Some(format!("检测到前缀数据，ZIP 结构位于偏移 {offset}"));
            }
            return Ok(Some(result));
        }
    }

    // 2. 7z：支持在头部窗口内查找 SFX 起始偏移。
    if head.starts_with(SEVEN_Z) {
        return Ok(Some(SniffResult::new(ArchiveFormat::SevenZ, 0)));
    }
    if let Some(pos) = find_subslice(&head, SEVEN_Z) {
        let mut result = SniffResult::new(ArchiveFormat::SevenZ, pos as u64);
        result.note = Some(format!("检测到 7z SFX 前缀，结构位于偏移 {pos}"));
        return Ok(Some(result));
    }

    // 3. RAR：只识别，当前版本不解压。
    if head.starts_with(RAR5) || head.starts_with(RAR4) {
        return Ok(Some(SniffResult::new(ArchiveFormat::Rar, 0)));
    }

    // 3.5 ZPAQ：libzpaq 归档标签 "7kSt"。
    if head.starts_with(b"7kSt") {
        return Ok(Some(SniffResult::new(ArchiveFormat::Zpaq, 0)));
    }

    // 4. 镜像/容器格式（通过原生后端或外部 7-Zip 读取）。
    if head.len() > 32774 && &head[32769..32774] == b"CD001" {
        return Ok(Some(SniffResult::new(ArchiveFormat::Iso, 0)));
    }
    if head.starts_with(b"MSWIM\0\0\0") {
        return Ok(Some(SniffResult::new(ArchiveFormat::Wim, 0)));
    }
    if head.starts_with(b"MSCF") {
        return Ok(Some(SniffResult::new(ArchiveFormat::Cab, 0)));
    }
    if head.len() > 6 && &head[2..4] == b"-l" {
        return Ok(Some(SniffResult::new(ArchiveFormat::Lzh, 0)));
    }
    if head.starts_with(&[0x60, 0xEA]) {
        return Ok(Some(SniffResult::new(ArchiveFormat::Arj, 0)));
    }
    if head.len() > 12
        && head
            .get(4..)
            .is_some_and(|rest| rest.starts_with(b"{\"files\""))
    {
        return Ok(Some(SniffResult::new(ArchiveFormat::Asar, 0)));
    }

    // 5. 压缩流：区分单文件流与 tar 复合流。
    if head.starts_with(&[0x1F, 0x8B]) {
        let format = if probe_compressed_tar(path, ArchiveFormat::Gz) {
            ArchiveFormat::TarGz
        } else {
            ArchiveFormat::Gz
        };
        return Ok(Some(SniffResult::new(format, 0)));
    }
    if head.starts_with(b"BZh") {
        let format = if probe_compressed_tar(path, ArchiveFormat::Bz2) {
            ArchiveFormat::TarBz2
        } else {
            ArchiveFormat::Bz2
        };
        return Ok(Some(SniffResult::new(format, 0)));
    }
    if head.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0x00]) {
        let format = if probe_compressed_tar(path, ArchiveFormat::Xz) {
            ArchiveFormat::TarXz
        } else {
            ArchiveFormat::Xz
        };
        return Ok(Some(SniffResult::new(format, 0)));
    }
    if head.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        let format = if probe_compressed_tar(path, ArchiveFormat::Zst) {
            ArchiveFormat::TarZst
        } else {
            ArchiveFormat::Zst
        };
        return Ok(Some(SniffResult::new(format, 0)));
    }

    // 6. 未压缩 tar。
    if looks_like_tar(&head) {
        return Ok(Some(SniffResult::new(ArchiveFormat::Tar, 0)));
    }

    Ok(None)
}

pub fn is_protected_container(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "docx"
            | "xlsx"
            | "pptx"
            | "docm"
            | "xlsm"
            | "pptm"
            | "apk"
            | "jar"
            | "epub"
            | "ipa"
            | "odt"
            | "ods"
            | "odp"
            | "vsdx"
            | "whl"
            | "xpi"
    )
}

fn find_zip_payload(path: &Path, file_len: u64) -> Result<Option<u64>> {
    let window = TAIL_WINDOW.min(file_len);
    let start = file_len - window;
    let mut file = File::open(to_extended(path))?;
    file.seek(SeekFrom::Start(start))?;
    let mut tail = Vec::with_capacity(window as usize);
    file.read_to_end(&mut tail)?;
    if tail.len() < 22 {
        return Ok(None);
    }

    // 从后向前扫描，优先接受注释恰好结束于文件末尾的记录。
    let mut relaxed: Option<u64> = None;
    for index in (0..=tail.len() - 22).rev() {
        if &tail[index..index + 4] != ZIP_EOCD {
            continue;
        }
        let comment_len = u16::from_le_bytes([tail[index + 20], tail[index + 21]]) as usize;
        if index + 22 + comment_len > tail.len() {
            continue;
        }
        let exact = start + (index + 22 + comment_len) as u64 == file_len;
        let Some((payload, central_absolute)) = eocd_payload(&tail, index, start) else {
            continue;
        };
        // 兼容两类 SFX：ZIP 偏移相对载荷起点（传统拼接），以及已按整个
        // EXE 起点修正偏移的标准自解压包。
        let payload = if payload_is_zip(path, payload)? {
            payload
        } else if let Some(adjusted) = adjusted_sfx_payload(path, central_absolute)? {
            if !payload_is_zip(path, adjusted)? {
                continue;
            }
            adjusted
        } else {
            continue;
        };
        if exact {
            return Ok(Some(payload));
        }
        relaxed.get_or_insert(payload);
    }
    Ok(relaxed)
}

/// 返回（按载荷相对偏移推算的起点，中央目录的绝对位置）。
fn eocd_payload(tail: &[u8], index: usize, tail_start: u64) -> Option<(u64, u64)> {
    let eocd_abs = tail_start + index as u64;
    let mut cd_size = u32::from_le_bytes(tail[index + 12..index + 16].try_into().ok()?) as u64;
    let mut cd_offset = u32::from_le_bytes(tail[index + 16..index + 20].try_into().ok()?) as u64;
    let mut central_absolute = eocd_abs.saturating_sub(cd_size);

    if cd_size == u32::MAX as u64 || cd_offset == u32::MAX as u64 {
        let mut zip64 = None;
        for position in (0..index.saturating_sub(4)).rev() {
            if &tail[position..position + 4] == ZIP64_EOCD && position + 56 <= tail.len() {
                zip64 = Some(position);
                break;
            }
        }
        let position = zip64?;
        cd_size = u64::from_le_bytes(tail[position + 40..position + 48].try_into().ok()?);
        cd_offset = u64::from_le_bytes(tail[position + 48..position + 56].try_into().ok()?);
        // ZIP64 中央目录位于 ZIP64 结束记录之前，而非紧邻 EOCD。
        central_absolute = (tail_start + position as u64).saturating_sub(cd_size);
    }

    if cd_size > eocd_abs {
        return None;
    }
    if cd_size + cd_offset > eocd_abs {
        // 标准 SFX 的中央目录偏移已包含前缀，仍可依据目录绝对位置继续探测。
        return Some((u64::MAX, central_absolute));
    }
    Some((eocd_abs - cd_size - cd_offset, central_absolute))
}

/// 读取中央目录首个条目记录的本地头绝对偏移，用于识别偏移已按 EXE 起点修正的 SFX。
fn adjusted_sfx_payload(path: &Path, central_absolute: u64) -> Result<Option<u64>> {
    let mut file = File::open(to_extended(path))?;
    file.seek(SeekFrom::Start(central_absolute))?;
    let mut header = [0u8; 46];
    if file.read_exact(&mut header).is_err()
        || u32::from_le_bytes(header[0..4].try_into().unwrap_or_default()) != 0x0201_4b50
    {
        return Ok(None);
    }
    let name_len = u16::from_le_bytes(header[28..30].try_into().unwrap_or_default()) as u64;
    let extra_len = u16::from_le_bytes(header[30..32].try_into().unwrap_or_default()) as u64;
    let local_offset = u32::from_le_bytes(header[42..46].try_into().unwrap_or_default());
    if local_offset != u32::MAX {
        return Ok(Some(local_offset as u64));
    }

    let mut extra = vec![0u8; extra_len as usize];
    file.seek(SeekFrom::Start(central_absolute + 46 + name_len))?;
    if file.read_exact(&mut extra).is_err() {
        return Ok(None);
    }
    let mut position = 0usize;
    while position + 4 <= extra.len() {
        let id = u16::from_le_bytes(extra[position..position + 2].try_into().unwrap_or_default());
        let length = u16::from_le_bytes(
            extra[position + 2..position + 4]
                .try_into()
                .unwrap_or_default(),
        ) as usize;
        let data_start = position + 4;
        if data_start + length > extra.len() {
            return Ok(None);
        }
        if id == 0x0001 {
            let mut inner = 0usize;
            if u32::from_le_bytes(header[24..28].try_into().unwrap_or_default()) == u32::MAX {
                inner += 8;
            }
            if u32::from_le_bytes(header[20..24].try_into().unwrap_or_default()) == u32::MAX {
                inner += 8;
            }
            if inner + 8 <= length {
                let start = data_start + inner;
                let value =
                    u64::from_le_bytes(extra[start..start + 8].try_into().unwrap_or_default());
                return Ok(Some(value));
            }
            return Ok(None);
        }
        position = data_start + length;
    }
    Ok(None)
}

fn payload_is_zip(path: &Path, payload: u64) -> Result<bool> {
    let mut file = File::open(to_extended(path))?;
    file.seek(SeekFrom::Start(payload))?;
    let mut magic = [0u8; 4];
    if file.read_exact(&mut magic).is_err() {
        return Ok(false);
    }
    Ok(&magic == ZIP_LOCAL || &magic == ZIP_EOCD || &magic == ZIP_SPANNED)
}

fn probe_compressed_tar(path: &Path, compression: ArchiveFormat) -> bool {
    let Ok(file) = File::open(to_extended(path)) else {
        return false;
    };
    let reader: Box<dyn Read> = match compression {
        ArchiveFormat::Gz => Box::new(flate2::read::GzDecoder::new(file)),
        ArchiveFormat::Bz2 => Box::new(bzip2::read::BzDecoder::new(file)),
        ArchiveFormat::Xz => Box::new(xz2::read::XzDecoder::new(file)),
        ArchiveFormat::Zst => match zstd::stream::read::Decoder::new(file) {
            Ok(decoder) => Box::new(decoder),
            Err(_) => return false,
        },
        _ => return false,
    };
    let mut head = [0u8; 512];
    let mut filled = 0;
    let mut reader = reader;
    while filled < head.len() {
        match reader.read(&mut head[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(_) => return false,
        }
    }
    looks_like_tar(&head[..filled])
}

fn looks_like_tar(header: &[u8]) -> bool {
    if header.len() < 512 {
        return false;
    }
    if &header[257..262] == b"ustar" {
        return true;
    }
    // 旧式 tar 没有 ustar 标记，用校验和判定。
    let stored = std::str::from_utf8(&header[148..156])
        .ok()
        .map(|value| value.trim_matches(['\0', ' ']))
        .and_then(|value| u32::from_str_radix(value, 8).ok());
    let Some(stored) = stored else {
        return false;
    };
    let expected: u32 = header
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            if (148..156).contains(&index) {
                b' ' as u32
            } else {
                *byte as u32
            }
        })
        .sum();
    stored == expected && header[0] != 0
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// 供 UI 提示：格式是否可在本轮打开。
pub fn ensure_supported(result: &SniffResult) -> Result<()> {
    match result.format {
        ArchiveFormat::Unknown => Err(EngineError::UnsupportedFormat(
            "无法识别的归档内容".to_string(),
        )),
        _ => Ok(()),
    }
}
