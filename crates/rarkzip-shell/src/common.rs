//! 与平台无关的纯函数：注册表路径、扩展名归一化与命令行格式化。
//!
//! 这些逻辑不依赖 Windows API，因此可以在任意平台上进行单元测试。

use std::path::Path;

use crate::{AssociationStatus, ShellOptions};

pub(crate) const CLASSES_PATH: &str = r"Software\Classes";
pub(crate) const FILE_SHELL_PATH: &str = r"*\shell";
pub(crate) const DIRECTORY_SHELL_PATH: &str = r"Directory\shell";
pub(crate) const DIRECTORY_BACKGROUND_SHELL_PATH: &str = r"Directory\Background\shell";

/// 所有 RarkZip 创建的注册表项都以该前缀开头，卸载时据此精确清理。
pub(crate) const RARKZIP_PREFIX: &str = "RarkZip.";

/// `ShellOptions::default()` 使用的压缩包扩展名，与文件关联默认列表保持一致。
pub(crate) const DEFAULT_ARCHIVE_EXTENSIONS: &[&str] = &[
    "zip", "zipx", "rar", "7z", "alz", "egg", "cab", "bh", "001", "arj", "lha", "lzh", "pma",
    "ace", "arc", "aes", "zpaq", "zstd", "br", "pea", "tar", "gz", "tgz", "bz", "bz2", "tbz",
    "tbz2", "xz", "txz", "lzma", "tlz", "lz", "uu", "uue", "xxe", "z", "lz4", "zst", "tzst",
];

const PLACEHOLDER: &str = "%1";
const PLACEHOLDER_DIR: &str = "%V";
const MAX_EXTENSION_LEN: usize = 32;

/// 一个经典资源管理器右键菜单动词的静态定义。
#[derive(Debug, Clone, Copy)]
pub(crate) struct VerbSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub args: &'static [&'static str],
    pub multi_select: Option<&'static str>,
}

pub(crate) const VERB_OPEN: VerbSpec = VerbSpec {
    key: "RarkZip.100.Open",
    label: "用 RarkZip 打开…",
    args: &["--gui", PLACEHOLDER],
    // 单文件动作：顶层平铺模式多选时隐藏；静态级联会忽略子项模型。
    multi_select: Some("Single"),
};

pub(crate) const VERB_EXTRACT_HERE: VerbSpec = VerbSpec {
    key: "RarkZip.130.ExtractHere",
    label: "解压到此处(&E)",
    args: &["--extract-here", PLACEHOLDER],
    multi_select: Some("Player"),
};

pub(crate) const VERB_EXTRACT_SMART: VerbSpec = VerbSpec {
    key: "RarkZip.110.ExtractSmart",
    label: "智能解压到此处(&S)",
    args: &["--extract-smart", PLACEHOLDER],
    multi_select: Some("Player"),
};

pub(crate) const VERB_EXTRACT_FOLDER: VerbSpec = VerbSpec {
    key: "RarkZip.120.ExtractToFolder",
    label: "解压到独立文件夹",
    args: &["--extract-folder", PLACEHOLDER],
    multi_select: Some("Player"),
};

/// 打开“选择解压路径”弹窗，完成路径 / 密码 / 冲突 / 进度流程。
pub(crate) const VERB_EXTRACT_OTHER: VerbSpec = VerbSpec {
    key: "RarkZip.140.ExtractOther",
    label: "解压到其他文件夹(&B)…",
    args: &["--quick-extract", PLACEHOLDER],
    // 弹窗一次处理一个压缩包；顶层平铺模式多选时隐藏（级联模式无法隐藏）。
    multi_select: Some("Single"),
};

/// 以管理员身份解压到压缩包所在目录（通过 UAC 重新启动）。
pub(crate) const VERB_EXTRACT_ADMIN: VerbSpec = VerbSpec {
    key: "RarkZip.300.ExtractAdmin",
    label: "以管理员身份解压",
    args: &["--elevated", "--quick-extract", PLACEHOLDER],
    // 每个条目都会触发一次 UAC，顶层平铺模式多选时隐藏。
    multi_select: Some("Single"),
};

