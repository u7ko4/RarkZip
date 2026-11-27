use aes::cipher::BlockDecrypt as AesBlockDecrypt;
use aes::cipher::BlockEncrypt as AesBlockEncrypt;
use aes::{cipher::KeyInit as AesKeyInit, Aes256};
use blowfish::Blowfish;
use cipher::typenum::{U20, U32};
use cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use rc6::RC6;
use serpent::Serpent;
use threefish::Threefish1024;
use twofish::Twofish;

use crate::algorithm::Algorithm;
use crate::error::{CryptError, Result};

pub mod ctr;
pub mod eax;
pub mod gost;
pub mod shacal2;

pub use gost::Gost28147;
pub use shacal2::Shacal2;

/// RC6 with a 128-bit block, 20 rounds and a 256-bit key.
pub type Rc6_256 = RC6<u32, U20, U32>;

/// Minimal block cipher interface used by CTR, CBC and EAX.
pub trait BlockCipher {
    fn block_size(&self) -> usize;
    fn encrypt_block(&self, block: &mut [u8]);
    fn decrypt_block(&self, block: &mut [u8]);
}

pub enum Cipher {
    Aes256(Box<Aes256>),
    Rc6(Box<Rc6_256>),
    Serpent(Box<Serpent>),
    Blowfish(Box<Blowfish>),
    Twofish(Box<Twofish>),
    Gost(Gost28147),
    Threefish(Box<Threefish1024>),
    Shacal2(Box<Shacal2>),
}

impl Cipher {
    pub fn new(algorithm: Algorithm, key: &[u8]) -> Result<Self> {
        if algorithm.is_composite() {
            return Err(CryptError::UnsupportedAlgorithm(algorithm.code()));
        }
        if key.len() != algorithm.key_bytes() {
            return Err(CryptError::InvalidParameters(format!(
                "{} key must be {} bytes, got {}",
                algorithm.label(),
                algorithm.key_bytes(),
                key.len()
            )));
        }
        let cipher = match algorithm {
            Algorithm::Aes256 => Self::Aes256(Box::new(
                <Aes256 as AesKeyInit>::new_from_slice(key)
                    .map_err(|_| CryptError::InvalidParameters("invalid AES key".into()))?,
            )),
            Algorithm::Rc6_256 => Self::Rc6(Box::new(
                <Rc6_256 as KeyInit>::new_from_slice(key)
                    .map_err(|_| CryptError::InvalidParameters("invalid RC6 key".into()))?,
            )),
            Algorithm::Serpent256 => {
                let mut reversed = key.to_vec();
                reversed.reverse();
                Self::Serpent(Box::new(
                    <Serpent as KeyInit>::new_from_slice(&reversed)
                        .map_err(|_| CryptError::InvalidParameters("invalid Serpent key".into()))?,
                ))
            }
            Algorithm::Blowfish256 | Algorithm::Blowfish448 => Self::Blowfish(Box::new(
                <Blowfish as KeyInit>::new_from_slice(key)
                    .map_err(|_| CryptError::InvalidParameters("invalid Blowfish key".into()))?,
            )),
            Algorithm::Twofish256 => Self::Twofish(Box::new(
                <Twofish as KeyInit>::new_from_slice(key)
                    .map_err(|_| CryptError::InvalidParameters("invalid Twofish key".into()))?,
            )),
            Algorithm::Gost28147 => Self::Gost(Gost28147::new(key)?),
            Algorithm::Threefish1024 => Self::Threefish(Box::new(
                <Threefish1024 as KeyInit>::new_from_slice(key)
                    .map_err(|_| CryptError::InvalidParameters("invalid Threefish key".into()))?,
            )),
            Algorithm::Shacal2_512 => Self::Shacal2(Box::new(Shacal2::new(key)?)),
            Algorithm::Composite2048 => {
                return Err(CryptError::UnsupportedAlgorithm(algorithm.code()))
            }
        };
        Ok(cipher)
    }
}

impl BlockCipher for Cipher {
    fn block_size(&self) -> usize {
        match self {
            Self::Aes256(_) | Self::Rc6(_) | Self::Serpent(_) | Self::Twofish(_) => 16,
            Self::Blowfish(_) | Self::Gost(_) => 8,
            Self::Threefish(_) => 128,
            Self::Shacal2(_) => 32,
        }
    }

    fn encrypt_block(&self, block: &mut [u8]) {
        match self {
            Self::Aes256(cipher) => encrypt_aes(cipher.as_ref(), block),
            Self::Rc6(cipher) => encrypt_cipher05(cipher.as_ref(), block),
            Self::Serpent(cipher) => {
                block.reverse();
                encrypt_cipher05(cipher.as_ref(), block);
                block.reverse();
            }
            Self::Blowfish(cipher) => encrypt_cipher05(cipher.as_ref(), block),
            Self::Twofish(cipher) => encrypt_cipher05(cipher.as_ref(), block),
            Self::Gost(cipher) => cipher.encrypt_block(block),
            Self::Threefish(cipher) => encrypt_cipher05(cipher.as_ref(), block),
            Self::Shacal2(cipher) => cipher.encrypt_block(block),
        }
    }

    fn decrypt_block(&self, block: &mut [u8]) {
        match self {
            Self::Aes256(cipher) => decrypt_aes(cipher.as_ref(), block),
            Self::Rc6(cipher) => decrypt_cipher05(cipher.as_ref(), block),
            Self::Serpent(cipher) => {
                block.reverse();
                decrypt_cipher05(cipher.as_ref(), block);
                block.reverse();
            }
            Self::Blowfish(cipher) => decrypt_cipher05(cipher.as_ref(), block),
            Self::Twofish(cipher) => decrypt_cipher05(cipher.as_ref(), block),
            Self::Gost(cipher) => cipher.decrypt_block(block),
            Self::Threefish(cipher) => decrypt_cipher05(cipher.as_ref(), block),
            Self::Shacal2(cipher) => cipher.decrypt_block(block),
        }
    }
}

fn encrypt_aes(cipher: &Aes256, block: &mut [u8]) {
    let mut value = aes::cipher::Block::<Aes256>::default();
    value.copy_from_slice(block);
    cipher.encrypt_block(&mut value);
    block.copy_from_slice(&value);
}

fn decrypt_aes(cipher: &Aes256, block: &mut [u8]) {
    let mut value = aes::cipher::Block::<Aes256>::default();
    value.copy_from_slice(block);
    cipher.decrypt_block(&mut value);
    block.copy_from_slice(&value);
}

fn encrypt_cipher05<C>(cipher: &C, block: &mut [u8])
where
    C: BlockCipherEncrypt,
{
    let mut value = cipher::Block::<C>::default();
    value.copy_from_slice(block);
    cipher.encrypt_block(&mut value);
    block.copy_from_slice(&value);
}

fn decrypt_cipher05<C>(cipher: &C, block: &mut [u8])
where
    C: BlockCipherDecrypt,
{
    let mut value = cipher::Block::<C>::default();
    value.copy_from_slice(block);
    cipher.decrypt_block(&mut value);
    block.copy_from_slice(&value);
}
