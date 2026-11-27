use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Datelike, Local, TimeZone, Timelike};

use crate::error::Result;
use crate::progress::{check_cancelled, Phase, Progress, Reporter};

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
const ZIP64_EOCD_SIG: u32 = 0x0606_4b50;
const ZIP64_LOCATOR_SIG: u32 = 0x0706_4b50;
const DATA_DESCRIPTOR_SIG: u32 = 0x0807_4b50;
const ZIP64_EXTRA_ID: u16 = 0x0001;
const FLAG_UTF8: u16 = 0x0800;
const FLAG_DESCRIPTOR: u16 = 0x0008;
const U32_MAX: u64 = u32::MAX as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZipCompression {
    Store,
    Deflate,
}

struct CentralEntry {
    name: Vec<u8>,
    method: u16,
    flags: u16,
    time: u16,
    date: u16,
    crc: u32,
    compressed_size: u64,
    uncompressed_size: u64,
    offset: u64,
    external_attrs: u32,
}

pub struct ZipWriter<'a, W: Write> {
    inner: W,
    entries: Vec<CentralEntry>,
    offset: u64,
    compression: ZipCompression,
    reporter: &'a dyn Reporter,
    total: u64,
    done: u64,
    next_report: u64,
}

impl<'a, W: Write> ZipWriter<'a, W> {
    pub fn new(
        inner: W,
        compression: ZipCompression,
        reporter: &'a dyn Reporter,
        total: u64,
    ) -> Self {
        Self {
            inner,
            entries: Vec::new(),
            offset: 0,
            compression,
            reporter,
            total,
            done: 0,
            next_report: 0,
        }
    }

    pub fn add_directory(&mut self, name: &str, mtime: SystemTime) -> Result<()> {
        let mut name = name.replace('\\', "/");
        if !name.ends_with('/') {
            name.push('/');
        }
        let (time, date) = dos_time_date(mtime);
        let name_bytes = name.as_bytes().to_vec();
        let offset = self.offset;
        let header = local_header(&name_bytes, 0, FLAG_UTF8, time, date, 0, 0, 0, None);
        self.write_all(&header)?;
        self.entries.push(CentralEntry {
            name: name_bytes,
            method: 0,
            flags: FLAG_UTF8,
            time,
            date,
            crc: 0,
            compressed_size: 0,
            uncompressed_size: 0,
            offset,
            external_attrs: 0x41ed_0010,
        });
        Ok(())
    }

    pub fn add_file(&mut self, name: &str, path: &Path, mtime: SystemTime) -> Result<()> {
        check_cancelled(self.reporter)?;
        let name = name.replace('\\', "/");
        let name_bytes = name.as_bytes().to_vec();
        let metadata = std::fs::metadata(path)?;
        let uncompressed_size = metadata.len();
        let (time, date) = dos_time_date(mtime);
        let offset = self.offset;
        let reporter = self.reporter;
        let total = self.total;
        let mut done = self.done;
        let mut next_report = self.next_report;

        let (flags, crc, compressed_size, use_descriptor) = match self.compression {
            ZipCompression::Store => {
                reporter.progress(Progress::new(
                    Phase::Preparing,
                    done.min(total),
                    total,
                    Some(name.clone()),
                ));
                let crc = file_crc(path, reporter)?;
                let zip64 = uncompressed_size >= U32_MAX;
                let header = local_header(
                    &name_bytes,
                    0,
                    FLAG_UTF8,
                    time,
                    date,
                    crc,
                    uncompressed_size.min(u32::MAX as u64) as u32,
                    uncompressed_size.min(u32::MAX as u64) as u32,
                    if zip64 {
                        Some((uncompressed_size, uncompressed_size))
                    } else {
                        None
                    },
                );
                self.write_all(&header)?;
                let mut reader = BufReader::with_capacity(IO_BUFFER, File::open(path)?);
                let mut counter = CountingWriter::new(&mut self.inner);
                let mut buffer = vec![0u8; IO_BUFFER];
                loop {
                    check_cancelled(reporter)?;
                    let read = reader.read(&mut buffer)?;
                    if read == 0 {
                        break;
                    }
                    counter.write_all(&buffer[..read])?;
                    done += read as u64;
                    if done >= next_report || done == total {
                        next_report = done + 1024 * 1024;
                        reporter.progress(Progress::new(
                            Phase::Compressing,
                            done.min(total),
                            total,
                            Some(name.clone()),
                        ));
                    }
                }
                (FLAG_UTF8, crc, counter.count, false)
            }
            ZipCompression::Deflate => {
                let zip64 = uncompressed_size >= U32_MAX;
                let header = local_header(
                    &name_bytes,
                    8,
                    FLAG_UTF8 | FLAG_DESCRIPTOR,
                    time,
                    date,
                    0,
                    0,
                    0,
                    if zip64 { Some((0, 0)) } else { None },
                );
                self.write_all(&header)?;
                let mut reader = BufReader::with_capacity(IO_BUFFER, File::open(path)?);
                let mut crc = crc32fast::Hasher::new();
                let mut buffer = vec![0u8; IO_BUFFER];
                let counter = CountingWriter::new(&mut self.inner);
                let mut encoder =
                    flate2::write::DeflateEncoder::new(counter, flate2::Compression::default());
                loop {
                    check_cancelled(reporter)?;
                    let read = reader.read(&mut buffer)?;
                    if read == 0 {
                        break;
                    }
                    crc.update(&buffer[..read]);
                    encoder.write_all(&buffer[..read])?;
                    done += read as u64;
                    if done >= next_report || done == total {
                        next_report = done + 1024 * 1024;
                        reporter.progress(Progress::new(
                            Phase::Compressing,
                            done.min(total),
                            total,
                            Some(name.clone()),
                        ));
                    }
                }
                let counter = encoder.finish()?;
                (
                    FLAG_UTF8 | FLAG_DESCRIPTOR,
                    crc.finalize(),
                    counter.count,
                    true,
                )
            }
        };
        self.done = done;
        self.next_report = next_report;
        self.offset = self.offset.saturating_add(compressed_size);
        if use_descriptor {
            self.write_data_descriptor(
                crc,
                compressed_size,
                uncompressed_size,
                uncompressed_size >= U32_MAX,
            )?;
        }
        self.entries.push(CentralEntry {
            name: name_bytes,
            method: self.compression.method(),
            flags,
            time,
            date,
            crc,
            compressed_size,
            uncompressed_size,
            offset,
            external_attrs: 0x81a4_0000,
        });
        Ok(())
    }

