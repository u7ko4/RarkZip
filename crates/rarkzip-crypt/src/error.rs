use std::io;

/// Errors returned by the encryption engine.
#[derive(Debug, thiserror::Error)]
pub enum CryptError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("Incorrect password or corrupted file")]
    WrongPassword,

    #[error("Corrupted data: {0}")]
    Corrupted(String),

    #[error("Unsupported format version: {0}")]
    UnsupportedVersion(u8),

    #[error("Unsupported algorithm code: {0}")]
    UnsupportedAlgorithm(u8),

    #[error("Unsupported or unsafe parameters: {0}")]
    InvalidParameters(String),

    #[error("Unsafe archive path: {0}")]
    UnsafePath(String),

    #[error("Operation cancelled")]
    Cancelled,

    #[error("ZIP error: {0}")]
    Zip(String),

    #[error("Crypto error: {0}")]
    Crypto(String),

    #[error("Image error: {0}")]
    Image(String),

    #[error("Steganography error: {0}")]
    Steganography(String),

    #[error("{0}")]
    Other(String),
}

impl From<zip::result::ZipError> for CryptError {
    fn from(value: zip::result::ZipError) -> Self {
        Self::Zip(value.to_string())
    }
}

impl From<image::ImageError> for CryptError {
    fn from(value: image::ImageError) -> Self {
        Self::Image(value.to_string())
    }
}

pub type Result<T> = std::result::Result<T, CryptError>;
