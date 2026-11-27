use std::fs::{self, File};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use blake2::digest::{KeyInit, Mac};
use constant_time_eq::constant_time_eq;
use tempfile::Builder;
use walkdir::WalkDir;

use crate::algorithm::Algorithm;
use crate::cipher::ctr::CtrStreams;
use crate::cipher::Cipher;
use crate::error::{CryptError, Result};
use crate::format::{
    self, encode_file_header, parse_file_header, FileVersion, CHECKCODE_LEN, HEADER_LEN, MAC_LEN,
    SALT_LEN,
};
use crate::kdf;
use crate::progress::{check_cancelled, Phase, Progress, Reporter};
use crate::stream_reader::{CbcReader, CtrReader};
use crate::util;
use crate::zip_writer::{ZipCompression, ZipWriter};

const IO_BUFFER: usize = 64 * 1024;
const MAC_MEMORY_CHUNK: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileCompression {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone)]
pub struct FileEncryptOptions {
    pub algorithm: Algorithm,
    pub compression: FileCompression,
    pub output_dir: Option<PathBuf>,
}

impl Default for FileEncryptOptions {
    fn default() -> Self {
        Self {
            algorithm: Algorithm::Aes256,
            compression: FileCompression::Auto,
            output_dir: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct FileDecryptOptions {
    pub output_dir: Option<PathBuf>,
}

struct Entry {
    name: String,
    path: Option<PathBuf>,
    is_dir: bool,
    size: u64,
    mtime: SystemTime,
}

/// Encrypt a file or directory into the compatible `.enc` container.
pub fn encrypt_file(
    input: &Path,
    password: &str,
    options: &FileEncryptOptions,
    reporter: &dyn Reporter,
) -> Result<PathBuf> {
    check_cancelled(reporter)?;
    let metadata = fs::metadata(input)?;
    let (entries, total) = collect_entries(input, &metadata)?;
    if entries.is_empty() {
        return Err(CryptError::Other("nothing to encrypt".into()));
    }
    let compression = choose_compression(input, &entries, options.compression);
    let zip_compression = if compression {
        ZipCompression::Deflate
    } else {
        ZipCompression::Store
    };

    let output_dir = options
        .output_dir
        .clone()
        .or_else(|| input.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    fs::create_dir_all(&output_dir)?;
    let file_name = input
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());
    let output = util::unique_path(&output_dir.join(format!("{file_name}.enc")));
    let temporary = PathBuf::from(format!("{}.unfinished", output.display()));
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }

    let result = (|| -> Result<()> {
        reporter.progress(Progress::new(
            Phase::Preparing,
            0,
            total,
            Some(file_name.clone()),
        ));
        let header = encode_file_header(options.algorithm);
        let salt = util::random_bytes(SALT_LEN)?;
        let (iterations, memory) = kdf::argon2_params(format::DEFAULT_CUSTOM_PARAMS)?;
        let keys = kdf::derive_file_keys(
            password,
            &salt,
            options.algorithm.key_bytes(),
            options.algorithm.nonce_bytes(),
            32,
            iterations,
            memory,
        )?;
        let checkcode = util::random_checkcode()?;

        let file = BufWriter::with_capacity(1024 * 1024, File::create(&temporary)?);
        let mut writer = file;
        writer.write_all(&header)?;
        writer.write_all(&salt)?;

        let auth_key: &[u8; 32] = keys
            .auth_key
            .as_slice()
            .try_into()
            .map_err(|_| CryptError::Crypto("invalid MAC key length".into()))?;
        let mut mac = blake3::Hasher::new_keyed(auth_key);
        mac.update(&keys.nonce);
        mac.update(&header);
        mac.update(&salt);

        let streams = CtrStreams::new(options.algorithm, &keys.key, &keys.nonce)?;
        let mut writer = MacCtrWriter::new(writer, streams, mac);
        writer.write_all(&checkcode)?;

        {
            let mut zip = ZipWriter::new(&mut writer, zip_compression, reporter, total);
            for entry in &entries {
                check_cancelled(reporter)?;
                if entry.is_dir {
                    zip.add_directory(&entry.name, entry.mtime)?;
                } else if let Some(path) = &entry.path {
                    zip.add_file(&entry.name, path, entry.mtime)?;
                }
            }
            zip.finish()?;
        }

        let (mut file, mac) = writer.finish();
        file.write_all(&mac)?;
        file.flush()?;
        drop(file);

        reporter.progress(Progress::new(
            Phase::Finished,
            total,
            total,
            Some(file_name.clone()),
        ));
        fs::rename(&temporary, &output)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok(output)
}

/// Decrypt a compatible `.enc` container. The MAC (or legacy checkcode) is
/// verified before the ZIP payload is extracted into a temporary directory.
pub fn decrypt_file(
    input: &Path,
    password: &str,
    options: &FileDecryptOptions,
    reporter: &dyn Reporter,
) -> Result<PathBuf> {
    check_cancelled(reporter)?;
    let mut file = File::open(input)?;
    let file_len = file.metadata()?.len();
    if file_len < (HEADER_LEN + SALT_LEN) as u64 {
        return Err(CryptError::Corrupted("file is too short".into()));
    }
    let mut header_bytes = [0u8; HEADER_LEN];
    file.read_exact(&mut header_bytes)?;
    let header = parse_file_header(&header_bytes)?;
    let mut salt = [0u8; SALT_LEN];
    file.read_exact(&mut salt)?;
    let output_dir = options
        .output_dir
        .clone()
        .or_else(|| input.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    fs::create_dir_all(&output_dir)?;
    let stem = input
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "extracted".to_string());

    match header.version {
        FileVersion::V2 => decrypt_v2(
            &mut file,
            file_len,
            &header,
            &salt,
            password,
            &output_dir,
            &stem,
            reporter,
        ),
        FileVersion::V3 | FileVersion::V4 => decrypt_v3_v4(
            &mut file,
            file_len,
            &header_bytes,
            &header,
            &salt,
            password,
            &output_dir,
            &stem,
            reporter,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn decrypt_v3_v4(
    file: &mut File,
    file_len: u64,
    header_bytes: &[u8; HEADER_LEN],
    header: &format::FileHeader,
    salt: &[u8; SALT_LEN],
    password: &str,
    output_dir: &Path,
    stem: &str,
    reporter: &dyn Reporter,
) -> Result<PathBuf> {
    let cipher_len = file_len
        .checked_sub((HEADER_LEN + SALT_LEN + MAC_LEN) as u64)
        .ok_or_else(|| CryptError::Corrupted("file is too short".into()))?;
    if cipher_len < CHECKCODE_LEN as u64 {
        return Err(CryptError::Corrupted("ciphertext is too short".into()));
    }
    let (iterations, memory) = kdf::argon2_params(header.custom_params)?;
    let auth_bytes = if header.version == FileVersion::V4 {
        32
    } else {
        64
    };
    let keys = kdf::derive_file_keys(
        password,
        salt,
        header.algorithm.key_bytes(),
        header.algorithm.nonce_bytes(),
        auth_bytes,
        iterations,
        memory,
    )?;

    reporter.progress(Progress::new(
        Phase::Verifying,
        0,
        file_len,
        Some("MAC".to_string()),
    ));
    verify_mac(
        file,
        header_bytes,
        salt,
        header.algorithm,
        &keys.key,
        &keys.nonce,
        &keys.auth_key,
        header.version,
        cipher_len,
    )?;

    let streams = CtrStreams::new(header.algorithm, &keys.key, &keys.nonce)?;
    let zip_len = cipher_len - CHECKCODE_LEN as u64;
    let reader = CtrReader::new(
        file.try_clone()?,
        streams,
        40,
        CHECKCODE_LEN as u64,
        zip_len,
    );
    let temp = Builder::new()
        .prefix(".rarkzip-crypt-")
        .tempdir_in(output_dir)?;
    extract_zip(reader, temp.path(), reporter)?;
    reporter.progress(Progress::new(Phase::Finished, file_len, file_len, None));
    publish_temp_dir(temp.path(), output_dir, stem)
}

#[allow(clippy::too_many_arguments)]
fn decrypt_v2(
    file: &mut File,
    file_len: u64,
    header: &format::FileHeader,
    salt: &[u8; SALT_LEN],
    password: &str,
    output_dir: &Path,
    stem: &str,
    reporter: &dyn Reporter,
) -> Result<PathBuf> {
    let cipher_len = file_len
        .checked_sub((HEADER_LEN + SALT_LEN) as u64)
        .ok_or_else(|| CryptError::Corrupted("file is too short".into()))?;
    let block = header.algorithm.block_bytes();
    if cipher_len == 0 || cipher_len % block as u64 != 0 {
        return Err(CryptError::Corrupted(
            "invalid legacy ciphertext length".into(),
        ));
    }
    let keys = kdf::derive_legacy_file_keys(
        password,
        salt,
        header.algorithm.key_bytes(),
        header.algorithm.nonce_bytes(),
    )?;
    let cipher = Cipher::new(header.algorithm, &keys.key)?;
    let mut reader = CbcReader::new(
        file.try_clone()?,
        cipher,
        keys.nonce.to_vec(),
        40,
        0,
        cipher_len,
    );
    let last_index = cipher_len / block as u64 - 1;
    let last = reader.decrypt_block_at(last_index)?;
    let raw_padding = *last.last().unwrap_or(&0) as i8;
    let padding = if raw_padding == -128 {
        128u64
    } else {
        raw_padding.max(0) as u64
    };
    if padding == 0 || padding > block as u64 {
        return Err(CryptError::WrongPassword);
    }
    let plain_len = cipher_len - padding;
    if plain_len < (CHECKCODE_LEN * 2) as u64 {
        return Err(CryptError::Corrupted("legacy plaintext too short".into()));
    }
    let mut first = vec![0u8; CHECKCODE_LEN];
    reader.seek(SeekFrom::Start(0))?;
    reader.read_exact(&mut first)?;
    let mut last_code = vec![0u8; CHECKCODE_LEN];
    reader.seek(SeekFrom::Start(plain_len - CHECKCODE_LEN as u64))?;
    reader.read_exact(&mut last_code)?;
    if !constant_time_eq(&first, &last_code) || !util::is_valid_checkcode(&first) {
        return Err(CryptError::WrongPassword);
    }

    let cipher = Cipher::new(header.algorithm, &keys.key)?;
    let zip_len = plain_len - (CHECKCODE_LEN * 2) as u64;
    let reader = CbcReader::new(
        file.try_clone()?,
        cipher,
        keys.nonce.to_vec(),
        40,
        CHECKCODE_LEN as u64,
        zip_len,
    );
    let temp = Builder::new()
        .prefix(".rarkzip-crypt-")
        .tempdir_in(output_dir)?;
    extract_zip(reader, temp.path(), reporter)?;
    reporter.progress(Progress::new(Phase::Finished, file_len, file_len, None));
    publish_temp_dir(temp.path(), output_dir, stem)
}

#[allow(clippy::too_many_arguments)]
fn verify_mac(
    file: &mut File,
    header: &[u8; HEADER_LEN],
    salt: &[u8; SALT_LEN],
    algorithm: Algorithm,
    key: &[u8],
    nonce: &[u8],
    auth_key: &[u8],
    version: FileVersion,
    cipher_len: u64,
) -> Result<()> {
    let checkcode = decrypt_checkcode(file, algorithm, key, nonce, cipher_len)?;
    if !util::is_valid_checkcode(&checkcode) {
        return Err(CryptError::WrongPassword);
    }

    let computed = match version {
        FileVersion::V4 => {
            let key: &[u8; 32] = auth_key
                .try_into()
                .map_err(|_| CryptError::Crypto("invalid MAC key length".into()))?;
            let mut mac = blake3::Hasher::new_keyed(key);
            mac.update(nonce);
            mac.update(header);
            mac.update(salt);
            let mut reader = FileChunkReader::new(file, 40, cipher_len)?;
            let mut buffer = vec![0u8; MAC_MEMORY_CHUNK];
            loop {
                let read = reader.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                mac.update(&buffer[..read]);
            }
            *mac.finalize().as_bytes()
        }
        FileVersion::V3 => {
            type Blake2bMac256 = blake2::Blake2bMac<blake2::digest::consts::U32>;
            let mut mac = <Blake2bMac256 as KeyInit>::new_from_slice(auth_key)
                .map_err(|error| CryptError::Crypto(format!("BLAKE2b key: {error}")))?;
            mac.update(nonce);
            mac.update(header);
            mac.update(salt);
            let mut reader = FileChunkReader::new(file, 40, cipher_len)?;
            let mut buffer = vec![0u8; MAC_MEMORY_CHUNK];
            loop {
                let read = reader.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                mac.update(&buffer[..read]);
            }
            let output = mac.finalize().into_bytes();
            let mut value = [0u8; 32];
            value.copy_from_slice(&output);
            value
        }
        FileVersion::V2 => return Err(CryptError::UnsupportedVersion(2)),
    };
    let mut expected = [0u8; MAC_LEN];
    file.seek(SeekFrom::Start(40 + cipher_len))?;
    file.read_exact(&mut expected)?;
    if !constant_time_eq(&computed, &expected) {
        return Err(CryptError::Corrupted("authentication code mismatch".into()));
    }
    Ok(())
}

fn decrypt_checkcode(
    file: &mut File,
    algorithm: Algorithm,
    key: &[u8],
    nonce: &[u8],
    cipher_len: u64,
) -> Result<Vec<u8>> {
    if cipher_len < CHECKCODE_LEN as u64 {
        return Err(CryptError::Corrupted("ciphertext is too short".into()));
    }
    let mut checkcode = vec![0u8; CHECKCODE_LEN];
    file.seek(SeekFrom::Start(40))?;
    file.read_exact(&mut checkcode)?;
    let mut streams = CtrStreams::new(algorithm, key, nonce)?;
    streams.apply_at(0, &mut checkcode);
    Ok(checkcode)
}

struct FileChunkReader<'a> {
    file: &'a mut File,
    remaining: u64,
}

impl<'a> FileChunkReader<'a> {
    fn new(file: &'a mut File, offset: u64, length: u64) -> Result<Self> {
        file.seek(SeekFrom::Start(offset))?;
        Ok(Self {
            file,
            remaining: length,
        })
    }
}

impl Read for FileChunkReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let want = (self.remaining).min(buffer.len() as u64) as usize;
        let read = self.file.read(&mut buffer[..want])?;
        self.remaining -= read as u64;
        Ok(read)
    }
}

struct MacCtrWriter<W: Write> {
    inner: W,
    streams: CtrStreams,
    mac: blake3::Hasher,
}

impl<W: Write> MacCtrWriter<W> {
    fn new(inner: W, streams: CtrStreams, mac: blake3::Hasher) -> Self {
        Self {
            inner,
            streams,
            mac,
        }
    }

    fn finish(self) -> (W, [u8; 32]) {
        (self.inner, *self.mac.finalize().as_bytes())
    }
}

impl<W: Write> Write for MacCtrWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let mut encrypted = buffer.to_vec();
        self.streams.apply(&mut encrypted);
        self.mac.update(&encrypted);
        self.inner.write_all(&encrypted)?;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn collect_entries(input: &Path, metadata: &fs::Metadata) -> Result<(Vec<Entry>, u64)> {
    let mut entries = Vec::new();
    let mut total = 0u64;
    if metadata.is_file() {
        let name = input
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| CryptError::Other("invalid input file name".into()))?;
        total = metadata.len();
        entries.push(Entry {
            name,
            path: Some(input.to_path_buf()),
            is_dir: false,
            size: total,
            mtime: metadata.modified().unwrap_or_else(|_| SystemTime::now()),
        });
        return Ok((entries, total));
    }
    let top = input
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| CryptError::Other("invalid input directory name".into()))?;
    for entry in WalkDir::new(input).follow_links(false) {
        let entry = entry.map_err(|error| CryptError::Other(error.to_string()))?;
        if entry.file_type().is_symlink() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(input)
            .map_err(|error| CryptError::Other(error.to_string()))?;
        let name = if relative.as_os_str().is_empty() {
            top.clone()
        } else {
            format!("{top}/{}", relative.to_string_lossy().replace('\\', "/"))
        };
        let entry_meta = entry
            .metadata()
            .map_err(|error| CryptError::Other(error.to_string()))?;
        let is_dir = entry_meta.is_dir();
        let size = if is_dir { 0 } else { entry_meta.len() };
        total += size;
        entries.push(Entry {
            name: if is_dir { format!("{name}/") } else { name },
            path: if is_dir {
                None
            } else {
                Some(entry.path().to_path_buf())
            },
            is_dir,
            size,
            mtime: entry_meta.modified().unwrap_or_else(|_| SystemTime::now()),
        });
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok((entries, total))
}

fn choose_compression(input: &Path, entries: &[Entry], mode: FileCompression) -> bool {
    match mode {
        FileCompression::Always => true,
        FileCompression::Never => false,
        FileCompression::Auto => {
            if input.is_file() {
                return entries
                    .first()
                    .map(|entry| is_compressible_name(&entry.name))
                    .unwrap_or(false);
            }
            let mut compressible = 0u64;
            let mut uncompressible = 0u64;
            for entry in entries.iter().filter(|entry| !entry.is_dir) {
                if is_compressible_name(&entry.name) {
                    compressible += entry.size;
                } else {
                    uncompressible += entry.size;
                }
            }
            compressible > 0 && uncompressible / 20 < compressible
        }
    }
}

fn is_compressible_name(name: &str) -> bool {
    const INCOMPRESSIBLE: &[&str] = &[
        "avi", "mov", "mpg", "mpeg", "mpe", "svi", "3gp", "mp4", "mkv", "wmv", "asf", "swf", "vob",
        "divx", "m1v", "m2v", "m4v", "flv", "mp3", "mp2", "ogg", "aac", "flac", "m4a", "ape",
        "wma", "mpc", "mp+", "mpp", "swa", "jpg", "jpeg", "jif", "jp2", "jps", "png", "gif",
        "webp", "heic", "heif", "rar", "zip", "arj", "jar", "tar", "s7z", "7z", "zipx", "ace",
        "cab", "gzip", "bz2", "arc", "lzma", "msi", "msix", "xz", "zst", "lz", "gz", "lz4", "lzo",
        "sfark", "sz", "enc", "apk", "dmg", "pkg", "deb", "docx", "dotx", "xlsx", "xltx", "pptx",
        "potx", "ppsx", "sldx",
    ];
    let extension = Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    !INCOMPRESSIBLE.contains(&extension.as_str())
}

fn extract_zip<R: Read + Seek>(
    reader: R,
    destination: &Path,
    reporter: &dyn Reporter,
) -> Result<()> {
    let mut archive = zip::ZipArchive::new(reader)?;
    let total = archive.len() as u64;
    let mut directory_times = Vec::new();
    for index in 0..archive.len() {
        check_cancelled(reporter)?;
        let mut entry = archive.by_index(index)?;
        let Some(relative) = entry.enclosed_name() else {
            return Err(CryptError::UnsafePath(entry.name().to_string()));
        };
        validate_relative_path(&relative)?;
        let output_path = destination.join(&relative);
        if entry.is_dir() || entry.name().ends_with('/') {
            fs::create_dir_all(&output_path)?;
            if let Some(timestamp) = zip_timestamp(&entry) {
                directory_times.push((output_path, timestamp));
            }
        } else {
            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = File::create(&output_path)?;
            let mut crc = crc32fast::Hasher::new();
            let mut buffer = vec![0u8; IO_BUFFER];
            loop {
                check_cancelled(reporter)?;
                let read = entry.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                crc.update(&buffer[..read]);
                output.write_all(&buffer[..read])?;
            }
            output.flush()?;
            if crc.finalize() != entry.crc32() {
                return Err(CryptError::Corrupted(format!(
                    "CRC mismatch in {}",
                    entry.name()
                )));
            }
            if let Some(timestamp) = zip_timestamp(&entry) {
                set_timestamp(&output_path, timestamp);
            }
        }
        reporter.progress(Progress::new(
            Phase::Extracting,
            (index + 1) as u64,
            total,
            Some(entry.name().to_string()),
        ));
    }
    directory_times.sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));
    for (path, timestamp) in directory_times {
        set_timestamp(&path, timestamp);
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<()> {
    for component in path.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(CryptError::UnsafePath(path.display().to_string()));
        };
        let text = name.to_string_lossy();
        if text.contains(':') || text.is_empty() {
            return Err(CryptError::UnsafePath(path.display().to_string()));
        }
        if cfg!(windows) {
            if text
                .chars()
                .any(|ch| matches!(ch, '*' | '?' | '"' | '<' | '>' | '|'))
            {
                return Err(CryptError::UnsafePath(path.display().to_string()));
            }
            let stem = text
                .split('.')
                .next()
                .unwrap_or(&text)
                .trim_end_matches([' ', '.'])
                .to_ascii_uppercase();
            if matches!(
                stem.as_str(),
                "CON"
                    | "PRN"
                    | "AUX"
                    | "NUL"
                    | "COM1"
                    | "COM2"
                    | "COM3"
                    | "COM4"
                    | "COM5"
                    | "COM6"
                    | "COM7"
                    | "COM8"
                    | "COM9"
                    | "LPT1"
                    | "LPT2"
                    | "LPT3"
                    | "LPT4"
                    | "LPT5"
                    | "LPT6"
                    | "LPT7"
                    | "LPT8"
                    | "LPT9"
            ) {
                return Err(CryptError::UnsafePath(path.display().to_string()));
            }
        }
    }
    Ok(())
}

fn zip_timestamp<R: Read>(entry: &zip::read::ZipFile<'_, R>) -> Option<i64> {
    entry
        .last_modified()
        .and_then(|value| time::OffsetDateTime::try_from(value).ok())
        .map(|value| value.unix_timestamp())
}

fn set_timestamp(path: &Path, unix_seconds: i64) {
    let time = filetime::FileTime::from_unix_time(unix_seconds, 0);
    let _ = filetime::set_file_mtime(path, time);
}

fn publish_temp_dir(temp: &Path, output_dir: &Path, stem: &str) -> Result<PathBuf> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(temp)? {
        let entry = entry?;
        entries.push(entry);
    }
    if entries.is_empty() {
        return Err(CryptError::Corrupted("archive is empty".into()));
    }
    if entries.len() == 1 {
        let entry = entries.pop().expect("one entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        let destination = if entry.file_type()?.is_dir() {
            util::unique_root(output_dir, &name)
        } else {
            util::unique_path(&output_dir.join(name))
        };
        fs::rename(entry.path(), &destination)?;
        return Ok(destination);
    }
    let root = util::unique_root(output_dir, stem);
    fs::create_dir(&root)?;
    for entry in entries {
        let destination = root.join(entry.file_name());
        fs::rename(entry.path(), destination)?;
    }
    Ok(root)
}
