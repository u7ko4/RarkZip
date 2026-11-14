use rarkzip_crypt::text::decrypt_text;

fn fixture(name: &str) -> String {
    format!("{}/tests/fixtures/alg/{name}", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn decrypts_java_text_samples_for_all_algorithms() {
    for code in 0u8..=9 {
        let encoded = std::fs::read_to_string(fixture(&format!("{code}.b64"))).unwrap();
        let plaintext = decrypt_text(&encoded, "test-password-123")
            .unwrap_or_else(|error| panic!("algorithm {code}: {error}"));
        assert_eq!(plaintext, format!("Algorithm round trip {code}"));
    }
}

#[test]
fn decrypts_java_base32_and_numeric_samples() {
    for (index, suffix) in ["enc1", "enc2"].iter().enumerate() {
        let encoded = std::fs::read_to_string(fixture(&format!("0.{suffix}"))).unwrap();
        let plaintext = decrypt_text(&encoded, "test-password-123")
            .unwrap_or_else(|error| panic!("encoding {index}: {error}"));
        assert_eq!(plaintext, format!("Encoding round trip {}", index + 1));
    }
}

#[test]
fn wrong_password_is_rejected() {
    let encoded = std::fs::read_to_string(fixture("0.b64")).unwrap();
    assert!(decrypt_text(&encoded, "wrong-password").is_err());
}
