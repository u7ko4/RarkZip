//! Linux 文件管理器集成。
//!
//! Linux 没有统一的 Shell 扩展接口，右键菜单通过各文件管理器的用户级
//! 动作/脚本目录实现：
//!
//! * Nemo（Cinnamon）：`~/.local/share/nemo/actions/*.nemo_action`
//! * Nautilus（GNOME Files）：`~/.local/share/nautilus/scripts/RarkZip/`
//! * Dolphin（KDE）：`~/.local/share/kio/servicemenus/*.desktop`
//!
//! 菜单实际执行的是一组放在 `~/.local/share/rarkzip/menu/` 的 shell 包装
//! 脚本，统一处理多选、删除源文件与 pkexec 提权等差异。双击关联通过
//! 用户级 `.desktop` 与 `mimeapps.list` 的“添加关联”实现，不强制覆盖
//! 用户当前的默认应用。

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::{AssociationStatus, ShellOptions, ShellScope};

const DESKTOP_ID: &str = "rarkzip.desktop";
/// 应用图标：安装到 hicolor 主题目录，供菜单与 .desktop 使用。
const APP_ICON: &[u8] = include_bytes!("../../rarkzip-gui/assets/app-icon.png");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Manager {
    Nemo,
    Nautilus,
    Dolphin,
}

impl Manager {
    fn name(self) -> &'static str {
        match self {
            Self::Nemo => "Nemo",
            Self::Nautilus => "Nautilus",
            Self::Dolphin => "Dolphin",
        }
    }
}

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_default()
}

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_default()
}

fn wrappers_dir() -> PathBuf {
    data_home().join("rarkzip/menu")
}

fn nemo_dir() -> PathBuf {
    data_home().join("nemo/actions")
}

fn nautilus_dir() -> PathBuf {
    data_home().join("nautilus/scripts/RarkZip")
}

fn servicemenus_dir() -> PathBuf {
    data_home().join("kio/servicemenus")
}

fn applications_dir() -> PathBuf {
    data_home().join("applications")
}

fn desktop_file_path() -> PathBuf {
    applications_dir().join(DESKTOP_ID)
}

fn mimeapps_path() -> PathBuf {
    config_home().join("mimeapps.list")
}

fn icon_path() -> PathBuf {
    data_home().join("icons/hicolor/512x512/apps/rarkzip.png")
}

fn install_icon() -> Result<()> {
    let root = data_home().join("icons/hicolor");
    let index = root.join("index.theme");
    if !index.is_file() {
        fs::create_dir_all(&root)
            .with_context(|| format!("创建图标主题目录失败：{}", root.display()))?;
        let _ = fs::write(
            &index,
            "[Icon Theme]
Name=Hicolor
Comment=Fallback icon theme
Hidden=true
             Directories=48x48/apps,64x64/apps,128x128/apps,256x256/apps,512x512/apps
             [48x48/apps]
Size=48
Context=Apps
Type=Fixed
             [64x64/apps]
Size=64
Context=Apps
Type=Fixed
             [128x128/apps]
Size=128
Context=Apps
Type=Fixed
             [256x256/apps]
Size=256
Context=Apps
Type=Fixed
             [512x512/apps]
Size=512
Context=Apps
Type=Fixed
",
        );
    }
    for size in ["48x48", "64x64", "128x128", "256x256", "512x512"] {
        let directory = root.join(size).join("apps");
        fs::create_dir_all(&directory)
            .with_context(|| format!("创建图标目录失败：{}", directory.display()))?;
        fs::write(directory.join("rarkzip.png"), APP_ICON)
            .with_context(|| format!("写入图标失败：{}", directory.display()))?;
    }
    let _ = std::process::Command::new("gtk-update-icon-cache")
        .args(["-f", "-t"])
        .arg(&root)
        .status();
    Ok(())
}

fn remove_icons() {
    let root = data_home().join("icons/hicolor");
    for size in ["48x48", "64x64", "128x128", "256x256", "512x512"] {
        let _ = fs::remove_file(root.join(size).join("apps/rarkzip.png"));
    }
}

fn command_exists(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let candidate = dir.join(name);
                candidate.is_file()
            })
        })
        .unwrap_or(false)
}

fn detected_managers() -> Vec<Manager> {
    let mut managers = Vec::new();
    // 先按当前桌面环境排序，避免同时装了多个文件管理器时菜单写到无关目录。
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let preferred = if desktop.contains("Cinnamon") {
        Manager::Nemo
    } else if desktop.contains("KDE") {
        Manager::Dolphin
    } else if desktop.contains("GNOME") || desktop.contains("Unity") {
        Manager::Nautilus
    } else {
        Manager::Nemo
    };
    if command_exists(preferred_binary(preferred)) {
        managers.push(preferred);
    }
    for manager in [Manager::Nemo, Manager::Nautilus, Manager::Dolphin] {
        if manager != preferred && command_exists(preferred_binary(manager)) {
            managers.push(manager);
        }
    }
    managers
}

fn preferred_binary(manager: Manager) -> &'static str {
    match manager {
        Manager::Nemo => "nemo",
        Manager::Nautilus => "nautilus",
        Manager::Dolphin => "dolphin",
    }
}

/// 当前文件管理器后端名称，供设置页展示。
pub fn backends() -> Vec<&'static str> {
    detected_managers().into_iter().map(Manager::name).collect()
}

fn normalize_extension(extension: &str) -> Option<String> {
    let extension = extension
        .trim()
        .rsplit(['/', '\\'])
        .next()?
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if extension.is_empty()
        || !extension
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-')
    {
        return None;
    }
    Some(extension)
}

fn archive_extensions(options: &ShellOptions) -> Vec<String> {
    let mut extensions: Vec<String> = options
        .archive_extensions
        .iter()
        .filter_map(|extension| normalize_extension(extension))
        .collect();
    extensions.sort();
    extensions.dedup();
    extensions
}