/// 在主窗口中预览压缩包内容。
pub(crate) const VERB_PREVIEW: VerbSpec = VerbSpec {
    key: "RarkZip.320.Preview",
    label: "压缩文件预览…",
    args: &["--gui", PLACEHOLDER],
    // 单文件动作：顶层平铺模式多选时隐藏。
    multi_select: Some("Single"),
};

pub(crate) const VERB_EXTRACT_HERE_DELETE: VerbSpec = VerbSpec {
    key: "RarkZip.200.ExtractHereDelete",
    label: "解压到此处并删除",
    args: &["--extract-here", PLACEHOLDER, "--delete"],
    multi_select: Some("Player"),
};

pub(crate) const VERB_EXTRACT_SMART_DELETE: VerbSpec = VerbSpec {
    key: "RarkZip.210.ExtractSmartDelete",
    label: "智能解压到此处并删除",
    args: &["--extract-smart", PLACEHOLDER, "--delete"],
    multi_select: Some("Player"),
};

pub(crate) const VERB_EXTRACT_FOLDER_DELETE: VerbSpec = VerbSpec {
    key: "RarkZip.220.ExtractToFolderDelete",
    label: "解压到独立文件夹并删除",
    args: &["--extract-folder", PLACEHOLDER, "--delete"],
    multi_select: Some("Player"),
};

pub(crate) const VERB_TEST: VerbSpec = VerbSpec {
    key: "RarkZip.310.Test",
    label: "测试压缩文件",
    args: &["--cli", "t", PLACEHOLDER],
    multi_select: Some("Player"),
};

pub(crate) const VERB_HASH: VerbSpec = VerbSpec {
    key: "RarkZip.330.Hash",
    label: "计算文件哈希…",
    args: &["--hash", PLACEHOLDER],
    multi_select: Some("Player"),
};

/// 压缩包菜单中的哈希入口；使用独立 key，避免平铺模式与普通文件菜单冲突。
pub(crate) const VERB_HASH_ARCHIVE: VerbSpec = VerbSpec {
    key: "RarkZip.331.HashArchive",
    label: "计算文件哈希…",
    args: &["--hash", PLACEHOLDER],
    multi_select: Some("Player"),
};

pub(crate) const VERB_COMPRESS_ZIP: VerbSpec = VerbSpec {
    key: "RarkZip.100.CompressZip",
    label: "压缩为“文件名.zip”",
    args: &["--quick-compress", PLACEHOLDER, "--format", "zip"],
    multi_select: Some("Player"),
};

pub(crate) const VERB_COMPRESS_ZIP_DELETE: VerbSpec = VerbSpec {
    key: "RarkZip.300.CompressZipDelete",
    label: "压缩为“文件名.zip”并删除",
    args: &["--cli", "ch", "-f", "zip", "--delete-sources", PLACEHOLDER],
    multi_select: Some("Player"),
};

pub(crate) const VERB_COMPRESS_SEVENZ: VerbSpec = VerbSpec {
    key: "RarkZip.110.Compress7z",
    label: "压缩为“文件名.7z”",
    args: &["--quick-compress", PLACEHOLDER, "--format", "7z"],
    multi_select: Some("Player"),
};

pub(crate) const VERB_COMPRESS_TAR: VerbSpec = VerbSpec {
    key: "RarkZip.120.CompressTar",
    label: "压缩为“文件名.tar”",
    args: &["--cli", "ch", "-f", "tar", PLACEHOLDER],
    multi_select: Some("Player"),
};

pub(crate) const VERB_COMPRESS_TGZ: VerbSpec = VerbSpec {
    key: "RarkZip.130.CompressTgz",
    label: "压缩为“文件名.tgz”",
    args: &["--cli", "ch", "-f", "tgz", PLACEHOLDER],
    multi_select: Some("Player"),
};

