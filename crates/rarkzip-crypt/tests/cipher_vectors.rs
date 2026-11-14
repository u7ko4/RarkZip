use std::collections::HashMap;

use rarkzip_crypt::algorithm::Algorithm;
use rarkzip_crypt::cipher::ctr::CtrStreams;
use rarkzip_crypt::cipher::eax;

const VECTORS: &str = include_str!("fixtures/ciphervec.txt");

fn parse_vectors() -> HashMap<String, String> {
    VECTORS
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

fn deterministic_bytes(length: usize, multiplier: usize, add: usize) -> Vec<u8> {
    (0..length)
        .map(|index| (index * multiplier + add) as u8)
        .collect()
}

#[test]
fn ctr_and_eax_match_java_vectors_for_all_algorithms() {
    let vectors = parse_vectors();
    for code in 0u8..=9 {
        let algorithm = Algorithm::from_code(code).expect("known algorithm");
        let key = deterministic_bytes(algorithm.key_bytes(), 7, 1);
        let nonce = deterministic_bytes(algorithm.nonce_bytes(), 11, 3);
        let data = deterministic_bytes(64, 13, 5);
        let aad = deterministic_bytes(16, 17, 9);

        let expected_ctr = vectors
            .get(&format!("CTR_{code}"))
            .unwrap_or_else(|| panic!("missing CTR_{code}"));
        let mut ctr_data = data.clone();
        let mut stream = CtrStreams::new(algorithm, &key, &nonce).expect("ctr init");
        stream.apply(&mut ctr_data);
        assert_eq!(
            hex::encode(&ctr_data),
            *expected_ctr,
            "CTR mismatch for {}",
            algorithm.label()
        );

        let expected_eax = vectors
            .get(&format!("EAX_{code}"))
            .unwrap_or_else(|| panic!("missing EAX_{code}"));
        let produced = eax::encrypt(algorithm, &key, &nonce, &aad, &data).expect("eax encrypt");
        assert_eq!(
            hex::encode(&produced),
            *expected_eax,
            "EAX mismatch for {}",
            algorithm.label()
        );

        let decrypted =
            eax::decrypt(algorithm, &key, &nonce, &aad, &produced).expect("eax decrypt");
        assert_eq!(decrypted, data);
    }
}