fn enabled(key: &str, verbs: &[String]) -> bool {
    verbs.iter().any(|verb| verb == key)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn script_for(exe: &Path, key: &str) -> Option<String> {
    let exe_quoted = shell_quote(&exe.to_string_lossy());
    let per_file = |args: &[&str], delete: bool| {
        let mut command = exe_quoted.clone();
        for arg in args {
            command.push(' ');
            command.push_str(&shell_quote(arg));
        }
        if delete {
            command.push_str(" --delete");
        }
        format!("{command} \"$f\"")
    };
    let body = match key {
        "here" => per_file(&["--extract-here"], false),
        "here_delete" => per_file(&["--extract-here"], true),
        "smart" => per_file(&["--extract-smart"], false),
        "smart_delete" => per_file(&["--extract-smart"], true),
        "folder" => per_file(&["--extract-folder"], false),
        "folder_delete" => per_file(&["--extract-folder"], true),
        "other" => per_file(&["--quick-extract"], false),
        "admin" => per_file(&["--extract-here"], false).replacen(
            &exe_quoted,
            &format!("pkexec {exe_quoted}"),
            1,
        ),
        "test" => per_file(&["--cli", "t"], false),
        "open" => format!("{exe_quoted} \"$f\""),
        "hash" => format!("{exe_quoted} --hash \"$f\""),
        "new" => format!("{exe_quoted} --gui --new \"$f\""),
        "new_here" => format!("{exe_quoted} --gui --new-here \"$1\""),
        "open_here" => format!("{exe_quoted} --gui \"$1\""),
        "zip" => per_file(&["--quick-compress", "--format", "zip"], false),
        "zip_delete" => per_file(&["--quick-compress", "--format", "zip"], true),
        "tar" => per_file(&["--quick-compress", "--format", "tar"], false),
        "tgz" => per_file(&["--quick-compress", "--format", "tgz"], false),
        "sevenz" => per_file(&["--quick-compress", "--format", "7z"], false),
        "gz" => per_file(&["--quick-compress", "--format", "gz"], false),
        "xz" => per_file(&["--quick-compress", "--format", "xz"], false),
        "zst" => per_file(&["--quick-compress", "--format", "zst"], false),
        "each_zip" => format!("{exe_quoted} --cli ce -f zip \"$f\""),
        "each_7z" => format!("{exe_quoted} --cli ce -f 7z \"$f\""),
        "gui" => format!("{exe_quoted} --gui --new \"$f\""),
        "admin_compress" => format!("pkexec {exe_quoted} --quick-compress \"$f\" --format zip"),
        _ => return None,
    };
    // `--quick-compress` 需要源路径位于参数末尾；上面的 per_file 已经按
    // “exe args f” 拼接，这里补齐参数顺序。
    let body = if matches!(
        key,
        "zip" | "zip_delete" | "tar" | "tgz" | "sevenz" | "gz" | "xz" | "zst"
    ) {
        let format = match key {
            "zip" | "zip_delete" => "zip",
            "tar" => "tar",
            "tgz" => "tgz",
            "sevenz" => "7z",
            "gz" => "gz",
            "xz" => "xz",
            "zst" => "zst",
            _ => unreachable!(),
        };
        let delete = if key == "zip_delete" {
            " --delete-sources"
        } else {
            ""
        };
        format!("{exe_quoted} --quick-compress \"$f\" --format {format}{delete}")
    } else {
        body
    };
    Some(format!(
        "#!/bin/sh\n\
         # 由 RarkZip 生成，可在“设置 → 右键菜单”中卸载。\n\
         writable=$(mktemp 2>/dev/null || echo /tmp/rarkzip-menu-$$)\n\
         if [ \"$#\" -gt 0 ]; then\n\
         \tprintf '%s\\n' \"$@\" > \"$writable\"\n\
         elif [ -n \"${{NAUTILUS_SCRIPT_SELECTED_FILE_PATHS-}}\" ]; then\n\
         \tprintf '%s\\n' \"$NAUTILUS_SCRIPT_SELECTED_FILE_PATHS\" > \"$writable\"\n\
         else\n\
         \tprintf '\\n' > \"$writable\"\n\
         fi\n\
         while IFS= read -r f; do\n\
         \t[ -n \"$f\" ] && {body}\n\
         done < \"$writable\"\n\
         rm -f \"$writable\"\n"
    ))
}

/// 生成一个供文件管理器调用的包装脚本。
fn write_script(path: &Path, body: &str) -> Result<()> {
    fs::write(path, body).with_context(|| format!("写入右键菜单脚本失败：{}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .with_context(|| format!("设置脚本可执行权限失败：{}", path.display()))
}

struct Verb {
    key: &'static str,
    label: &'static str,
    /// `None` 表示对任意文件/目录显示。
    only_archives: bool,
    multiple: bool,
}

const EXTRACT_VERBS: &[Verb] = &[
    Verb {
        key: "here",
        label: "解压到此处",
        only_archives: true,
        multiple: false,
    },
    Verb {
        key: "here_delete",
        label: "解压到此处并删除",
        only_archives: true,
        multiple: false,
    },
    Verb {
        key: "smart",
        label: "智能解压到此处",
        only_archives: true,
        multiple: false,
    },
    Verb {
        key: "smart_delete",
        label: "智能解压到此处并删除",
        only_archives: true,
        multiple: false,
    },
    Verb {
        key: "folder",
        label: "解压到独立文件夹",
        only_archives: true,
        multiple: false,
    },
    Verb {
        key: "folder_delete",
        label: "解压到独立文件夹并删除",
        only_archives: true,
        multiple: false,
    },
    Verb {
        key: "other",
        label: "解压到其他文件夹…",
        only_archives: true,
        multiple: false,
    },
    Verb {
        key: "admin",
        label: "以管理员身份解压",
        only_archives: true,
        multiple: false,
    },
    Verb {
        key: "test",
        label: "测试压缩文件",
        only_archives: true,
        multiple: false,
    },
    Verb {
        key: "open",
        label: "用 RarkZip 打开",
        only_archives: true,
        multiple: false,
    },
];

const COMPRESS_VERBS: &[Verb] = &[
    Verb {
        key: "zip",
        label: "压缩为 ZIP",
        only_archives: false,
        multiple: false,
    },
    Verb {
        key: "zip_delete",
        label: "压缩为 ZIP 并删除",
        only_archives: false,
        multiple: false,
    },
    Verb {
        key: "tar",
        label: "压缩为 TAR",
        only_archives: false,
        multiple: false,
    },
    Verb {
        key: "tgz",
        label: "压缩为 TGZ",
        only_archives: false,
        multiple: false,
    },
    Verb {
        key: "sevenz",
        label: "压缩为 7Z",
        only_archives: false,
        multiple: false,
    },
    Verb {
        key: "gz",
        label: "压缩为 GZ",
        only_archives: false,
        multiple: false,
    },
    Verb {
        key: "xz",
        label: "压缩为 XZ",
        only_archives: false,
        multiple: false,
    },
    Verb {
        key: "zst",
        label: "压缩为 ZST",
        only_archives: false,
        multiple: false,
    },
    Verb {
        key: "each_zip",
        label: "各压缩为独立 ZIP",
        only_archives: false,
        multiple: true,
    },
    Verb {
        key: "each_7z",
        label: "各压缩为独立 7Z",
        only_archives: false,
        multiple: true,
    },
    Verb {
        key: "gui",
        label: "新建压缩包…",
        only_archives: false,
        multiple: true,
    },
    Verb {
        key: "admin_compress",
        label: "以管理员身份压缩",
        only_archives: false,
        multiple: false,
    },
];

fn extract_verbs(options: &ShellOptions) -> Vec<&'static Verb> {
    EXTRACT_VERBS
        .iter()
        .filter(|verb| enabled(verb.key, &options.extract_verbs))
        .collect()
}

fn compress_verbs(options: &ShellOptions) -> Vec<&'static Verb> {
    COMPRESS_VERBS
        .iter()
        .filter(|verb| enabled(verb.key, &options.compress_verbs))
        .collect()
}

fn empty_verbs(options: &ShellOptions) -> Vec<&'static Verb> {
    const EMPTY: &[Verb] = &[
        Verb {
            key: "open_here",
            label: "在此处打开 RarkZip",
            only_archives: false,
            multiple: false,
        },
        Verb {
            key: "new_here",
            label: "在此处新建压缩包",
            only_archives: false,
            multiple: false,
        },
    ];
    EMPTY
        .iter()
        .filter(|verb| enabled(verb.key, &options.empty_verbs))
        .collect()
}

