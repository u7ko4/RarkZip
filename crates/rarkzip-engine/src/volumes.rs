//! 分卷归档识别与合并。
//!
//! `.zip.001` / `.7z.001` / `.tar.gz.001` 等格式本质上是把单个归档按字节切块。
//! 库后端通常只接受完整文件，因此这里把连续分卷按顺序合并到临时目录，并把
//! 原始分卷保持只读、不改名。RAR 的 `.part1.rar` / `.r00` 由 libunrar 原生处理，
//! 不走本模块的合并逻辑。
//!
//! 识别不依赖扩展名结尾：分卷编号可以被文字/伪装后缀包住，例如
//! `xxx.7z.001`、`xxx.7z.第001集.mp4`、`xxx.zip.001.mp4`、`xxx_0001_.7z`。
//! apate 伪装的每一卷也会保留分卷编号，还原后再按同一套规则合并。

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::format::ArchiveFormat;
use crate::paths::to_extended;

/// 分卷的命名方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeKind {
    /// `name.zip.001` / `name.zip.001.mp4`：按数字递增。
    Numbered,
    /// `name.part1.rar` / `name.part1.rar.mp4`：WinRAR 的 partN 命名。
    RarParts,
    /// `name.rar` + `name.r00` / `name.r00.mp4`：经典 RAR 命名。
    RarClassic,
}

#[derive(Debug, Clone)]
pub struct VolumeSet {
    /// 按卷序号升序排列的现有文件。
    pub members: Vec<PathBuf>,
    /// 调用方传入的文件在 `members` 中的位置。
    pub index: usize,
    pub kind: VolumeKind,
    /// 卷号之前的名称（不含编号）。
    pub prefix: String,
    /// 卷号之后的名称（可能包含伪装扩展名）。
    pub suffix: String,
}

impl VolumeSet {
    pub fn first(&self) -> &Path {
        &self.members[0]
    }

    pub fn is_rar(&self) -> bool {
        matches!(self.kind, VolumeKind::RarParts | VolumeKind::RarClassic)
    }
}

/// 识别 `path` 所属的分卷集合；不是分卷时返回 `None`。
///
/// 只做名称/目录判断，不读取内容。调用方需要自己校验第一卷一整串是否
/// 真的是归档，避免把 `IMG001.jpg, IMG002.jpg` 之类的文件误当分卷。
pub fn detect(path: &Path) -> Result<Option<VolumeSet>> {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(None);
    };
    if name.is_empty() {
        return Ok(None);
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));

    if let Some(set) = detect_rar_parts(parent, name)? {
        return Ok(Some(set));
    }
    if let Some(set) = detect_rar_classic(parent, name)? {
        return Ok(Some(set));
    }
    detect_numbered(parent, name)
}

fn detect_rar_parts(parent: &Path, name: &str) -> Result<Option<VolumeSet>> {
    let lower = name.to_ascii_lowercase();
    let Some(rar_pos) = lower.find(".rar") else {
        return Ok(None);
    };
    // `.rar` 之后只能是空或另一个伪装扩展名（如 `.mp4`）。
    let after_rar = &name[rar_pos + 4..];
    if !after_rar.is_empty() && !after_rar.starts_with('.') {
        return Ok(None);
    }
    let before_lower = &lower[..rar_pos];
    let Some(part_pos) = before_lower.rfind("part") else {
        return Ok(None);
    };
    let digits = &name[part_pos + 4..rar_pos];
    if digits.is_empty() || digits.len() > 4 || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return Ok(None);
    }
    let width = digits.len();
    let prefix = name[..part_pos].to_string();
    let suffix = name[rar_pos..].to_string();
    let members = consecutive_members(parent, &prefix, "part", width, 1, 999, &suffix);
    Ok(build_set(
        members,
        name,
        VolumeKind::RarParts,
        prefix,
        suffix,
    ))
}

