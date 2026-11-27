use crate::error::{CryptError, Result};
use crate::f5::prng::{F5Random, Permutation};
use crate::f5::{bit, read_image, write_image};

fn is_usable(flat: &[i16], index: usize) -> bool {
    index % 64 != 0 && flat[index] != 0
}

fn set_parity(flat: &mut [i16], index: usize, desired: u32) {
    let value = flat[index];
    if bit(value) == desired {
        return;
    }
    if value > 0 {
        flat[index] = value - 1;
    } else {
        flat[index] = value + 1;
    }
}

fn select_k(flat: &[i16], payload_len: usize) -> (usize, usize) {
    let coeff_count = flat.len();
    let mut zero = 0i64;
    let mut one = 0i64;
    for (index, value) in flat.iter().enumerate() {
        if index % 64 == 0 {
            continue;
        }
        if *value == 1 || *value == -1 {
            one += 1;
        }
        if *value == 0 {
            zero += 1;
        }
    }
    let large = coeff_count as i64 - zero - one - (coeff_count / 64) as i64;
    let expected = large + (0.49 * one as f64) as i64;
    let mut i = 1usize;
    while i < 8 {
        let n = (1i64 << i) - 1;
        let usable = expected * i as i64 / n - (expected * i as i64 / n) % n;
        if usable / 8 == 0 || usable / 8 < payload_len as i64 + 4 {
            break;
        }
        i += 1;
    }
    let k = i.saturating_sub(1);
    let n = if k == 0 { 1 } else { (1usize << k) - 1 };
    (k, n)
}

/// Embed a raw payload into the coefficients of a baseline JPEG.
pub fn embed_payload(jpeg: &[u8], payload: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    let mut image = read_image(jpeg)?;
    let (k, n) = select_k(&image.flat, payload.len());
    let capacity_bytes = image.flat.len() / 8;
    if payload.len() > capacity_bytes {
        return Err(CryptError::Steganography(
            "carrier image has insufficient capacity".into(),
        ));
    }
    let mut random = F5Random::new(key);
    let permutation = Permutation::new(image.flat.len(), &mut random);

    let mut header = payload.len() as i32 | ((k as i32) << 24);
    header ^= random.next_byte() as i8 as i32;
    header ^= (random.next_byte() as i8 as i32) << 8;
    header ^= (random.next_byte() as i8 as i32) << 16;
    header ^= (random.next_byte() as i8 as i32) << 24;

    let mut position = 0usize;
    let mut header_bits = 0usize;
    while header_bits < 32 {
        if position >= permutation.len() {
            return Err(CryptError::Steganography(
                "carrier image has insufficient capacity".into(),
            ));
        }
        let index = permutation.get(position);
        if is_usable(&image.flat, index) {
            let desired = ((header >> header_bits) & 1) as u32;
            set_parity(&mut image.flat, index, desired);
            if image.flat[index] != 0 {
                header_bits += 1;
            }
        }
        position += 1;
    }

    let mut payload_bits = Vec::with_capacity(payload.len() * 8);
    for byte in payload {
        let value = *byte ^ random.next_byte();
        for shift in 0..8 {
            payload_bits.push(((value >> shift) & 1) as u32);
        }
    }

    let mut start = position;
    for chunk in payload_bits.chunks(k.max(1)) {
        let mut word = 0u32;
        for (offset, bit) in chunk.iter().enumerate() {
            word |= *bit << offset;
        }
        loop {
            if start >= permutation.len() {
                return Err(CryptError::Steganography(
                    "carrier image has insufficient capacity".into(),
                ));
            }
            let mut group = Vec::with_capacity(n);
            let mut scan = start;
            while group.len() < n {
                if scan >= permutation.len() {
                    return Err(CryptError::Steganography(
                        "carrier image has insufficient capacity".into(),
                    ));
                }
                let index = permutation.get(scan);
                if is_usable(&image.flat, index) {
                    group.push(index);
                }
                scan += 1;
            }
            let mut hash = 0u32;
            for (offset, index) in group.iter().enumerate() {
                if bit(image.flat[*index]) == 1 {
                    hash ^= (offset + 1) as u32;
                }
            }
            let delta = (hash ^ word) as usize;
            if delta == 0 {
                start = scan;
                break;
            }
            let index = group[delta - 1];
            let desired = 1 - bit(image.flat[index]);
            set_parity(&mut image.flat, index, desired);
            if image.flat[index] != 0 {
                start = scan;
                break;
            }
            // Shrinkage: the coefficient became zero, so recompute the group
            // from the same start position with one fewer usable coefficient.
        }
    }

    write_image(jpeg, &mut image)
}
