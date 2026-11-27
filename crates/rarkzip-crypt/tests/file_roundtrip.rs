use std::io::Read;
use std::path::Path;

use rarkzip_crypt::algorithm::Algorithm;
use rarkzip_crypt::file::{decrypt_file, encrypt_file, FileDecryptOptions, FileEncryptOptions};
use rarkzip_crypt::progress::NoopReporter;

fn fixture(name: &str) -> String {
    format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn encrypt_decrypt_round_trip_aes() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("hello.txt");
    std::fs::write(&input, b"round trip content\n").unwrap();
    let reporter = NoopReporter;
    let encrypted = encrypt_file(
        &input,
        "test-password-123",
        &FileEncryptOptions {
            algorithm: Algorithm::Aes256,
            ..Default::default()
        },
        &reporter,
    )
    .unwrap();
    assert!(encrypted.exists());
    let out_dir = temp.path().join("out");
    let decrypted = decrypt_file(
        &encrypted,
        "test-password-123",
        &FileDecryptOptions {
            output_dir: Some(out_dir.clone()),
        },
        &reporter,
    )
    .unwrap();
    let mut content = String::new();
    std::fs::File::open(&decrypted)
        .unwrap()
        .read_to_string(&mut content)
        .unwrap();
    assert_eq!(content, "round trip content\n");
}

#[test]
fn encrypt_decrypt_directory_round_trip() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("folder");
    std::fs::create_dir_all(input.join("sub")).unwrap();
    std::fs::create_dir_all(input.join("empty")).unwrap();
    std::fs::write(input.join("root.txt"), b"root\n").unwrap();
    std::fs::write(input.join("sub").join("nested.txt"), b"nested\n").unwrap();
    let reporter = NoopReporter;
    let encrypted = encrypt_file(
        &input,
        "test-password-123",
        &FileEncryptOptions {
            algorithm: Algorithm::Composite2048,
            ..Default::default()
        },
        &reporter,
    )
    .unwrap();
    let out_dir = temp.path().join("out");
    let decrypted = decrypt_file(
        &encrypted,
        "test-password-123",
        &FileDecryptOptions {
            output_dir: Some(out_dir),
        },
        &reporter,
    )
    .unwrap();
    assert!(decrypted.is_dir());
    assert_eq!(
        std::fs::read_to_string(decrypted.join("root.txt")).unwrap(),
        "root\n"
    );
    assert_eq!(
        std::fs::read_to_string(decrypted.join("sub").join("nested.txt")).unwrap(),
        "nested\n"
    );
    assert!(decrypted.join("empty").is_dir());
}

#[test]
fn decrypts_java_samples_for_all_algorithms() {
    let reporter = NoopReporter;
    for code in 0u8..=9 {
        let input = Path::new(&fixture(&format!("alg/{code}.enc"))).to_path_buf();
        assert!(input.exists(), "missing fixture {code}");
        let out_dir = tempfile::tempdir().unwrap();
        let decrypted = decrypt_file(
            &input,
            "test-password-123",
            &FileDecryptOptions {
                output_dir: Some(out_dir.path().to_path_buf()),
            },
            &reporter,
        )
        .unwrap_or_else(|error| panic!("algorithm {code}: {error}"));
        let mut content = String::new();
        std::fs::File::open(&decrypted)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        assert_eq!(content, "Hello encryption sample!\nSecond line.\n");
    }
}

#[test]
fn encrypts_and_decrypts_extended_algorithms() {
    let reporter = NoopReporter;
    for algorithm in [
        Algorithm::Threefish1024,
        Algorithm::Shacal2_512,
        Algorithm::Composite2048,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("pro.txt");
        std::fs::write(&input, b"pro round trip\n").unwrap();
        let encrypted = encrypt_file(
            &input,
            "test-password-123",
            &FileEncryptOptions {
                algorithm,
                ..Default::default()
            },
            &reporter,
        )
        .unwrap_or_else(|error| panic!("{} encrypt: {error}", algorithm.label()));
        let out_dir = temp.path().join("out");
        let decrypted = decrypt_file(
            &encrypted,
            "test-password-123",
            &FileDecryptOptions {
                output_dir: Some(out_dir),
            },
            &reporter,
        )
        .unwrap_or_else(|error| panic!("{} decrypt: {error}", algorithm.label()));
        assert_eq!(
            std::fs::read_to_string(decrypted).unwrap(),
            "pro round trip\n"
        );
    }
}