fn mime_for_extension(extension: &str) -> Option<&'static str> {
    Some(match extension {
        "zip" | "zipx" => "application/zip",
        "jar" => "application/java-archive",
        "war" => "application/x-war",
        "apk" => "application/vnd.android.package-archive",
        "ipa" => "application/x-itunes-ipa",
        "xpi" => "application/x-xpinstall",
        "rar" => "application/vnd.rar",
        "7z" | "001" => "application/x-7z-compressed",
        "alz" | "egg" => "application/x-alz",
        "cab" => "application/vnd.ms-cab-compressed",
        "arj" => "application/x-arj",
        "lha" | "lzh" => "application/x-lha",
        "ace" => "application/x-ace-compressed",
        "zpaq" => "application/x-zpaq",
        "zstd" | "zst" | "tzst" => "application/zstd",
        "br" => "application/x-brotli",
        "pea" => "application/x-pea",
        "tar" | "tlz" => "application/x-tar",
        "gz" | "tgz" => "application/gzip",
        "bz" => "application/x-bzip",
        "bz2" | "tbz" | "tbz2" => "application/x-bzip2",
        "xz" | "txz" => "application/x-xz",
        "lzma" => "application/x-lzma",
        "lz" => "application/x-lzip",
        "z" => "application/x-compress",
        "lz4" => "application/x-lz4",
        "uu" | "uue" | "xxe" => "application/x-uuencode",
        "iso" | "isz" | "udf" => "application/x-iso9660-image",
        "img" | "bin" | "i00" => "application/x-cd-image",
        "wim" => "application/x-ms-wim",
        "deb" => "application/vnd.debian.binary-package",
        "asar" => "application/x-asar",
        "pma" | "arc" | "aes" | "bh" => "application/x-archive",
        _ => return None,
    })
}

fn archive_mimetypes(extensions: &[String]) -> Vec<&'static str> {
    let mut mimes: Vec<&'static str> = extensions
        .iter()
        .filter_map(|extension| mime_for_extension(extension))
        .collect();
    mimes.sort();
    mimes.dedup();
    mimes
}

fn extension_list(extensions: &[String]) -> String {
    if extensions.is_empty() {
        "any;".to_string()
    } else {
        let mut value = extensions.join(";");
        value.push(';');
        value
    }
}

fn nemo_action_content(
    sort: usize,
    label: &str,
    wrapper: &Path,
    token: &str,
    selection: &str,
    extensions: &str,
    icon: Option<&Path>,
) -> String {
    let mut content = String::from("[Nemo Action]\n");
    // Nemo 按 Name 排序，用数字前缀保证解压、压缩、工具的顺序稳定。
    content.push_str(&format!("Name=RarkZip {sort:03} {label}\n"));
    content.push_str(&format!("Comment=RarkZip：{label}\n"));
    content.push_str(&format!("Exec={} {token}\n", wrapper.display()));
    content.push_str(&format!("Selection={selection}\n"));
    content.push_str(&format!("Extensions={extensions}\n"));
    content.push_str("Quote=double\n");
    if icon.is_some() {
        content.push_str("Icon-Name=rarkzip\n");
    }
    content.push_str("Active=true\n");
    content
}

fn nemo_config_path() -> PathBuf {
    wrappers_dir().join("config.json")
}

