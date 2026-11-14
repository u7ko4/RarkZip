use std::path::PathBuf;

use rarkzip_crypt::stego::extract_text_from_jpeg;

#[test]
fn extracts_text_from_java_steganogram() {
    let path = PathBuf::from(format!(
        "{}/tests/fixtures/stego.jpg",
        env!("CARGO_MANIFEST_DIR")
    ));
    let text = extract_text_from_jpeg(&path, "test-password-123").expect("extract steganogram");
    assert_eq!(text, "hidden message for f5 round trip");
}

#[test]
fn embeds_and_extracts_text_round_trip() {
    use rarkzip_crypt::algorithm::Algorithm;
    use rarkzip_crypt::stego::{embed_text_in_jpeg, extract_text_from_jpeg, StegoOptions};

    let fixture = PathBuf::from(format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR")));
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("stego.jpg");
    let text = "Rust steganography round trip";
    embed_text_in_jpeg(
        text,
        "test-password-123",
        &StegoOptions {
            carrier: fixture.join("carrier.png"),
            output: output.clone(),
            quality: 90,
            max_side: Some(512),
            algorithm: Algorithm::Aes256,
        },
    )
    .unwrap();
    if let Ok(path) = std::env::var("RARKZIP_CRYPT_STEGO_DUMP") {
        std::fs::copy(&output, path).unwrap();
    }
    let extracted = extract_text_from_jpeg(&output, "test-password-123").unwrap();
    assert_eq!(extracted, text);
}
