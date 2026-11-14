use std::path::{Path, PathBuf};

use getrandom::fill;
use zeroize::Zeroizing;

use crate::error::{CryptError, Result};

pub fn random_bytes(length: usize) -> Result<Zeroizing<Vec<u8>>> {
    let mut buffer = Zeroizing::new(vec![0u8; length]);
    fill(&mut buffer).map_err(|error| CryptError::Other(format!("random generator: {error}")))?;
    Ok(buffer)
}

pub fn random_checkcode() -> Result<[u8; 32]> {
    const ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let random = random_bytes(32)?;
    let mut output = [0u8; 32];
    for (index, byte) in random.iter().enumerate() {
        output[index] = ALPHABET[*byte as usize % ALPHABET.len()];
    }
    Ok(output)
}

pub fn is_valid_checkcode(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

/// Append ` (2)`, ` (3)`, ... until the path does not exist.
pub fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_string());
    let extension = path
        .extension()
        .map(|value| value.to_string_lossy().into_owned());
    for index in 2..10_000u32 {
        let name = match &extension {
            Some(extension) => format!("{stem} ({index}).{extension}"),
            None => format!("{stem} ({index})"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

pub fn unique_root(parent: &Path, name: &str) -> PathBuf {
    let candidate = parent.join(name);
    if !candidate.exists() {
        return candidate;
    }
    for index in 2..10_000u32 {
        let candidate = parent.join(format!("{name} ({index})"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkcode_is_lowercase_ascii() {
        let code = random_checkcode().unwrap();
        assert!(is_valid_checkcode(&code));
    }
}