    pub fn finish(mut self) -> Result<W> {
        let central_offset = self.offset;
        let entries = std::mem::take(&mut self.entries);
        let mut zip64 = entries.len() > u16::MAX as usize;
        for entry in &entries {
            if entry.compressed_size >= U32_MAX
                || entry.uncompressed_size >= U32_MAX
                || entry.offset >= U32_MAX
            {
                zip64 = true;
            }
            self.write_central_entry(entry)?;
        }
        let central_size = self.offset - central_offset;
        if central_offset >= U32_MAX || central_size >= U32_MAX {
            zip64 = true;
        }
        if zip64 {
            let zip64_offset = self.offset;
            let mut record = Vec::with_capacity(56);
            push_u32(&mut record, ZIP64_EOCD_SIG);
            push_u64(&mut record, 44);
            push_u16(&mut record, 45);
            push_u16(&mut record, 45);
            push_u32(&mut record, 0);
            push_u32(&mut record, 0);
            push_u64(&mut record, entries.len() as u64);
            push_u64(&mut record, entries.len() as u64);
            push_u64(&mut record, central_size);
            push_u64(&mut record, central_offset);
            self.write_all(&record)?;
            let mut locator = Vec::with_capacity(20);
            push_u32(&mut locator, ZIP64_LOCATOR_SIG);
            push_u32(&mut locator, 0);
            push_u64(&mut locator, zip64_offset);
            push_u32(&mut locator, 1);
            self.write_all(&locator)?;
        }
        let mut eocd = Vec::with_capacity(22);
        push_u32(&mut eocd, EOCD_SIG);
        push_u16(&mut eocd, 0);
        push_u16(&mut eocd, 0);
        if zip64 {
            push_u16(&mut eocd, u16::MAX);
            push_u16(&mut eocd, u16::MAX);
            push_u32(&mut eocd, u32::MAX);
            push_u32(&mut eocd, u32::MAX);
        } else {
            push_u16(&mut eocd, entries.len() as u16);
            push_u16(&mut eocd, entries.len() as u16);
            push_u32(&mut eocd, central_size as u32);
            push_u32(&mut eocd, central_offset as u32);
        }
        push_u16(&mut eocd, 0);
        self.write_all(&eocd)?;
        self.inner.flush()?;
        Ok(self.inner)
    }

    fn write_central_entry(&mut self, entry: &CentralEntry) -> Result<()> {
        let mut zip64_values = Vec::new();
        let mut uncompressed = entry.uncompressed_size as u32;
        let mut compressed = entry.compressed_size as u32;
        let mut offset = entry.offset as u32;
        if entry.uncompressed_size >= U32_MAX {
            uncompressed = u32::MAX;
            zip64_values.extend_from_slice(&entry.uncompressed_size.to_le_bytes());
        }
        if entry.compressed_size >= U32_MAX {
            compressed = u32::MAX;
            zip64_values.extend_from_slice(&entry.compressed_size.to_le_bytes());
        }
        if entry.offset >= U32_MAX {
            offset = u32::MAX;
            zip64_values.extend_from_slice(&entry.offset.to_le_bytes());
        }
        let mut extra = Vec::new();
        if !zip64_values.is_empty() {
            push_u16(&mut extra, ZIP64_EXTRA_ID);
            push_u16(&mut extra, zip64_values.len() as u16);
            extra.extend_from_slice(&zip64_values);
        }
        let mut header = Vec::with_capacity(46 + entry.name.len() + extra.len());
        push_u32(&mut header, CENTRAL_SIG);
        push_u16(&mut header, 0x0314);
        push_u16(&mut header, if extra.is_empty() { 20 } else { 45 });
        push_u16(&mut header, entry.flags);
        push_u16(&mut header, entry.method);
        push_u16(&mut header, entry.time);
        push_u16(&mut header, entry.date);
        push_u32(&mut header, entry.crc);
        push_u32(&mut header, compressed);
        push_u32(&mut header, uncompressed);
        push_u16(&mut header, entry.name.len() as u16);
        push_u16(&mut header, extra.len() as u16);
        push_u16(&mut header, 0);
        push_u16(&mut header, 0);
        push_u16(&mut header, 0);
        push_u32(&mut header, entry.external_attrs);
        push_u32(&mut header, offset);
        header.extend_from_slice(&entry.name);
        header.extend_from_slice(&extra);
        self.write_all(&header)
    }

