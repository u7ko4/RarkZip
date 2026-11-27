use constant_time_eq::constant_time_eq;

use crate::algorithm::Algorithm;
use crate::cipher::ctr::CtrStream;
use crate::cipher::{BlockCipher, Cipher};
use crate::error::{CryptError, Result};

/// Encrypt with EAX. Returns `ciphertext || tag`.
pub fn encrypt(
    algorithm: Algorithm,
    key: &[u8],
    nonce: &[u8],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    if let Some(layers) = algorithm.layers() {
        let (last, prefix) = layers.split_last().expect("composite has layers");
        let mut data = plaintext.to_vec();
        apply_prefix_ctr(prefix, key, nonce, &mut data)?;
        let key_offset: usize =
            layers.iter().map(|layer| layer.key_bytes).sum::<usize>() - last.key_bytes;
        let nonce_offset: usize =
            layers.iter().map(|layer| layer.nonce_bytes).sum::<usize>() - last.nonce_bytes;
        let cipher = Cipher::new(
            last.algorithm,
            &key[key_offset..key_offset + last.key_bytes],
        )?;
        return eax_with_cipher(
            &cipher,
            &nonce[nonce_offset..nonce_offset + last.nonce_bytes],
            aad,
            &data,
            algorithm.tag_bytes(),
        );
    }
    let cipher = Cipher::new(algorithm, key)?;
    eax_with_cipher(&cipher, nonce, aad, plaintext, algorithm.tag_bytes())
}

/// Decrypt an EAX `ciphertext || tag` buffer.
pub fn decrypt(
    algorithm: Algorithm,
    key: &[u8],
    nonce: &[u8],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
) -> Result<Vec<u8>> {
    if ciphertext_and_tag.len() < algorithm.tag_bytes() {
        return Err(CryptError::Corrupted("EAX ciphertext too short".into()));
    }
    if let Some(layers) = algorithm.layers() {
        let (last, prefix) = layers.split_last().expect("composite has layers");
        let key_offset: usize =
            layers.iter().map(|layer| layer.key_bytes).sum::<usize>() - last.key_bytes;
        let nonce_offset: usize =
            layers.iter().map(|layer| layer.nonce_bytes).sum::<usize>() - last.nonce_bytes;
        let cipher = Cipher::new(
            last.algorithm,
            &key[key_offset..key_offset + last.key_bytes],
        )?;
        let mut data = eax_decrypt_with_cipher(
            &cipher,
            &nonce[nonce_offset..nonce_offset + last.nonce_bytes],
            aad,
            ciphertext_and_tag,
            algorithm.tag_bytes(),
        )?;
        apply_prefix_ctr(prefix, key, nonce, &mut data)?;
        return Ok(data);
    }
    let cipher = Cipher::new(algorithm, key)?;
    eax_decrypt_with_cipher(
        &cipher,
        nonce,
        aad,
        ciphertext_and_tag,
        algorithm.tag_bytes(),
    )
}

fn apply_prefix_ctr(
    layers: &[crate::algorithm::Layer],
    key: &[u8],
    nonce: &[u8],
    data: &mut [u8],
) -> Result<()> {
    let mut key_offset = 0usize;
    let mut nonce_offset = 0usize;
    for layer in layers {
        let cipher = Cipher::new(
            layer.algorithm,
            &key[key_offset..key_offset + layer.key_bytes],
        )?;
        let mut stream = CtrStream::new(
            cipher,
            &nonce[nonce_offset..nonce_offset + layer.nonce_bytes],
        )?;
        stream.apply(data);
        key_offset += layer.key_bytes;
        nonce_offset += layer.nonce_bytes;
    }
    Ok(())
}

fn eax_with_cipher(
    cipher: &Cipher,
    nonce: &[u8],
    aad: &[u8],
    plaintext: &[u8],
    tag_len: usize,
) -> Result<Vec<u8>> {
    let block = cipher.block_size();
    let nonce_mac = cmac(cipher, &with_domain(0, block, nonce));
    let aad_mac = cmac(cipher, &with_domain(1, block, aad));
    let mut output = plaintext.to_vec();
    ctr_xor(cipher, &nonce_mac, &mut output);
    let cipher_mac = cmac(cipher, &with_domain(2, block, &output));
    let mut tag = vec![0u8; block];
    for index in 0..block {
        tag[index] = nonce_mac[index] ^ aad_mac[index] ^ cipher_mac[index];
    }
    tag.truncate(tag_len);
    output.extend_from_slice(&tag);
    Ok(output)
}

