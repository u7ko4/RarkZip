use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::error::Result;
use crate::paths::to_extended;
use crate::progress::{check_cancelled, Phase, Progress, ProgressSink};
use crate::util::COPY_BUFFER_SIZE;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HashAlgorithm {
    Md5,
    Sha1,
    Sha256,
    Sha512,
    Sha3,
    Blake3,
    XxHash64,
    Crc32,
}

impl HashAlgorithm {
    pub const ALL: [HashAlgorithm; 8] = [
        HashAlgorithm::Sha256,
        HashAlgorithm::Sha1,
        HashAlgorithm::Md5,
        HashAlgorithm::Sha512,
        HashAlgorithm::Sha3,
        HashAlgorithm::Blake3,
        HashAlgorithm::XxHash64,
        HashAlgorithm::Crc32,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
            Self::Sha3 => "sha3-256",
            Self::Blake3 => "blake3",
            Self::XxHash64 => "xxhash64",
            Self::Crc32 => "crc32",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Sha1 => "SHA-1",
            Self::Sha256 => "SHA-256",
            Self::Sha512 => "SHA-512",
            Self::Sha3 => "SHA3-256",
            Self::Blake3 => "BLAKE3",
            Self::XxHash64 => "xxHash64",
            Self::Crc32 => "CRC32",
        }
    }

    pub fn checksum_extension(self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
            Self::Sha3 => "sha3",
            Self::Blake3 => "blake3",
            Self::XxHash64 => "xxh64",
            Self::Crc32 => "crc32",
        }
    }

    pub fn from_name(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "md5" => Self::Md5,
            "sha1" | "sha-1" => Self::Sha1,
            "sha256" | "sha-256" => Self::Sha256,
            "sha512" | "sha-512" => Self::Sha512,
            "sha3" | "sha3-256" | "sha3_256" => Self::Sha3,
            "blake3" => Self::Blake3,
            "xxhash64" | "xxh64" | "xxhash" => Self::XxHash64,
            "crc32" | "crc" => Self::Crc32,
            _ => return None,
        })
    }
}

impl std::fmt::Display for HashAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

enum HasherState {
    Md5(md5::Md5),
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
    Sha3(sha3::Sha3_256),
    Blake3(Box<blake3::Hasher>),
    XxHash64(xxhash_rust::xxh64::Xxh64),
    Crc32(crc32fast::Hasher),
}

impl HasherState {
    fn new(algorithm: HashAlgorithm) -> Self {
        match algorithm {
            HashAlgorithm::Md5 => Self::Md5(md5::Md5::new()),
            HashAlgorithm::Sha1 => Self::Sha1(sha1::Sha1::new()),
            HashAlgorithm::Sha256 => Self::Sha256(sha2::Sha256::new()),
            HashAlgorithm::Sha512 => Self::Sha512(sha2::Sha512::new()),
            HashAlgorithm::Sha3 => Self::Sha3(sha3::Sha3_256::new()),
            HashAlgorithm::Blake3 => Self::Blake3(Box::new(blake3::Hasher::new())),
            HashAlgorithm::XxHash64 => Self::XxHash64(xxhash_rust::xxh64::Xxh64::new(0)),
            HashAlgorithm::Crc32 => Self::Crc32(crc32fast::Hasher::new()),
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Self::Md5(hasher) => hasher.update(data),
            Self::Sha1(hasher) => hasher.update(data),
            Self::Sha256(hasher) => hasher.update(data),
            Self::Sha512(hasher) => hasher.update(data),
            Self::Sha3(hasher) => hasher.update(data),
            Self::Blake3(hasher) => {
                hasher.update(data);
            }
            Self::XxHash64(hasher) => hasher.update(data),
            Self::Crc32(hasher) => hasher.update(data),
        }
    }