pub(crate) const VERB_COMPRESS_GZ: VerbSpec = VerbSpec {
    key: "RarkZip.140.CompressGz",
    label: "压缩为“文件名.gz”",
    args: &["--cli", "ch", "-f", "gz", PLACEHOLDER],
    multi_select: Some("Player"),
};

pub(crate) const VERB_COMPRESS_XZ: VerbSpec = VerbSpec {
    key: "RarkZip.150.CompressXz",
    label: "压缩为“文件名.xz”",
    args: &["--cli", "ch", "-f", "xz", PLACEHOLDER],
    multi_select: Some("Player"),
};

pub(crate) const VERB_COMPRESS_ZST: VerbSpec = VerbSpec {
    key: "RarkZip.160.CompressZst",
    label: "压缩为“文件名.zst”",
    args: &["--cli", "ch", "-f", "zst", PLACEHOLDER],
    multi_select: Some("Player"),
};

/// 每个被选中的文件/文件夹各生成一个以自身名称命名的 ZIP。
/// `Document` 让资源管理器对多选中的每一项分别调用一次。
pub(crate) const VERB_COMPRESS_EACH_ZIP: VerbSpec = VerbSpec {
    key: "RarkZip.200.CompressEachZip",
    label: "压缩为独立的“文件名/文件夹名.zip”",
    args: &["--cli", "ce", "-f", "zip", PLACEHOLDER],
    multi_select: Some("Document"),
};

pub(crate) const VERB_COMPRESS_EACH_SEVENZ: VerbSpec = VerbSpec {
    key: "RarkZip.210.CompressEach7z",
    label: "压缩为独立的“文件名/文件夹名.7z”",
    args: &["--cli", "ce", "-f", "7z", PLACEHOLDER],
    multi_select: Some("Document"),
};

pub(crate) const VERB_COMPRESS_GUI: VerbSpec = VerbSpec {
    key: "RarkZip.000.NewArchive",
    label: "新建压缩包…(&C)",
    args: &["--gui", "--new", PLACEHOLDER],
    multi_select: Some("Player"),
};

/// 以管理员身份快速压缩为 ZIP（通过 UAC 重新启动）。
pub(crate) const VERB_COMPRESS_ADMIN: VerbSpec = VerbSpec {
    key: "RarkZip.400.CompressAdmin",
    label: "以管理员身份压缩",
    args: &[
        "--elevated",
        "--quick-compress",
        PLACEHOLDER,
        "--format",
        "zip",
    ],
    multi_select: Some("Player"),
};

pub(crate) const VERB_BACKGROUND_OPEN: VerbSpec = VerbSpec {
    key: "RarkZip.BackgroundOpen",
    label: "在此处打开 RarkZip",
    args: &["--gui"],
    multi_select: None,
};

pub(crate) const VERB_BACKGROUND_NEW: VerbSpec = VerbSpec {
    key: "RarkZip.BackgroundNew",
    label: "在此处新建压缩包",
    args: &["--gui", "--new-here", PLACEHOLDER_DIR],
    multi_select: None,
};

// 菜单顺序：打开 → 智能解压 → 同名文件夹 → 解压到此处 →
// 其他文件夹 → 删除系列 → 管理员解压 → 测试 → 预览。资源管理器实际
// 按注册表子键字典序显示，因此各动词 key 使用数字前缀固定该顺序。
const EXTRACT_VERBS: &[(&str, &VerbSpec)] = &[
    ("open", &VERB_OPEN),
    ("smart", &VERB_EXTRACT_SMART),
    ("folder", &VERB_EXTRACT_FOLDER),
    ("here", &VERB_EXTRACT_HERE),
    ("other", &VERB_EXTRACT_OTHER),
    ("here_delete", &VERB_EXTRACT_HERE_DELETE),
    ("smart_delete", &VERB_EXTRACT_SMART_DELETE),
    ("folder_delete", &VERB_EXTRACT_FOLDER_DELETE),
    ("admin", &VERB_EXTRACT_ADMIN),
    ("test", &VERB_TEST),
    ("preview", &VERB_PREVIEW),
];

