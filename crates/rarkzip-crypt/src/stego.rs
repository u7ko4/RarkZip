use std::path::{Path, PathBuf};

use image::imageops::FilterType;
use image::ImageEncoder;

use crate::algorithm::Algorithm;
use crate::error::{CryptError, Result};
use crate::kdf;
use crate::text;

pub struct StegoOptions {
    pub carrier: PathBuf,
    pub output: PathBuf,
    pub quality: u8,
    pub max_side: Option<u32>,
    pub algorithm: Algorithm,
}

/// Encrypt text and embed it into a JPEG carrier.
pub fn embed_text_in_jpeg(text: &str, password: &str, options: &StegoOptions) -> Result<PathBuf> {
    let source = image::open(&options.carrier)?;
    let mut rgb = source.to_rgb8();
    if let Some(max_side) = options.max_side {
        let (width, height) = rgb.dimensions();
        let longer = width.max(height);
        if longer > max_side {
            let scale = max_side as f64 / longer as f64;
            let new_width = ((width as f64 * scale).round() as u32).max(1);
            let new_height = ((height as f64 * scale).round() as u32).max(1);
            rgb = image::imageops::resize(&rgb, new_width, new_height, FilterType::Lanczos3);
        }
    }
    let (width, height) = rgb.dimensions();
    let mut carrier_jpeg = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
        &mut carrier_jpeg,
        options.quality.clamp(1, 100),
    );
    encoder
        .write_image(rgb.as_raw(), width, height, image::ExtendedColorType::Rgb8)
        .map_err(|error| CryptError::Image(error.to_string()))?;

    let payload = text::encrypt_binary(text, password, options.algorithm, true)?;
    let key = kdf::derive_stego_key(password, width, height)?;
    let embedded = crate::f5::embed::embed_payload(&carrier_jpeg, &payload, &key)?;
    let check = crate::f5::extract_payload(&embedded, &key)?;
    if check != payload {
        return Err(CryptError::Steganography(
            "steganogram verification failed".into(),
        ));
    }

    if let Some(parent) = options.output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&options.output, embedded)?;
    Ok(options.output.clone())
}

/// Extract and decrypt text hidden in a baseline JPEG steganogram.
pub fn extract_text_from_jpeg(input: &Path, password: &str) -> Result<String> {
    let bytes = std::fs::read(input)?;
    let (width, height) = image::image_dimensions(input)?;
    let key = kdf::derive_stego_key(password, width, height)?;
    let payload = crate::f5::extract_payload(&bytes, &key)?;
    text::decrypt_binary(&payload, password)
}
