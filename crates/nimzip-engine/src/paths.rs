use std::path::{Component, Path, PathBuf};

use crate::error::{EngineError, Result};

/// 将归档内条目名规范化为 `/` 分隔、无前导 `./` 的形式，便于展示和建树。
pub fn normalize_entry_name(name: &str) -> String {
    let mut parts = Vec::new();
    let normalized = name.replace('\\', "/");
    for part in normalized.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        parts.push(part);
    }
    parts.join("/")
}

/// 拆分条目名并拒绝越界、绝对路径、盘符、NTFS ADS 与 Windows 非法字符。
pub fn entry_segments(name: &str) -> Result<Vec<String>> {
    let mut segments = Vec::new();
    for part in name.replace('\\', "/").split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(EngineError::UnsafePath(name.to_string()));
        }
        if part.contains('\0') || part.contains(':') {
            return Err(EngineError::UnsafePath(name.to_string()));
        }
        if cfg!(windows)
            && part
                .chars()
                .any(|c| matches!(c, '*' | '?' | '"' | '<' | '>' | '|'))
        {
            return Err(EngineError::UnsafePath(name.to_string()));
        }
        if cfg!(windows) && is_reserved_windows_name(part) {
            return Err(EngineError::UnsafePath(name.to_string()));
        }
        segments.push(part.to_string());
    }
    Ok(segments)
}

fn is_reserved_windows_name(part: &str) -> bool {
    let stem = part
        .split('.')
        .next()
        .unwrap_or(part)
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    matches!(
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
    )
}

/// 把归档条目安全地拼接到目标目录下。
pub fn safe_join(root: &Path, name: &str) -> Result<PathBuf> {
    let segments = entry_segments(name)?;
    let mut path = root.to_path_buf();
    for segment in segments {
        path.push(segment);
    }
    Ok(path)
}

/// 条目名是否以目录分隔符结尾。
pub fn is_dir_entry_name(name: &str) -> bool {
    name.ends_with('/') || name.ends_with('\\')
}

/// 去掉归档文件的多重扩展名，得到用于同名文件夹的名称。
pub fn archive_stem(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "archive".to_string());
    let lower = name.to_ascii_lowercase();
    for suffix in [
        ".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst", ".tgz", ".tbz2", ".tbz", ".txz", ".tzst",
    ] {
        if lower.ends_with(suffix) {
            return name[..name.len() - suffix.len()].to_string();
        }
    }
    Path::new(&name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or(name)
}

/// 生成不冲突的子路径：`name`、`name (2)`、`name (3)`…
pub fn unique_path(parent: &Path, stem: &str, extension: Option<&str>) -> PathBuf {
    let make = |n: u32| -> PathBuf {
        let suffix = if n <= 1 {
            String::new()
        } else {
            format!(" ({n})")
        };
        let file = match extension {
            Some(ext) if !ext.is_empty() => format!("{stem}{suffix}.{ext}"),
            _ => format!("{stem}{suffix}"),
        };
        parent.join(file)
    };
    let mut index = 1;
    let mut candidate = make(index);
    while candidate.exists() {
        index += 1;
        if index > 9999 {
            break;
        }
        candidate = make(index);
    }
    candidate
}

/// Windows 长路径归一化：为绝对路径添加 `\\?\` 前缀。
pub fn to_extended(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let raw = path.as_os_str().to_string_lossy();
        if raw.starts_with(r"\\?\") {
            return path.to_path_buf();
        }
        if raw.starts_with(r"\\") {
            return PathBuf::from(format!(r"\\?\UNC\{}", raw.trim_start_matches(r"\\")));
        }
        if path.is_absolute() {
            return PathBuf::from(format!(r"\\?\{}", raw.replace('/', r"\")));
        }
        path.to_path_buf()
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

/// 校验一个归档内路径是否可作为单一路径组件使用。
pub fn is_safe_component(name: &str) -> bool {
    let path = Path::new(name);
    !name.is_empty()
        && !path.is_absolute()
        && path.components().all(|c| matches!(c, Component::Normal(_)))
}
