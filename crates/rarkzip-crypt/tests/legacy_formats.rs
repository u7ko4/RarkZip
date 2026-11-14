use std::io::Cursor;
use std::path::PathBuf;
use std::time::SystemTime;

use blake2::digest::{KeyInit, Mac};
use rarkzip_crypt::algorithm::Algorithm;
use rarkzip_crypt::cipher::ctr::CtrStreams;
use rarkzip_crypt::cipher::{BlockCipher, Cipher};
use rarkzip_crypt::file::{decrypt_file, FileDecryptOptions};
use rarkzip_crypt::format::{DEFAULT_CUSTOM_PARAMS, SALT_LEN};
use rarkzip_crypt::kdf;
use rarkzip_crypt::progress::NoopReporter;
use rarkzip_crypt::util;
use rarkzip_crypt::zip_writer::{ZipCompression, ZipWriter};

fn plaintext() -> &'static [u8] {
    b"legacy container payload\n"
}

fn zip_bytes() -> Vec<u8> {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("hello.txt");
    std::fs::write(&file, plaintext()).unwrap();
    let cursor = Cursor::new(Vec::new());
    let reporter = NoopReporter;
    let total = plaintext().len() as u64;
    let mut writer = ZipWriter::new(cursor, ZipCompression::Store, &reporter, total);
    writer
        .add_file("hello.txt", &file, SystemTime::now())
        .unwrap();
    writer.finish().unwrap().into_inner()
}

fn build_v3(algorithm: Algorithm) -> Vec<u8> {
    let salt = util::random_bytes(SALT_LEN).unwrap();
    let (iterations, memory) = kdf::argon2_params(DEFAULT_CUSTOM_PARAMS).unwrap();
    let keys = kdf::derive_file_keys(
        "test-password-123",
        &salt,
        algorithm.key_bytes(),
        algorithm.nonce_bytes(),
        64,
        iterations,
        memory,
    )
    .unwrap();
    let mut header = [0u8; 8];
    header[..5].copy_from_slice(b"SSEFE");
    header[5] = 3;
    header[6] = algorithm.code();
    header[7] = DEFAULT_CUSTOM_PARAMS;
    let checkcode = util::random_checkcode().unwrap();
    let mut plain = checkcode.to_vec();
    plain.extend_from_slice(&zip_bytes());
    let mut streams = CtrStreams::new(algorithm, &keys.key, &keys.nonce).unwrap();
    streams.apply(&mut plain);

    type Blake2bMac256 = blake2::Blake2bMac<blake2::digest::consts::U32>;
    let mut mac = <Blake2bMac256 as KeyInit>::new_from_slice(&keys.auth_key).unwrap();
    mac.update(&keys.nonce);
    mac.update(&header);
    mac.update(&salt);
    mac.update(&plain);
    let tag = mac.finalize().into_bytes();

    let mut output = Vec::new();
    output.extend_from_slice(&header);
    output.extend_from_slice(&salt);
    output.extend_from_slice(&plain);
    output.extend_from_slice(&tag);
    output
}

fn build_v2() -> Vec<u8> {
    let algorithm = Algorithm::Aes256;
    let salt = util::random_bytes(SALT_LEN).unwrap();
    let keys = kdf::derive_legacy_file_keys("test-password-123", &salt, 32, 16).unwrap();
    let cipher = Cipher::new(algorithm, &keys.key).unwrap();
    let checkcode = util::random_checkcode().unwrap();
    let mut plain = checkcode.to_vec();
    plain.extend_from_slice(&zip_bytes());
    plain.extend_from_slice(&checkcode);
    let block = cipher.block_size();
    let padding = block - (plain.len() % block);
    let mut random = util::random_bytes(padding - 1).unwrap();
    plain.extend_from_slice(&random);
    random.clear();
    plain.push(padding as u8);

    let mut previous = keys.nonce.to_vec();
    let mut ciphertext = Vec::with_capacity(plain.len());
    for chunk in plain.chunks(block) {
        let mut value = chunk.to_vec();
        for (byte, previous_byte) in value.iter_mut().zip(previous.iter()) {
            *byte ^= previous_byte;
        }
        cipher.encrypt_block(&mut value);
        ciphertext.extend_from_slice(&value);
        previous = value;
    }
    let mut header = [0u8; 8];
    header[..5].copy_from_slice(b"SSEFE");
    header[5] = 2;
    header[6] = algorithm.code();
    header[7] = 0;
    let mut output = Vec::new();
    output.extend_from_slice(&header);
    output.extend_from_slice(&salt);
    output.extend_from_slice(&ciphertext);
    output
}

fn decrypt_to_contents(bytes: Vec<u8>, name: &str) -> String {
    let temp = tempfile::tempdir().unwrap();
    let path: PathBuf = temp.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    let output = temp.path().join("out");
    let decrypted = decrypt_file(
        &path,
        "test-password-123",
        &FileDecryptOptions {
            output_dir: Some(output),
        },
        &NoopReporter,
    )
    .unwrap();
    std::fs::read_to_string(decrypted).unwrap()
}

fn dump_if_requested(name: &str, bytes: &[u8]) {
    if let Ok(dir) = std::env::var("RARKZIP_CRYPT_DUMP") {
        let dir = PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(name), bytes);
    }
}

#[test]
fn reads_generated_v3_for_aes_and_composite() {
    for algorithm in [Algorithm::Aes256, Algorithm::Composite2048] {
        let bytes = build_v3(algorithm);
        dump_if_requested(&format!("v3-{}.enc", algorithm.code()), &bytes);
        let content = decrypt_to_contents(bytes, &format!("v3-{}.enc", algorithm.code()));
        assert_eq!(content, "legacy container payload\n");
    }
}

#[test]
fn reads_generated_v2() {
    let bytes = build_v2();
    dump_if_requested("v2.enc", &bytes);
    let content = decrypt_to_contents(bytes, "v2.enc");
    assert_eq!(content, "legacy container payload\n");
}