fn nemo_extension_path() -> PathBuf {
    data_home().join("nemo-python/extensions/rarkzip.py")
}

/// Nemo 自带的 Python 扩展接口可用时，优先使用真正的层叠子菜单。
fn nemo_python_available() -> bool {
    command_exists("python3")
        && std::process::Command::new("python3")
            .args([
                "-c",
                "import gi; gi.require_version('Nemo', '3.0'); from gi.repository import Nemo",
            ])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
}

const NEMO_EXTENSION: &str = r#"# 由 RarkZip 生成，可在“设置 -> 右键菜单”中卸载。
import json
import os
import subprocess

from gi.repository import GObject, Nemo

CONFIG = os.path.join(
    os.environ.get("XDG_DATA_HOME") or os.path.expanduser("~/.local/share"),
    "rarkzip", "menu", "config.json",
)


class RarkZipExtension(GObject.GObject, Nemo.MenuProvider):
    def __init__(self):
        super().__init__()
        self._config = None

    def _load(self):
        if self._config is None:
            try:
                with open(CONFIG, encoding="utf-8") as handle:
                    self._config = json.load(handle)
            except Exception:
                self._config = {}
        return self._config

    @staticmethod
    def _path(file_info):
        try:
            return file_info.get_location().get_path()
        except Exception:
            return None

    @staticmethod
    def _run(script, paths):
        try:
            subprocess.Popen(
                [script] + paths, close_fds=True, start_new_session=True
            )
        except Exception:
            pass

    @staticmethod
    def _icon(config):
        return None if config.get("hide_icon") else "rarkzip"

    def _nemo_item(self, config, name, label, script, paths):
        kwargs = {"name": name, "label": label, "tip": "RarkZip: " + label}
        icon = self._icon(config)
        if icon:
            kwargs["icon"] = icon
        item = Nemo.MenuItem(**kwargs)
        item.connect("activate", lambda _item: self._run(script, paths))
        return item

    def _build(self, config, entries, paths):
        entries = [
            entry
            for entry in entries
            if entry.get("script") and os.path.isfile(entry["script"])
        ]
        if not entries:
            return []
        if config.get("cascade"):
            top = self._nemo_item(config, "RarkZip::root", "RarkZip", "", paths)
            submenu = Nemo.Menu()
            for entry in entries:
                submenu.append_item(
                    self._nemo_item(
                        config,
                        "RarkZip::" + os.path.basename(entry["script"]),
                        entry.get("label", ""),
                        entry["script"],
                        paths,
                    )
                )
            top.set_submenu(submenu)
            return [top]
        return [
            self._nemo_item(
                config,
                "RarkZip::" + os.path.basename(entry["script"]),
                entry.get("label", ""),
                entry["script"],
                paths,
            )
            for entry in entries
        ]

    @staticmethod
    def _is_archive(path, config):
        base = os.path.basename(path).lower()
        if "." not in base:
            return False
        return base.rsplit(".", 1)[1] in config.get("archive_extensions", [])

    def get_file_items(self, *args):
        files = args[-1] if args else []
        config = self._load()
        paths = [path for path in (self._path(file) for file in files) if path]
        if not config or not paths:
            return []
        entries = []
        if len(paths) == 1 and (
            config.get("extract_any_file") or self._is_archive(paths[0], config)
        ):
            entries.extend(config.get("extract", []))
        entries.extend(config.get("compress", []))
        hash_entry = config.get("hash")
        if hash_entry:
            entries.append(hash_entry)
        return self._build(config, entries, paths)

    def get_background_items(self, *args):
        config = self._load()
        current = args[-1] if args else None
        if not config or current is None:
            return []
        path = self._path(current)
        if not path:
            return []
        return self._build(config, config.get("empty", []), [path])
"#;

fn nemo_entry(label: &str, wrapper: &Path) -> serde_json::Value {
    serde_json::json!({
        "label": label,
        "script": wrapper.display().to_string(),
    })
}

