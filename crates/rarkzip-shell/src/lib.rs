//! RarkZip Windows 资源管理器集成：经典右键菜单与文件关联。
//!
//! 所有注册表操作都限定在 `HKCU\Software\Classes`（当前用户）或
//! `HKLM\Software\Classes`（本机，需要管理员权限）内。非 Windows 平台上
//! 写操作返回错误，查询返回 `false` 或 [`AssociationStatus::None`]。
//!
//! 本 crate 不依赖 GUI/CLI，两者都可以调用这里的 API 完成安装、卸载和
//! 关联状态查询。

use std::path::PathBuf;

mod common;

#[cfg(windows)]
mod context_menu;
#[cfg(windows)]
mod preview;
#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use context_menu::{
    context_menu_clsid, context_menu_clsid_string, context_menu_installed, register_context_menu,
    unregister_context_menu,
};
#[cfg(windows)]
pub use preview::{
    preview_clsid, preview_clsid_string, preview_handler_installed, register_preview_handler,
    unregister_preview_handler,
};

/// 注册表写入范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ShellScope {
    /// 仅当前用户，写入 `HKCU\Software\Classes`，不需要管理员权限。
    #[default]
    CurrentUser,
    /// 本机所有用户，写入 `HKLM\Software\Classes`，需要管理员权限。
    LocalMachine,
}

/// 右键菜单安装选项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOptions {
    /// RarkZip 可执行文件路径，应为绝对路径。
    pub exe: PathBuf,
    /// 注册表写入范围。
    pub scope: ShellScope,
    /// 支持压缩包专用右键菜单的扩展名（不含前导点）。
    ///
    /// 这些扩展名会注册解压/打开/测试等动作；其他文件只显示压缩动作，
    /// 多选时仅保留可批量执行的压缩菜单。
    pub archive_extensions: Vec<String>,
    /// 文件右键菜单中启用的解压/打开动作 key。
    pub extract_verbs: Vec<String>,
    /// 在文件右键菜单中显示“计算文件哈希”。
    pub hash_menu: bool,
    /// 文件/文件夹右键菜单中启用的压缩动作 key。
    pub compress_verbs: Vec<String>,
    /// 目录空白处右键菜单动作 key。
    pub empty_verbs: Vec<String>,
    /// 双击压缩包的动作：open / smart / folder / custom。
    pub double_click: String,
    /// 双击动作为 custom 时的目标目录。
    pub double_click_folder: String,
    /// 将 RarkZip 动作收进级联子菜单。
    pub cascade: bool,
    /// 右键菜单不显示 RarkZip 图标。
    pub hide_icon: bool,
}

impl Default for ShellOptions {
    fn default() -> Self {
        Self {
            exe: std::env::current_exe().unwrap_or_default(),
            scope: ShellScope::CurrentUser,
            archive_extensions: crate::common::DEFAULT_ARCHIVE_EXTENSIONS
                .iter()
                .map(|extension| extension.to_string())
                .collect(),
            extract_verbs: [
                "here",
                "here_delete",
                "smart",
                "smart_delete",
                "folder",
                "folder_delete",
                "other",
            ]
            .into_iter()
            .map(ToString::to_string)
            .collect(),
            hash_menu: true,
            compress_verbs: ["zip", "sevenz", "each_zip", "gui"]
                .into_iter()
                .map(ToString::to_string)
                .collect(),
            empty_verbs: Vec::new(),
            double_click: "open".to_string(),
            double_click_folder: String::new(),
            cascade: true,
            hide_icon: false,
        }
    }
}

/// 某个扩展名当前关联的应用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssociationStatus {
    /// 已关联到 RarkZip（RarkZip 是有效默认项，或已在 OpenWithProgids 中注册）。
    RarkZip,
    /// 已关联到其他应用，值为默认 ProgID（例如 `zipfile`、`Applications\7zFM.exe`）。
    Other(String),
    /// 没有找到默认关联，也没有找到 RarkZip 的 OpenWithProgids 注册。
    None,
}

