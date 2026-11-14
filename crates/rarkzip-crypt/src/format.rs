use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::algorithm::Algorithm;
use crate::error::{CryptError, Result};

pub const MAGIC: &[u8; 5] = b"SSEFE";
pub const HEADER_LEN: usize = 8;
pub const SALT_LEN: usize = 32;
pub const CHECKCODE_LEN: usize = 32;
pub const MAC_LEN: usize = 32;
pub const DEFAULT_CUSTOM_PARAMS: u8 = 0x11;
pub const CURRENT_FILE_VERSION: u8 = 4;
pub const CURRENT_TEXT_VERSION: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileVersion {
    V2,
    V3,
    V4,
}

impl FileVersion {
    pub fn number(self) -> u8 {
        match self {
            Self::V2 => 2,
            Self::V3 => 3,
            Self::V4 => 4,
        }
    }

    pub fn from_number(number: u8) -> Option<Self> {
        match number {
            2 => Some(Self::V2),
            3 => Some(Self::V3),
            4 => Some(Self::V4),
            _ => None,
        }
    }

    pub fn has_mac(self) -> bool {
        matches!(self, Self::V3 | Self::V4)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FileHeader {
    pub version: FileVersion,
    pub algorithm: Algorithm,
    pub custom_params: u8,
}

pub fn encode_file_header(algorithm: Algorithm) -> [u8; HEADER_LEN] {
    let mut header = [0u8; HEADER_LEN];
    header[..5].copy_from_slice(MAGIC);
    header[5] = CURRENT_FILE_VERSION;
    header[6] = algorithm.code();
    header[7] = DEFAULT_CUSTOM_PARAMS;
    header
}

pub fn parse_file_header(header: &[u8; HEADER_LEN]) -> Result<FileHeader> {
    if &header[..5] != MAGIC {
        return Err(CryptError::Corrupted("invalid file header".into()));
    }
    let version =
        FileVersion::from_number(header[5]).ok_or(CryptError::UnsupportedVersion(header[5]))?;
    let algorithm =
        Algorithm::from_code(header[6]).ok_or(CryptError::UnsupportedAlgorithm(header[6]))?;
    Ok(FileHeader {
        version,
        algorithm,
        custom_params: header[7],
    })
}

pub fn detect_file(path: &Path) -> Result<Option<FileVersion>> {
    let mut file = File::open(path)?;
    let mut header = [0u8; HEADER_LEN];
    let mut read = 0usize;
    loop {
        match file.read(&mut header[read..]) {
            Ok(0) => break,
            Ok(count) => read += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
        if read == HEADER_LEN {
            break;
        }
    }
    if read < HEADER_LEN || &header[..5] != MAGIC {
        return Ok(None);
    }
    Ok(FileVersion::from_number(header[5]))
}

#[derive(Debug, Clone, Copy)]
pub struct TextConfig {
    pub version: u8,
    pub algorithm: Algorithm,
    pub compressed: bool,
}

pub fn encode_text_config(algorithm: Algorithm, compressed: bool) -> u8 {
    ((compressed as u8) << 7) | (CURRENT_TEXT_VERSION << 4) | algorithm.code()
}

pub fn parse_text_config(byte: u8) -> Result<TextConfig> {
    let version = (byte >> 4) & 0x07;
    if version != 2 && version != CURRENT_TEXT_VERSION {
        return Err(CryptError::UnsupportedVersion(version));
    }
    let algorithm =
        Algorithm::from_code(byte & 0x0f).ok_or(CryptError::UnsupportedAlgorithm(byte & 0x0f))?;
    Ok(TextConfig {
        version,
        algorithm,
        compressed: byte & 0x80 != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_round_trip() {
        let header = encode_file_header(Algorithm::Composite2048);
        assert_eq!(&header[..5], MAGIC);
        let parsed = parse_file_header(&header).unwrap();
        assert_eq!(parsed.version, FileVersion::V4);
        assert_eq!(parsed.algorithm, Algorithm::Composite2048);
    }

    #[test]
    fn text_config_round_trip() {
        let byte = encode_text_config(Algorithm::Aes256, true);
        assert_eq!(byte, 0xb0);
        let parsed = parse_text_config(byte).unwrap();
        assert!(parsed.compressed);
        assert_eq!(parsed.algorithm, Algorithm::Aes256);
    }
}