fn write_nemo(
    options: &ShellOptions,
    exe: &Path,
    extensions: &[String],
    wrappers: &Path,
) -> Result<usize> {
    let directory = nemo_dir();
    fs::create_dir_all(&directory)
        .with_context(|| format!("创建 Nemo 动作目录失败：{}", directory.display()))?;
    let mut index = 10usize;
    let mut written = 0usize;
    let archive_ext = extension_list(extensions);
    let any_ext = "any;";
    let icon_file = icon_path();
    let icon = (!options.hide_icon).then_some(icon_file.as_path());
    let mut extract_entries = Vec::new();
    let mut compress_entries = Vec::new();
    let mut empty_entries = Vec::new();

    for verb in extract_verbs(options) {
        let Some(body) = script_for(exe, verb.key) else {
            continue;
        };
        let wrapper = wrappers.join(format!("{}.sh", verb.key));
        write_script(&wrapper, &body)?;
        extract_entries.push(nemo_entry(verb.label, &wrapper));
        let extension_match = if options.extract_any_file {
            any_ext
        } else {
            &archive_ext
        };
        let content = nemo_action_content(
            index,
            verb.label,
            &wrapper,
            "%F",
            if verb.multiple { "m" } else { "s" },
            extension_match,
            icon,
        );
        fs::write(
            directory.join(format!(
                "rarkzip-{index:03}-{}.nemo_action",
                verb.key.replace('_', "-")
            )),
            content,
        )
        .with_context(|| "写入 Nemo 动作失败".to_string())?;
        index += 10;
        written += 1;
    }

    index = 200;
    for verb in compress_verbs(options) {
        let Some(body) = script_for(exe, verb.key) else {
            continue;
        };
        let wrapper = wrappers.join(format!("compress-{}.sh", verb.key));
        write_script(&wrapper, &body)?;
        compress_entries.push(nemo_entry(verb.label, &wrapper));
        let content =
            nemo_action_content(index, verb.label, &wrapper, "%F", "notnone", any_ext, icon);
        fs::write(
            directory.join(format!(
                "rarkzip-{index:03}-{}.nemo_action",
                verb.key.replace('_', "-")
            )),
            content,
        )
        .with_context(|| "写入 Nemo 动作失败".to_string())?;
        index += 10;
        written += 1;
    }

    index = 300;
    for verb in empty_verbs(options) {
        let Some(body) = script_for(exe, verb.key) else {
            continue;
        };
        let wrapper = wrappers.join(format!("empty-{}.sh", verb.key));
        write_script(&wrapper, &body)?;
        empty_entries.push(nemo_entry(verb.label, &wrapper));
        let content = nemo_action_content(index, verb.label, &wrapper, "%P", "none", any_ext, icon);
        fs::write(
            directory.join(format!(
                "rarkzip-{index:03}-{}.nemo_action",
                verb.key.replace('_', "-")
            )),
            content,
        )
        .with_context(|| "写入 Nemo 动作失败".to_string())?;
        index += 10;
        written += 1;
    }

    let mut hash_entry = None;
    if options.hash_menu {
        let body = script_for(exe, "hash").unwrap_or_default();
        let wrapper = wrappers.join("hash.sh");
        write_script(&wrapper, &body)?;
        hash_entry = Some(nemo_entry("计算哈希", &wrapper));
        let content =
            nemo_action_content(400, "计算哈希", &wrapper, "%F", "notnone", any_ext, icon);
        fs::write(directory.join("rarkzip-400-hash.nemo_action"), content)
            .with_context(|| "写入 Nemo 动作失败".to_string())?;
        written += 1;
    }

    if options.cascade && nemo_python_available() {
        let extension_config = serde_json::json!({
            "cascade": true,
            "hide_icon": options.hide_icon,
            "extract_any_file": options.extract_any_file,
            "archive_extensions": extensions,
            "extract": extract_entries,
            "compress": compress_entries,
            "empty": empty_entries,
            "hash": hash_entry,
        });
        fs::write(
            nemo_config_path(),
            serde_json::to_vec_pretty(&extension_config).context("序列化 Nemo 层叠菜单配置失败")?,
        )
        .with_context(|| "写入 Nemo 层叠菜单配置失败".to_string())?;
        let extension = nemo_extension_path();
        if let Some(parent) = extension.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("创建 Nemo 扩展目录失败：{}", parent.display()))?;
        }
        fs::write(&extension, NEMO_EXTENSION)
            .with_context(|| format!("写入 Nemo 扩展失败：{}", extension.display()))?;
        // 扩展本身负责全部菜单项，移除扁平动作避免重复。
        remove_files_with_prefix(&directory, "rarkzip-", ".nemo_action");
    } else {
        let _ = fs::remove_file(nemo_config_path());
        let _ = fs::remove_file(nemo_extension_path());
    }

    Ok(written)
}
fn write_nautilus(options: &ShellOptions, exe: &Path) -> Result<usize> {
    let directory = nautilus_dir();
    fs::create_dir_all(&directory)
        .with_context(|| format!("创建 Nautilus 脚本目录失败：{}", directory.display()))?;
    let mut index = 10usize;
    let mut written = 0usize;

    let mut verbs: Vec<&Verb> = extract_verbs(options);
    verbs.extend(compress_verbs(options));
    verbs.extend(empty_verbs(options));
    for verb in verbs {
        let Some(body) = script_for(exe, verb.key) else {
            continue;
        };
        let path = directory.join(format!("{index:02}-{}.sh", verb.label));
        write_script(&path, &body)?;
        index += 10;
        written += 1;
    }
    if options.hash_menu {
        let body = script_for(exe, "hash").unwrap_or_default();
        write_script(&directory.join("90-计算哈希.sh"), &body)?;
        written += 1;
    }
    Ok(written)
}

fn desktop_exec(exe: &Path, action: &str, folder: &str) -> String {
    let exe = desktop_quote(&exe.to_string_lossy());
    match action {
        "smart" => format!("{exe} --extract-smart %f"),
        "folder" => format!("{exe} --extract-folder %f"),
        "custom" if !folder.trim().is_empty() => {
            format!("{exe} --cli x --output {} %f", desktop_quote(folder))
        }
        _ => format!("{exe} %f"),
    }
}

fn desktop_quote(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('`', "\\`")
            .replace('$', "\\$")
    )
}

fn desktop_entry(options: &ShellOptions, action: &str, folder: &str) -> String {
    let exe = options.exe.clone();
    // 一个 .desktop 承载所有已支持的 MIME；逐扩展注册时不会互相覆盖。
    let mut mimes: Vec<&'static str> = crate::common::DEFAULT_ARCHIVE_EXTENSIONS
        .iter()
        .filter_map(|extension| mime_for_extension(extension))
        .collect();
    mimes.sort();
    mimes.dedup();
    let mut content = String::from("[Desktop Entry]\n");
    content.push_str("Type=Application\n");
    content.push_str("Name=RarkZip\n");
    content.push_str("Comment=打开、解压与创建压缩包\n");
    content.push_str(&format!("Exec={}\n", desktop_exec(&exe, action, folder)));
    content.push_str("Icon=rarkzip\n");
    content.push_str("Terminal=false\n");
    content.push_str("Categories=Utility;Archiving;\n");
    content.push_str("StartupNotify=true\n");
    if !mimes.is_empty() {
        content.push_str(&format!("MimeType={};\n", mimes.join(";")));
    }
    content
}

/// 写入或更新 `rarkzip.desktop`，用于“打开方式”候选。
fn write_desktop_entry(options: &ShellOptions, action: &str, folder: &str) -> Result<()> {
    let directory = applications_dir();
    fs::create_dir_all(&directory)
        .with_context(|| format!("创建 applications 目录失败：{}", directory.display()))?;
    fs::write(desktop_file_path(), desktop_entry(options, action, folder))
        .with_context(|| "写入 rarkzip.desktop 失败".to_string())
}