const COMPRESS_VERBS: &[(&str, &VerbSpec)] = &[
    // “新建压缩包”固定在菜单最上方（用户此前明确要求）。
    ("gui", &VERB_COMPRESS_GUI),
    ("zip", &VERB_COMPRESS_ZIP),
    ("sevenz", &VERB_COMPRESS_SEVENZ),
    ("tar", &VERB_COMPRESS_TAR),
    ("tgz", &VERB_COMPRESS_TGZ),
    ("gz", &VERB_COMPRESS_GZ),
    ("xz", &VERB_COMPRESS_XZ),
    ("zst", &VERB_COMPRESS_ZST),
    ("each_zip", &VERB_COMPRESS_EACH_ZIP),
    ("each_7z", &VERB_COMPRESS_EACH_SEVENZ),
    ("zip_delete", &VERB_COMPRESS_ZIP_DELETE),
    ("admin", &VERB_COMPRESS_ADMIN),
];

const EMPTY_VERBS: &[(&str, &VerbSpec)] = &[
    ("open", &VERB_BACKGROUND_OPEN),
    ("new", &VERB_BACKGROUND_NEW),
];

fn enabled_verbs(
    keys: &[String],
    table: &'static [(&'static str, &'static VerbSpec)],
) -> Vec<&'static VerbSpec> {
    let mut verbs = Vec::new();
    for (key, spec) in table {
        if keys.iter().any(|enabled| enabled == key) {
            verbs.push(*spec);
        }
    }
    verbs
}

/// 按设置返回文件右键菜单中的解压/打开动作。
pub(crate) fn enabled_extract_verbs(options: &ShellOptions) -> Vec<&'static VerbSpec> {
    enabled_verbs(&options.extract_verbs, EXTRACT_VERBS)
}

/// 按设置返回文件/文件夹右键菜单中的压缩动作。
pub(crate) fn enabled_compress_verbs(options: &ShellOptions) -> Vec<&'static VerbSpec> {
    enabled_verbs(&options.compress_verbs, COMPRESS_VERBS)
}

/// 按设置返回目录空白处右键菜单动作。
pub(crate) fn enabled_empty_verbs(options: &ShellOptions) -> Vec<&'static VerbSpec> {
    enabled_verbs(&options.empty_verbs, EMPTY_VERBS)
}

