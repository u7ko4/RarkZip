use crate::cipher::BlockCipher;
use crate::error::{CryptError, Result};

/// SHACAL-2 with the legacy 512-bit key schedule.
pub struct Shacal2 {
    working_key: [u32; 64],
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

impl Shacal2 {
    pub fn new(key: &[u8]) -> Result<Self> {
        if key.is_empty() || key.len() > 64 || key.len() < 16 || key.len() % 8 != 0 {
            return Err(CryptError::InvalidParameters(
                "SHACAL-2 key must be 16..64 bytes and a multiple of 8".into(),
            ));
        }
        let mut working_key = [0u32; 64];
        for (index, chunk) in key.chunks_exact(4).enumerate() {
            working_key[index] = u32::from_be_bytes(chunk.try_into().expect("four bytes"));
        }
        for index in 16..64 {
            let s0 = working_key[index - 15].rotate_right(7)
                ^ working_key[index - 15].rotate_right(18)
                ^ (working_key[index - 15] >> 3);
            let s1 = working_key[index - 2].rotate_right(17)
                ^ working_key[index - 2].rotate_right(19)
                ^ (working_key[index - 2] >> 10);
            working_key[index] = s1
                .wrapping_add(working_key[index - 7])
                .wrapping_add(s0)
                .wrapping_add(working_key[index - 16]);
        }
        Ok(Self { working_key })
    }

    pub fn encrypt_block(&self, block: &mut [u8]) {
        let mut state = [0u32; 8];
        for (index, chunk) in block.chunks_exact(4).enumerate() {
            state[index] = u32::from_be_bytes(chunk.try_into().expect("four bytes"));
        }
        for (round, round_constant) in K.iter().enumerate() {
            let big_sigma1 =
                state[4].rotate_right(6) ^ state[4].rotate_right(11) ^ state[4].rotate_right(25);
            let choose = (state[4] & state[5]) ^ (!state[4] & state[6]);
            let tmp = big_sigma1
                .wrapping_add(choose)
                .wrapping_add(state[7])
                .wrapping_add(*round_constant)
                .wrapping_add(self.working_key[round]);
            state[7] = state[6];
            state[6] = state[5];
            state[5] = state[4];
            state[4] = state[3].wrapping_add(tmp);
            state[3] = state[2];
            state[2] = state[1];
            state[1] = state[0];
            let big_sigma0 =
                state[0].rotate_right(2) ^ state[0].rotate_right(13) ^ state[0].rotate_right(22);
            let majority = (state[0] & state[2]) ^ (state[0] & state[3]) ^ (state[2] & state[3]);
            state[0] = tmp.wrapping_add(big_sigma0).wrapping_add(majority);
        }
        for (index, value) in state.iter().enumerate() {
            block[index * 4..index * 4 + 4].copy_from_slice(&value.to_be_bytes());
        }
    }

    pub fn decrypt_block(&self, block: &mut [u8]) {
        let mut state = [0u32; 8];
        for (index, chunk) in block.chunks_exact(4).enumerate() {
            state[index] = u32::from_be_bytes(chunk.try_into().expect("four bytes"));
        }
        for (round, round_constant) in K.iter().enumerate().rev() {
            let sigma0 =
                state[1].rotate_right(2) ^ state[1].rotate_right(13) ^ state[1].rotate_right(22);
            let majority = (state[1] & state[2]) ^ (state[1] & state[3]) ^ (state[2] & state[3]);
            let tmp = state[0].wrapping_sub(sigma0).wrapping_sub(majority);
            state[0] = state[1];
            state[1] = state[2];
            state[2] = state[3];
            state[3] = state[4].wrapping_sub(tmp);
            state[4] = state[5];
            state[5] = state[6];
            state[6] = state[7];
            let sigma1 =
                state[4].rotate_right(6) ^ state[4].rotate_right(11) ^ state[4].rotate_right(25);
            let choose = (state[4] & state[5]) ^ (!state[4] & state[6]);
            state[7] = tmp
                .wrapping_sub(*round_constant)
                .wrapping_sub(self.working_key[round])
                .wrapping_sub(sigma1)
                .wrapping_sub(choose);
        }
        for (index, value) in state.iter().enumerate() {
            block[index * 4..index * 4 + 4].copy_from_slice(&value.to_be_bytes());
        }
    }
}

impl BlockCipher for Shacal2 {
    fn block_size(&self) -> usize {
        32
    }

    fn encrypt_block(&self, block: &mut [u8]) {
        Self::encrypt_block(self, block);
    }

    fn decrypt_block(&self, block: &mut [u8]) {
        Self::decrypt_block(self, block);
    }
}
