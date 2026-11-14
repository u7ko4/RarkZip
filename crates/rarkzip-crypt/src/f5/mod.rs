use dct_io::{JpegCoefficients, JpegInfo};

use crate::error::{CryptError, Result};

pub mod embed;
pub mod prng;

pub struct CoefficientImage {
    pub info: JpegInfo,
    pub coefficients: JpegCoefficients,
    /// Natural-order quantized coefficients, flattened in JPEG MCU scan order.
    pub flat: Vec<i16>,
}

pub fn read_image(jpeg: &[u8]) -> Result<CoefficientImage> {
    let info = dct_io::inspect(jpeg)
        .map_err(|error| CryptError::Steganography(format!("JPEG inspect: {error}")))?;
    let coefficients = dct_io::read_coefficients(jpeg)
        .map_err(|error| CryptError::Steganography(format!("JPEG coefficients: {error}")))?;
    let (mcu_cols, mcu_rows) = mcu_geometry(&info);
    let mut pointers = vec![0usize; info.components.len()];
    let mut flat = Vec::new();
    for _ in 0..mcu_cols * mcu_rows {
        for (component_index, component) in info.components.iter().enumerate() {
            for _ in 0..component.h_samp as usize * component.v_samp as usize {
                let block = coefficients.components[component_index]
                    .blocks
                    .get(pointers[component_index])
                    .ok_or_else(|| {
                        CryptError::Steganography("JPEG block index out of range".into())
                    })?;
                pointers[component_index] += 1;
                flat.extend_from_slice(block);
            }
        }
    }
    Ok(CoefficientImage {
        info,
        coefficients,
        flat,
    })
}

pub fn write_image(jpeg: &[u8], image: &mut CoefficientImage) -> Result<Vec<u8>> {
    let (mcu_cols, mcu_rows) = mcu_geometry(&image.info);
    let mut pointers = vec![0usize; image.info.components.len()];
    let mut offset = 0usize;
    for _ in 0..mcu_cols * mcu_rows {
        for (component_index, component) in image.info.components.iter().enumerate() {
            for _ in 0..component.h_samp as usize * component.v_samp as usize {
                let block: [i16; 64] = image
                    .flat
                    .get(offset..offset + 64)
                    .ok_or_else(|| CryptError::Steganography("coefficient array too short".into()))?
                    .try_into()
                    .expect("64 coefficients");
                if let Some(target) = image.coefficients.components[component_index]
                    .blocks
                    .get_mut(pointers[component_index])
                {
                    *target = block;
                }
                pointers[component_index] += 1;
                offset += 64;
            }
        }
    }
    dct_io::write_coefficients(jpeg, &image.coefficients)
        .map_err(|error| CryptError::Steganography(format!("JPEG encode: {error}")))
}

fn mcu_geometry(info: &JpegInfo) -> (usize, usize) {
    let max_h = info
        .components
        .iter()
        .map(|component| component.h_samp as usize)
        .max()
        .unwrap_or(1);
    let max_v = info
        .components
        .iter()
        .map(|component| component.v_samp as usize)
        .max()
        .unwrap_or(1);
    let mcu_cols = (info.width as usize).div_ceil(max_h * 8);
    let mcu_rows = (info.height as usize).div_ceil(max_v * 8);
    (mcu_cols, mcu_rows)
}

fn bit(value: i16) -> u32 {
    if value > 0 {
        (value & 1) as u32
    } else {
        1 - ((value & 1) as u32)
    }
}

/// Extract the raw F5 payload from a baseline JPEG.
pub fn extract_payload(jpeg: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    let image = read_image(jpeg)?;
    let mut random = prng::F5Random::new(key);
    let permutation = prng::Permutation::new(image.flat.len(), &mut random);

    let mut index = 0usize;
    let mut available_bits = 0u32;
    let mut length: u32 = 0;
    while available_bits < 32 {
        if index >= permutation.len() {
            return Err(CryptError::Steganography(
                "no F5 header found in image".into(),
            ));
        }
        let shuffled = permutation.get(index);
        if shuffled % 64 != 0 && image.flat[shuffled] != 0 {
            length |= bit(image.flat[shuffled]) << available_bits;
            available_bits += 1;
        }
        index += 1;
    }
    let mut header = length as i32;
    header ^= random.next_byte() as i8 as i32;
    header ^= (random.next_byte() as i8 as i32) << 8;
    header ^= (random.next_byte() as i8 as i32) << 16;
    header ^= (random.next_byte() as i8 as i32) << 24;
    let k = ((header >> 24) & 0x1f) as usize;
    let n = if k == 0 { 0usize } else { (1usize << k) - 1 };
    let payload_length = (header & 0x7F_FFFF) as usize;
    let mut output = Vec::with_capacity(payload_length);

    if n == 0 {
        let mut byte = 0u32;
        let mut bits = 0u32;
        while index < permutation.len() {
            let shuffled = permutation.get(index);
            if shuffled % 64 != 0 && image.flat[shuffled] != 0 {
                byte |= bit(image.flat[shuffled]) << bits;
                bits += 1;
                if bits == 8 {
                    output.push((byte ^ random.next_byte() as u32) as u8);
                    byte = 0;
                    bits = 0;
                    if output.len() == payload_length {
                        break;
                    }
                }
            }
            index += 1;
        }
    } else {
        let mut start_of_n = index;
        let mut finished = false;
        let mut byte = 0u32;
        let mut bits = 0u32;
        while !finished {
            let mut hash = 0u32;
            let mut code = 1usize;
            let mut used = 0usize;
            while code <= n {
                if start_of_n + used >= permutation.len() {
                    return Err(CryptError::Steganography("incomplete F5 payload".into()));
                }
                let shuffled = permutation.get(start_of_n + used);
                if shuffled % 64 != 0 && image.flat[shuffled] != 0 {
                    if bit(image.flat[shuffled]) == 1 {
                        hash ^= code as u32;
                    }
                    code += 1;
                }
                used += 1;
            }
            start_of_n += used;

            for bit_index in 0..k {
                byte |= ((hash >> bit_index) & 1) << bits;
                bits += 1;
                if bits == 8 {
                    output.push((byte ^ random.next_byte() as u32) as u8);
                    byte = 0;
                    bits = 0;
                    if output.len() == payload_length {
                        finished = true;
                        break;
                    }
                }
            }
            if start_of_n >= permutation.len() && output.len() < payload_length {
                return Err(CryptError::Steganography("incomplete F5 payload".into()));
            }
        }
    }

    if output.len() != payload_length {
        return Err(CryptError::Steganography(format!(
            "incomplete F5 payload: {} of {} bytes",
            output.len(),
            payload_length
        )));
    }
    Ok(output)
}
