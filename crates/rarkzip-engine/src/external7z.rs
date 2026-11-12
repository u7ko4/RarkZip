//! 外部 7-Zip / NanaZip 兼容后端。
//!
//! 用于 ISO / WIM / CAB / LZH / ARJ / ASAR 等重型或受格式约束的容器。TDD 允许
//! 将这类格式交给外部 7z 引擎；解压始终先隔离到临时目录，再按安全路径复制，
//! 避免第三方工具直接写入用户目录。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

use crate::archive::{ArchiveEntry, ArchiveInfo, VerifyReport};
use crate::error::{EngineError, Result};
use crate::format::ArchiveFormat;
use crate::paths::{normalize_entry_name, safe_join, to_extended};
use crate::progress::{check_cancelled, Phase, Progress, ProgressSink};
use crate::security::{apply_motw, set_mtime};
use crate::zip_backend::ExtractOutcome;

static SEVEN_ZIP: OnceLock<Option<PathBuf>> = OnceLock::new();

pub fn available() -> Option<PathBuf> {
    SEVEN_ZIP.get_or_init(find_exe).clone()
}

fn find_exe() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("RARKZIP_7Z") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            for name in ["7z.exe", "7zz.exe", "7za.exe", "7z", "7zz", "7za"] {
                let candidate = parent.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    for candidate in [
        "C:\\Program Files\\7-Zip\\7z.exe",
        "C:\\Program Files (x86)\\7-Zip\\7z.exe",
        "C:\\Program Files\\NanaZip\\7z.exe",
        "/usr/bin/7z",
        "/usr/bin/7zz",
        "/usr/bin/7zzs",
        "/usr/bin/7za",
        "/usr/local/bin/7z",
        "/usr/local/bin/7zz",
    ] {
        if Path::new(candidate).is_file() {
            return Some(PathBuf::from(candidate));
        }
    }
    // 依次探测 PATH 中的 7z / 7zz，兼容不同发行版的命名。
    for name in ["7z", "7zz", "7zzs"] {
        if Command::new(name).arg("i").output().is_ok() {
            return Some(PathBuf::from(name));
        }
    }
    None
}

