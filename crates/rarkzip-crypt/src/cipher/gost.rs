use crate::cipher::BlockCipher;
use crate::error::{CryptError, Result};

/// GOST 28147-89 with the legacy default S-box.
pub struct Gost28147 {
    key: [u32; 8],
}

const SBOX: [u8; 128] = [
    0x04, 0x0a, 0x09, 0x02, 0x0d, 0x08, 0x00, 0x0e, 0x06, 0x0b, 0x01, 0x0c, 0x07, 0x0f, 0x05, 0x03,
    0x0e, 0x0b, 0x04, 0x0c, 0x06, 0x0d, 0x0f, 0x0a, 0x02, 0x03, 0x08, 0x01, 0x00, 0x07, 0x05, 0x09,
    0x05, 0x08, 0x01, 0x0d, 0x0a, 0x03, 0x04, 0x02, 0x0e, 0x0f, 0x0c, 0x07, 0x06, 0x00, 0x09, 0x0b,
    0x07, 0x0d, 0x0a, 0x01, 0x00, 0x08, 0x09, 0x0f, 0x0e, 0x04, 0x06, 0x0c, 0x0b, 0x02, 0x05, 0x03,
    0x06, 0x0c, 0x07, 0x01, 0x05, 0x0f, 0x0d, 0x08, 0x04, 0x0a, 0x09, 0x0e, 0x00, 0x03, 0x0b, 0x02,
    0x04, 0x0b, 0x0a, 0x00, 0x07, 0x02, 0x01, 0x0d, 0x03, 0x06, 0x08, 0x05, 0x09, 0x0c, 0x0f, 0x0e,
    0x0d, 0x0b, 0x04, 0x01, 0x03, 0x0f, 0x05, 0x09, 0x00, 0x0a, 0x0e, 0x07, 0x06, 0x08, 0x02, 0x0c,
    0x01, 0x0f, 0x0d, 0x00, 0x05, 0x07, 0x0a, 0x04, 0x09, 0x02, 0x03, 0x0e, 0x06, 0x0b, 0x08, 0x0c,
];

impl Gost28147 {
    pub fn new(key: &[u8]) -> Result<Self> {
        if key.len() != 32 {
            return Err(CryptError::InvalidParameters(
                "GOST28147 key must be 32 bytes".into(),
            ));
        }
        let mut words = [0u32; 8];
        for (index, chunk) in key.chunks_exact(4).enumerate() {
            words[index] = u32::from_le_bytes(chunk.try_into().expect("four bytes"));
        }
        Ok(Self { key: words })
    }

    fn main_step(n1: u32, key: u32) -> u32 {
        let cm = key.wrapping_add(n1);
        let mut om = SBOX[(cm & 0x0f) as usize] as u32;
        om = om.wrapping_add((SBOX[16 + ((cm >> 4) & 0x0f) as usize] as u32) << 4);
        om = om.wrapping_add((SBOX[32 + ((cm >> 8) & 0x0f) as usize] as u32) << 8);
        om = om.wrapping_add((SBOX[48 + ((cm >> 12) & 0x0f) as usize] as u32) << 12);
        om = om.wrapping_add((SBOX[64 + ((cm >> 16) & 0x0f) as usize] as u32) << 16);
        om = om.wrapping_add((SBOX[80 + ((cm >> 20) & 0x0f) as usize] as u32) << 20);
        om = om.wrapping_add((SBOX[96 + ((cm >> 24) & 0x0f) as usize] as u32) << 24);
        om = om.wrapping_add((SBOX[112 + ((cm >> 28) & 0x0f) as usize] as u32) << 28);
        om.rotate_left(11)
    }

    pub fn encrypt_block(&self, block: &mut [u8]) {
        let mut n1 = u32::from_le_bytes(block[0..4].try_into().expect("four bytes"));
        let mut n2 = u32::from_le_bytes(block[4..8].try_into().expect("four bytes"));
        for _ in 0..3 {
            for &key in &self.key {
                let tmp = n1;
                n1 = n2 ^ Self::main_step(n1, key);
                n2 = tmp;
            }
        }
        for index in (1..8).rev() {
            let tmp = n1;
            n1 = n2 ^ Self::main_step(n1, self.key[index]);
            n2 = tmp;
        }
        n2 ^= Self::main_step(n1, self.key[0]);
        block[0..4].copy_from_slice(&n1.to_le_bytes());
        block[4..8].copy_from_slice(&n2.to_le_bytes());
    }

    pub fn decrypt_block(&self, block: &mut [u8]) {
        let mut n1 = u32::from_le_bytes(block[0..4].try_into().expect("four bytes"));
        let mut n2 = u32::from_le_bytes(block[4..8].try_into().expect("four bytes"));
        for &key in &self.key {
            let tmp = n1;
            n1 = n2 ^ Self::main_step(n1, key);
            n2 = tmp;
        }
        for round in 0..3 {
            for index in (0..8).rev() {
                if round == 2 && index == 0 {
                    break;
                }
                let tmp = n1;
                n1 = n2 ^ Self::main_step(n1, self.key[index]);
                n2 = tmp;
            }
        }
        n2 ^= Self::main_step(n1, self.key[0]);
        block[0..4].copy_from_slice(&n1.to_le_bytes());
        block[4..8].copy_from_slice(&n2.to_le_bytes());
    }
}

impl BlockCipher for Gost28147 {
    fn block_size(&self) -> usize {
        8
    }

    fn encrypt_block(&self, block: &mut [u8]) {
        Self::encrypt_block(self, block);
    }

    fn decrypt_block(&self, block: &mut [u8]) {
        Self::decrypt_block(self, block);
    }
}
