//! NimZip 归档引擎。
//!
//! 引擎不依赖任何 UI 框架：GUI、CLI 与未来服务端共用同一套识别、读写、
//! 智能解压、密码与安全能力。

pub mod amsi;
pub mod archive;
pub mod create;
pub mod disguise;
pub mod edit;
pub mod error;
pub mod external7z;
pub mod extract;
pub mod format;
pub mod hash;
pub mod iso_backend;
pub mod lzh_backend;
pub mod password;
pub mod paths;
pub mod progress;
pub mod rar_backend;
pub mod recovery;
pub mod repair;
pub mod security;
pub mod settings;
pub mod sevenz_backend;
pub mod sfx;
pub mod sniff;
pub mod tar_backend;
pub mod util;
pub mod volumes;
pub mod zip_backend;
pub mod zpaq_backend;

pub use amsi::{scan_bytes, scan_file, AmsiVerdict};
pub use archive::{
    archive_summary, open_archive, supports_quick_password_test, test_password, verify_archive,
    ArchiveEntry, ArchiveInfo, OpenOptions, VerifyReport,
};
pub use create::{create_archive, create_summary, CreateOptions, CreateOutcome, ZipAlgorithm};
pub use disguise::{detect_apate, is_disguised, resolve_payload, reveal_to_file};
pub use edit::{
    add_zip_directory, add_zip_entries, edit_zip, ZipAddReport, ZipEditOptions, ZipEditReport,
};
pub use error::{EngineError, Result};
pub use extract::{
    expand_selection, extract_archive, smart_target, ExtractMode, ExtractOptions, ExtractOutcome,
    SmartTarget,
};
pub use format::{ArchiveFormat, Codepage};
pub use hash::{
    checksum_contents, checksum_expectations, compare_hash, hash_bytes, hash_file, hash_file_many,
    hash_file_with_progress, load_checksum_file, parse_checksum_text, save_checksum_file,
    ChecksumFileEntry, HashAlgorithm,
};
pub use password::{PasswordBook, PasswordEntry};
pub use progress::{NoopSink, Phase, Progress, ProgressSink};
pub use recovery::{
    recover_password, RecoveryControl, RecoveryMode, RecoveryOptions, RecoveryOutcome,
};
pub use repair::{repair_zip, RepairReport};
pub use settings::{
    data_dir, passwords_path, settings_path, AdvancedSettings, CompressionSettings,
    ExtractionSettings, GeneralSettings, HashSettings, SecuritySettings, Settings, UiSettings,
    ViewSettings,
};
pub use sfx::{create_sfx, detect_sfx, extract_sfx, SfxConfig, SfxInfo};
pub use util::{format_ratio, format_size, format_unix_time};