    fn finish_hex(self) -> String {
        match self {
            Self::Md5(hasher) => hex(&hasher.finalize()),
            Self::Sha1(hasher) => hex(&hasher.finalize()),
            Self::Sha256(hasher) => hex(&hasher.finalize()),
            Self::Sha512(hasher) => hex(&hasher.finalize()),
            Self::Sha3(hasher) => hex(&hasher.finalize()),
            Self::Blake3(hasher) => hasher.finalize().to_hex().to_string(),
            Self::XxHash64(hasher) => format!("{:016x}", hasher.digest()),
            Self::Crc32(hasher) => format!("{:08x}", hasher.finalize()),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

pub fn hash_bytes(algorithm: HashAlgorithm, data: &[u8]) -> String {
    let mut hasher = HasherState::new(algorithm);
    hasher.update(data);
    hasher.finish_hex()
}

pub fn hash_reader(algorithm: HashAlgorithm, reader: &mut dyn Read) -> Result<String> {
    let mut hasher = HasherState::new(algorithm);
    let mut buffer = vec![0u8; COPY_BUFFER_SIZE];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finish_hex())
}

pub fn hash_file(algorithm: HashAlgorithm, path: &Path) -> Result<String> {
    let mut file = File::open(to_extended(path))?;
    hash_reader(algorithm, &mut file)
}

/// 流式分块计算文件哈希，默认 1 MiB 缓冲，不整载内存。
pub fn hash_file_with_progress(
    algorithm: HashAlgorithm,
    path: &Path,
    sink: &dyn ProgressSink,
) -> Result<String> {
    let mut file = File::open(to_extended(path))?;
    let total = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let mut hasher = HasherState::new(algorithm);
    let mut buffer = vec![0u8; COPY_BUFFER_SIZE];
    let mut done = 0u64;
    loop {
        check_cancelled(sink)?;
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        done += read as u64;
        sink.update(&Progress::new(
            Phase::Hashing,
            done,
            total,
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default(),
        ));
    }
    Ok(hasher.finish_hex())
}

/// 单次读取文件，同时计算多个哈希。用于哈希窗口的多列显示。
pub fn hash_file_many(
    algorithms: &[HashAlgorithm],
    path: &Path,
    sink: &dyn ProgressSink,
) -> Result<Vec<(HashAlgorithm, String)>> {
    let mut file = File::open(to_extended(path))?;
    let total = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let mut hashers: Vec<(HashAlgorithm, HasherState)> = algorithms
        .iter()
        .copied()
        .map(|algorithm| (algorithm, HasherState::new(algorithm)))
        .collect();
    let mut buffer = vec![0u8; COPY_BUFFER_SIZE];
    let mut done = 0u64;
    loop {
        check_cancelled(sink)?;
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        for (_, hasher) in &mut hashers {
            hasher.update(&buffer[..read]);
        }
        done += read as u64;
        sink.update(&Progress::new(
            Phase::Hashing,
            done,
            total,
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default(),
        ));
    }
    Ok(hashers
        .into_iter()
        .map(|(algorithm, hasher)| (algorithm, hasher.finish_hex()))
        .collect())
}

/// 生成标准校验文件内容：`<hash>  <filename>`。
pub fn checksum_contents(entries: &[(String, PathBuf)]) -> String {
    let mut output = String::new();
    for (hash, path) in entries {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        output.push_str(hash);
        output.push_str("  ");
        output.push_str(&name);
        output.push('\n');
    }
    output
}

pub fn save_checksum_file(path: &Path, entries: &[(String, PathBuf)]) -> Result<()> {
    std::fs::write(to_extended(path), checksum_contents(entries))?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecksumFileEntry {
    pub hash: String,
    pub file_name: String,
}

/// 解析常见校验文件：`<hash>  <file>`、`<hash> *<file>`、`SHA256 (file) = hash`。
pub fn parse_checksum_text(text: &str) -> Vec<ChecksumFileEntry> {
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(entry) = parse_bsd_line(line).or_else(|| parse_line(line)) {
            entries.push(entry);
        }
    }
    entries
}

fn parse_bsd_line(line: &str) -> Option<ChecksumFileEntry> {
    let (left, hash) = line.rsplit_once('=')?;
    let left = left.trim();
    let hash = hash.trim();
    if !looks_like_hash(hash) {
        return None;
    }
    let name = left
        .strip_suffix(')')?
        .rsplit_once('(')
        .map(|(_, name)| name.trim())?;
    if name.is_empty() {
        return None;
    }
    Some(ChecksumFileEntry {
        hash: hash.to_ascii_lowercase(),
        file_name: name.to_string(),
    })
}

fn parse_line(line: &str) -> Option<ChecksumFileEntry> {
    let mut parts = line.splitn(2, char::is_whitespace);
    let hash = parts.next()?.trim();
    let name = parts.next()?.trim();
    if !looks_like_hash(hash) || name.is_empty() {
        return None;
    }
    let name = name.strip_prefix('*').unwrap_or(name).trim();
    if name.is_empty() {
        return None;
    }
    Some(ChecksumFileEntry {
        hash: hash.to_ascii_lowercase(),
        file_name: name.to_string(),
    })
}

fn looks_like_hash(value: &str) -> bool {
    value.len() >= 8 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

pub fn load_checksum_file(path: &Path) -> Result<Vec<ChecksumFileEntry>> {
    let text = std::fs::read_to_string(to_extended(path))?;
    Ok(parse_checksum_text(&text))
}

/// 用文件名（不区分大小写、忽略路径）建立期望哈希表。
pub fn checksum_expectations(
    entries: &[ChecksumFileEntry],
) -> std::collections::HashMap<String, String> {
    entries
        .iter()
        .map(|entry| {
            let name = entry
                .file_name
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&entry.file_name)
                .to_ascii_lowercase();
            (name, entry.hash.clone())
        })
        .collect()
}

/// 比较实际哈希与期望表；文件名匹配不区分路径大小写。
pub fn compare_hash(
    path: &Path,
    actual: &str,
    expected: &std::collections::HashMap<String, String>,
) -> Option<bool> {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    expected
        .get(&name)
        .map(|expected| expected.eq_ignore_ascii_case(actual))
}