/// 归一化扩展名：去掉可选的前导点、转小写并校验字符集。
///
/// 返回的字符串不包含前导点；非法输入返回 `None`。
pub(crate) fn normalize_extension(extension: &str) -> Option<String> {
    let trimmed = extension.trim();
    let trimmed = trimmed.strip_prefix('.').unwrap_or(trimmed);
    if trimmed.is_empty() || trimmed.len() > MAX_EXTENSION_LEN {
        return None;
    }
    if trimmed.split('.').any(|part| part.is_empty()) {
        return None;
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
    {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

/// 生成压缩包专用菜单的 `AppliesTo` 查询：文件名以任一压缩包扩展名结尾。
pub(crate) fn archive_applies_to(extensions: &[String]) -> Option<String> {
    let mut terms: Vec<String> = Vec::new();
    for extension in extensions {
        let Some(extension) = normalize_extension(extension) else {
            continue;
        };
        let term = format!("System.FileName:~>\".{extension}\"");
        if !terms.contains(&term) {
            terms.push(term);
        }
    }
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

/// 非压缩包文件的 `AppliesTo` 查询，用于把压缩动作从压缩包上排除。
pub(crate) fn non_archive_applies_to(extensions: &[String]) -> Option<String> {
    archive_applies_to(extensions).map(|query| format!("NOT ({query})"))
}

pub(crate) fn prog_id(extension: &str) -> String {
    format!("RarkZip.{extension}")
}

pub(crate) fn extension_key(extension: &str) -> String {
    format!(".{extension}")
}

pub(crate) fn association_label(extension: &str) -> String {
    format!("RarkZip {} archive", extension.to_ascii_uppercase())
}

pub(crate) fn icon_value(exe: &Path) -> String {
    format!("{},0", quote_windows_arg_always(&exe.to_string_lossy()))
}

/// 按 Windows 参数解析规则为单个参数加引号。
///
/// 反斜杠只在引号前和结尾处加倍，因此包含空格或引号的路径不会被提前截断。
pub(crate) fn quote_windows_arg_always(arg: &str) -> String {
    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => {
                backslashes += 1;
                quoted.push('\\');
            }
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                backslashes = 0;
                quoted.push(ch);
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes));
    quoted.push('"');
    quoted
}

/// 仅在需要时加引号；`%1` 等占位符由 [`build_command`] 单独处理。
pub(crate) fn quote_windows_arg(arg: &str) -> String {
    if arg.is_empty() || arg.chars().any(|ch| ch.is_whitespace() || ch == '"') {
        quote_windows_arg_always(arg)
    } else {
        arg.to_owned()
    }
}

/// 构造注册表 `command` 值：可执行文件始终加引号，`%1` / `%V` 始终加引号。
pub(crate) fn build_command(exe: &Path, args: &[&str]) -> String {
    let mut command = quote_windows_arg_always(&exe.to_string_lossy());
    for arg in args {
        command.push(' ');
        if *arg == PLACEHOLDER || *arg == PLACEHOLDER_DIR {
            command.push('"');
            command.push_str(arg);
            command.push('"');
        } else {
            command.push_str(&quote_windows_arg(arg));
        }
    }
    command
}

/// 将当前关联 ProgID 归类为 RarkZip、其他应用或未分类。
pub(crate) fn classify_association(current: &str, our_prog_id: &str) -> AssociationStatus {
    if current.eq_ignore_ascii_case(our_prog_id) {
        AssociationStatus::RarkZip
    } else {
        AssociationStatus::Other(current.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::ShellScope;

    fn options() -> ShellOptions {
        ShellOptions {
            exe: PathBuf::from(r"C:\Program Files\RarkZip\rarkzip.exe"),
            scope: ShellScope::CurrentUser,
            ..Default::default()
        }
    }

    #[test]
    fn builds_archive_and_non_archive_applies_to() {
        let extensions = vec![
            "zip".to_string(),
            ".7z".to_string(),
            "invalid ext".to_string(),
        ];
        assert_eq!(
            archive_applies_to(&extensions).as_deref(),
            Some("System.FileName:~>\".zip\" OR System.FileName:~>\".7z\"")
        );
        assert_eq!(
            non_archive_applies_to(&extensions).as_deref(),
            Some("NOT (System.FileName:~>\".zip\" OR System.FileName:~>\".7z\")")
        );
        assert_eq!(archive_applies_to(&[]), None);
        assert_eq!(non_archive_applies_to(&[]), None);
    }

    #[test]
    fn normalizes_extensions_with_or_without_leading_dot() {
        assert_eq!(normalize_extension("zip").as_deref(), Some("zip"));
        assert_eq!(normalize_extension(".ZIP").as_deref(), Some("zip"));
        assert_eq!(
            normalize_extension("  .Tar.Gz  ").as_deref(),
            Some("tar.gz")
        );
        assert_eq!(normalize_extension("7z").as_deref(), Some("7z"));
    }

    #[test]
    fn rejects_invalid_extensions() {
        for invalid in [
            "",
            ".",
            "..",
            "zip.",
            ".zip.",
            "zip x",
            "zip/evil",
            r"zip\evil",
            "zi*p",
            "档案",
        ] {
            assert!(
                normalize_extension(invalid).is_none(),
                "{invalid:?} should be rejected"
            );
        }
        assert!(normalize_extension(&"a".repeat(MAX_EXTENSION_LEN + 1)).is_none());
    }

    #[test]
    fn builds_registry_key_names_and_labels() {
        assert_eq!(prog_id("zip"), "RarkZip.zip");
        assert_eq!(prog_id("tar.gz"), "RarkZip.tar.gz");
        assert_eq!(extension_key("zip"), ".zip");
        assert_eq!(association_label("zip"), "RarkZip ZIP archive");
        assert_eq!(association_label("tar.gz"), "RarkZip TAR.GZ archive");
    }

    #[test]
    fn quotes_commands_for_registry() {
        let exe = Path::new(r"C:\Program Files\RarkZip\rarkzip.exe");
        assert_eq!(
            build_command(exe, &["--gui", "%1"]),
            r#""C:\Program Files\RarkZip\rarkzip.exe" --gui "%1""#
        );
        assert_eq!(
            build_command(exe, &["--cli", "x", "%1", "--here"]),
            r#""C:\Program Files\RarkZip\rarkzip.exe" --cli x "%1" --here"#
        );
        assert_eq!(
            build_command(exe, &["--cli", "h", "%1", "-a", "sha256"]),
            r#""C:\Program Files\RarkZip\rarkzip.exe" --cli h "%1" -a sha256"#
        );
        assert_eq!(
            build_command(Path::new(r"C:\RarkZip\rarkzip.exe"), &[]),
            r#""C:\RarkZip\rarkzip.exe""#
        );
        assert_eq!(
            icon_value(exe),
            r#""C:\Program Files\RarkZip\rarkzip.exe",0"#
        );
        assert_eq!(
            build_command(
                Path::new(r"C:\RarkZip\rarkzip.exe"),
                &["--gui", "--new-here", "%V"]
            ),
            r#""C:\RarkZip\rarkzip.exe" --gui --new-here "%V""#
        );
        assert_eq!(
            build_command(exe, &["--quick-extract", "%1"]),
            r#""C:\Program Files\RarkZip\rarkzip.exe" --quick-extract "%1""#
        );
        assert_eq!(
            build_command(exe, &["--quick-compress", "%1", "--format", "7z"]),
            r#""C:\Program Files\RarkZip\rarkzip.exe" --quick-compress "%1" --format 7z"#
        );
        assert_eq!(
            build_command(exe, &["--elevated", "--quick-extract", "%1"]),
            r#""C:\Program Files\RarkZip\rarkzip.exe" --elevated --quick-extract "%1""#
        );
    }

    #[test]
    fn enabled_verbs_follow_settings_keys() {
        let mut options = options();
        options.extract_verbs = vec!["smart".into(), "folder_delete".into(), "unknown".into()];
        options.compress_verbs = vec!["zip".into(), "gui".into()];
        options.empty_verbs = vec!["new".into()];
        let extract: Vec<_> = enabled_extract_verbs(&options)
            .iter()
            .map(|spec| spec.key)
            .collect();
        assert_eq!(
            extract,
            vec![
                "RarkZip.110.ExtractSmart",
                "RarkZip.220.ExtractToFolderDelete"
            ]
        );
        let compress: Vec<_> = enabled_compress_verbs(&options)
            .iter()
            .map(|spec| spec.key)
            .collect();
        assert_eq!(
            compress,
            vec!["RarkZip.000.NewArchive", "RarkZip.100.CompressZip"]
        );
        let empty: Vec<_> = enabled_empty_verbs(&options)
            .iter()
            .map(|spec| spec.key)
            .collect();
        assert_eq!(empty, vec!["RarkZip.BackgroundNew"]);
    }

    #[test]
    fn escapes_quotes_and_trailing_backslashes() {
        assert_eq!(quote_windows_arg("plain"), "plain");
        assert_eq!(quote_windows_arg("two words"), "\"two words\"");
        assert_eq!(quote_windows_arg_always(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(
            quote_windows_arg_always(r"C:\dir with space\"),
            r#""C:\dir with space\\""#
        );
        assert_eq!(quote_windows_arg_always(""), "\"\"");
    }

    #[test]
    fn classifies_association_owner() {
        assert_eq!(
            classify_association("rarkzip.zip", "RarkZip.zip"),
            AssociationStatus::RarkZip
        );
        assert_eq!(
            classify_association("CompressedFolder", "RarkZip.zip"),
            AssociationStatus::Other("CompressedFolder".to_owned())
        );
    }
}
