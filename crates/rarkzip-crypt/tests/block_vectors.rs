use rarkzip_crypt::algorithm::Algorithm;
use rarkzip_crypt::cipher::{BlockCipher, Cipher};

const EXPECTED: &[(u8, &str)] = &[
    (0, "f424cc437be43f01bdcfb5acffac1eeb"),
    (1, "5d6fffd8c66529e1889c1667c6f179dc"),
    (2, "75c8e1236867464fc0b0a891d6d5aa88"),
    (3, "1e7fe5301329b42a"),
    (4, "e31f83b7b5a346823f773419d4d1a0b2"),
    (5, "8809fc6472ede5ec"),
    (6, "11b6fb3c850c05ff"),
    (7, "8058b040059f2ca4a6a2f1d40a87f358267c095955f5a19d94932e34a2dec7be9b708fc9048b1ca764759136fe73814791883f8bc40d77444fc15c95a68c5ce7d4fba41d24fd4f8a2f3345f1620776f5917bc6e5ec74fc9d347984aca469859198d12dd747590b5610dad643c9049a83e355324934771189b8b2a4698e5bc72c"),
    (8, "669735e12107548cd7cc8b94c37204223a60eb3ca43f67e6f212c16773119c0e"),
];

#[test]
fn raw_blocks_match_java() {
    for (code, expected) in EXPECTED {
        let algorithm = Algorithm::from_code(*code).unwrap();
        let key: Vec<u8> = (0..algorithm.key_bytes())
            .map(|index| (index * 7 + 1) as u8)
            .collect();
        let cipher = Cipher::new(algorithm, &key).unwrap();
        let mut block = vec![0u8; cipher.block_size()];
        cipher.encrypt_block(&mut block);
        assert_eq!(
            hex::encode(&block),
            *expected,
            "block mismatch for {}",
            algorithm.label()
        );
    }
}