fn run(args: &[String]) -> Result<Output> {
    let exe = available().ok_or_else(|| {
        EngineError::UnsupportedFormat(
            "该格式需要外部 7-Zip / NanaZip；请安装后重试，或设置 RARKZIP_7Z".to_string(),
        )
    })?;
    let output = Command::new(&exe)
        .args(args)
        .output()
        .map_err(|error| EngineError::Other(format!("无法启动 7-Zip：{error}")))?;
    if output.status.success() {
        return Ok(output);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let message = if stderr.trim().is_empty() {
        stdout.trim().to_string()
    } else {
        stderr.trim().to_string()
    };
    let lower = message.to_ascii_lowercase();
    if lower.contains("wrong password")
        || lower.contains("data error in encrypted file")
        || lower.contains("can not open encrypted archive")
    {
        return Err(EngineError::WrongPassword);
    }
    if lower.contains("enter password")
        || lower.contains("missing password")
        || lower.contains("password is required")
    {
        return Err(EngineError::PasswordRequired);
    }
    Err(EngineError::DataError(format!(
        "7-Zip 返回错误：{}",
        message.lines().last().unwrap_or("未知错误")
    )))
}

pub(crate) fn list(
    path: &Path,
    format: ArchiveFormat,
    password: Option<&str>,
) -> Result<ArchiveInfo> {
    let mut args = vec![
        "l".to_string(),
        "-slt".to_string(),
        path.to_string_lossy().to_string(),
    ];
    if let Some(password) = password {
        args.insert(1, format!("-p{password}"));
    }
    let output = run(&args)?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();
    let mut block: Vec<(String, String)> = Vec::new();
    let mut any_encrypted = false;
    let mut index = 0usize;
    for line in text.lines().chain(std::iter::once("")) {
        if let Some((key, value)) = line.split_once(" = ") {
            block.push((key.trim().to_string(), value.trim().to_string()));
            continue;
        }
        if line.trim().is_empty() && !block.is_empty() {
            let field = |name: &str| {
                block
                    .iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.as_str())
            };
            let Some(entry_path) = field("Path") else {
                block.clear();
                continue;
            };
            // 归档自身信息块没有 Folder/Attributes。
            if field("Folder").is_none() && field("Attributes").is_none() {
                block.clear();
                continue;
            }
            let path_name = normalize_entry_name(&entry_path.replace('\\', "/"));
            if path_name.is_empty() {
                block.clear();
                continue;
            }
            let is_dir = field("Folder") == Some("+");
            let encrypted = field("Encrypted") == Some("+");
            any_encrypted |= encrypted;
            let size = field("Size")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            let compressed = field("Packed Size")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            let modified = field("Modified").and_then(parse_time);
            let crc = field("CRC")
                .filter(|value| !value.is_empty())
                .and_then(|value| u32::from_str_radix(value, 16).ok())
                .map(u64::from);
            let method = field("Method").unwrap_or("—").to_string();
            let comment = field("Comment")
                .filter(|value| !value.is_empty())
                .map(ToString::to_string);
            entries.push(ArchiveEntry {
                index,
                path: path_name,
                is_dir,
                size,
                compressed_size: compressed,
                modified,
                method,
                encrypted,
                crc32: crc,
                is_symlink: false,
                comment,
            });
            index += 1;
            block.clear();
        }
    }
    Ok(ArchiveInfo {
        path: path.to_path_buf(),
        payload_path: path.to_path_buf(),
        disguised: false,
        format,
        entries,
        comment: None,
        encrypted: any_encrypted,
        protected_container: false,
        payload_offset: 0,
        note: Some("使用外部 7-Zip/NanaZip 后端读取".to_string()),
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn extract(
    path: &Path,
    info: &ArchiveInfo,
    dest: &Path,
    strip_prefix: Option<&str>,
    selected: Option<&[usize]>,
    password: Option<&str>,
    overwrite: bool,
    sink: &dyn ProgressSink,
    motw: Option<&[u8]>,
) -> Result<ExtractOutcome> {
    let selected_set: Option<HashSet<usize>> =
        selected.map(|indices| crate::archive::expand_selection(info, indices));
    let temp = std::env::temp_dir()
        .join("RarkZip")
        .join("external7z")
        .join(format!("{}-{}", std::process::id(), hash_path(path)));
    if temp.exists() {
        let _ = std::fs::remove_dir_all(to_extended(&temp));
    }
    std::fs::create_dir_all(to_extended(&temp))?;
    let mut args = vec![
        "x".to_string(),
        "-y".to_string(),
        format!("-o{}", temp.display()),
    ];
    if let Some(password) = password {
        args.push(format!("-p{password}"));
    }
    args.push(path.to_string_lossy().to_string());
    sink.update(&Progress::new(
        Phase::Extracting,
        0,
        1,
        format!("{}：调用 7-Zip 解包", info.format.label()),
    ));
    let result = run(&args);
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(to_extended(&temp));
        return Err(error);
    }

    let mut outcome = ExtractOutcome {
        files: 0,
        dirs: 0,
        bytes: 0,
        written: Vec::new(),
        skipped: Vec::new(),
    };
    let total = info.total_size();
    let mut done = 0u64;
    for entry in &info.entries {
        check_cancelled(sink)?;
        if let Some(set) = &selected_set {
            if !set.contains(&entry.index) {
                continue;
            }
        }
        let relative = strip_entry_prefix(&entry.path, strip_prefix);
        if relative.is_empty() {
            continue;
        }
        let target = safe_join(dest, relative)?;
        if entry.is_dir {
            std::fs::create_dir_all(to_extended(&target))?;
            outcome.dirs += 1;
            continue;
        }
        let source = temp.join(entry.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let source = if source.exists() {
            source
        } else {
            outcome
                .skipped
                .push(format!("{}（7-Zip 未输出该条目）", entry.path));
            continue;
        };
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(to_extended(parent))?;
        }
        if target.exists() && !overwrite {
            outcome
                .skipped
                .push(format!("{}（目标已存在）", entry.path));
            continue;
        }
        std::fs::copy(to_extended(&source), to_extended(&target))?;
        set_mtime(&target, entry.modified)?;
        if let Some(zone) = motw {
            if let Err(error) = apply_motw(&target, zone) {
                sink.warn(&format!("写入 MOTW 失败：{error}"));
            }
        }
        done += entry.size;
        outcome.files += 1;
        outcome.bytes += entry.size;
        outcome.written.push(target);
        sink.update(&Progress::new(
            Phase::Extracting,
            done,
            total,
            entry.path.clone(),
        ));
    }
    let _ = std::fs::remove_dir_all(to_extended(&temp));
    sink.update(&Progress::new(
        Phase::Extracting,
        total,
        total,
        "解压完成".to_string(),
    ));
    Ok(outcome)
}

pub(crate) fn verify(
    path: &Path,
    password: Option<&str>,
    sink: &dyn ProgressSink,
) -> Result<VerifyReport> {
    let mut args = vec!["t".to_string(), path.to_string_lossy().to_string()];
    if let Some(password) = password {
        args.insert(1, format!("-p{password}"));
    }
    sink.update(&Progress::new(Phase::Testing, 0, 1, "7-Zip 完整性测试"));
    run(&args)?;
    let info = list(
        path,
        crate::sniff::sniff(path)?
            .map(|value| value.format)
            .unwrap_or(ArchiveFormat::Unknown),
        password,
    )?;
    Ok(VerifyReport {
        format: info.format,
        entries: info.file_count(),
        bytes: info.total_size(),
    })
}

pub(crate) fn test_password(path: &Path, password: &str) -> Result<bool> {
    let args = vec![
        "t".to_string(),
        format!("-p{password}"),
        path.to_string_lossy().to_string(),
    ];
    match run(&args) {
        Ok(_) => Ok(true),
        Err(EngineError::WrongPassword) => Ok(false),
        Err(error) => Err(error),
    }
}

fn parse_time(value: &str) -> Option<i64> {
    use chrono::{Local, NaiveDateTime, TimeZone};
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .ok()
        .and_then(|value| Local.from_local_datetime(&value).single())
        .map(|value| value.timestamp())
}

fn hash_path(path: &Path) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    std::fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .hash(&mut hasher);
    hasher.finish()
}

fn strip_entry_prefix<'a>(name: &'a str, prefix: Option<&str>) -> &'a str {
    match prefix {
        Some(prefix) if !prefix.is_empty() => {
            let with_slash = format!("{prefix}/");
            name.strip_prefix(&with_slash)
                .or_else(|| (name == prefix).then_some(""))
                .unwrap_or(name)
        }
        _ => name,
    }
}