fn detect_rar_classic(parent: &Path, name: &str) -> Result<Option<VolumeSet>> {
    let lower = name.to_ascii_lowercase();

    // 形态一：`name.rar` 或 `name.rar.mp4`。
    if let Some(rar_pos) = lower.find(".rar") {
        let after = &name[rar_pos + 4..];
        if after.is_empty() || after.starts_with('.') {
            let prefix = name[..rar_pos].to_string();
            let suffix = name[rar_pos..].to_string();
            // `name.part1.rar` 属于 WinRAR 编号，交给 detect_rar_parts。
            if !lower[..rar_pos].ends_with("part1") {
                let mut members = vec![parent.join(name)];
                let mask = &after;
                for number in 0..=99 {
                    let candidate = parent.join(format!("{prefix}.r{number:02}{mask}"));
                    if !candidate.is_file() {
                        break;
                    }
                    members.push(candidate);
                }
                if members.len() >= 2 {
                    return Ok(Some(VolumeSet {
                        members,
                        index: 0,
                        kind: VolumeKind::RarClassic,
                        prefix,
                        suffix,
                    }));
                }
            }
            // `.rar` 单卷：继续检查它是不是 `name.rar` 以外的编号形态。
            if !after.is_empty() && !after.starts_with('.') {
                return Ok(None);
            }
        }
    }

    // 形态二：`name.r00` / `name.r07.mp4`。从右向左找 `.rNN` 段，允许后面
    // 继续跟伪装扩展名。
    for dot in name.match_indices('.').map(|(index, _)| index).rev() {
        let rest = &name[dot + 1..];
        if rest.len() < 3 || !rest.starts_with('r') {
            continue;
        }
        let digits_len = rest[1..]
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .count();
        if !(2..=4).contains(&digits_len) {
            continue;
        }
        let refix = &name[..dot];
        let suffix = &rest[1 + digits_len..];
        let first = parent.join(format!("{refix}.rar{suffix}"));
        if !first.is_file() {
            continue;
        }
        let mut members = vec![first];
        for number in 0..=99 {
            let candidate = parent.join(format!("{refix}.r{number:02}{suffix}"));
            if !candidate.is_file() {
                break;
            }
            members.push(candidate);
        }
        if members.len() < 2 {
            continue;
        }
        let Some(index) = members
            .iter()
            .position(|member| member.file_name().and_then(|value| value.to_str()) == Some(name))
        else {
            continue;
        };
        return Ok(Some(VolumeSet {
            members,
            index,
            kind: VolumeKind::RarClassic,
            prefix: refix.to_string(),
            suffix: format!(".rar{suffix}"),
        }));
    }

    Ok(None)
}

fn detect_numbered(parent: &Path, name: &str) -> Result<Option<VolumeSet>> {
    struct Candidate {
        score: i32,
        prefix: String,
        suffix: String,
        members: Vec<PathBuf>,
        index: usize,
    }

    let mut best: Option<Candidate> = None;
    let bytes = name.as_bytes();
    let lower = name.to_ascii_lowercase();
    let named = contains_archive_token(&lower);
    let mut position = 0usize;
    while position < bytes.len() {
        if !bytes[position].is_ascii_digit() {
            position += 1;
            continue;
        }
        let start = position;
        while position < bytes.len() && bytes[position].is_ascii_digit() {
            position += 1;
        }
        let width = position - start;
        if !(2..=4).contains(&width) {
            continue;
        }
        // 没有归档标记的普通文件名只接受“编号被分隔符包住”的形态，
        // 避免为 `Concat1110 26.11 040.mp4` 之类的名字反复探测目录。
        let previous = start
            .checked_sub(1)
            .and_then(|index| bytes.get(index))
            .map(|byte| *byte as char);
        let next = bytes.get(position).map(|byte| *byte as char);
        let delimited =
            previous.is_none_or(is_number_delimiter) && next.is_none_or(is_number_delimiter);
        if !named && !delimited {
            continue;
        }
        let prefix = name[..start].to_string();
        let suffix = name[position..].to_string();

        let members = consecutive_members(parent, &prefix, "", width, 0, 999, &suffix);
        if members.len() < 2 {
            continue;
        }
        let Some(index) = members
            .iter()
            .position(|member| member.file_name().and_then(|value| value.to_str()) == Some(name))
        else {
            continue;
        };

        let surroundings = format!("{prefix}{suffix}").to_ascii_lowercase();
        let mut score = 0;
        if contains_archive_token(&surroundings) {
            score += 8;
        }
        if suffix.starts_with('.') {
            score += 2;
        }
        if prefix.ends_with('.') {
            score += 1;
        }
        // 优先把编号紧跟在归档扩展名之后的形态（`.7z.001`）当成卷号。
        if prefix.to_ascii_lowercase().ends_with(".zip")
            || prefix.to_ascii_lowercase().ends_with(".7z")
            || prefix.to_ascii_lowercase().ends_with(".rar")
        {
            score += 2;
        }
        if best.as_ref().is_none_or(|current| score > current.score) {
            best = Some(Candidate {
                score,
                prefix,
                suffix,
                members,
                index,
            });
        }
    }

    Ok(best.map(|candidate| VolumeSet {
        members: candidate.members,
        index: candidate.index,
        kind: VolumeKind::Numbered,
        prefix: candidate.prefix,
        suffix: candidate.suffix,
    }))
}