fn eax_decrypt_with_cipher(
    cipher: &Cipher,
    nonce: &[u8],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
    tag_len: usize,
) -> Result<Vec<u8>> {
    let block = cipher.block_size();
    let split = ciphertext_and_tag.len() - tag_len;
    let ciphertext = &ciphertext_and_tag[..split];
    let tag = &ciphertext_and_tag[split..];
    let nonce_mac = cmac(cipher, &with_domain(0, block, nonce));
    let aad_mac = cmac(cipher, &with_domain(1, block, aad));
    let cipher_mac = cmac(cipher, &with_domain(2, block, ciphertext));
    let mut expected = vec![0u8; block];
    for index in 0..block {
        expected[index] = nonce_mac[index] ^ aad_mac[index] ^ cipher_mac[index];
    }
    if !constant_time_eq(&expected[..tag_len], tag) {
        return Err(CryptError::WrongPassword);
    }
    let mut output = ciphertext.to_vec();
    ctr_xor(cipher, &nonce_mac, &mut output);
    Ok(output)
}

fn with_domain(domain: u8, block: usize, data: &[u8]) -> Vec<u8> {
    let mut output = vec![0u8; block];
    output[block - 1] = domain;
    output.extend_from_slice(data);
    output
}

fn ctr_xor(cipher: &Cipher, iv: &[u8], data: &mut [u8]) {
    let block = cipher.block_size();
    let mut counter = iv.to_vec();
    let mut offset = 0usize;
    while offset < data.len() {
        let mut keystream = counter.clone();
        cipher.encrypt_block(&mut keystream);
        let take = block.min(data.len() - offset);
        for index in 0..take {
            data[offset + index] ^= keystream[index];
        }
        add_counter(&mut counter, 1);
        offset += take;
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

fn cmac(cipher: &Cipher, message: &[u8]) -> Vec<u8> {
    let block = cipher.block_size();
    let mut zero = vec![0u8; block];
    cipher.encrypt_block(&mut zero);
    let k1 = double_block(&zero);
    let k2 = double_block(&k1);

    let block_count = message.len().div_ceil(block).max(1);
    let mut state = vec![0u8; block];
    for index in 0..block_count {
        let start = index * block;
        let end = (start + block).min(message.len());
        let mut last = vec![0u8; block];
        last[..end - start].copy_from_slice(&message[start..end]);
        if index + 1 == block_count {
            if end - start == block {
                xor_in_place(&mut last, &k1);
            } else {
                last[end - start] = 0x80;
                xor_in_place(&mut last, &k2);
            }
        }
        xor_in_place(&mut state, &last);
        cipher.encrypt_block(&mut state);
    }
    state
}

fn double_block(block: &[u8]) -> Vec<u8> {
    let mut output = vec![0u8; block.len()];
    let mut carry = 0u8;
    for index in (0..block.len()).rev() {
        let value = block[index];
        output[index] = (value << 1) | carry;
        carry = value >> 7;
    }
    if carry == 1 {
        let polynomial: u32 = match block.len() {
            8 => 0x1b,
            16 => 0x87,
            32 => 0x425,
            128 => 0x0008_0043,
            _ => 0x87,
        };
        let length = block.len();
        output[length - 1] ^= (polynomial & 0xff) as u8;
        if length >= 2 {
            output[length - 2] ^= ((polynomial >> 8) & 0xff) as u8;
        }
        if length >= 3 {
            output[length - 3] ^= ((polynomial >> 16) & 0xff) as u8;
        }
    }
    output
}

fn xor_in_place(target: &mut [u8], other: &[u8]) {
    for (left, right) in target.iter_mut().zip(other) {
        *left ^= right;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aes_eax_round_trip() {
        let key = [0x42u8; 32];
        let nonce = [0x24u8; 16];
        let aad = b"header";
        let ciphertext = encrypt(Algorithm::Aes256, &key, &nonce, aad, b"hello eax").unwrap();
        let plaintext = decrypt(Algorithm::Aes256, &key, &nonce, aad, &ciphertext).unwrap();
        assert_eq!(plaintext, b"hello eax");
    }

    #[test]
    fn composite_round_trip() {
        let key = [0x42u8; 256];
        let nonce = [0x24u8; 192];
        let aad = b"header";
        let ciphertext = encrypt(
            Algorithm::Composite2048,
            &key,
            &nonce,
            aad,
            b"hello composite",
        )
        .unwrap();
        let plaintext = decrypt(Algorithm::Composite2048, &key, &nonce, aad, &ciphertext).unwrap();
        assert_eq!(plaintext, b"hello composite");
    }
}
