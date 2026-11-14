use std::io::{Read, Write};

use lzma_rust2::{LzmaOptions, LzmaReader, LzmaWriter};
use num_bigint::BigUint;

use crate::algorithm::Algorithm;
use crate::cipher::eax;
use crate::error::{CryptError, Result};
use crate::format::{encode_text_config, parse_text_config};
use crate::kdf;
use crate::password;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextEncoding {
    #[default]
    Base64,
    Base32,
    Base10,
}

impl TextEncoding {
    pub fn label(self) -> &'static str {
        match self {
            Self::Base64 => "Base64",
            Self::Base32 => "Base32",
            Self::Base10 => "Numbers",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TextEncryptOptions {
    pub algorithm: Algorithm,
    pub encoding: TextEncoding,
    pub compress: bool,
    /// Insert a space after every Nth character; 0 disables spacing.
    pub space_every: u8,
}

impl Default for TextEncryptOptions {
    fn default() -> Self {
        Self {
            algorithm: Algorithm::Aes256,
            encoding: TextEncoding::Base64,
            compress: true,
            space_every: 0,
        }
    }
}

pub(crate) fn encrypt_binary(
    text: &str,
    password: &str,
    algorithm: Algorithm,
    compress: bool,
) -> Result<Vec<u8>> {
    let plain = password::trim_java(text).into_owned().into_bytes();
    let salt_len = algorithm.tag_bytes();
    let salt = crate::util::random_bytes(salt_len)?;
    let keys = kdf::derive_text_keys(
        password,
        &salt,
        algorithm.key_bytes(),
        algorithm.nonce_bytes(),
    )?;
    let (payload, compressed) = if compress {
        let compressed = lzma_compress(&plain)?;
        if compressed.len() <= plain.len() {
            (compressed, true)
        } else {
            (plain, false)
        }
    } else {
        (plain, false)
    };
    let config = encode_text_config(algorithm, compressed);
    let mut aad = salt.to_vec();
    aad.push(config);
    let encrypted = eax::encrypt(algorithm, &keys.key, &keys.nonce, &aad, &payload)?;
    let mut binary = salt.to_vec();
    binary.extend_from_slice(&encrypted);
    binary.push(config);
    Ok(binary)
}

pub fn encrypt_text(text: &str, password: &str, options: &TextEncryptOptions) -> Result<String> {
    let binary = encrypt_binary(text, password, options.algorithm, options.compress)?;
    let encoded = match options.encoding {
        TextEncoding::Base64 => {
            let mut value = base64_encode(&binary);
            value.push('!');
            value
        }
        TextEncoding::Base32 => base32_encode(&binary),
        TextEncoding::Base10 => base10_encode(&binary),
    };
    let mut output = if options.space_every > 0 {
        insert_periodically(&encoded, " ", options.space_every as usize)
    } else {
        encoded
    };
    if output.len() > 500_000 {
        output = insert_periodically(&output, "\n", 500_000);
    }
    Ok(output)
}

pub fn decrypt_text(text: &str, password: &str) -> Result<String> {
    let text = strip_whitespace(text);
    let sample_end = text.len().min(1000);
    let sample = &text[..sample_end];

    let binary = if text.ends_with('!') || sample.contains('-') {
        let mut value = text.clone();
        if let Some(index) = sample.rfind('=') {
            value = text[index + 1..].to_string();
        } else if let Some(index) = sample.rfind('/') {
            value = text[index + 1..].to_string();
        }
        if sample.contains('-') {
            value = value.replace('-', "").replace('~', "]").replace('.', ")");
        }
        if value.ends_with('!') {
            value.pop();
        }
        base64_decode(&value)?
    } else if text.chars().all(|ch| ch.is_ascii_digit()) {
        match base10_decode(&text) {
            Ok(value) => value,
            Err(_) => base32_decode(&text)?,
        }
    } else if text
        .to_ascii_uppercase()
        .chars()
        .all(|ch| ch.is_ascii_uppercase() || ('2'..='7').contains(&ch))
    {
        base32_decode(&text)?
    } else {
        return Err(CryptError::UnsupportedVersion(1));
    };

    decrypt_binary(&binary, password)
}

pub(crate) fn decrypt_binary(binary: &[u8], password: &str) -> Result<String> {
    if binary.is_empty() {
        return Err(CryptError::Corrupted("empty ciphertext".into()));
    }
    let config = parse_text_config(binary[binary.len() - 1])?;
    let salt_len = config.algorithm.tag_bytes();
    if binary.len() < salt_len + 1 {
        return Err(CryptError::Corrupted("ciphertext is too short".into()));
    }
    let salt = &binary[..salt_len];
    let body = &binary[salt_len..binary.len() - 1];
    let mut aad = salt.to_vec();
    aad.push(binary[binary.len() - 1]);

    let keys = if config.version == 2 {
        kdf::derive_legacy_text_keys(
            password,
            salt,
            config.algorithm.key_bytes(),
            config.algorithm.nonce_bytes(),
        )?
    } else {
        kdf::derive_text_keys(
            password,
            salt,
            config.algorithm.key_bytes(),
            config.algorithm.nonce_bytes(),
        )?
    };
    let mut plain = eax::decrypt(config.algorithm, &keys.key, &keys.nonce, &aad, body)?;
    if config.compressed {
        plain = if config.version == 2 {
            deflate_decompress(&plain)?
        } else {
            lzma_decompress(&plain)?
        };
    }
    String::from_utf8(plain).map_err(|_| CryptError::Corrupted("invalid UTF-8".into()))
}

fn lzma_compress(data: &[u8]) -> Result<Vec<u8>> {
    let options = LzmaOptions {
        dict_size: 1 << 20,
        ..Default::default()
    };
    let mut output = Vec::new();
    {
        let mut writer = LzmaWriter::new_use_header(&mut output, &options, Some(data.len() as u64))
            .map_err(|error| CryptError::Crypto(format!("LZMA init: {error}")))?;
        writer
            .write_all(data)
            .map_err(|error| CryptError::Crypto(format!("LZMA write: {error}")))?;
        writer
            .finish()
            .map_err(|error| CryptError::Crypto(format!("LZMA finish: {error}")))?;
    }
    Ok(output)
}

fn lzma_decompress(data: &[u8]) -> Result<Vec<u8>> {
    let mut reader = LzmaReader::new_mem_limit(data, u32::MAX, None)
        .map_err(|error| CryptError::Crypto(format!("LZMA header: {error}")))?;
    let mut output = Vec::new();
    reader
        .read_to_end(&mut output)
        .map_err(|error| CryptError::Crypto(format!("LZMA read: {error}")))?;
    Ok(output)
}

fn deflate_decompress(data: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = flate2::read::ZlibDecoder::new(data);
    let mut output = Vec::new();
    decoder
        .read_to_end(&mut output)
        .map_err(|error| CryptError::Corrupted(format!("deflate: {error}")))?;
    Ok(output)
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789)]";
const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn base64_encode(data: &[u8]) -> String {
    let mut output = String::new();
    for chunk in data.chunks(3) {
        let a = chunk[0] as u32;
        let b = chunk.get(1).copied().unwrap_or(0) as u32;
        let c = chunk.get(2).copied().unwrap_or(0) as u32;
        let value = (a << 16) | (b << 8) | c;
        output.push(BASE64_ALPHABET[((value >> 18) & 0x3f) as usize] as char);
        output.push(BASE64_ALPHABET[((value >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            output.push(BASE64_ALPHABET[((value >> 6) & 0x3f) as usize] as char);
        }
        if chunk.len() > 2 {
            output.push(BASE64_ALPHABET[(value & 0x3f) as usize] as char);
        }
    }
    output
}

fn base64_decode(text: &str) -> Result<Vec<u8>> {
    let trimmed = text.trim_end_matches('.');
    let remainder = trimmed.len() % 4;
    if remainder == 1 {
        return Err(CryptError::Corrupted("invalid Base64 length".into()));
    }
    let mut padded = trimmed.as_bytes().to_vec();
    while padded.len() % 4 != 0 {
        padded.push(b'.');
    }
    let chunks = padded.len() / 4;
    let mut output = Vec::new();
    for (chunk_index, chunk) in padded.chunks(4).enumerate() {
        let mut values = [0u8; 4];
        for (index, byte) in chunk.iter().enumerate() {
            values[index] = base64_value(*byte)
                .ok_or_else(|| CryptError::Corrupted("invalid Base64".into()))?;
        }
        let value = ((values[0] as u32) << 18)
            | ((values[1] as u32) << 12)
            | ((values[2] as u32) << 6)
            | values[3] as u32;
        output.push((value >> 16) as u8);
        let last = chunk_index + 1 == chunks;
        let bytes = if last {
            match remainder {
                0 => 3,
                2 => 1,
                3 => 2,
                _ => 3,
            }
        } else {
            3
        };
        if bytes >= 2 {
            output.push((value >> 8) as u8);
        }
        if bytes >= 3 {
            output.push(value as u8);
        }
    }
    Ok(output)
}

fn base64_value(byte: u8) -> Option<u8> {
    if byte == b'.' {
        Some(0)
    } else {
        BASE64_ALPHABET
            .iter()
            .position(|candidate| *candidate == byte)
            .map(|index| index as u8)
    }
}

fn base32_encode(data: &[u8]) -> String {
    let mut output = String::new();
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for byte in data {
        buffer = (buffer << 8) | *byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            output.push(BASE32_ALPHABET[((buffer >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        output.push(BASE32_ALPHABET[((buffer << (5 - bits)) & 0x1f) as usize] as char);
    }
    output
}

fn base32_decode(text: &str) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for byte in text.to_ascii_uppercase().bytes() {
        let value = BASE32_ALPHABET
            .iter()
            .position(|candidate| *candidate == byte)
            .ok_or_else(|| CryptError::Corrupted("invalid Base32".into()))?;
        buffer = (buffer << 5) | value as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
        }
    }
    Ok(output)
}

fn base10_encode(data: &[u8]) -> String {
    let mut value = vec![1u8];
    value.extend_from_slice(data);
    BigUint::from_bytes_be(&value).to_str_radix(10)
}

fn base10_decode(text: &str) -> Result<Vec<u8>> {
    let value = BigUint::parse_bytes(text.as_bytes(), 10)
        .ok_or_else(|| CryptError::Corrupted("invalid numeric ciphertext".into()))?;
    let bytes = value.to_bytes_be();
    if bytes.is_empty() {
        return Err(CryptError::Corrupted("invalid numeric ciphertext".into()));
    }
    Ok(bytes[1..].to_vec())
}

fn strip_whitespace(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect()
}

fn insert_periodically(text: &str, insert: &str, period: usize) -> String {
    if period == 0 {
        return text.to_string();
    }
    let mut output = String::with_capacity(text.len() + insert.len() * (text.len() / period + 1));
    let mut index = 0usize;
    let mut prefix = "";
    while index < text.len() {
        output.push_str(prefix);
        prefix = insert;
        let end = (index + period).min(text.len());
        output.push_str(&text[index..end]);
        index = end;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trip_all_encodings() {
        for encoding in [
            TextEncoding::Base64,
            TextEncoding::Base32,
            TextEncoding::Base10,
        ] {
            for compress in [false, true] {
                let encrypted = encrypt_text(
                    "hello text",
                    "password",
                    &TextEncryptOptions {
                        algorithm: Algorithm::Aes256,
                        encoding,
                        compress,
                        space_every: 7,
                    },
                )
                .unwrap();
                let decrypted = decrypt_text(&encrypted, "password").unwrap();
                assert_eq!(decrypted, "hello text");
            }
        }
    }
}