/// 枚举连续存在的卷。`infix` 用于 `partN` 命名，`start` 允许编号从 0 开始。
fn consecutive_members(
    parent: &Path,
    prefix: &str,
    infix: &str,
    width: usize,
    start: u32,
    limit: u32,
    suffix: &str,
) -> Vec<PathBuf> {
    let mut members = Vec::new();
    let mut started = false;
    for number in start..=limit {
        let digits = format!("{number:0width$}");
        if digits.len() > width {
            break;
        }
        let candidate = parent.join(format!("{prefix}{infix}{digits}{suffix}"));
        if candidate.is_file() {
            members.push(candidate);
            started = true;
        } else if number == 0 && !started {
            // 第 0 卷可以不存在；第 1 卷必须存在，否则不是一套分卷。
            continue;
        } else {
            break;
        }
    }
    members
}

fn build_set(
    members: Vec<PathBuf>,
    name: &str,
    kind: VolumeKind,
    prefix: String,
    suffix: String,
) -> Option<VolumeSet> {
    if members.len() < 2 {
        return None;
    }
    let index = members
        .iter()
        .position(|member| member.file_name().and_then(|value| value.to_str()) == Some(name))?;
    Some(VolumeSet {
        members,
        index,
        kind,
        prefix,
        suffix,
    })
}

/// 判断 `path` 是否属于一个分卷集合，且不是第一卷。
pub fn is_continuation(path: &Path) -> Result<bool> {
    Ok(detect(path)?.is_some_and(|set| set.index > 0))
}

/// 返回分卷集合的全部成员；不是分卷时返回自身。
pub fn part_paths(path: &Path) -> Result<Vec<PathBuf>> {
    Ok(detect(path)?
        .map(|set| set.members)
        .unwrap_or_else(|| vec![path.to_path_buf()]))
}

/// 名称里是否同时带有归档标记和可能的卷号。
///
/// 嵌套解压扫描先做这个轻量判断，避免对普通图片/视频做目录级分卷识别。
pub fn looks_like_archive_volume_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    if !contains_archive_token(&lower) {
        return false;
    }
    if lower.contains(".part") {
        return true;
    }
    let bytes = lower.as_bytes();
    let mut position = 0usize;
    while position < bytes.len() {
        if !bytes[position].is_ascii_digit() {
            position += 1;
            continue;
        }
        let start = position;
        while position < bytes.len() && bytes[position].is_ascii_digit() {
            position += 1;
        }
        if (2..=4).contains(&(position - start)) {
            return true;
        }
    }
    // 经典 RAR 续卷：`.r00` / `.r07`。
    lower.rfind('.').is_some_and(|dot| {
        let segment = &lower[dot + 1..];
        segment.len() >= 3
            && segment.starts_with('r')
            && segment[1..].chars().all(|ch| ch.is_ascii_digit())
    })
}

/// 按名称猜测分卷的归档格式（只用于嵌套扫描的轻量提示）。
pub fn name_format(path: &Path) -> Option<ArchiveFormat> {
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    let known: &[(&str, ArchiveFormat)] = &[
        (".tar.gz", ArchiveFormat::TarGz),
        (".tar.bz2", ArchiveFormat::TarBz2),
        (".tar.xz", ArchiveFormat::TarXz),
        (".tar.zst", ArchiveFormat::TarZst),
        (".zip", ArchiveFormat::Zip),
        (".zipx", ArchiveFormat::Zipx),
        (".7z", ArchiveFormat::SevenZ),
        (".rar", ArchiveFormat::Rar),
        (".tar", ArchiveFormat::Tar),
        (".gz", ArchiveFormat::Gz),
        (".bz2", ArchiveFormat::Bz2),
        (".xz", ArchiveFormat::Xz),
        (".zst", ArchiveFormat::Zst),
        (".cab", ArchiveFormat::Cab),
        (".iso", ArchiveFormat::Iso),
        (".lzh", ArchiveFormat::Lzh),
        (".lha", ArchiveFormat::Lzh),
        (".zpaq", ArchiveFormat::Zpaq),
    ];
    known
        .iter()
        .find_map(|(suffix, format)| name.contains(suffix).then_some(*format))
}

fn contains_archive_token(lower: &str) -> bool {
    [
        ".zip", ".zipx", ".7z", ".rar", ".tar", ".tgz", ".tbz", ".txz", ".tzst", ".gz", ".bz2",
        ".xz", ".zst", ".cab", ".iso", ".lzh", ".lha", ".zpaq",
    ]
    .iter()
    .any(|token| lower.contains(token))
}

