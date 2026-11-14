use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rarkzip_engine::{
    create_archive, extract_archive, hash_bytes, hash_file_many, open_archive, smart_target,
    test_password, verify_archive, ArchiveFormat, CreateOptions, ExtractMode, ExtractOptions,
    HashAlgorithm, NoopSink, OpenOptions, PasswordBook, PasswordPrompt, Progress, ProgressSink,
    RecoveryControl, RecoveryMode, RecoveryOptions, ZipAlgorithm,
};
use tempfile::TempDir;

fn write(path: &Path, content: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn sample_tree(root: &Path) -> PathBuf {
    let source = root.join("photos");
    write(&source.join("2026/01 冬季/IMG_1042.jpg"), b"jpeg-one");
    write(&source.join("2026/02 春节/全家福.heic"), b"heic-two");
    write(&source.join("README.txt"), b"RarkZip sample archive");
    source
}

fn options(format: ArchiveFormat) -> CreateOptions {
    CreateOptions {
        format,
        level: 5,
        overwrite: true,
        include_root: true,
        ..Default::default()
    }
}

#[test]
fn create_list_extract_zip() {
    let temp = TempDir::new().unwrap();
    let source = sample_tree(temp.path());
    let archive = temp.path().join("archives/photos.zip");
    let outcome = create_archive(
        std::slice::from_ref(&source),
        &archive,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();
    assert_eq!(outcome.entries, 3);
    assert!(archive.exists());

    let info = open_archive(&archive, &OpenOptions::default()).unwrap();
    assert_eq!(info.format, ArchiveFormat::Zip);
    assert!(info
        .entries
        .iter()
        .any(|entry| entry.path == "photos/README.txt"));
    assert!(info
        .entries
        .iter()
        .any(|entry| entry.is_dir && entry.path == "photos"));

    let target = smart_target(&archive, &info);
    assert!(target.strip_prefix.is_none());

    let result = extract_archive(&archive, &ExtractOptions::default(), &NoopSink).unwrap();
    assert_eq!(result.files, 3);
    assert!(result.nested_archives == 0);
    assert_eq!(
        fs::read(temp.path().join("archives/photos/README.txt")).unwrap(),
        b"RarkZip sample archive"
    );
    assert_eq!(
        fs::read(
            temp.path()
                .join("archives/photos/2026/01 冬季/IMG_1042.jpg")
        )
        .unwrap(),
        b"jpeg-one"
    );

    let report = verify_archive(&archive, None, &NoopSink).unwrap();
    assert_eq!(report.entries, 3);
    assert_eq!(report.bytes, 8 + 8 + 22);
}

#[test]
fn encrypted_zip_roundtrip() {
    let temp = TempDir::new().unwrap();
    let source = sample_tree(temp.path());
    let archive = temp.path().join("secret.zip");
    let mut opts = options(ArchiveFormat::Zip);
    opts.password = Some("s3cret-密码".to_string());
    opts.zip_algorithm = ZipAlgorithm::Deflate;
    create_archive(std::slice::from_ref(&source), &archive, &opts, &NoopSink).unwrap();

    let info = open_archive(&archive, &OpenOptions::default()).unwrap();
    assert!(info.encrypted);
    assert!(info.encrypted_entry_count() >= 1);

    assert!(!test_password(&archive, "wrong").unwrap());
    assert!(test_password(&archive, "s3cret-密码").unwrap());

    let out = temp.path().join("out");
    let extract_opts = ExtractOptions {
        mode: ExtractMode::Custom(out.clone()),
        password: Some("wrong".to_string()),
        nested_depth: 0,
        ..Default::default()
    };
    let error = extract_archive(&archive, &extract_opts, &NoopSink).unwrap_err();
    assert!(error.is_password_related(), "unexpected error: {error:?}");

    let secret_out = temp.path().join("secret-out");
    let extract_opts = ExtractOptions {
        mode: ExtractMode::Custom(secret_out.clone()),
        password: Some("s3cret-密码".to_string()),
        strip_root: true,
        nested_depth: 0,
        ..Default::default()
    };
    extract_archive(&archive, &extract_opts, &NoopSink).unwrap();
    assert!(secret_out.join("README.txt").exists());
}

#[test]
fn zip_with_prefixed_payload_is_detected() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("notes.txt");
    write(&source, b"hello prefix");
    let plain = temp.path().join("plain.zip");
    create_archive(
        std::slice::from_ref(&source),
        &plain,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();

    let mut disguised = Vec::new();
    disguised.extend_from_slice(b"MZ");
    disguised.extend(std::iter::repeat_n(0x41u8, 4096));
    disguised.extend_from_slice(&fs::read(&plain).unwrap());
    let prefixed = temp.path().join("disguised.bin");
    fs::write(&prefixed, disguised).unwrap();

    let info = open_archive(&prefixed, &OpenOptions::default()).unwrap();
    assert_eq!(info.format, ArchiveFormat::Zip);
    assert!(info.payload_offset >= 4096);
    assert!(info.entries.iter().any(|entry| entry.path == "notes.txt"));

    let out = temp.path().join("out");
    let extract_opts = ExtractOptions {
        mode: ExtractMode::Custom(out.clone()),
        nested_depth: 0,
        ..Default::default()
    };
    extract_archive(&prefixed, &extract_opts, &NoopSink).unwrap();
    assert_eq!(fs::read(out.join("notes.txt")).unwrap(), b"hello prefix");
}

#[test]
fn zip_with_prefix_and_trailing_video_data_is_detected() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("sam.txt");
    write(&source, b"embedded zip payload");
    let plain = temp.path().join("plain.zip");
    create_archive(
        std::slice::from_ref(&source),
        &plain,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();

    // 真实样本：视频头 + ZIP + ZIP 结束记录之后的视频尾数据。
    let mut disguised = vec![0x41u8; 64 * 1024];
    disguised.extend_from_slice(&fs::read(&plain).unwrap());
    disguised.extend_from_slice(&[0x42u8; 12_307]);
    let embedded = temp.path().join("MiokPawN35M91ODb_1.mp4");
    fs::write(&embedded, disguised).unwrap();

    let info = open_archive(&embedded, &OpenOptions::default()).unwrap();
    assert_eq!(info.format, ArchiveFormat::Zip);
    assert_eq!(info.file_count(), 1);
    assert!(info.entries.iter().any(|entry| entry.path == "sam.txt"));

    let out = temp.path().join("embedded-out");
    extract_archive(
        &embedded,
        &ExtractOptions {
            mode: ExtractMode::Custom(out.clone()),
            nested_depth: 0,
            ..Default::default()
        },
        &NoopSink,
    )
    .unwrap();
    assert_eq!(
        fs::read(out.join("sam.txt")).unwrap(),
        b"embedded zip payload"
    );
}

#[test]
fn tar_gz_roundtrip() {
    let temp = TempDir::new().unwrap();
    let source = sample_tree(temp.path());
    let archive = temp.path().join("photos.tar.gz");
    create_archive(
        std::slice::from_ref(&source),
        &archive,
        &options(ArchiveFormat::TarGz),
        &NoopSink,
    )
    .unwrap();

    let info = open_archive(&archive, &OpenOptions::default()).unwrap();
    assert_eq!(info.format, ArchiveFormat::TarGz);
    assert_eq!(info.file_count(), 3);
    assert!(info
        .entries
        .iter()
        .any(|entry| entry.path == "photos/README.txt"));

    let out = temp.path().join("tar-out");
    let extract_opts = ExtractOptions {
        mode: ExtractMode::Custom(out.clone()),
        nested_depth: 0,
        ..Default::default()
    };
    extract_archive(&archive, &extract_opts, &NoopSink).unwrap();
    assert_eq!(
        fs::read(out.join("photos/2026/02 春节/全家福.heic")).unwrap(),
        b"heic-two"
    );

    let report = verify_archive(&archive, None, &NoopSink).unwrap();
    assert_eq!(report.entries, 3);
}

#[test]
fn seven_z_roundtrip() {
    let temp = TempDir::new().unwrap();
    let source = sample_tree(temp.path());
    let archive = temp.path().join("photos.7z");
    create_archive(
        std::slice::from_ref(&source),
        &archive,
        &options(ArchiveFormat::SevenZ),
        &NoopSink,
    )
    .unwrap();

    let info = open_archive(&archive, &OpenOptions::default()).unwrap();
    assert_eq!(info.format, ArchiveFormat::SevenZ);
    assert_eq!(info.file_count(), 3);

    let out = temp.path().join("7z-out");
    let extract_opts = ExtractOptions {
        mode: ExtractMode::Custom(out.clone()),
        nested_depth: 0,
        ..Default::default()
    };
    extract_archive(&archive, &extract_opts, &NoopSink).unwrap();
    assert_eq!(
        fs::read(out.join("photos/README.txt")).unwrap(),
        b"RarkZip sample archive"
    );

    let report = verify_archive(&archive, None, &NoopSink).unwrap();
    assert_eq!(report.entries, 3);
}

#[test]
fn single_gzip_stream_roundtrip() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("messages.log");
    write(&source, b"line one\nline two\n");
    let archive = temp.path().join("messages.log.gz");
    create_archive(
        std::slice::from_ref(&source),
        &archive,
        &options(ArchiveFormat::Gz),
        &NoopSink,
    )
    .unwrap();

    let info = open_archive(&archive, &OpenOptions::default()).unwrap();
    assert_eq!(info.format, ArchiveFormat::Gz);
    assert_eq!(info.entries.len(), 1);
    assert_eq!(info.entries[0].path, "messages.log");

    let out = temp.path().join("gz-out");
    let extract_opts = ExtractOptions {
        mode: ExtractMode::Custom(out.clone()),
        nested_depth: 0,
        ..Default::default()
    };
    extract_archive(&archive, &extract_opts, &NoopSink).unwrap();
    assert_eq!(
        fs::read(out.join("messages.log")).unwrap(),
        b"line one\nline two\n"
    );
}

#[test]
fn single_bzip2_stream_roundtrip() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("messages.log");
    write(&source, b"line one\nline two\n");
    let archive = temp.path().join("messages.log.bz2");
    create_archive(
        std::slice::from_ref(&source),
        &archive,
        &options(ArchiveFormat::Bz2),
        &NoopSink,
    )
    .unwrap();

    let info = open_archive(&archive, &OpenOptions::default()).unwrap();
    assert_eq!(info.format, ArchiveFormat::Bz2);
    assert_eq!(info.entries.len(), 1);
    assert_eq!(info.entries[0].path, "messages.log");
    assert_eq!(info.entries[0].size, 18);

    let out = temp.path().join("bz2-out");
    let extract_opts = ExtractOptions {
        mode: ExtractMode::Custom(out.clone()),
        nested_depth: 0,
        ..Default::default()
    };
    extract_archive(&archive, &extract_opts, &NoopSink).unwrap();
    assert_eq!(
        fs::read(out.join("messages.log")).unwrap(),
        b"line one\nline two\n"
    );
}