fn remove_desktop_entry() {
    let _ = fs::remove_file(desktop_file_path());
}

fn edit_mimeapps(edit: impl FnOnce(&mut Vec<String>)) {
    let path = mimeapps_path();
    let mut lines: Vec<String> = fs::read_to_string(&path)
        .map(|text| text.lines().map(ToString::to_string).collect())
        .unwrap_or_default();
    edit(&mut lines);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    let _ = fs::write(path, text);
}

fn mimeapps_add(mime: &str) {
    edit_mimeapps(|lines| {
        let mut section = String::new();
        let mut found_section = false;
        let mut inserted = false;
        let mut index = 0usize;
        while index < lines.len() {
            let line = lines[index].trim().to_string();
            if line.starts_with('[') && line.ends_with(']') {
                if section == "[Added Associations]" && !inserted {
                    lines.insert(index, format!("{mime}={DESKTOP_ID};"));
                    inserted = true;
                    index += 1;
                }
                section = line;
                if section == "[Added Associations]" {
                    found_section = true;
                }
                index += 1;
                continue;
            }
            if section == "[Added Associations]" {
                if let Some(rest) = line.strip_prefix(&format!("{mime}=")) {
                    let mut ids: Vec<&str> = rest.split(';').filter(|id| !id.is_empty()).collect();
                    if !ids.contains(&DESKTOP_ID) {
                        ids.push(DESKTOP_ID);
                    }
                    lines[index] = format!("{mime}={};", ids.join(";"));
                    inserted = true;
                }
            }
            index += 1;
        }
        if !inserted {
            if !found_section {
                if !lines.is_empty() {
                    lines.push(String::new());
                }
                lines.push("[Added Associations]".to_string());
            }
            lines.push(format!("{mime}={DESKTOP_ID};"));
        }
    });
}

fn mimeapps_remove(mime: &str, desktop_id: &str) {
    edit_mimeapps(|lines| {
        let mut section = String::new();
        let mut index = 0usize;
        while index < lines.len() {
            let line = lines[index].trim().to_string();
            if line.starts_with('[') && line.ends_with(']') {
                section = line;
                index += 1;
                continue;
            }
            if !section.starts_with('[') {
                index += 1;
                continue;
            }
            if let Some(rest) = line.strip_prefix(&format!("{mime}=")) {
                let ids: Vec<&str> = rest
                    .split(';')
                    .filter(|id| !id.is_empty() && *id != desktop_id)
                    .collect();
                if ids.is_empty() {
                    lines.remove(index);
                    continue;
                }
                lines[index] = format!("{mime}={};", ids.join(";"));
            }
            index += 1;
        }
    });
}

fn mimeapps_values(mime: &str, section_name: &str) -> Vec<String> {
    let mut section = String::new();
    let mut values = Vec::new();
    let Ok(text) = fs::read_to_string(mimeapps_path()) else {
        return values;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line.to_string();
            continue;
        }
        if section != section_name {
            continue;
        }
        if let Some(rest) = line.strip_prefix(&format!("{mime}=")) {
            values.extend(
                rest.split(';')
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string),
            );
        }
    }
    values
}

fn write_servicemenus(
    options: &ShellOptions,
    exe: &Path,
    extensions: &[String],
    wrappers: &Path,
) -> Result<usize> {
    let directory = servicemenus_dir();
    fs::create_dir_all(&directory)
        .with_context(|| format!("创建 KDE 服务菜单目录失败：{}", directory.display()))?;
    let mut written = 0usize;
    let submenu = if options.cascade {
        "X-KDE-Submenu=RarkZip\n"
    } else {
        ""
    };
    let archive_mimes = archive_mimetypes(extensions);

    let mut extract_actions = String::new();
    for verb in extract_verbs(options) {
        let Some(body) = script_for(exe, verb.key) else {
            continue;
        };
        let wrapper = wrappers.join(format!("{}.sh", verb.key));
        write_script(&wrapper, &body)?;
        let id = verb.key.replace('_', "");
        extract_actions.push_str(&format!("{id};"));
        written += 1;
    }
    if !extract_actions.is_empty() {
        let mime = if options.extract_any_file || archive_mimes.is_empty() {
            "all/all".to_string()
        } else {
            archive_mimes.join(";")
        };
        let mut content = String::from("[Desktop Entry]\n");
        content.push_str("Type=Service\n");
        content.push_str("ServiceTypes=KonqPopupMenu/Plugin\n");
        content.push_str(&format!("MimeType={mime};\n"));
        content.push_str(&format!("Actions={extract_actions}\n"));
        content.push_str("Icon=rarkzip\n");
        content.push_str("X-KDE-Priority=TopLevel\n");
        content.push_str(submenu);
        for verb in extract_verbs(options) {
            content.push_str(&format!(
                "[Desktop Action {}]\nName=RarkZip: {}\nExec={} %F\n",
                verb.key.replace('_', ""),
                verb.label,
                wrappers.join(format!("{}.sh", verb.key)).display()
            ));
        }
        fs::write(directory.join("rarkzip-extract.desktop"), content)
            .with_context(|| "写入 KDE 解压菜单失败".to_string())?;
    }

    let mut compress_actions = String::new();
    let mut compress_sections = String::new();
    for verb in compress_verbs(options) {
        let Some(body) = script_for(exe, verb.key) else {
            continue;
        };
        let wrapper = wrappers.join(format!("compress-{}.sh", verb.key));
        write_script(&wrapper, &body)?;
        let id = format!("compress{}", verb.key.replace('_', ""));
        compress_actions.push_str(&format!("{id};"));
        compress_sections.push_str(&format!(
            "[Desktop Action {id}]\nName=RarkZip: {}\nExec={} %F\n",
            verb.label,
            wrapper.display()
        ));
        written += 1;
    }
    if !compress_actions.is_empty() {
        let mut content = String::from("[Desktop Entry]\n");
        content.push_str("Type=Service\n");
        content.push_str("ServiceTypes=KonqPopupMenu/Plugin\n");
        content.push_str("MimeType=all/all;\n");
        content.push_str(&format!("Actions={compress_actions}\n"));
        content.push_str("Icon=rarkzip\n");
        content.push_str("X-KDE-Priority=TopLevel\n");
        content.push_str(submenu);
        content.push_str(&compress_sections);
        fs::write(directory.join("rarkzip-compress.desktop"), content)
            .with_context(|| "写入 KDE 压缩菜单失败".to_string())?;
    }

    if options.hash_menu {
        let body = script_for(exe, "hash").unwrap_or_default();
        let wrapper = wrappers.join("hash.sh");
        write_script(&wrapper, &body)?;
        let content = format!(
            "[Desktop Entry]\nType=Service\nServiceTypes=KonqPopupMenu/Plugin\n\
             MimeType=all/all;\nActions=hash;\nIcon=rarkzip\nX-KDE-Priority=TopLevel\n\
             {submenu}[Desktop Action hash]\nName=RarkZip: 计算哈希\nExec={} %F\n",
            wrapper.display()
        );
        fs::write(directory.join("rarkzip-hash.desktop"), content)
            .with_context(|| "写入 KDE 哈希菜单失败".to_string())?;
        written += 1;
    }

    Ok(written)
}