fn is_number_delimiter(ch: char) -> bool {
    matches!(ch, '.' | '_' | '-' | ' ' | '(' | ')' | '[' | ']')
}

/// 把连续分卷按字节顺序合并到临时目录，返回合并后的路径。
///
/// 缓存按成员路径 + 大小 + 修改时间命中；原始分卷只读，不会被修改。
pub fn merge(members: &[PathBuf]) -> Result<PathBuf> {
    let mut total = 0u64;
    for member in members {
        total += member.metadata()?.len();
    }
    let cache = cache_path(members, total)?;
    if cache
        .metadata()
        .map(|meta| meta.len() == total)
        .unwrap_or(false)
    {
        return Ok(cache);
    }
    let temporary = cache.with_extension("vol.part");
    {
        let mut output = File::create(to_extended(&temporary))?;
        for member in members {
            let mut input = File::open(to_extended(member))?;
            std::io::copy(&mut input, &mut output)?;
        }
        output.flush()?;
    }
    fs::rename(to_extended(&temporary), to_extended(&cache))?;
    Ok(cache)
}

fn cache_path(members: &[PathBuf], total: u64) -> Result<PathBuf> {
    let mut hasher = DefaultHasher::new();
    for member in members {
        let metadata = member.metadata()?;
        member
            .canonicalize()
            .unwrap_or_else(|_| member.clone())
            .hash(&mut hasher);
        metadata.len().hash(&mut hasher);
        metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .hash(&mut hasher);
    }
    total.hash(&mut hasher);
    let dir = std::env::temp_dir().join("RarkZip").join("volumes");
    fs::create_dir_all(to_extended(&dir))?;
    Ok(dir.join(format!("{:016x}.vol", hasher.finish())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(directory: &Path, name: &str) {
        fs::write(directory.join(name), b"x").unwrap();
    }

    #[test]
    fn detects_masked_numbered_volumes() {
        let temp = tempfile::tempdir().unwrap();
        touch(temp.path(), "CH34.zip.001.mp4");
        touch(temp.path(), "CH34.zip.002.mp4");
        let set = detect(&temp.path().join("CH34.zip.002.mp4"))
            .unwrap()
            .unwrap();
        assert_eq!(set.kind, VolumeKind::Numbered);
        assert_eq!(set.index, 1);
        assert_eq!(set.members.len(), 2);
        assert!(is_continuation(&temp.path().join("CH34.zip.002.mp4")).unwrap());
        assert!(!is_continuation(&temp.path().join("CH34.zip.001.mp4")).unwrap());
    }

    #[test]
    fn detects_volumes_with_text_around_the_number() {
        let temp = tempfile::tempdir().unwrap();
        touch(temp.path(), "xxx.7z.第001集.mp4");
        touch(temp.path(), "xxx.7z.第002集.mp4");
        let set = detect(&temp.path().join("xxx.7z.第001集.mp4"))
            .unwrap()
            .unwrap();
        assert_eq!(set.members.len(), 2);
        assert_eq!(set.index, 0);
    }

    #[test]
    fn detects_rar_part_and_classic_names() {
        let temp = tempfile::tempdir().unwrap();
        touch(temp.path(), "movie.part1.rar");
        touch(temp.path(), "movie.part2.rar");
        let set = detect(&temp.path().join("movie.part2.rar"))
            .unwrap()
            .unwrap();
        assert_eq!(set.kind, VolumeKind::RarParts);
        assert_eq!(set.index, 1);

        touch(temp.path(), "old.rar");
        touch(temp.path(), "old.r00");
        touch(temp.path(), "old.r01");
        let set = detect(&temp.path().join("old.r00")).unwrap().unwrap();
        assert_eq!(set.kind, VolumeKind::RarClassic);
        assert_eq!(set.index, 1);
        assert_eq!(set.members.len(), 3);
    }

    #[test]
    fn ignores_unrelated_numbered_files_at_payload_level() {
        let temp = tempfile::tempdir().unwrap();
        touch(temp.path(), "IMG_001.jpg");
        touch(temp.path(), "IMG_002.jpg");
        // 名称本身会被识别成编号序列，调用方必须再做内容校验。
        assert!(detect(&temp.path().join("IMG_001.jpg")).unwrap().is_some());
        assert!(!looks_like_archive_volume_name(
            &temp.path().join("IMG_001.jpg")
        ));
    }
}