#[test]
fn zip_lzma_roundtrip_for_zip_and_zipx() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("repeat.txt");
    write(&source, b"lzma zip content\n".repeat(400).as_slice());

    for (format, name) in [
        (ArchiveFormat::Zip, "lzma.zip"),
        (ArchiveFormat::Zipx, "lzma.zipx"),
    ] {
        let archive = temp.path().join(name);
        create_archive(
            std::slice::from_ref(&source),
            &archive,
            &CreateOptions {
                zip_algorithm: ZipAlgorithm::Lzma,
                ..options(format)
            },
            &NoopSink,
        )
        .unwrap();
        let info = open_archive(&archive, &OpenOptions::default()).unwrap();
        assert_eq!(info.format, format);
        assert_eq!(info.entries[0].method, "LZMA");
        assert!(rarkzip_engine::supports_quick_password_test(&archive));

        let out = temp.path().join(format!("out-{name}"));
        let extract_opts = ExtractOptions {
            mode: ExtractMode::Custom(out.clone()),
            nested_depth: 0,
            ..Default::default()
        };
        extract_archive(&archive, &extract_opts, &NoopSink).unwrap();
        assert_eq!(
            fs::read(out.join("repeat.txt")).unwrap(),
            fs::read(&source).unwrap()
        );
    }
}

#[test]
fn single_xz_and_zstd_stream_sizes_are_reported() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("payload.bin");
    let content = b"single stream size check\n".repeat(100);
    write(&source, &content);

    for (format, extension) in [(ArchiveFormat::Xz, "xz"), (ArchiveFormat::Zst, "zst")] {
        let archive = temp.path().join(format!("payload.bin.{extension}"));
        create_archive(
            std::slice::from_ref(&source),
            &archive,
            &options(format),
            &NoopSink,
        )
        .unwrap();
        let info = open_archive(&archive, &OpenOptions::default()).unwrap();
        assert_eq!(info.entries[0].size, content.len() as u64);
    }
}

#[test]
fn rar_extraction_does_not_auto_try_password_book() {
    let temp = TempDir::new().unwrap();
    let rar = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/crypted.rar");
    let options = ExtractOptions {
        mode: ExtractMode::Custom(temp.path().join("rar-out")),
        passwords: vec!["unrar".to_string()],
        nested_depth: 0,
        ..Default::default()
    };
    let error = extract_archive(&rar, &options, &NoopSink).unwrap_err();
    assert!(matches!(
        error,
        rarkzip_engine::EngineError::PasswordRequired
    ));
}

#[test]
fn seven_z_sfx_prefix_is_opened_and_extracted() {
    let temp = TempDir::new().unwrap();
    let source = sample_tree(temp.path());
    let archive = temp.path().join("photos.7z");
    create_archive(
        std::slice::from_ref(&source),
        &archive,
        &options(ArchiveFormat::SevenZ),
        &NoopSink,
    )
    .unwrap();

    let mut sfx = b"MZ fake self-extracting stub\r\n".to_vec();
    sfx.extend(fs::read(&archive).unwrap());
    let sfx_path = temp.path().join("photos-sfx.exe");
    fs::write(&sfx_path, sfx).unwrap();

    let info = open_archive(&sfx_path, &OpenOptions::default()).unwrap();
    assert_eq!(info.format, ArchiveFormat::SevenZ);
    assert_eq!(info.file_count(), 3);
    assert!(info.payload_path != sfx_path);

    let out = temp.path().join("7z-sfx-out");
    let extract_opts = ExtractOptions {
        mode: ExtractMode::Custom(out.clone()),
        nested_depth: 0,
        ..Default::default()
    };
    extract_archive(&sfx_path, &extract_opts, &NoopSink).unwrap();
    assert_eq!(
        fs::read(out.join("photos/README.txt")).unwrap(),
        b"RarkZip sample archive"
    );
    verify_archive(&sfx_path, None, &NoopSink).unwrap();
}

