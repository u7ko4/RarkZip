use std::io::{Cursor, Read, Seek, SeekFrom};
use std::time::SystemTime;

use rarkzip_crypt::progress::NoopReporter;
use rarkzip_crypt::zip_writer::{ZipCompression, ZipWriter};

fn build_zip(compression: ZipCompression) -> Vec<u8> {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("hello.txt");
    std::fs::write(&file_path, b"hello zip writer\n").unwrap();
    let dir_path = temp.path().join("empty");
    std::fs::create_dir(&dir_path).unwrap();
    let total = std::fs::metadata(&file_path).unwrap().len();
    let cursor = Cursor::new(Vec::new());
    let reporter = NoopReporter;
    let mut writer = ZipWriter::new(cursor, compression, &reporter, total);
    writer.add_directory("folder/", SystemTime::now()).unwrap();
    writer
        .add_file("folder/hello.txt", &file_path, SystemTime::now())
        .unwrap();
    let cursor = writer.finish().unwrap();
    cursor.into_inner()
}

#[test]
fn zip_writer_round_trips_through_zip_crate() {
    for compression in [ZipCompression::Store, ZipCompression::Deflate] {
        let bytes = build_zip(compression);
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), 2);
        let directory = archive.by_name("folder/").unwrap();
        assert!(directory.is_dir());
        drop(directory);
        let mut file = archive.by_name("folder/hello.txt").unwrap();
        let mut content = Vec::new();
        file.read_to_end(&mut content).unwrap();
        assert_eq!(content, b"hello zip writer\n");
        drop(file);
        let mut cursor = archive.into_inner();
        cursor.seek(SeekFrom::Start(0)).unwrap();
        assert_eq!(cursor.position(), 0);
    }
}
