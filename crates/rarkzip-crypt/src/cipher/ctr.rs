use crate::algorithm::Algorithm;
use crate::cipher::{BlockCipher, Cipher};
use crate::error::{CryptError, Result};

/// One big-endian counter-mode layer.
pub struct CtrStream {
    cipher: Cipher,
    nonce: Vec<u8>,
    counter: Vec<u8>,
    keystream: Vec<u8>,
    keystream_offset: usize,
    next_skip: usize,
    position: u64,
}

impl CtrStream {
    pub fn new(cipher: Cipher, nonce: &[u8]) -> Result<Self> {
        let block = cipher.block_size();
        if nonce.len() != block {
            return Err(CryptError::InvalidParameters(format!(
                "CTR nonce must be {block} bytes, got {}",
                nonce.len()
            )));
        }
        Ok(Self {
            cipher,
            nonce: nonce.to_vec(),
            counter: nonce.to_vec(),
            keystream: Vec::new(),
            keystream_offset: block,
            next_skip: 0,
            position: 0,
        })
    }

    pub fn block_size(&self) -> usize {
        self.cipher.block_size()
    }

    pub fn position(&self) -> u64 {
        self.position
    }

    /// Reset the stream to an absolute byte position.
    pub fn seek(&mut self, position: u64) {
        let block = self.block_size() as u64;
        let blocks = position / block;
        self.counter = self.nonce.clone();
        add_counter(&mut self.counter, blocks);
        self.keystream.clear();
        self.keystream_offset = self.block_size();
        self.next_skip = (position % block) as usize;
        self.position = position;
    }

    fn next_keystream_block(&mut self) {
        self.keystream.clear();
        self.keystream.extend_from_slice(&self.counter);
        self.cipher.encrypt_block(&mut self.keystream);
        self.keystream_offset = self.next_skip.min(self.keystream.len());
        self.next_skip = 0;
        add_counter(&mut self.counter, 1);
    }

    pub fn apply(&mut self, data: &mut [u8]) {
        let mut offset = 0usize;
        while offset < data.len() {
            if self.keystream_offset >= self.keystream.len() {
                self.next_keystream_block();
            }
            let available = self.keystream.len() - self.keystream_offset;
            let take = available.min(data.len() - offset);
            for index in 0..take {
                data[offset + index] ^= self.keystream[self.keystream_offset + index];
            }
            self.keystream_offset += take;
            offset += take;
        }
        self.position = self.position.wrapping_add(data.len() as u64);
    }

    pub fn apply_at(&mut self, position: u64, data: &mut [u8]) {
        if self.position != position {
            self.seek(position);
        }
        self.apply(data);
    }
}

/// A single algorithm or the four-layer 2048-bit composite stream.
pub struct CtrStreams {
    layers: Vec<CtrStream>,
}

impl CtrStreams {
    pub fn new(algorithm: Algorithm, key: &[u8], nonce: &[u8]) -> Result<Self> {
        if key.len() != algorithm.key_bytes() || nonce.len() != algorithm.nonce_bytes() {
            return Err(CryptError::InvalidParameters(format!(
                "{} expects {}-byte key and {}-byte nonce",
                algorithm.label(),
                algorithm.key_bytes(),
                algorithm.nonce_bytes()
            )));
        }
        let mut layers = Vec::new();
        if let Some(layer_specs) = algorithm.layers() {
            let mut key_offset = 0usize;
            let mut nonce_offset = 0usize;
            for spec in layer_specs {
                let layer_key = &key[key_offset..key_offset + spec.key_bytes];
                let layer_nonce = &nonce[nonce_offset..nonce_offset + spec.nonce_bytes];
                let cipher = Cipher::new(spec.algorithm, layer_key)?;
                layers.push(CtrStream::new(cipher, layer_nonce)?);
                key_offset += spec.key_bytes;
                nonce_offset += spec.nonce_bytes;
            }
        } else {
            let cipher = Cipher::new(algorithm, key)?;
            layers.push(CtrStream::new(cipher, nonce)?);
        }
        Ok(Self { layers })
    }

    pub fn apply(&mut self, data: &mut [u8]) {
        for layer in &mut self.layers {
            layer.apply(data);
        }
    }

    pub fn seek(&mut self, position: u64) {
        for layer in &mut self.layers {
            layer.seek(position);
        }
    }

    pub fn position(&self) -> u64 {
        self.layers.first().map(CtrStream::position).unwrap_or(0)
    }

    pub fn apply_at(&mut self, position: u64, data: &mut [u8]) {
        if self.position() != position {
            self.seek(position);
        }
        self.apply(data);
    }
}

fn add_counter(counter: &mut [u8], value: u64) {
    let mut carry = value;
    for byte in counter.iter_mut().rev() {
        let sum = *byte as u64 + (carry & 0xff);
        *byte = sum as u8;
        carry = (carry >> 8) + (sum >> 8);
        if carry == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algorithm::Algorithm;

    #[test]
    fn aes_ctr_round_trip_and_seek() {
        let key = [0x11u8; 32];
        let nonce = [0x22u8; 16];
        let mut stream = CtrStreams::new(Algorithm::Aes256, &key, &nonce).unwrap();
        let plaintext = b"the quick brown fox jumps over the lazy dog";
        let mut encrypted = plaintext.to_vec();
        stream.apply(&mut encrypted);
        assert_ne!(&encrypted[..], &plaintext[..]);

        let mut second = CtrStreams::new(Algorithm::Aes256, &key, &nonce).unwrap();
        second.seek(7);
        let mut tail = encrypted[7..].to_vec();
        second.apply(&mut tail);
        assert_eq!(&tail[..], &plaintext[7..]);
    }
}