fn remove_files_with_prefix(directory: &Path, prefix: &str, suffix: &str) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(prefix) && name.ends_with(suffix) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn clean_menu_files() {
    remove_files_with_prefix(&nemo_dir(), "rarkzip-", ".nemo_action");
    let _ = fs::remove_file(nemo_extension_path());
    let _ = fs::remove_file(nemo_config_path());
    remove_files_with_prefix(&servicemenus_dir(), "rarkzip-", ".desktop");
    let _ = fs::remove_dir_all(nautilus_dir());
    let _ = fs::remove_dir_all(wrappers_dir());
}

pub fn install(options: &ShellOptions) -> Result<()> {
    // Linux 集成只写当前用户的 XDG 目录；LocalMachine 与 CurrentUser 路径相同。
    if options.exe.as_os_str().is_empty() {
        anyhow::bail!("无法定位 RarkZip 可执行文件");
    }
    let managers = detected_managers();
    if managers.is_empty() {
        anyhow::bail!("未检测到受支持的文件管理器（Nemo、Nautilus 或 Dolphin）");
    }
    clean_menu_files();
    install_icon()?;
    let wrappers = wrappers_dir();
    fs::create_dir_all(&wrappers)
        .with_context(|| format!("创建菜单脚本目录失败：{}", wrappers.display()))?;
    let extensions = archive_extensions(options);
    let exe = &options.exe;
    let mut written = 0usize;
    for manager in managers {
        written += match manager {
            Manager::Nemo => write_nemo(options, exe, &extensions, &wrappers)?,
            Manager::Nautilus => write_nautilus(options, exe)?,
            Manager::Dolphin => write_servicemenus(options, exe, &extensions, &wrappers)?,
        };
    }
    if written == 0 {
        anyhow::bail!("右键菜单选项为空，没有写入任何动作");
    }
    Ok(())
}

pub fn uninstall(_scope: ShellScope) -> Result<()> {
    clean_menu_files();
    remove_icons();
    remove_desktop_entry();
    // 清掉所有我们注册过的 MIME 条目。
    for mime in [
        "application/zip",
        "application/java-archive",
        "application/x-war",
        "application/vnd.android.package-archive",
        "application/x-itunes-ipa",
        "application/x-xpinstall",
        "application/vnd.rar",
        "application/x-7z-compressed",
        "application/x-alz",
        "application/vnd.ms-cab-compressed",
        "application/x-arj",
        "application/x-lha",
        "application/x-ace-compressed",
        "application/x-zpaq",
        "application/zstd",
        "application/x-brotli",
        "application/x-pea",
        "application/x-tar",
        "application/gzip",
        "application/x-bzip",
        "application/x-bzip2",
        "application/x-xz",
        "application/x-lzma",
        "application/x-lzip",
        "application/x-compress",
        "application/x-lz4",
        "application/x-uuencode",
        "application/x-iso9660-image",
        "application/x-cd-image",
        "application/x-ms-wim",
        "application/vnd.debian.binary-package",
        "application/x-asar",
        "application/x-archive",
    ] {
        mimeapps_remove(mime, DESKTOP_ID);
    }
    Ok(())
}

pub fn is_installed(_scope: ShellScope) -> bool {
    let exists = |directory: PathBuf, prefix: &str, suffix: &str| {
        fs::read_dir(directory)
            .map(|entries| {
                entries.flatten().any(|entry| {
                    let name = entry.file_name().to_string_lossy().to_string();
                    name.starts_with(prefix) && name.ends_with(suffix)
                })
            })
            .unwrap_or(false)
    };
    nemo_extension_path().is_file()
        || exists(nemo_dir(), "rarkzip-", ".nemo_action")
        || exists(servicemenus_dir(), "rarkzip-", ".desktop")
        || nautilus_dir().is_dir()
}

pub fn register_association_with_action(
    _scope: ShellScope,
    extension: &str,
    exe: &Path,
    action: &str,
    folder: &str,
) -> Result<()> {
    let extension =
        normalize_extension(extension).ok_or_else(|| anyhow::anyhow!("无效的扩展名"))?;
    let mime = mime_for_extension(&extension)
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("application/x-{extension}"));
    let options = ShellOptions {
        exe: exe.to_path_buf(),
        archive_extensions: vec![extension],
        ..Default::default()
    };
    write_desktop_entry(&options, action, folder)?;
    mimeapps_add(&mime);
    Ok(())
}