#[test]
fn zip_new_folder_entry_is_created() {
    let temp = TempDir::new().unwrap();
    let source = sample_tree(temp.path());
    let archive = temp.path().join("folder.zip");
    create_archive(
        std::slice::from_ref(&source),
        &archive,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();

    let report = rarkzip_engine::add_zip_directory(&archive, "photos/新建文件夹", None).unwrap();
    assert_eq!(report.added, 1);
    let info = open_archive(&archive, &OpenOptions::default()).unwrap();
    assert!(info
        .entries
        .iter()
        .any(|entry| entry.path == "photos/新建文件夹" && entry.is_dir));

    let out = temp.path().join("folder-out");
    let extract_opts = ExtractOptions {
        mode: ExtractMode::Custom(out.clone()),
        nested_depth: 0,
        ..Default::default()
    };
    extract_archive(&archive, &extract_opts, &NoopSink).unwrap();
    assert!(out.join("photos/新建文件夹").is_dir());
}

#[test]
fn nested_archive_is_extracted() {
    let temp = TempDir::new().unwrap();
    let inner_source = temp.path().join("inner.txt");
    write(&inner_source, b"nested content");
    let inner = temp.path().join("inner.zip");
    create_archive(
        std::slice::from_ref(&inner_source),
        &inner,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();

    let outer_source = temp.path().join("bundle");
    write(&outer_source.join("readme.txt"), b"outer");
    fs::copy(&inner, outer_source.join("inner.zip")).unwrap();
    let outer = temp.path().join("bundle.zip");
    create_archive(
        std::slice::from_ref(&outer_source),
        &outer,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();

    let out = temp.path().join("nested-out");
    let extract_opts = ExtractOptions {
        mode: ExtractMode::Custom(out.clone()),
        nested_depth: 3,
        ..Default::default()
    };
    let result = extract_archive(&outer, &extract_opts, &NoopSink).unwrap();
    assert_eq!(result.nested_archives, 1);
    assert_eq!(
        fs::read(out.join("bundle/inner/inner.txt")).unwrap(),
        b"nested content"
    );
}

#[test]
fn zip_slip_is_rejected() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("evil.zip");
    let file = File::create(&archive).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    writer
        .start_file("../evil.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"evil").unwrap();
    writer.finish().unwrap();

    let info = open_archive(&archive, &OpenOptions::default()).unwrap();
    assert!(info.entries.iter().any(|entry| entry.path == "../evil.txt"));

    let out = temp.path().join("safe");
    let extract_opts = ExtractOptions {
        mode: ExtractMode::Custom(out.clone()),
        nested_depth: 0,
        ..Default::default()
    };
    let error = extract_archive(&archive, &extract_opts, &NoopSink).unwrap_err();
    assert!(
        matches!(error, rarkzip_engine::EngineError::UnsafePath(_)),
        "unexpected error: {error:?}"
    );
    assert!(!temp.path().join("evil.txt").exists());
}

#[test]
fn hash_known_vectors() {
    assert_eq!(
        hash_bytes(HashAlgorithm::Md5, b"abc"),
        "900150983cd24fb0d6963f7d28e17f72"
    );
    assert_eq!(
        hash_bytes(HashAlgorithm::Sha1, b"abc"),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
    assert_eq!(
        hash_bytes(HashAlgorithm::Sha256, b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(hash_bytes(HashAlgorithm::Crc32, b"123456789"), "cbf43926");
    assert_eq!(hash_bytes(HashAlgorithm::Blake3, b"").len(), 64);
}

#[test]
fn multi_hash_file_matches_single_hash_results() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("payload.bin");
    write(&file, b"abc");
    let algorithms = [
        HashAlgorithm::Sha256,
        HashAlgorithm::Sha1,
        HashAlgorithm::Md5,
        HashAlgorithm::Crc32,
    ];
    let hashes = hash_file_many(&algorithms, &file, &NoopSink).unwrap();
    assert_eq!(hashes.len(), algorithms.len());
    for (algorithm, hash) in hashes {
        assert_eq!(hash, hash_bytes(algorithm, b"abc"));
    }
}

#[test]
fn password_book_roundtrip() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("passwords.json");
    let mut book = PasswordBook::load(path.clone());
    book.add("first-pass", "note one");
    book.add("second-pass", "note two");
    book.save().unwrap();

    let loaded = PasswordBook::load(path);
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded.entries()[0].password, "first-pass");
    assert_eq!(loaded.entries()[1].password, "second-pass");
}

#[test]
fn smart_extract_single_empty_root_folder_stays_in_current_dir() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("empty-folder.zip");
    let file = File::create(&archive).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    writer
        .add_directory("root/", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.finish().unwrap();

    let result = extract_archive(&archive, &ExtractOptions::default(), &NoopSink).unwrap();
    assert_eq!(result.dest, temp.path());
    assert!(temp.path().join("root").is_dir());
    assert!(!temp.path().join("empty-folder").exists());
}

#[test]
fn smart_extract_multiple_roots_uses_archive_folder() {
    let temp = TempDir::new().unwrap();
    let first = temp.path().join("first.txt");
    let second = temp.path().join("second.txt");
    write(&first, b"first");
    write(&second, b"second");
    let archive = temp.path().join("multi.zip");
    create_archive(
        &[first, second],
        &archive,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();

    let result = extract_archive(&archive, &ExtractOptions::default(), &NoopSink).unwrap();
    assert_eq!(result.files, 2);
    assert_eq!(
        fs::read(temp.path().join("multi/first.txt")).unwrap(),
        b"first"
    );
    assert_eq!(
        fs::read(temp.path().join("multi/second.txt")).unwrap(),
        b"second"
    );
}

#[test]
fn smart_extract_uses_unique_archive_folder_when_name_exists() {
    let temp = TempDir::new().unwrap();
    let first = temp.path().join("first.txt");
    let second = temp.path().join("second.txt");
    write(&first, b"first");
    write(&second, b"second");
    let archive = temp.path().join("multi.zip");
    create_archive(
        &[first, second],
        &archive,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();
    fs::create_dir(temp.path().join("multi")).unwrap();

    let result = extract_archive(&archive, &ExtractOptions::default(), &NoopSink).unwrap();
    assert_eq!(result.dest, temp.path().join("multi (2)"));
    assert!(temp.path().join("multi").is_dir());
    assert_eq!(
        fs::read(temp.path().join("multi (2)/first.txt")).unwrap(),
        b"first"
    );
    assert_eq!(
        fs::read(temp.path().join("multi (2)/second.txt")).unwrap(),
        b"second"
    );
}

#[test]
fn archive_with_single_top_level_file_smart_targets_current_dir() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("only.txt");
    write(&source, b"only");
    let archive = temp.path().join("single.zip");
    create_archive(
        std::slice::from_ref(&source),
        &archive,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();
    let info = open_archive(&archive, &OpenOptions::default()).unwrap();
    let target = smart_target(&archive, &info);
    assert_eq!(target.dest, temp.path());
    assert!(target.strip_prefix.is_none());
}

fn make_apate(original: &[u8], mask_len: usize) -> Vec<u8> {
    assert!(mask_len >= 16 && mask_len < original.len());
    let mut disguised = vec![0x5Au8; mask_len];
    disguised.extend_from_slice(&original[mask_len..]);
    let mut head = original[..mask_len].to_vec();
    head.reverse();
    disguised.extend_from_slice(&head);
    disguised.extend_from_slice(&(mask_len as u32).to_le_bytes());
    disguised
}

#[test]
fn apate_disguise_is_revealed_and_extracted() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("secret.txt");
    write(&source, b"apate payload");
    let archive = temp.path().join("plain.zip");
    create_archive(
        std::slice::from_ref(&source),
        &archive,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();

    assert!(rarkzip_engine::detect_apate(&archive, true)
        .unwrap()
        .is_none());

    let original = fs::read(&archive).unwrap();
    let disguised = make_apate(&original, 64);
    let masked = temp.path().join("movie.mp4");
    fs::write(&masked, disguised).unwrap();

    let detected = rarkzip_engine::detect_apate(&masked, true)
        .unwrap()
        .unwrap();
    assert!(detected.validated);
    assert_eq!(detected.mask_length, 64);

    let info = open_archive(&masked, &OpenOptions::default()).unwrap();
    assert!(info.disguised);
    assert_ne!(info.payload_path, masked);
    assert!(info.payload_path.exists());
    assert!(info.entries.iter().any(|entry| entry.path == "secret.txt"));

    let out = temp.path().join("revealed-out");
    let extract_opts = ExtractOptions {
        mode: ExtractMode::Custom(out.clone()),
        nested_depth: 0,
        ..Default::default()
    };
    extract_archive(&masked, &extract_opts, &NoopSink).unwrap();
    assert_eq!(fs::read(out.join("secret.txt")).unwrap(), b"apate payload");
}

#[test]
fn extraction_auto_tries_candidate_passwords() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("secret.txt");
    write(&source, b"auto password");
    let archive = temp.path().join("secret.zip");
    let mut opts = options(ArchiveFormat::Zip);
    opts.password = Some("book-pass".to_string());
    create_archive(std::slice::from_ref(&source), &archive, &opts, &NoopSink).unwrap();

    let out = temp.path().join("auto-out");
    let extract_opts = ExtractOptions {
        mode: ExtractMode::Custom(out.clone()),
        password: None,
        passwords: vec!["wrong-pass".to_string(), "book-pass".to_string()],
        nested_depth: 0,
        ..Default::default()
    };
    let outcome = extract_archive(&archive, &extract_opts, &NoopSink).unwrap();
    assert_eq!(outcome.files, 1);
    assert_eq!(fs::read(out.join("secret.txt")).unwrap(), b"auto password");
}

#[test]
fn nested_archive_reuses_verified_password() {
    let temp = TempDir::new().unwrap();
    let inner_source = temp.path().join("inner.txt");
    write(&inner_source, b"inner shared");
    let inner = temp.path().join("bundle/inner.zip");
    let mut shared = options(ArchiveFormat::Zip);
    shared.password = Some("shared-pass".to_string());
    create_archive(
        std::slice::from_ref(&inner_source),
        &inner,
        &shared,
        &NoopSink,
    )
    .unwrap();

    let bundle = temp.path().join("bundle");
    write(&bundle.join("readme.txt"), b"outer");
    let outer = temp.path().join("shared.7z");
    create_archive(
        std::slice::from_ref(&bundle),
        &outer,
        &options(ArchiveFormat::SevenZ),
        &NoopSink,
    )
    .unwrap();

    let out = temp.path().join("reuse-out");
    let extract_opts = ExtractOptions {
        mode: ExtractMode::Custom(out.clone()),
        password: None,
        passwords: vec!["shared-pass".to_string()],
        nested_depth: 3,
        ..Default::default()
    };
    let result = extract_archive(&outer, &extract_opts, &NoopSink).unwrap();
    assert_eq!(result.nested_archives, 1);
    assert_eq!(
        fs::read(out.join("bundle/inner/inner.txt")).unwrap(),
        b"inner shared"
    );
}

#[test]
fn nested_encrypted_archive_tries_book_when_parent_password_differs() {
    let temp = TempDir::new().unwrap();
    let inner_source = temp.path().join("inner.txt");
    write(&inner_source, b"inner secret");
    let inner = temp.path().join("bundle/inner.zip");
    let mut inner_options = options(ArchiveFormat::Zip);
    inner_options.password = Some("inner-pass".to_string());
    create_archive(
        std::slice::from_ref(&inner_source),
        &inner,
        &inner_options,
        &NoopSink,
    )
    .unwrap();

    let bundle = temp.path().join("bundle");
    write(&bundle.join("outer.txt"), b"outer");
    let outer = temp.path().join("outer.zip");
    let mut outer_options = options(ArchiveFormat::Zip);
    outer_options.password = Some("outer-pass".to_string());
    create_archive(
        std::slice::from_ref(&bundle),
        &outer,
        &outer_options,
        &NoopSink,
    )
    .unwrap();

    let out = temp.path().join("book-nested-out");
    let extract_opts = ExtractOptions {
        mode: ExtractMode::Custom(out.clone()),
        password: Some("outer-pass".to_string()),
        passwords: vec!["inner-pass".to_string()],
        nested_depth: 3,
        ..Default::default()
    };
    let result = extract_archive(&outer, &extract_opts, &NoopSink).unwrap();
    assert_eq!(result.nested_archives, 1);
    assert_eq!(
        fs::read(out.join("bundle/inner/inner.txt")).unwrap(),
        b"inner secret"
    );
}

#[test]
fn nested_disguised_archive_is_detected() {
    let temp = TempDir::new().unwrap();
    let hidden_source = temp.path().join("hidden.txt");
    write(&hidden_source, b"hidden nested");
    let plain = temp.path().join("plain-inner.zip");
    create_archive(
        std::slice::from_ref(&hidden_source),
        &plain,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();
    let disguised = make_apate(&fs::read(&plain).unwrap(), 64);
    let bundle = temp.path().join("bundle");
    write(&bundle.join("readme.txt"), b"outer");
    fs::write(bundle.join("movie.mp4"), disguised).unwrap();

    let outer = temp.path().join("outer.zip");
    create_archive(
        std::slice::from_ref(&bundle),
        &outer,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();

    let out = temp.path().join("disguised-nested-out");
    let extract_opts = ExtractOptions {
        mode: ExtractMode::Custom(out.clone()),
        nested_depth: 3,
        ..Default::default()
    };
    let result = extract_archive(&outer, &extract_opts, &NoopSink).unwrap();
    assert_eq!(result.nested_archives, 1);
    assert_eq!(
        fs::read(out.join("bundle/movie/hidden.txt")).unwrap(),
        b"hidden nested"
    );
}

#[test]
fn rar_list_extract_and_password() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/version.rar");
    let info = open_archive(&fixture, &OpenOptions::default()).unwrap();
    assert_eq!(info.format, ArchiveFormat::Rar);
    assert_eq!(info.file_count(), 1);
    assert!(info.entries.iter().any(|entry| entry.path == "VERSION"));

    let temp = TempDir::new().unwrap();
    let dest = temp.path().join("rar-out");
    let result = extract_archive(
        &fixture,
        &ExtractOptions {
            mode: ExtractMode::Custom(dest.clone()),
            propagate_motw: false,
            nested_depth: 0,
            ..Default::default()
        },
        &NoopSink,
    )
    .unwrap();
    assert_eq!(result.files, 1);
    assert_eq!(fs::read(dest.join("VERSION")).unwrap(), b"unrar-0.4.0");

    let encrypted = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/crypted.rar");
    assert!(
        test_password(&encrypted, "nope").is_err() || !test_password(&encrypted, "nope").unwrap()
    );
    assert!(test_password(&encrypted, "unrar").unwrap());
    let info = open_archive(
        &encrypted,
        &OpenOptions {
            password: Some("unrar".to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(info.encrypted);
    let target = temp.path().join("crypted-out");
    extract_archive(
        &encrypted,
        &ExtractOptions {
            mode: ExtractMode::Custom(target.clone()),
            password: Some("unrar".to_string()),
            propagate_motw: false,
            nested_depth: 0,
            ..Default::default()
        },
        &NoopSink,
    )
    .unwrap();
    assert!(target.join(".gitignore").exists());
}

#[test]
fn split_zip_volumes_are_assembled_without_touching_sources() {
    let temp = TempDir::new().unwrap();
    let source = sample_tree(temp.path());
    let archive = temp.path().join("split.zip");
    create_archive(
        std::slice::from_ref(&source),
        &archive,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();
    let bytes = fs::read(&archive).unwrap();
    let split = bytes.len() / 2;
    let first = temp.path().join("split.zip.001");
    let second = temp.path().join("split.zip.002");
    fs::write(&first, &bytes[..split]).unwrap();
    fs::write(&second, &bytes[split..]).unwrap();
    fs::remove_file(&archive).unwrap();

    let info = open_archive(&second, &OpenOptions::default()).unwrap();
    assert_eq!(info.format, ArchiveFormat::Zip);
    assert!(info.note.as_deref().unwrap_or_default().contains("分卷"));
    assert_eq!(
        fs::read(&first).unwrap().len() + fs::read(&second).unwrap().len(),
        bytes.len()
    );

    let dest = temp.path().join("split-out");
    extract_archive(
        &second,
        &ExtractOptions {
            mode: ExtractMode::Custom(dest.clone()),
            propagate_motw: false,
            nested_depth: 0,
            ..Default::default()
        },
        &NoopSink,
    )
    .unwrap();
    assert!(dest.join("photos/README.txt").exists());
}

#[test]
fn repair_zip_recovers_local_entries_after_central_directory_damage() {
    let temp = TempDir::new().unwrap();
    let source = sample_tree(temp.path());
    let archive = temp.path().join("damaged.zip");
    create_archive(
        std::slice::from_ref(&source),
        &archive,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();
    let mut bytes = fs::read(&archive).unwrap();
    let central = bytes
        .windows(4)
        .position(|window| window == b"PK\x01\x02")
        .expect("central directory");
    bytes.truncate(central);
    fs::write(&archive, &bytes).unwrap();
    assert!(open_archive(&archive, &OpenOptions::default()).is_err());

    let repaired = temp.path().join("repaired.zip");
    let report = rarkzip_engine::repair_zip(&archive, &repaired, &NoopSink).unwrap();
    assert!(report.recovered >= 3);
    let info = open_archive(&repaired, &OpenOptions::default()).unwrap();
    assert_eq!(info.file_count(), 3);
    assert!(info
        .entries
        .iter()
        .any(|entry| entry.path == "photos/README.txt"));
}

#[test]
fn password_recovery_brute_and_mask() {
    let temp = TempDir::new().unwrap();
    let source = sample_tree(temp.path());
    let archive = temp.path().join("recover.zip");
    let mut opts = options(ArchiveFormat::Zip);
    opts.password = Some("abc".to_string());
    create_archive(std::slice::from_ref(&source), &archive, &opts, &NoopSink).unwrap();

    let control = RecoveryControl::new();
    let outcome = rarkzip_engine::recover_password(
        &archive,
        &RecoveryOptions {
            mode: RecoveryMode::Brute {
                charset: "abc".to_string(),
                min_len: 1,
                max_len: 3,
            },
            threads: 2,
        },
        &control,
        |_, _| {},
    )
    .unwrap();
    assert_eq!(outcome.password.as_deref(), Some("abc"));

    let outcome = rarkzip_engine::recover_password(
        &archive,
        &RecoveryOptions {
            mode: RecoveryMode::Mask("?l?l?l".to_string()),
            threads: 2,
        },
        &control,
        |_, _| {},
    )
    .unwrap();
    assert_eq!(outcome.password.as_deref(), Some("abc"));

    let outcome = rarkzip_engine::recover_password(
        &archive,
        &RecoveryOptions {
            mode: RecoveryMode::Dictionary(vec!["wrong".to_string(), "abc".to_string()]),
            threads: 2,
        },
        &control,
        |_, _| {},
    )
    .unwrap();
    assert_eq!(outcome.password.as_deref(), Some("abc"));
}

#[test]
fn split_7z_volumes_are_assembled() {
    let temp = TempDir::new().unwrap();
    let source = sample_tree(temp.path());
    let archive = temp.path().join("split.7z");
    create_archive(
        std::slice::from_ref(&source),
        &archive,
        &options(ArchiveFormat::SevenZ),
        &NoopSink,
    )
    .unwrap();
    let bytes = fs::read(&archive).unwrap();
    let cut = bytes.len() * 2 / 3;
    fs::write(temp.path().join("split.7z.001"), &bytes[..cut]).unwrap();
    fs::write(temp.path().join("split.7z.002"), &bytes[cut..]).unwrap();
    fs::remove_file(&archive).unwrap();
    let info = open_archive(&temp.path().join("split.7z.002"), &OpenOptions::default()).unwrap();
    assert_eq!(info.format, ArchiveFormat::SevenZ);
    assert_eq!(info.file_count(), 3);
}

#[test]
fn zip_edit_rename_delete_and_comment() {
    let temp = TempDir::new().unwrap();
    let source = sample_tree(temp.path());
    let archive = temp.path().join("edit.zip");
    create_archive(
        std::slice::from_ref(&source),
        &archive,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();
    let info = open_archive(&archive, &OpenOptions::default()).unwrap();
    let readme = info
        .entries
        .iter()
        .find(|entry| entry.path == "photos/README.txt")
        .unwrap()
        .index;
    let mut rename = std::collections::HashMap::new();
    rename.insert(readme, "photos/README-renamed.txt".to_string());
    let report = rarkzip_engine::edit_zip(
        &archive,
        &archive,
        None,
        &rarkzip_engine::ZipEditOptions {
            rename,
            comment: Some("edited by test".to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(report.written >= 3);
    let info = open_archive(&archive, &OpenOptions::default()).unwrap();
    assert_eq!(info.comment.as_deref(), Some("edited by test"));
    assert!(info
        .entries
        .iter()
        .any(|entry| entry.path == "photos/README-renamed.txt"));

    let readme = info
        .entries
        .iter()
        .find(|entry| entry.path == "photos/README-renamed.txt")
        .unwrap()
        .index;
    rarkzip_engine::edit_zip(
        &archive,
        &archive,
        None,
        &rarkzip_engine::ZipEditOptions {
            delete: vec![readme],
            ..Default::default()
        },
    )
    .unwrap();
    let info = open_archive(&archive, &OpenOptions::default()).unwrap();
    assert!(!info
        .entries
        .iter()
        .any(|entry| entry.path == "photos/README-renamed.txt"));
}

#[test]
fn zip_add_entries_appends_files_and_directories() {
    let temp = TempDir::new().unwrap();
    let source = sample_tree(temp.path());
    let archive = temp.path().join("append.zip");
    create_archive(
        std::slice::from_ref(&source),
        &archive,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();
    let extra = temp.path().join("extra");
    write(&extra.join("nested/new.txt"), b"new content");
    let report = rarkzip_engine::add_zip_entries(&archive, &[extra], None).unwrap();
    assert!(report.added >= 2);
    let info = open_archive(&archive, &OpenOptions::default()).unwrap();
    assert!(info
        .entries
        .iter()
        .any(|entry| entry.path == "extra/nested/new.txt"));
}

#[test]
fn sfx_payload_roundtrip() {
    let temp = TempDir::new().unwrap();
    let source = sample_tree(temp.path());
    let zip = temp.path().join("payload.zip");
    create_archive(
        std::slice::from_ref(&source),
        &zip,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();
    let stub = temp.path().join("stub.bin");
    let mut stub_bytes = b"MZ".to_vec();
    stub_bytes.resize(4096, 0x90);
    // 让 stub 内出现伪中央目录签名，验证 SFX 偏移修正后仍能正确定位。
    stub_bytes[100..104].copy_from_slice(b"PK\x01\x02");
    fs::write(&stub, stub_bytes).unwrap();
    let out = temp.path().join("package.exe");
    let dest = temp.path().join("sfx-out");
    let config = rarkzip_engine::SfxConfig {
        destination: dest.display().to_string(),
        ..Default::default()
    };
    rarkzip_engine::sfx::create_sfx(&zip, &out, &stub, &config).unwrap();
    let info = rarkzip_engine::sfx::detect_sfx(&out).unwrap().unwrap();
    assert!(info.payload_len > 0);
    // 生成的 SFX 本身必须能作为普通 ZIP 打开（外部工具兼容）。
    let opened = open_archive(&out, &OpenOptions::default()).unwrap();
    assert!(opened
        .entries
        .iter()
        .any(|entry| entry.path == "photos/README.txt"));
    rarkzip_engine::sfx::extract_sfx(&out, &out, &NoopSink).unwrap();
    assert!(dest.join("photos/README.txt").exists());
}

#[test]
fn checksum_file_parsing_and_comparison() {
    let text = "\u{feff}# comment\n\
        ABCDEF0123456789  photos/README.txt\n\
        abcdef0123456789 *photo.jpg\n\
        SHA256 (notes.txt) = 0123456789abcdef\n";
    let entries = rarkzip_engine::parse_checksum_text(text);
    assert_eq!(entries.len(), 3);
    let expected = rarkzip_engine::checksum_expectations(&entries);
    assert_eq!(
        expected.get("readme.txt").map(String::as_str),
        Some("abcdef0123456789")
    );
    assert_eq!(
        rarkzip_engine::compare_hash(
            Path::new("C:/work/README.TXT"),
            "ABCDEF0123456789",
            &expected
        ),
        Some(true)
    );
    assert_eq!(
        rarkzip_engine::compare_hash(
            Path::new("C:/work/photo.jpg"),
            "ffffffffffffffff",
            &expected
        ),
        Some(false)
    );
    assert_eq!(
        rarkzip_engine::compare_hash(Path::new("unknown.bin"), "00", &expected),
        None
    );
}

#[test]
fn encrypted_7z_write_and_read() {
    let temp = TempDir::new().unwrap();
    let source = sample_tree(temp.path());
    let archive = temp.path().join("secret.7z");
    let mut opts = options(ArchiveFormat::SevenZ);
    opts.password = Some("seven-secret".to_string());
    create_archive(std::slice::from_ref(&source), &archive, &opts, &NoopSink).unwrap();

    let err = open_archive(&archive, &OpenOptions::default()).unwrap_err();
    assert!(err.is_password_related(), "unexpected error: {err:?}");
    assert!(!test_password(&archive, "wrong").unwrap());
    assert!(test_password(&archive, "seven-secret").unwrap());
    let info = open_archive(
        &archive,
        &OpenOptions {
            password: Some("seven-secret".to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(info.format, ArchiveFormat::SevenZ);
    assert!(info.encrypted);

    let out = temp.path().join("seven-out");
    extract_archive(
        &archive,
        &ExtractOptions {
            mode: ExtractMode::Custom(out.clone()),
            password: Some("seven-secret".to_string()),
            nested_depth: 0,
            ..Default::default()
        },
        &NoopSink,
    )
    .unwrap();
    assert!(out.join("photos/README.txt").exists());
}

#[test]
fn creates_zipx_lzh_iso_zpaq_roundtrip() {
    let temp = TempDir::new().unwrap();
    let source = sample_tree(temp.path());
    for format in [
        ArchiveFormat::Zipx,
        ArchiveFormat::Lzh,
        ArchiveFormat::Iso,
        ArchiveFormat::Zpaq,
    ] {
        let archive = temp.path().join(format!("sample.{}", format.extension()));
        create_archive(
            std::slice::from_ref(&source),
            &archive,
            &options(format),
            &NoopSink,
        )
        .unwrap_or_else(|error| panic!("create {format:?}: {error}"));
        assert!(archive.exists(), "{format:?} 未生成文件");

        let info = open_archive(&archive, &OpenOptions::default())
            .unwrap_or_else(|error| panic!("open {format:?}: {error}"));
        assert_eq!(info.format, format);
        assert!(
            info.entries
                .iter()
                .any(|entry| entry.path.ends_with("README.txt")),
            "{format:?} 缺少 README.txt：{:?}",
            info.entries
                .iter()
                .map(|entry| &entry.path)
                .collect::<Vec<_>>()
        );

        verify_archive(&archive, None, &NoopSink)
            .unwrap_or_else(|error| panic!("verify {format:?}: {error}"));

        let out = temp.path().join(format!("out-{}", format.extension()));
        let extract = ExtractOptions {
            mode: ExtractMode::Custom(out.clone()),
            overwrite: true,
            propagate_motw: false,
            nested_depth: 0,
            ..ExtractOptions::default()
        };
        extract_archive(&archive, &extract, &NoopSink)
            .unwrap_or_else(|error| panic!("extract {format:?}: {error}"));
        assert_eq!(
            fs::read(out.join("photos/README.txt")).unwrap(),
            b"RarkZip sample archive",
            "{format:?} 解压内容不一致"
        );
    }
}

#[test]
fn slow_nested_archive_reuses_parent_password() {
    let temp = TempDir::new().unwrap();
    let inner_source = temp.path().join("inner.txt");
    write(&inner_source, b"inner slow shared");
    let inner = temp.path().join("bundle/inner.7z");
    let mut inner_options = options(ArchiveFormat::SevenZ);
    inner_options.password = Some("shared-pass".to_string());
    create_archive(
        std::slice::from_ref(&inner_source),
        &inner,
        &inner_options,
        &NoopSink,
    )
    .unwrap();

    let bundle = temp.path().join("bundle");
    write(&bundle.join("readme.txt"), b"outer");
    let outer = temp.path().join("outer.7z");
    let mut outer_options = options(ArchiveFormat::SevenZ);
    outer_options.password = Some("shared-pass".to_string());
    create_archive(
        std::slice::from_ref(&bundle),
        &outer,
        &outer_options,
        &NoopSink,
    )
    .unwrap();

    let out = temp.path().join("slow-reuse-out");
    let extract_options = ExtractOptions {
        mode: ExtractMode::Custom(out.clone()),
        password: Some("shared-pass".to_string()),
        nested_depth: 3,
        ..Default::default()
    };
    let result = extract_archive(&outer, &extract_options, &NoopSink).unwrap();
    assert_eq!(result.nested_archives, 1);
    assert_eq!(
        fs::read(out.join("bundle/inner/inner.txt")).unwrap(),
        b"inner slow shared"
    );
}

#[test]
fn parent_password_survives_unencrypted_intermediate_layer() {
    let temp = TempDir::new().unwrap();
    let inner_source = temp.path().join("inner.txt");
    write(&inner_source, b"deep inner");
    let inner = temp.path().join("inner.7z");
    let mut inner_options = options(ArchiveFormat::SevenZ);
    inner_options.password = Some("shared-pass".to_string());
    create_archive(
        std::slice::from_ref(&inner_source),
        &inner,
        &inner_options,
        &NoopSink,
    )
    .unwrap();

    let middle_dir = temp.path().join("middle");
    std::fs::create_dir_all(&middle_dir).unwrap();
    std::fs::copy(&inner, middle_dir.join("inner.7z")).unwrap();
    let middle = temp.path().join("middle.zip");
    create_archive(
        std::slice::from_ref(&middle_dir),
        &middle,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();

    let outer_dir = temp.path().join("outer");
    std::fs::create_dir_all(&outer_dir).unwrap();
    std::fs::copy(&middle, outer_dir.join("middle.zip")).unwrap();
    let outer = temp.path().join("outer.7z");
    let mut outer_options = options(ArchiveFormat::SevenZ);
    outer_options.password = Some("shared-pass".to_string());
    create_archive(
        std::slice::from_ref(&outer_dir),
        &outer,
        &outer_options,
        &NoopSink,
    )
    .unwrap();

    let out = temp.path().join("deep-out");
    let extract_options = ExtractOptions {
        mode: ExtractMode::Custom(out.clone()),
        password: Some("shared-pass".to_string()),
        nested_depth: 4,
        ..Default::default()
    };
    let result = extract_archive(&outer, &extract_options, &NoopSink).unwrap();
    assert_eq!(result.nested_archives, 2);
    assert_eq!(
        fs::read(out.join("outer/middle/middle/inner/inner.txt")).unwrap(),
        b"deep inner"
    );
}

struct PasswordSink {
    password: String,
    requests: AtomicUsize,
}

impl PasswordSink {
    fn new(password: &str) -> Self {
        Self {
            password: password.to_string(),
            requests: AtomicUsize::new(0),
        }
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::Relaxed)
    }
}

impl ProgressSink for PasswordSink {
    fn update(&self, _progress: &Progress) {}

    fn request_password(&self, prompt: &PasswordPrompt) -> Option<String> {
        assert!(prompt.attempt >= 1);
        self.requests.fetch_add(1, Ordering::Relaxed);
        Some(self.password.clone())
    }
}

fn write_apate_volumes(
    directory: &Path,
    stem: &str,
    bytes: &[u8],
    parts: usize,
    masks: &[usize],
) -> Vec<PathBuf> {
    assert!(parts >= 2);
    assert_eq!(masks.len(), parts);
    let chunk = bytes.len().div_ceil(parts);
    let mut paths = Vec::new();
    for (index, mask) in masks.iter().enumerate().take(parts) {
        let start = index * chunk;
        let end = ((index + 1) * chunk).min(bytes.len());
        let part = &bytes[start..end];
        assert!(part.len() > *mask + 16);
        let disguised = make_apate(part, *mask);
        let path = directory.join(format!("{stem}.{:03}.mp4", index + 1));
        write(&path, &disguised);
        paths.push(path);
    }
    paths
}

#[test]
fn apate_disguised_split_volumes_are_revealed_and_extracted() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("secret.txt");
    let mut state = 0x1234_5678u32;
    let content: Vec<u8> = (0..20_000)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state & 0xff) as u8
        })
        .collect();
    write(&source, &content);
    let plain = temp.path().join("plain.zip");
    let mut create = options(ArchiveFormat::Zip);
    create.password = Some("vol-pass".to_string());
    create_archive(std::slice::from_ref(&source), &plain, &create, &NoopSink).unwrap();

    let bytes = fs::read(&plain).unwrap();
    fs::remove_file(&plain).unwrap();
    let volumes = write_apate_volumes(temp.path(), "CH34.zip", &bytes, 3, &[64, 80, 48]);
    let sizes: Vec<u64> = volumes
        .iter()
        .map(|path| fs::metadata(path).unwrap().len())
        .collect();

    // 从任意一卷打开都能找回整套分卷并还原 apate 伪装。
    let info = open_archive(&volumes[1], &OpenOptions::default()).unwrap();
    assert_eq!(info.format, ArchiveFormat::Zip);
    assert!(info.disguised);
    assert!(info
        .entries
        .iter()
        .any(|entry| entry.path.ends_with("secret.txt")));
    assert!(rarkzip_engine::supports_quick_password_test(&volumes[1]));
    assert!(test_password(&volumes[0], "vol-pass").unwrap());
    assert!(!test_password(&volumes[0], "wrong-pass").unwrap());

    let out = temp.path().join("apate-volumes-out");
    extract_archive(
        &volumes[0],
        &ExtractOptions {
            mode: ExtractMode::Custom(out.clone()),
            password: Some("vol-pass".to_string()),
            propagate_motw: false,
            nested_depth: 0,
            ..Default::default()
        },
        &NoopSink,
    )
    .unwrap();
    assert_eq!(fs::read(out.join("secret.txt")).unwrap(), content);

    // 原始伪装分卷只读。
    for (volume, expected) in volumes.iter().zip(sizes) {
        assert_eq!(fs::metadata(volume).unwrap().len(), expected);
    }
}

#[test]
fn nested_apate_volumes_reuse_parent_password() {
    let temp = TempDir::new().unwrap();
    let inner_source = temp.path().join("inner.txt");
    write(&inner_source, b"inner apate volume");
    let inner = temp.path().join("inner.zip");
    let mut inner_create = options(ArchiveFormat::Zip);
    inner_create.password = Some("shared-pass".to_string());
    inner_create.include_root = false;
    create_archive(
        std::slice::from_ref(&inner_source),
        &inner,
        &inner_create,
        &NoopSink,
    )
    .unwrap();

    let bundle = temp.path().join("bundle");
    write(&bundle.join("readme.txt"), b"outer");
    let inner_bytes = fs::read(&inner).unwrap();
    fs::remove_file(&inner).unwrap();
    write_apate_volumes(&bundle, "inner.zip", &inner_bytes, 2, &[24, 40]);

    let outer = temp.path().join("outer.zip");
    let mut outer_create = options(ArchiveFormat::Zip);
    outer_create.password = Some("shared-pass".to_string());
    create_archive(
        std::slice::from_ref(&bundle),
        &outer,
        &outer_create,
        &NoopSink,
    )
    .unwrap();

    let out = temp.path().join("nested-apate-out");
    let sink = PasswordSink::new("unused");
    let result = extract_archive(
        &outer,
        &ExtractOptions {
            mode: ExtractMode::Custom(out.clone()),
            password: Some("shared-pass".to_string()),
            passwords: Vec::new(),
            propagate_motw: false,
            nested_depth: 3,
            ..Default::default()
        },
        &sink,
    )
    .unwrap();
    assert_eq!(result.nested_archives, 1);
    assert_eq!(sink.requests(), 0);
    assert_eq!(
        fs::read(out.join("bundle/inner.zip.001/inner.txt")).unwrap(),
        b"inner apate volume"
    );
}

#[test]
fn nested_password_prompt_is_used_when_parent_password_is_missing() {
    let temp = TempDir::new().unwrap();
    let inner_source = temp.path().join("inner.txt");
    write(&inner_source, b"prompt inner");
    let inner = temp.path().join("inner.zip");
    let mut inner_create = options(ArchiveFormat::Zip);
    inner_create.password = Some("inner-pass".to_string());
    inner_create.include_root = false;
    create_archive(
        std::slice::from_ref(&inner_source),
        &inner,
        &inner_create,
        &NoopSink,
    )
    .unwrap();

    let bundle = temp.path().join("bundle");
    std::fs::create_dir_all(&bundle).unwrap();
    fs::copy(&inner, bundle.join("inner.zip")).unwrap();
    write(&bundle.join("readme.txt"), b"outer");
    let outer = temp.path().join("outer.zip");
    create_archive(
        std::slice::from_ref(&bundle),
        &outer,
        &options(ArchiveFormat::Zip),
        &NoopSink,
    )
    .unwrap();

    let out = temp.path().join("prompt-nested-out");
    let sink = PasswordSink::new("inner-pass");
    let result = extract_archive(
        &outer,
        &ExtractOptions {
            mode: ExtractMode::Custom(out.clone()),
            password: None,
            passwords: Vec::new(),
            propagate_motw: false,
            nested_depth: 3,
            ..Default::default()
        },
        &sink,
    )
    .unwrap();
    assert_eq!(result.nested_archives, 1);
    assert_eq!(sink.requests(), 1);
    assert_eq!(
        fs::read(out.join("bundle/inner/inner.txt")).unwrap(),
        b"prompt inner"
    );
}

#[test]
fn nested_rar_reuses_parent_password_without_prompt() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/crypted.rar");
    let temp = TempDir::new().unwrap();
    let bundle = temp.path().join("bundle");
    std::fs::create_dir_all(&bundle).unwrap();
    fs::copy(&fixture, bundle.join("crypted.rar")).unwrap();
    write(&bundle.join("readme.txt"), b"outer");
    let outer = temp.path().join("outer.zip");
    let mut outer_create = options(ArchiveFormat::Zip);
    outer_create.password = Some("unrar".to_string());
    create_archive(
        std::slice::from_ref(&bundle),
        &outer,
        &outer_create,
        &NoopSink,
    )
    .unwrap();

    let out = temp.path().join("nested-rar-out");
    let sink = PasswordSink::new("unused");
    let result = extract_archive(
        &outer,
        &ExtractOptions {
            mode: ExtractMode::Custom(out.clone()),
            password: Some("unrar".to_string()),
            passwords: Vec::new(),
            propagate_motw: false,
            nested_depth: 3,
            ..Default::default()
        },
        &sink,
    )
    .unwrap();
    assert_eq!(result.nested_archives, 1);
    assert_eq!(sink.requests(), 0);
    assert!(out.join("bundle/crypted/.gitignore").exists());
}

#[test]
fn nested_rar_wrong_parent_password_prompts() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/crypted.rar");
    let temp = TempDir::new().unwrap();
    let bundle = temp.path().join("bundle");
    std::fs::create_dir_all(&bundle).unwrap();
    fs::copy(&fixture, bundle.join("crypted.rar")).unwrap();
    write(&bundle.join("readme.txt"), b"outer");
    let outer = temp.path().join("outer.zip");
    let mut outer_create = options(ArchiveFormat::Zip);
    outer_create.password = Some("outer-pass".to_string());
    create_archive(
        std::slice::from_ref(&bundle),
        &outer,
        &outer_create,
        &NoopSink,
    )
    .unwrap();

    let out = temp.path().join("nested-rar-prompt-out");
    let sink = PasswordSink::new("unrar");
    let result = extract_archive(
        &outer,
        &ExtractOptions {
            mode: ExtractMode::Custom(out.clone()),
            password: Some("outer-pass".to_string()),
            passwords: Vec::new(),
            propagate_motw: false,
            nested_depth: 3,
            ..Default::default()
        },
        &sink,
    )
    .unwrap();
    assert_eq!(result.nested_archives, 1);
    assert_eq!(sink.requests(), 1);
    assert!(out.join("bundle/crypted/.gitignore").exists());
}

fn pseudo_random_bytes(len: usize, seed: u32) -> Vec<u8> {
    let mut state = seed;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state & 0xff) as u8
        })
        .collect()
}

#[test]
fn split_zip_creation_roundtrip() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("payload.bin");
    let content = pseudo_random_bytes(220_000, 0x1234_5678);
    write(&source, &content);
    let archive = temp.path().join("split.zip");
    let mut create = options(ArchiveFormat::Zip);
    create.volume_size = Some(64 * 1024);

    let outcome =
        create_archive(std::slice::from_ref(&source), &archive, &create, &NoopSink).unwrap();
    assert!(outcome.volumes >= 3);
    assert_eq!(outcome.dest, temp.path().join("split.zip.001"));
    assert!(!archive.exists());

    let mut total = 0u64;
    for index in 1..=outcome.volumes {
        let volume = temp.path().join(format!("split.zip.{index:03}"));
        let size = fs::metadata(&volume).unwrap().len();
        assert!(size <= 64 * 1024);
        if index < outcome.volumes {
            assert_eq!(size, 64 * 1024);
        }
        total += size;
    }
    assert_eq!(total, outcome.compressed_bytes);

    let info = open_archive(&outcome.dest, &OpenOptions::default()).unwrap();
    assert_eq!(info.format, ArchiveFormat::Zip);
    assert_eq!(info.file_count(), 1);

    let from_second =
        open_archive(&temp.path().join("split.zip.002"), &OpenOptions::default()).unwrap();
    assert_eq!(from_second.file_count(), 1);

    let out = temp.path().join("split-out");
    extract_archive(
        &outcome.dest,
        &ExtractOptions {
            mode: ExtractMode::Custom(out.clone()),
            nested_depth: 0,
            ..Default::default()
        },
        &NoopSink,
    )
    .unwrap();
    assert_eq!(fs::read(out.join("payload.bin")).unwrap(), content);
}

#[test]
fn split_7z_creation_with_password_roundtrip() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("payload.bin");
    let content = pseudo_random_bytes(180_000, 0x8765_4321);
    write(&source, &content);
    let archive = temp.path().join("split.7z");
    let mut create = options(ArchiveFormat::SevenZ);
    create.password = Some("split-pass".to_string());
    create.volume_size = Some(64 * 1024);
    create.test_after = true;

    let outcome =
        create_archive(std::slice::from_ref(&source), &archive, &create, &NoopSink).unwrap();
    assert!(outcome.volumes >= 2);

    let info = open_archive(
        &outcome.dest,
        &OpenOptions {
            password: Some("split-pass".to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(info.format, ArchiveFormat::SevenZ);
    assert_eq!(info.file_count(), 1);

    let out = temp.path().join("split-7z-out");
    extract_archive(
        &outcome.dest,
        &ExtractOptions {
            mode: ExtractMode::Custom(out.clone()),
            password: Some("split-pass".to_string()),
            nested_depth: 0,
            ..Default::default()
        },
        &NoopSink,
    )
    .unwrap();
    assert_eq!(fs::read(out.join("payload.bin")).unwrap(), content);
}

#[test]
fn split_creation_overwrite_removes_stale_volumes() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("payload.bin");
    write(&source, &pseudo_random_bytes(180_000, 0xABCD_EF01));
    let archive = temp.path().join("stale.zip");

    let mut many = options(ArchiveFormat::Zip);
    many.volume_size = Some(64 * 1024);
    let first = create_archive(std::slice::from_ref(&source), &archive, &many, &NoopSink).unwrap();
    assert!(first.volumes >= 3);

    let mut fewer = options(ArchiveFormat::Zip);
    fewer.volume_size = Some(1024 * 1024);
    let second =
        create_archive(std::slice::from_ref(&source), &archive, &fewer, &NoopSink).unwrap();
    assert_eq!(second.volumes, 1);
    assert!(archive.with_file_name("stale.zip.001").exists());
    assert!(!archive.with_file_name("stale.zip.002").exists());
}

#[test]
fn split_sfx_is_rejected() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("payload.txt");
    write(&source, b"sfx");
    let archive = temp.path().join("sfx.exe");
    let mut create = options(ArchiveFormat::Zip);
    create.sfx = true;
    create.volume_size = Some(64 * 1024);
    let error =
        create_archive(std::slice::from_ref(&source), &archive, &create, &NoopSink).unwrap_err();
    assert!(error.to_string().contains("SFX"));
}