    fn write_data_descriptor(
        &mut self,
        crc: u32,
        compressed_size: u64,
        uncompressed_size: u64,
        zip64: bool,
    ) -> Result<()> {
        let mut descriptor = Vec::with_capacity(24);
        push_u32(&mut descriptor, DATA_DESCRIPTOR_SIG);
        push_u32(&mut descriptor, crc);
        if zip64 {
            push_u64(&mut descriptor, compressed_size);
            push_u64(&mut descriptor, uncompressed_size);
        } else {
            push_u32(&mut descriptor, compressed_size as u32);
            push_u32(&mut descriptor, uncompressed_size as u32);
        }
        self.write_all(&descriptor)
    }

    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        self.inner.write_all(data)?;
        self.offset = self.offset.saturating_add(data.len() as u64);
        Ok(())
    }
}

impl ZipCompression {
    fn method(self) -> u16 {
        match self {
            Self::Store => 0,
            Self::Deflate => 8,
        }
    }
}

struct CountingWriter<'a, W: Write> {
    inner: &'a mut W,
    count: u64,
}

impl<'a, W: Write> CountingWriter<'a, W> {
    fn new(inner: &'a mut W) -> Self {
        Self { inner, count: 0 }
    }
}

impl<W: Write> Write for CountingWriter<'_, W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buffer)?;
        self.count += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[allow(clippy::too_many_arguments)]
fn local_header(
    name: &[u8],
    method: u16,
    flags: u16,
    time: u16,
    date: u16,
    crc: u32,
    compressed_size: u32,
    uncompressed_size: u32,
    zip64_sizes: Option<(u64, u64)>,
) -> Vec<u8> {
    let mut extra = Vec::new();
    let (compressed_size, uncompressed_size) = if let Some((uncompressed, compressed)) = zip64_sizes
    {
        push_u16(&mut extra, ZIP64_EXTRA_ID);
        push_u16(&mut extra, 16);
        push_u64(&mut extra, uncompressed);
        push_u64(&mut extra, compressed);
        (u32::MAX, u32::MAX)
    } else {
        (compressed_size, uncompressed_size)
    };
    let mut header = Vec::with_capacity(30 + name.len() + extra.len());
    push_u32(&mut header, LOCAL_SIG);
    push_u16(&mut header, if zip64_sizes.is_some() { 45 } else { 20 });
    push_u16(&mut header, flags);
    push_u16(&mut header, method);
    push_u16(&mut header, time);
    push_u16(&mut header, date);
    push_u32(&mut header, crc);
    push_u32(&mut header, compressed_size);
    push_u32(&mut header, uncompressed_size);
    push_u16(&mut header, name.len() as u16);
    push_u16(&mut header, extra.len() as u16);
    header.extend_from_slice(name);
    header.extend_from_slice(&extra);
    header
}

fn file_crc(path: &Path, reporter: &dyn Reporter) -> Result<u32> {
    let mut reader = BufReader::with_capacity(IO_BUFFER, File::open(path)?);
    let mut hasher = crc32fast::Hasher::new();
    let mut buffer = vec![0u8; IO_BUFFER];
    loop {
        check_cancelled(reporter)?;
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize())
}

fn dos_time_date(time: SystemTime) -> (u16, u16) {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let datetime = Local
        .timestamp_opt(seconds, 0)
        .single()
        .unwrap_or_else(|| Local.timestamp_opt(0, 0).single().expect("epoch"));
    let year = datetime.year().clamp(1980, 2107) as u16;
    let month = datetime.month() as u16;
    let day = datetime.day() as u16;
    let hour = datetime.hour() as u16;
    let minute = datetime.minute() as u16;
    let second = datetime.second() as u16;
    let dos_time = (hour << 11) | (minute << 5) | (second / 2);
    let dos_date = ((year - 1980) << 9) | (month << 5) | day;
    (dos_time, dos_date)
}

fn push_u16(buffer: &mut Vec<u8>, value: u16) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(buffer: &mut Vec<u8>, value: u32) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(buffer: &mut Vec<u8>, value: u64) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

const IO_BUFFER: usize = 64 * 1024;