pub fn unregister_association(_scope: ShellScope, extension: &str) -> Result<()> {
    let extension =
        normalize_extension(extension).ok_or_else(|| anyhow::anyhow!("无效的扩展名"))?;
    let mime = mime_for_extension(&extension)
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("application/x-{extension}"));
    mimeapps_remove(&mime, DESKTOP_ID);
    if !desktop_file_path().is_file() {
        return Ok(());
    }
    let text = fs::read_to_string(mimeapps_path()).unwrap_or_default();
    if !text.contains(DESKTOP_ID) {
        remove_desktop_entry();
    }
    Ok(())
}

pub fn is_association_registered(_scope: ShellScope, extension: &str) -> bool {
    let Some(extension) = normalize_extension(extension) else {
        return false;
    };
    let Some(mime) = mime_for_extension(&extension) else {
        return false;
    };
    mimeapps_values(mime, "[Default Applications]").contains(&DESKTOP_ID.to_string())
        || mimeapps_values(mime, "[Added Associations]").contains(&DESKTOP_ID.to_string())
}

pub fn association_status(_scope: ShellScope, extension: &str) -> AssociationStatus {
    let Some(extension) = normalize_extension(extension) else {
        return AssociationStatus::None;
    };
    let Some(mime) = mime_for_extension(&extension) else {
        return AssociationStatus::None;
    };
    if let Some(default) = mimeapps_values(mime, "[Default Applications]")
        .into_iter()
        .next()
    {
        if default == DESKTOP_ID {
            return AssociationStatus::RarkZip;
        }
        return AssociationStatus::Other(default);
    }
    if mimeapps_values(mime, "[Added Associations]").contains(&DESKTOP_ID.to_string()) {
        return AssociationStatus::RarkZip;
    }
    AssociationStatus::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_extensions() {
        assert_eq!(normalize_extension(".ZIP").as_deref(), Some("zip"));
        assert_eq!(normalize_extension("../zip").as_deref(), Some("zip"));
        assert!(normalize_extension("").is_none());
    }

    #[test]
    fn maps_common_mime_types() {
        assert_eq!(
            mime_for_extension("7z"),
            Some("application/x-7z-compressed")
        );
        assert_eq!(mime_for_extension("tar"), Some("application/x-tar"));
        assert_eq!(mime_for_extension("unknown"), None);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rarkzip-shell-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn installs_and_removes_nemo_actions_and_associations() {
        let root = temp_dir("install");
        let data = root.join("data");
        let config = root.join("config");
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        for manager in ["nemo", "nautilus", "dolphin"] {
            fs::write(bin.join(manager), "").unwrap();
        }

        let old_home = std::env::var_os("HOME");
        let old_data = std::env::var_os("XDG_DATA_HOME");
        let old_config = std::env::var_os("XDG_CONFIG_HOME");
        let old_path = std::env::var_os("PATH");
        // SAFETY: this test is the only one touching these variables.
        unsafe {
            std::env::set_var("HOME", &root);
            std::env::set_var("XDG_DATA_HOME", &data);
            std::env::set_var("XDG_CONFIG_HOME", &config);
            std::env::set_var("PATH", &bin);
        }

        let options = ShellOptions {
            exe: PathBuf::from("/usr/bin/rarkzip"),
            archive_extensions: vec!["zip".into(), "7z".into()],
            extract_verbs: vec!["here".into(), "smart".into()],
            compress_verbs: vec!["zip".into()],
            empty_verbs: vec!["new_here".into()],
            hash_menu: true,
            cascade: true,
            hide_icon: true,
            ..Default::default()
        };

        install(&options).unwrap();
        assert!(is_installed(ShellScope::CurrentUser));
        let actions = nemo_dir();
        assert!(actions.join("rarkzip-010-here.nemo_action").is_file());
        assert!(actions.join("rarkzip-400-hash.nemo_action").is_file());
        let here = fs::read_to_string(actions.join("rarkzip-010-here.nemo_action")).unwrap();
        assert!(here.contains("Selection=s"));
        assert!(here.contains("Extensions=7z;zip;"));
        assert!(!here.contains("Icon-Name"));
        let wrapper = fs::read_to_string(wrappers_dir().join("here.sh")).unwrap();
        assert!(wrapper.contains("--extract-here"));
        let metadata = fs::metadata(wrappers_dir().join("here.sh")).unwrap();
        assert!(metadata.permissions().mode() & 0o111 != 0);
        let service =
            fs::read_to_string(servicemenus_dir().join("rarkzip-extract.desktop")).unwrap();
        assert!(service.contains("X-KDE-Submenu=RarkZip"));
        assert!(service.contains("here.sh"));
        assert!(nautilus_dir().join("10-解压到此处.sh").is_file());

        register_association_with_action(
            ShellScope::CurrentUser,
            "zip",
            Path::new("/usr/bin/rarkzip"),
            "smart",
            "",
        )
        .unwrap();
        assert!(is_association_registered(ShellScope::CurrentUser, "zip"));
        assert_eq!(
            association_status(ShellScope::CurrentUser, "zip"),
            AssociationStatus::RarkZip
        );
        let desktop = fs::read_to_string(desktop_file_path()).unwrap();
        assert!(desktop.contains("--extract-smart %f"));
        let mimeapps = fs::read_to_string(mimeapps_path()).unwrap();
        assert!(mimeapps.contains("application/zip=rarkzip.desktop;"));

        uninstall(ShellScope::CurrentUser).unwrap();
        assert!(!is_installed(ShellScope::CurrentUser));
        assert!(!desktop_file_path().is_file());
        let mimeapps = fs::read_to_string(mimeapps_path()).unwrap_or_default();
        assert!(!mimeapps.contains("rarkzip.desktop"));

        unsafe {
            match old_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match old_data {
                Some(value) => std::env::set_var("XDG_DATA_HOME", value),
                None => std::env::remove_var("XDG_DATA_HOME"),
            }
            match old_config {
                Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
            match old_path {
                Some(value) => std::env::set_var("PATH", value),
                None => std::env::remove_var("PATH"),
            }
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn escapes_shell_and_desktop_quotes() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(desktop_quote("a b"), "\"a b\"");
    }
}
