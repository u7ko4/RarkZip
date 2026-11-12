use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

fn bx() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bx"))
}

fn write(path: &Path, content: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

#[test]
fn cli_roundtrip_and_exit_codes() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("src");
    write(&source.join("README.txt"), b"hello rarkzip\n");
    write(&source.join("2026/01 冬季/IMG_1042.jpg"), b"jpeg");
    let archive = temp.path().join("demo.zip");

    let output = bx()
        .args([
            "a",
            archive.to_str().unwrap(),
            source.to_str().unwrap(),
            "-y",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "create failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(archive.exists());

    let output = bx()
        .args(["l", archive.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let listing_text = String::from_utf8_lossy(&output.stdout);
    let listing: serde_json::Value = serde_json::from_str(&listing_text).unwrap();
    let entries = listing["entries"].as_array().unwrap();
    assert!(entries
        .iter()
        .any(|entry| entry["path"] == "src/README.txt"));

    let output = bx()
        .args(["t", archive.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    let out = temp.path().join("out");
    let output = bx()
        .args(["x", archive.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "extract failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(out.join("src/README.txt")).unwrap(),
        b"hello rarkzip\n"
    );

    let output = bx()
        .args([
            "h",
            source.join("README.txt").to_str().unwrap(),
            "-a",
            "sha256",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let hash_text = String::from_utf8_lossy(&output.stdout);
    assert!(
        hash_text.starts_with("479d23da8045efb5a7321c6d5225b769ae3a352f3f9085f0d064cb9dceedd0d0  ")
    );
}

#[test]
fn cli_password_errors_use_exit_code_two() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("secret.txt");
    write(&source, b"top secret");
    let archive: PathBuf = temp.path().join("secret.zip");

    let output = bx()
        .args([
            "a",
            archive.to_str().unwrap(),
            source.to_str().unwrap(),
            "-p",
            "correct-password",
            "-y",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = bx()
        .args(["t", archive.to_str().unwrap(), "-p", "wrong-password"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));

    let output = bx()
        .args(["t", archive.to_str().unwrap(), "-p", "correct-password"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "test failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
