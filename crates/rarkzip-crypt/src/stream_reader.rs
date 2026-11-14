use std::io::{self, Read, Seek, SeekFrom};

use crate::cipher::ctr::CtrStreams;
use crate::cipher::{BlockCipher, Cipher};

/// Seekable CTR-decrypting reader used to feed the ZIP parser.
pub struct CtrReader<R: Read + Seek> {
    inner: R,
    streams: CtrStreams,
    base: u64,
    start: u64,
    length: u64,
    position: u64,
}

impl<R: Read + Seek> CtrReader<R> {
    /// `base` is the file offset of the ciphertext stream, `start` is the
    /// plaintext offset exposed as byte 0, and `length` is the exposed length.
    pub fn new(inner: R, streams: CtrStreams, base: u64, start: u64, length: u64) -> Self {
        Self {
            inner,
            streams,
            base,
            start,
            length,
            position: 0,
        }
    }

    pub fn inner_mut(&mut self) -> &mut R {
        &mut self.inner
    }
}

impl<R: Read + Seek> Read for CtrReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.length || buffer.is_empty() {
            return Ok(0);
        }
        let want = (self.length - self.position).min(buffer.len() as u64) as usize;
        let plaintext_position = self.start + self.position;
        self.inner
            .seek(SeekFrom::Start(self.base + plaintext_position))?;
        let read = self.inner.read(&mut buffer[..want])?;
        if read == 0 {
            return Ok(0);
        }
        self.streams
            .apply_at(plaintext_position, &mut buffer[..read]);
        self.position += read as u64;
        Ok(read)
    }
}

impl<R: Read + Seek> Seek for CtrReader<R> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let next = match position {
            SeekFrom::Start(value) => value as i128,
            SeekFrom::Current(delta) => self.position as i128 + delta as i128,
            SeekFrom::End(delta) => self.length as i128 + delta as i128,
        };
        if next < 0 || next > self.length as i128 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek outside decrypted stream",
            ));
        }
        self.position = next as u64;
        Ok(self.position)
    }
}

/// Seekable CBC-decrypting reader for the legacy v2 container.
pub struct CbcReader<R: Read + Seek> {
    inner: R,
    cipher: Cipher,
    iv: Vec<u8>,
    block: usize,
    base: u64,
    start: u64,
    length: u64,
    position: u64,
}

impl<R: Read + Seek> CbcReader<R> {
    pub fn new(inner: R, cipher: Cipher, iv: Vec<u8>, base: u64, start: u64, length: u64) -> Self {
        let block = cipher.block_size();
        Self {
            inner,
            cipher,
            iv,
            block,
            base,
            start,
            length,
            position: 0,
        }
    }

    pub fn block_size(&self) -> usize {
        self.block
    }

    /// Decrypt one full CBC block by index.
    pub fn decrypt_block_at(&mut self, block_index: u64) -> io::Result<Vec<u8>> {
        let block = self.block;
        let mut ciphertext = vec![0u8; block];
        self.inner
            .seek(SeekFrom::Start(self.base + block_index * block as u64))?;
        self.inner.read_exact(&mut ciphertext)?;
        let previous = if block_index == 0 {
            self.iv.clone()
        } else {
            let mut previous = vec![0u8; block];
            self.inner.seek(SeekFrom::Start(
                self.base + (block_index - 1) * block as u64,
            ))?;
            self.inner.read_exact(&mut previous)?;
            previous
        };
        let mut plaintext = ciphertext;
        self.cipher.decrypt_block(&mut plaintext);
        for (byte, previous) in plaintext.iter_mut().zip(previous) {
            *byte ^= previous;
        }
        Ok(plaintext)
    }
}

impl<R: Read + Seek> Read for CbcReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.length || buffer.is_empty() {
            return Ok(0);
        }
        let want = (self.length - self.position).min(buffer.len() as u64) as usize;
        let mut written = 0usize;
        while written < want {
            let absolute = self.start + self.position;
            let block_index = absolute / self.block as u64;
            let intra = (absolute % self.block as u64) as usize;
            let plaintext = self.decrypt_block_at(block_index)?;
            let take = (self.block - intra).min(want - written);
            buffer[written..written + take].copy_from_slice(&plaintext[intra..intra + take]);
            written += take;
            self.position += take as u64;
        }
        Ok(written)
    }
}

impl<R: Read + Seek> Seek for CbcReader<R> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let next = match position {
            SeekFrom::Start(value) => value as i128,
            SeekFrom::Current(delta) => self.position as i128 + delta as i128,
            SeekFrom::End(delta) => self.length as i128 + delta as i128,
        };
        if next < 0 || next > self.length as i128 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek outside decrypted stream",
            ));
        }
        self.position = next as u64;
        Ok(self.position)
    }
}
