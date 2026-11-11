use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::archive::{open_archive, test_password, ArchiveInfo, OpenOptions};
use crate::error::{EngineError, Result};
use crate::format::{ArchiveFormat, Codepage};
use crate::paths::archive_stem;
use crate::progress::{check_cancelled, Phase, Progress, ProgressSink};
use crate::security::{delete_to_recycle, read_motw};
use crate::sniff;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ExtractMode {
    /// 智能解压：按包内结构决定目标。
    #[default]
    Smart,
    /// 解压到压缩包所在目录。
    Here,
    /// 解压到与压缩包同名的文件夹。
    Folder,
    /// 解压到指定目录。
    Custom(PathBuf),
}

#[derive(Debug, Clone)]
pub struct ExtractOptions {
    pub mode: ExtractMode,
    /// 已确认可用的密码（父包密码或用户输入）。
    pub password: Option<String>,
    /// 自动尝试的候选密码（密码本顺序）；嵌套包会先用已验证密码，再试这里。
    pub passwords: Vec<String>,
    pub codepage: Codepage,
    /// 只解压这些条目索引；None 表示全部。目录索引会自动展开。
    pub selected: Option<Vec<usize>>,
    pub overwrite: bool,
    pub strip_root: bool,
    pub propagate_motw: bool,
    /// 嵌套解压剩余深度；0 表示不展开。
    pub nested_depth: u8,
    pub delete_archive: bool,
    /// 解压产物写入后执行 Windows AMSI 扫描。
    pub amsi_scan: bool,
    /// AMSI 单文件扫描缓冲上限（KB）。
    pub amsi_buffer_kb: u32,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            mode: ExtractMode::Smart,
            password: None,
            passwords: Vec::new(),
            codepage: Codepage::Auto,
            selected: None,
            overwrite: true,
            strip_root: false,
            propagate_motw: true,
            nested_depth: 3,
            delete_archive: false,
            amsi_scan: false,
            amsi_buffer_kb: 1024,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExtractOutcome {
    pub dest: PathBuf,
    pub format: ArchiveFormat,
    pub files: usize,
    pub dirs: usize,
    pub bytes: u64,
    pub nested_archives: usize,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SmartTarget {
    pub dest: PathBuf,
    /// 单根文件夹直达时需要剥掉的前缀。
    pub strip_prefix: Option<String>,
    pub reason: String,
}

/// 智能解压目标决策：
/// 1. 单个文件 → 当前目录；
/// 2. 单个根文件夹 → 当前目录，保留该根文件夹；
/// 3. 其他结构 → 与压缩包同名的文件夹。
pub fn smart_target(path: &Path, info: &ArchiveInfo) -> SmartTarget {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let files: Vec<_> = info.entries.iter().filter(|entry| !entry.is_dir).collect();

    if info.entries.len() == 1 && files.len() == 1 {
        return SmartTarget {
            dest: parent,
            strip_prefix: None,
            reason: "单文件：解压到当前目录".to_string(),
        };
    }

    if single_root(info).is_some() {
        return SmartTarget {
            dest: parent,
            strip_prefix: None,
            reason: "单根文件夹：解压到当前目录并保留根文件夹".to_string(),
        };
    }

    SmartTarget {
        dest: parent.join(archive_stem(path)),
        strip_prefix: None,
        reason: "常规：解压到同名文件夹".to_string(),
    }
}

fn single_root(info: &ArchiveInfo) -> Option<String> {
    let mut root: Option<String> = None;
    let mut has_nested = false;
    for entry in &info.entries {
        if entry.path.is_empty() {
            continue;
        }
        let first = entry.path.split('/').next()?.to_string();
        if entry.path.contains('/') {
            has_nested = true;
        } else if !entry.is_dir {
            // 顶层文件说明不是“单一根文件夹”。
            return None;
        }
        match &root {
            Some(existing) if existing != &first => return None,
            None => root = Some(first),
            _ => {}
        }
    }
    if has_nested {
        root
    } else {
        None
    }
}

pub fn extract_archive(
    path: &Path,
    options: &ExtractOptions,
    sink: &dyn ProgressSink,
) -> Result<ExtractOutcome> {
    let mut visited = HashSet::new();
    let depth = options.nested_depth;
    extract_inner(path, options, sink, depth, &mut visited)
}

/// 打开归档并自动解决密码：
///
/// 1. 已知密码（父包/用户输入）；
/// 2. 空密码；
/// 3. 密码本顺序。
///
/// 全部失败才返回 `PasswordRequired`，由 UI/CLI 弹出输入框。
fn open_with_password_resolution(
    path: &Path,
    options: &ExtractOptions,
    sink: &dyn ProgressSink,
) -> Result<(ArchiveInfo, Option<String>)> {
    let open = |password: Option<String>| {
        open_archive(
            path,
            &OpenOptions {
                password,
                codepage: options.codepage,
            },
        )
    };

    let mut candidates = Vec::new();
    if let Some(password) = &options.password {
        if !password.is_empty() {
            candidates.push(password.clone());
        }
    }
    // 只有能快速校验密码的格式才自动尝试空密码/密码本；RAR、7z 等格式
    // 每个候选都意味着完整解压，直接交给 UI/CLI 请求用户输入。
    if crate::archive::supports_quick_password_test(path) {
        if !candidates.iter().any(|candidate| candidate.is_empty()) {
            candidates.push(String::new());
        }
        for password in &options.passwords {
            if !password.is_empty() && !candidates.contains(password) {
                candidates.push(password.clone());
            }
        }
    }

    match open(options.password.clone()) {
        Ok(info) if !info.encrypted => return Ok((info, None)),
        Ok(info) => {
            if let Some(password) = &options.password {
                match test_password(path, password) {
                    Ok(true) => return Ok((info, Some(password.clone()))),
                    Ok(false) => {}
                    Err(EngineError::WrongPassword) | Err(EngineError::PasswordRequired) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Err(EngineError::PasswordRequired) | Err(EngineError::WrongPassword) => {}
        Err(error) => return Err(error),
    }

    let total = candidates.len();
    for (index, candidate) in candidates.iter().enumerate() {
        check_cancelled(sink)?;
        sink.update(&Progress::new(
            Phase::Password,
            index as u64,
            total as u64,
            format!("尝试密码 {}/{}", index + 1, total),
        ));
        match test_password(path, candidate) {
            Ok(true) => {
                let info = open(Some(candidate.clone()))?;
                return Ok((info, Some(candidate.clone())));
            }
            Ok(false) => {}
            Err(EngineError::WrongPassword) | Err(EngineError::PasswordRequired) => {}
            Err(error) => return Err(error),
        }
    }
    Err(EngineError::PasswordRequired)
}

fn extract_inner(
    path: &Path,
    options: &ExtractOptions,
    sink: &dyn ProgressSink,
    remaining_depth: u8,
    visited: &mut HashSet<PathBuf>,
) -> Result<ExtractOutcome> {
    if !path.exists() {
        return Err(EngineError::NotFound(path.to_path_buf()));
    }
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical) {
        return Err(EngineError::Other("检测到嵌套递归".to_string()));
    }
    check_cancelled(sink)?;
    sink.update(&Progress::new(
        Phase::Preparing,
        0,
        0,
        path.file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default(),
    ));

    let (info, working_password) = open_with_password_resolution(path, options, sink)?;

    let (dest, strip_prefix) = resolve_destination(path, &info, options);
    std::fs::create_dir_all(crate::paths::to_extended(&dest))?;

    let motw = if options.propagate_motw {
        read_motw(path)
    } else {
        None
    };
    let motw = motw.as_deref();
    let selected = options.selected.as_deref();

    let backend = match info.format {
        ArchiveFormat::Zip | ArchiveFormat::Zipx => crate::zip_backend::extract(
            &info.payload_path,
            &info,
            &dest,
            strip_prefix.as_deref(),
            selected,
            working_password.as_deref(),
            options.overwrite,
            sink,
            motw,
        )?,
        ArchiveFormat::SevenZ => crate::sevenz_backend::extract(
            &info.payload_path,
            &info,
            &dest,
            strip_prefix.as_deref(),
            selected,
            working_password.as_deref(),
            sink,
            motw,
        )?,
        ArchiveFormat::Rar => crate::rar_backend::extract(
            &info.payload_path,
            &info,
            &dest,
            strip_prefix.as_deref(),
            selected,
            working_password.as_deref(),
            options.overwrite,
            sink,
            motw,
        )?,
        ArchiveFormat::Iso => crate::iso_backend::extract(
            &info.payload_path,
            &info,
            &dest,
            strip_prefix.as_deref(),
            selected,
            working_password.as_deref(),
            options.overwrite,
            sink,
            motw,
        )?,
        ArchiveFormat::Lzh => crate::lzh_backend::extract(
            &info.payload_path,
            &info,
            &dest,
            strip_prefix.as_deref(),
            selected,
            working_password.as_deref(),
            options.overwrite,
            sink,
            motw,
        )?,
        ArchiveFormat::Zpaq => crate::zpaq_backend::extract(
            &info.payload_path,
            &info,
            &dest,
            strip_prefix.as_deref(),
            selected,
            working_password.as_deref(),
            options.overwrite,
            sink,
            motw,
        )?,
        ArchiveFormat::Wim | ArchiveFormat::Cab | ArchiveFormat::Arj | ArchiveFormat::Asar => {
            crate::external7z::extract(
                &info.payload_path,
                &info,
                &dest,
                strip_prefix.as_deref(),
                selected,
                working_password.as_deref(),
                options.overwrite,
                sink,
                motw,
            )?
        }
        ArchiveFormat::Tar
        | ArchiveFormat::TarGz
        | ArchiveFormat::TarBz2
        | ArchiveFormat::TarXz
        | ArchiveFormat::TarZst => crate::tar_backend::extract(
            &info.payload_path,
            &info,
            &dest,
            strip_prefix.as_deref(),
            selected,
            sink,
            motw,
        )?,
        ArchiveFormat::Gz | ArchiveFormat::Bz2 | ArchiveFormat::Xz | ArchiveFormat::Zst => {
            crate::tar_backend::extract_single(
                &info.payload_path,
                &info,
                &dest,
                options.overwrite,
                sink,
                motw,
            )?
        }
        other => {
            return Err(EngineError::UnsupportedFormat(format!(
                "{} 暂不支持解压",
                other.label()
            )))
        }
    };

    let mut outcome = ExtractOutcome {
        dest: dest.clone(),
        format: info.format,
        files: backend.files,
        dirs: backend.dirs,
        bytes: backend.bytes,
        nested_archives: 0,
        skipped: backend.skipped,
    };

    let mut amsi_blocked = HashSet::new();
    if options.amsi_scan && !backend.written.is_empty() {
        let total_scan = backend.written.len() as u64;
        let mut warned_unavailable = false;
        for (index, written) in backend.written.iter().enumerate() {
            check_cancelled(sink)?;
            sink.update(&Progress::new(
                Phase::Scanning,
                index as u64,
                total_scan,
                written
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default(),
            ));
            match crate::amsi::scan_file(written, options.amsi_buffer_kb as u64 * 1024) {
                Ok(crate::amsi::AmsiVerdict::Detected) => {
                    let size = written.metadata().map(|meta| meta.len()).unwrap_or(0);
                    let _ = std::fs::remove_file(crate::paths::to_extended(written));
                    outcome.files = outcome.files.saturating_sub(1);
                    outcome.bytes = outcome.bytes.saturating_sub(size);
                    outcome.skipped.push(format!(
                        "{}（AMSI 检测到恶意内容，已删除）",
                        written.display()
                    ));
                    sink.warn(&format!(
                        "AMSI 检测到恶意内容并已删除：{}",
                        written.display()
                    ));
                    amsi_blocked.insert(written.clone());
                }
                Ok(crate::amsi::AmsiVerdict::Clean) => {}
                Ok(crate::amsi::AmsiVerdict::Unavailable(error)) => {
                    if !warned_unavailable {
                        warned_unavailable = true;
                        sink.warn(&format!("AMSI 扫描不可用：{error}"));
                    }
                }
                Err(error) => {
                    if !warned_unavailable {
                        warned_unavailable = true;
                        sink.warn(&format!("AMSI 扫描失败：{error}"));
                    }
                }
            }
        }
    }

    if remaining_depth > 0 && !backend.written.is_empty() {
        sink.update(&Progress::new(
            Phase::Nested,
            0,
            backend.written.len() as u64,
            "扫描解压产物…".to_string(),
        ));
        const MAX_NESTED_SCAN: usize = 4096;
        let scan_total = backend.written.len().min(MAX_NESTED_SCAN);
        let mut processed = 0u64;
        for (scan_index, written) in backend.written.iter().enumerate() {
            check_cancelled(sink)?;
            if amsi_blocked.contains(written) {
                continue;
            }
            if scan_index >= MAX_NESTED_SCAN {
                sink.warn("解压产物超过 4096 个，已停止嵌套扫描");
                break;
            }
            processed += 1;
            sink.update(&Progress::new(
                Phase::Nested,
                processed,
                scan_total as u64,
                written
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default(),
            ));
            if sniff::is_protected_container(written) {
                continue;
            }
            if crate::volumes::is_continuation(written).unwrap_or(false) {
                continue;
            }
            let Ok(Some(format)) = crate::archive::probe_archive(written) else {
                continue;
            };
            match format {
                ArchiveFormat::Zip
                | ArchiveFormat::SevenZ
                | ArchiveFormat::Rar
                | ArchiveFormat::Iso
                | ArchiveFormat::Wim
                | ArchiveFormat::Cab
                | ArchiveFormat::Lzh
                | ArchiveFormat::Arj
                | ArchiveFormat::Asar
                | ArchiveFormat::Tar
                | ArchiveFormat::TarGz
                | ArchiveFormat::TarBz2
                | ArchiveFormat::TarXz
                | ArchiveFormat::TarZst
                | ArchiveFormat::Gz
                | ArchiveFormat::Bz2
                | ArchiveFormat::Xz
                | ArchiveFormat::Zst => {}
                _ => continue,
            }
            let nested_options = ExtractOptions {
                mode: ExtractMode::Folder,
                password: working_password.clone(),
                passwords: options.passwords.clone(),
                codepage: options.codepage,
                selected: None,
                overwrite: options.overwrite,
                strip_root: false,
                propagate_motw: options.propagate_motw,
                nested_depth: remaining_depth - 1,
                delete_archive: false,
                amsi_scan: options.amsi_scan,
                amsi_buffer_kb: options.amsi_buffer_kb,
            };
            match extract_inner(written, &nested_options, sink, remaining_depth - 1, visited) {
                Ok(nested) => {
                    outcome.nested_archives += 1 + nested.nested_archives;
                    outcome.files += nested.files;
                    outcome.dirs += nested.dirs;
                    outcome.bytes += nested.bytes;
                    outcome.skipped.extend(nested.skipped);
                }
                Err(EngineError::PasswordRequired) | Err(EngineError::WrongPassword) => {
                    outcome
                        .skipped
                        .push(format!("{}（嵌套包需要密码）", written.display()));
                }
                Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
                Err(err) => {
                    outcome
                        .skipped
                        .push(format!("{}（嵌套解压失败：{err}）", written.display()));
                }
            }
        }
    }

    if options.delete_archive {
        for part in crate::volumes::part_paths(path)? {
            delete_to_recycle(&part)?;
        }
    }

    sink.update(&Progress::new(
        Phase::Finished,
        1,
        1,
        format!(
            "完成 · {} 个文件 · {}",
            outcome.files,
            crate::util::format_size(outcome.bytes)
        ),
    ));
    Ok(outcome)
}

fn resolve_destination(
    path: &Path,
    info: &ArchiveInfo,
    options: &ExtractOptions,
) -> (PathBuf, Option<String>) {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    match &options.mode {
        ExtractMode::Smart => {
            let target = smart_target(path, info);
            let strip = if options.strip_root {
                target.strip_prefix.clone().or_else(|| single_root(info))
            } else {
                target.strip_prefix
            };
            (target.dest, strip)
        }
        ExtractMode::Here => {
            let strip = if options.strip_root {
                single_root(info)
            } else {
                None
            };
            (parent, strip)
        }
        ExtractMode::Folder => {
            let strip = if options.strip_root {
                single_root(info)
            } else {
                None
            };
            (parent.join(archive_stem(path)), strip)
        }
        ExtractMode::Custom(dest) => {
            let strip = if options.strip_root {
                single_root(info)
            } else {
                None
            };
            (dest.clone(), strip)
        }
    }
}

/// 把目录索引展开成包含其下所有条目的索引集合。
pub fn expand_selection(info: &ArchiveInfo, selected: &[usize]) -> HashSet<usize> {
    let mut set: HashSet<usize> = selected.iter().copied().collect();
    let prefixes: Vec<String> = info
        .entries
        .iter()
        .filter(|entry| entry.is_dir && set.contains(&entry.index))
        .map(|entry| format!("{}/", entry.path))
        .collect();
    if prefixes.is_empty() {
        return set;
    }
    for entry in &info.entries {
        if prefixes.iter().any(|prefix| entry.path.starts_with(prefix)) {
            set.insert(entry.index);
        }
    }
    set
}
