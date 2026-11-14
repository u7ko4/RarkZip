use argon2::{Argon2, Params, Version};
use blake2::digest::{KeyInit, Mac};
use blake2::{Blake2b512, Blake2bMac};
use sha3::{Digest, Sha3_512};
use skein::digest::Digest as SkeinDigest;
use skein::Skein1024_1024;
use zeroize::Zeroizing;

use crate::error::{CryptError, Result};
use crate::password;

const FE_MEMORY_SALT: &[u8] = b"memorySalt";
const FE_MEMORY_INFO: &[u8] = b"memoryInfo";
const TE_MEMORY_SALT: &[u8] = b"memorySaltTE";
const TE_MEMORY_INFO: &[u8] = b"memoryInfoTE";
const ENC_KEY_SALT: &[u8] = b"encKeySalt";
const ENC_KEY_INFO: &[u8] = b"encKeyInfo";
const AUTH_KEY_SALT: &[u8] = b"authKeySalt";
const AUTH_KEY_INFO: &[u8] = b"authKeyInfo";
const NONCE_SALT: &[u8] = b"nonceSalt";
const NONCE_INFO: &[u8] = b"nonceInfo";

const MAX_ARGON2_MEMORY_KIB: u32 = 512 * 1024;
const MAX_ARGON2_ITERATIONS: u32 = 128;
const ARGON2_PARALLELISM: u32 = 4;

pub struct FileKeys {
    pub key: Zeroizing<Vec<u8>>,
    pub nonce: Zeroizing<Vec<u8>>,
    pub auth_key: Zeroizing<Vec<u8>>,
}

pub struct TextKeys {
    pub key: Zeroizing<Vec<u8>>,
    pub nonce: Zeroizing<Vec<u8>>,
}

/// Parse the one-byte Argon2 cost selector used by file v3/v4 headers.
pub fn argon2_params(custom: u8) -> Result<(u32, u32)> {
    let mm = (custom >> 4) as u32;
    let tm = (custom & 0x0f) as u32;
    let iterations = 10u32
        .checked_mul(1u32 << tm)
        .ok_or_else(|| CryptError::InvalidParameters("invalid Argon2 time cost".into()))?;
    let memory = 10_240u32
        .checked_mul(1u32 << mm)
        .ok_or_else(|| CryptError::InvalidParameters("invalid Argon2 memory cost".into()))?;
    if iterations > MAX_ARGON2_ITERATIONS || memory > MAX_ARGON2_MEMORY_KIB {
        return Err(CryptError::InvalidParameters(format!(
            "Argon2 cost out of range (t={iterations}, m={memory} KiB)"
        )));
    }
    Ok((iterations, memory))
}

fn argon2id(
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    memory: u32,
    output_len: usize,
) -> Result<Zeroizing<Vec<u8>>> {
    let params = Params::new(memory, iterations, ARGON2_PARALLELISM, Some(output_len))
        .map_err(|error| CryptError::Crypto(format!("Argon2 parameters: {error}")))?;
    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, Version::V0x13, params);
    let mut output = Zeroizing::new(vec![0u8; output_len]);
    argon2
        .hash_password_into(password, salt, &mut output)
        .map_err(|error| CryptError::Crypto(format!("Argon2: {error}")))?;
    Ok(output)
}

fn hkdf_sha3_512(ikm: &[u8], salt: &[u8], info: &[u8], len: usize) -> Result<Zeroizing<Vec<u8>>> {
    let hkdf = hkdf::Hkdf::<Sha3_512>::new(Some(salt), ikm);
    let mut output = Zeroizing::new(vec![0u8; len]);
    hkdf.expand(info, &mut output)
        .map_err(|error| CryptError::Crypto(format!("HKDF-SHA3-512: {error}")))?;
    Ok(output)
}

const SKEIN_BLOCK: usize = 128;