/// 安装资源管理器右键菜单。
///
/// 该操作是幂等的：会先移除已有的 `RarkZip.*` 动词，再按 `options` 重新创建。
/// 只写入 RarkZip 自己的键，不会影响其他应用。`LocalMachine` 需要管理员权限。
pub fn install(options: &ShellOptions) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        windows::install(options)
    }

    #[cfg(not(windows))]
    {
        let _ = options;
        Err(unsupported())
    }
}

/// 卸载 RarkZip 创建的右键菜单、ProgID 与 OpenWithProgids 条目。
///
/// 不会删除扩展名本身的注册表项，也不会触碰 Windows 保护的 `UserChoice`。
pub fn uninstall(scope: ShellScope) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        windows::uninstall(scope)
    }

    #[cfg(not(windows))]
    {
        let _ = scope;
        Err(unsupported())
    }
}

/// 是否已安装 RarkZip 右键菜单（存在任意 `RarkZip.*` 动词即视为已安装）。
pub fn is_installed(scope: ShellScope) -> bool {
    #[cfg(windows)]
    {
        windows::is_installed(scope)
    }

    #[cfg(not(windows))]
    {
        let _ = scope;
        false
    }
}

/// 为一个扩展名注册 RarkZip ProgID，并加入 `OpenWithProgids`。
///
/// `extension` 可以带或不带前导点（例如 `zip`、`.zip`、`tar.gz`）。该函数
/// 不会写入 `UserChoice`，因此 Windows 10/11 仍由用户决定默认应用；用户
/// 可以在“打开方式”中选择 RarkZip。
pub fn register_association(
    scope: ShellScope,
    extension: &str,
    exe: &std::path::Path,
) -> anyhow::Result<()> {
    register_association_with_action(scope, extension, exe, "open", "")
}

/// 注册文件关联，并指定双击压缩包时执行的动作。
///
/// `action` 支持 `open`（打开主窗口）、`smart`（智能解压）、
/// `folder`（解压到同名文件夹）与 `custom`（解压到 `folder`）。
pub fn register_association_with_action(
    scope: ShellScope,
    extension: &str,
    exe: &std::path::Path,
    action: &str,
    folder: &str,
) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        windows::register_association_with_action(scope, extension, exe, action, folder)
    }

    #[cfg(not(windows))]
    {
        let _ = (scope, extension, exe, action, folder);
        Err(unsupported())
    }
}

/// 移除某个扩展名的 RarkZip ProgID 与 `OpenWithProgids` 候选。
///
/// 只清理 RarkZip 自己的键与值，不影响其他应用的默认关联，也不修改
/// Windows 保护的 `UserChoice`。
pub fn unregister_association(scope: ShellScope, extension: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        windows::unregister_association(scope, extension)
    }

    #[cfg(not(windows))]
    {
        let _ = (scope, extension);
        Err(unsupported())
    }
}

/// 是否已经为扩展名创建了 RarkZip ProgID（即已加入打开方式候选）。
pub fn is_association_registered(scope: ShellScope, extension: &str) -> bool {
    #[cfg(windows)]
    {
        windows::is_association_registered(scope, extension)
    }

    #[cfg(not(windows))]
    {
        let _ = (scope, extension);
        false
    }
}

/// 查询扩展名当前的关联状态。
///
/// 优先读取 `UserChoice`，其次读取 `Software\Classes\.<ext>` 的默认值；
/// 都没有默认关联时，若扩展名的 `OpenWithProgids` 中存在 RarkZip 条目，
/// 返回 [`AssociationStatus::RarkZip`]。
pub fn association_status(scope: ShellScope, extension: &str) -> AssociationStatus {
    #[cfg(windows)]
    {
        windows::association_status(scope, extension)
    }

    #[cfg(not(windows))]
    {
        let _ = (scope, extension);
        AssociationStatus::None
    }
}

#[cfg(not(windows))]
fn unsupported() -> anyhow::Error {
    anyhow::anyhow!("RarkZip 资源管理器集成仅支持 Windows")
}
