//! File and text encryption engine with compatibility for the legacy
//! `.enc` container and text ciphertext formats used by RarkZip's encryption
//! tool.
//!
//! The engine is intentionally independent of the GUI. It exposes streaming
//! file encryption/decryption, text encryption/decryption and JPEG
//! steganography.

pub mod algorithm;
pub mod cipher;
pub mod error;
pub mod f5;
pub mod file;
pub mod format;
pub mod kdf;
pub mod password;
pub mod progress;
pub mod stego;
pub mod stream_reader;
pub mod text;
pub mod util;
pub mod zip_writer;

pub use algorithm::Algorithm;
pub use error::{CryptError, Result};