fn hmac_skein(key: &[u8], parts: &[&[u8]]) -> [u8; SKEIN_BLOCK] {
    let mut normalized = [0u8; SKEIN_BLOCK];
    if key.len() > SKEIN_BLOCK {
        let mut hasher = Skein1024_1024::new();
        hasher.update(key);
        let digest = hasher.finalize();
        normalized.copy_from_slice(&digest);
    } else {
        normalized[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; SKEIN_BLOCK];
    let mut opad = [0x5cu8; SKEIN_BLOCK];
    for index in 0..SKEIN_BLOCK {
        ipad[index] ^= normalized[index];
        opad[index] ^= normalized[index];
    }

    let mut inner = Skein1024_1024::new();
    inner.update(ipad);
    for part in parts {
        inner.update(part);
    }
    let inner_digest = inner.finalize();

    let mut outer = Skein1024_1024::new();
    outer.update(opad);
    outer.update(inner_digest);
    let outer_digest = outer.finalize();
    let mut output = [0u8; SKEIN_BLOCK];
    output.copy_from_slice(&outer_digest);
    output
}

fn hkdf_skein(ikm: &[u8], salt: &[u8], info: &[u8], len: usize) -> Zeroizing<Vec<u8>> {
    let prk = hmac_skein(salt, &[ikm]);
    let mut output = Zeroizing::new(Vec::with_capacity(len));
    let mut previous: Vec<u8> = Vec::new();
    let mut counter = 1u8;
    while output.len() < len {
        let block = hmac_skein(
            &prk,
            &[previous.as_slice(), info, std::slice::from_ref(&counter)],
        );
        output.extend_from_slice(&block);
        previous = block.to_vec();
        counter = counter.wrapping_add(1);
    }
    output.truncate(len);
    output
}

pub fn l0_for_file(pwb: &[u8]) -> Zeroizing<Vec<u8>> {
    hkdf_skein(pwb, FE_MEMORY_SALT, FE_MEMORY_INFO, 256)
}

pub fn l0_for_text(pwb: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    hkdf_sha3_512(pwb, TE_MEMORY_SALT, TE_MEMORY_INFO, 256)
}

pub fn legacy_l0(pwb: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut hasher = Skein1024_1024::new();
    hasher.update(pwb);
    let digest = hasher.finalize();
    Zeroizing::new(digest.to_vec())
}

pub fn l0_sha3(pwb: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut hasher = Sha3_512::new();
    hasher.update(pwb);
    Zeroizing::new(hasher.finalize().to_vec())
}

pub fn derive_file_keys(
    password: &str,
    salt: &[u8],
    key_bytes: usize,
    nonce_bytes: usize,
    auth_bytes: usize,
    iterations: u32,
    memory: u32,
) -> Result<FileKeys> {
    let pwb = password::normalize(password);
    let l0 = l0_for_file(&pwb);
    let argon = argon2id(&l0, salt, iterations, memory, 256)?;
    Ok(FileKeys {
        key: hkdf_sha3_512(&argon, ENC_KEY_SALT, ENC_KEY_INFO, key_bytes)?,
        nonce: hkdf_sha3_512(&argon, NONCE_SALT, NONCE_INFO, nonce_bytes)?,
        auth_key: hkdf_sha3_512(&argon, AUTH_KEY_SALT, AUTH_KEY_INFO, auth_bytes)?,
    })
}

pub fn derive_text_keys(
    password: &str,
    salt: &[u8],
    key_bytes: usize,
    nonce_bytes: usize,
) -> Result<TextKeys> {
    let pwb = password::normalize(password);
    let l0 = l0_for_text(&pwb)?;
    let argon = argon2id(&l0, salt, 3, 30_720, 256)?;
    Ok(TextKeys {
        key: hkdf_sha3_512(&argon, ENC_KEY_SALT, ENC_KEY_INFO, key_bytes)?,
        nonce: hkdf_sha3_512(&argon, NONCE_SALT, NONCE_INFO, nonce_bytes)?,
    })
}

fn scrypt_key(
    password: &[u8],
    salt: &[u8],
    log_n: u8,
    r: u32,
    p: u32,
    len: usize,
) -> Result<Zeroizing<Vec<u8>>> {
    let params = scrypt::Params::new(log_n, r, p, len)
        .map_err(|error| CryptError::Crypto(format!("scrypt parameters: {error}")))?;
    let mut output = Zeroizing::new(vec![0u8; len]);
    scrypt::scrypt(password, salt, &params, &mut output)
        .map_err(|error| CryptError::Crypto(format!("scrypt: {error}")))?;
    Ok(output)
}

/// Legacy v2 file key/IV: scrypt with p=10 over the Skein-1024 password hash.
pub fn derive_legacy_file_keys(
    password: &str,
    salt: &[u8],
    key_bytes: usize,
    nonce_bytes: usize,
) -> Result<FileKeys> {
    let pwb = password::normalize(password);
    let l0 = legacy_l0(&pwb);
    let output = scrypt_key(&l0, salt, 11, 8, 10, key_bytes + nonce_bytes)?;
    let key = Zeroizing::new(output[..key_bytes].to_vec());
    let nonce = Zeroizing::new(output[key_bytes..].to_vec());
    Ok(FileKeys {
        key,
        nonce,
        auth_key: Zeroizing::new(Vec::new()),
    })
}

/// Legacy v2 text key/nonce: scrypt with p=5 over the Skein-1024 password hash.
pub fn derive_legacy_text_keys(
    password: &str,
    salt: &[u8],
    key_bytes: usize,
    nonce_bytes: usize,
) -> Result<TextKeys> {
    let pwb = password::normalize(password);
    let l0 = legacy_l0(&pwb);
    let output = scrypt_key(&l0, salt, 11, 8, 5, key_bytes + nonce_bytes)?;
    Ok(TextKeys {
        key: Zeroizing::new(output[..key_bytes].to_vec()),
        nonce: Zeroizing::new(output[key_bytes..].to_vec()),
    })
}

/// Steganography key: scrypt over SHA3-512(password), salted with the carrier
/// dimensions as decimal ASCII.
pub fn derive_stego_key(password: &str, width: u32, height: u32) -> Result<Zeroizing<Vec<u8>>> {
    let pwb = password::normalize(password);
    let l0 = l0_sha3(&pwb);
    let salt = format!("{width}{height}");
    scrypt_key(&l0, salt.as_bytes(), 11, 8, 5, 64)
}

pub fn blake2b_keyed(key: &[u8], data: &[u8]) -> Result<[u8; 32]> {
    type Blake2bMac256 = Blake2bMac<blake2::digest::consts::U32>;
    let mut mac = <Blake2bMac256 as KeyInit>::new_from_slice(key)
        .map_err(|error| CryptError::Crypto(format!("BLAKE2b key: {error}")))?;
    mac.update(data);
    let output = mac.finalize().into_bytes();
    let mut result = [0u8; 32];
    result.copy_from_slice(&output);
    Ok(result)
}

pub fn blake3_keyed(key: &[u8; 32], data: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_keyed(key);
    hasher.update(data);
    *hasher.finalize().as_bytes()
}

pub fn blake2b_512(data: &[u8]) -> [u8; 64] {
    let mut hasher = Blake2b512::new();
    hasher.update(data);
    let output = hasher.finalize();
    let mut result = [0u8; 64];
    result.copy_from_slice(&output);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn salt() -> Vec<u8> {
        (0u8..32).collect()
    }

    #[test]
    fn file_kdf_matches_reference_vector() {
        let keys = derive_file_keys("test-password-123", &salt(), 32, 16, 32, 20, 20_480).unwrap();
        assert_eq!(
            hex::encode(&*keys.key),
            "5aacd3be0d11f446ef9f6ccb5809830388dce958c3b20af81d5378dcbce45299"
        );
        assert_eq!(
            hex::encode(&*keys.nonce),
            "bbb72badcb1fbf8fc45df522f8ae137b"
        );
        assert_eq!(
            hex::encode(&*keys.auth_key),
            "283b14684b2e55612f2a065b03bd6a4929bd9430642674cb220b29c9e837705f"
        );
    }

    #[test]
    fn text_kdf_matches_reference_vector() {
        let keys = derive_text_keys("test-password-123", &salt(), 32, 16).unwrap();
        assert_eq!(
            hex::encode(&*keys.key),
            "08c79684dc2c7741d9dfedcb06f47dfa4669c9ead5ce209915b0674c1628cfd5"
        );
        assert_eq!(
            hex::encode(&*keys.nonce),
            "23f5e0b7a37a8d306d7ccd7657281864"
        );
    }

    #[test]
    fn text_l0_matches_reference_vector() {
        let pwb = password::normalize("test-password-123");
        let l0 = l0_for_text(&pwb).unwrap();
        assert_eq!(
            hex::encode(&*l0),
            concat!(
                "dd944c0c337778667ff6d907a4503e41b79ece9f2a23a5f5feeba729553180b2",
                "6deced990001ab3ddf7a4b04b910778170c2c10d8194633223faff6f9ce8a4e5",
                "b4e110f9e4b5ab8508ffbbd5e9e92e51539fac5592788b83faf1cfb57712ee27",
                "3d25a798f894eb6f6eebeaab5f99aac72850a25a84867de81e0901a2a7665a01",
                "f002edcdde7d2f75533b3cf28cadb0ddc50ea23e225e4d81d2516b5cf6119c7e",
                "cf35fdfd3072b2d625af237b02c39a77ce7d99ef5bbfe3ec889b607e636cd24e",
                "818d498efdbe225af4e1e7a9b36bf320c243427ba3d68b02935653b9adb1232b",
                "074ddfe4b48679951b74459e0a795dd41f960d6b15e94c480358dd522ee2808b"
            )
        );
    }
}
