//! Windows 注册表实现。
//!
//! 所有键都相对于 `HKCU\Software\Classes` 或 `HKLM\Software\Classes`，
//! 这样安装、卸载与关联逻辑可以共享同一套相对路径。

use std::io;
use std::path::Path;

use anyhow::{bail, Context, Result};
use winreg::enums::KEY_ALL_ACCESS;
use winreg::{RegKey, HKCU, HKLM};

use crate::common::{
    archive_applies_to, association_label, build_command, classify_association,
    enabled_compress_verbs, enabled_empty_verbs, enabled_extract_verbs, extension_key, icon_value,
    non_archive_applies_to, normalize_extension, prog_id, VerbSpec, CLASSES_PATH,
    DIRECTORY_BACKGROUND_SHELL_PATH, DIRECTORY_SHELL_PATH, FILE_SHELL_PATH, NIMZIP_PREFIX,
};
use crate::{AssociationStatus, ShellOptions, ShellScope};

pub(crate) fn install(options: &ShellOptions) -> Result<()> {
    validate_exe(&options.exe)?;
    let root = classes_root(options.scope);
    let hive = hive_name(options.scope);
    let (classes, _) = root.create_subkey(CLASSES_PATH).with_context(|| {
        format!("无法写入 {hive}\\{CLASSES_PATH}（LocalMachine 需要管理员权限）")
    })?;
    // 多选动态菜单需要同目录的 COM 右键扩展；DLL 不存在时退回静态 Player 菜单。
    let dll = options
        .exe
        .parent()
        .map(|parent| parent.join("nimzip_shell.dll"));
    let com_menu = dll
        .as_deref()
        .is_some_and(|dll| dll.exists() && crate::context_menu::register_context_menu(dll).is_ok());
    install_in(&classes, options, com_menu)
}

fn install_in(classes: &RegKey, options: &ShellOptions, com_menu: bool) -> Result<()> {
    let icon = (!options.hide_icon).then(|| icon_value(&options.exe));
    let archive_extensions: Vec<String> = options
        .archive_extensions
        .iter()
        .filter_map(|extension| normalize_extension(extension))
        .collect();
    let non_archive = non_archive_applies_to(&archive_extensions);
    let archive = archive_applies_to(&archive_extensions);

    // 先清掉旧版本按扩展名挂载的菜单，统一改为 *\shell + AppliesTo。
    clean_extension_shells(classes)?;

    let (file_shell, _) = classes
        .create_subkey(FILE_SHELL_PATH)
        .with_context(|| format!("无法创建注册表项：{FILE_SHELL_PATH}"))?;
    delete_prefixed_subkeys(&file_shell, NIMZIP_PREFIX)?;

    let (directory_shell, _) = classes
        .create_subkey(DIRECTORY_SHELL_PATH)
        .with_context(|| format!("无法创建注册表项：{DIRECTORY_SHELL_PATH}"))?;
    delete_prefixed_subkeys(&directory_shell, NIMZIP_PREFIX)?;

    if !com_menu {
        // DLL 不存在时退回静态菜单，保证基本可用；动态排序/名称由静态动词提供。
        let extract = enabled_extract_verbs(options);
        let compress = enabled_compress_verbs(options);
        install_verbs(
            &file_shell,
            &compress,
            &options.exe,
            icon.as_deref(),
            options.cascade,
            non_archive.as_deref(),
            false,
            "NimZip.CompressMenu",
        )?;
        if archive.is_some() {
            install_verbs(
                &file_shell,
                &extract,
                &options.exe,
                icon.as_deref(),
                options.cascade,
                archive.as_deref(),
                false,
                "NimZip.ArchiveMenu",
            )?;
        }
        install_verbs(
            &directory_shell,
            &compress,
            &options.exe,
            icon.as_deref(),
            options.cascade,
            None,
            false,
            "NimZip.Menu",
        )?;
    }

    // 空白处菜单保持静态，只在背景菜单提供独立命令。
    let (background_shell, _) = classes
        .create_subkey(DIRECTORY_BACKGROUND_SHELL_PATH)
        .with_context(|| format!("无法创建注册表项：{DIRECTORY_BACKGROUND_SHELL_PATH}"))?;
    delete_prefixed_subkeys(&background_shell, NIMZIP_PREFIX)?;
    install_verbs(
        &background_shell,
        &enabled_empty_verbs(options),
        &options.exe,
        icon.as_deref(),
        options.cascade,
        None,
        false,
        "NimZip.Menu",
    )?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn install_verbs(
    parent: &RegKey,
    verbs: &[&VerbSpec],
    exe: &Path,
    icon: Option<&str>,
    cascade: bool,
    applies_to: Option<&str>,
    single_select: bool,
    menu_key: &str,
) -> Result<()> {
    if verbs.is_empty() {
        return Ok(());
    }
    if cascade {
        let (menu, _) = parent
            .create_subkey(menu_key)
            .with_context(|| "无法创建 NimZip 级联菜单")?;
        menu.set_value("MUIVerb", &"NimZip (&N)")
            .with_context(|| "无法写入 NimZip 级联菜单标题")?;
        // 空 SubCommands 是 Win7+ 静态级联菜单的开关；仅有 shell 子键不会自动展开。
        menu.set_value("SubCommands", &"")
            .with_context(|| "无法写入 NimZip 级联菜单 SubCommands")?;
        menu.set_value(
            "MultiSelectModel",
            &if single_select { "Single" } else { "Player" },
        )
        .with_context(|| "无法写入 NimZip 级联菜单多选模式")?;
        if let Some(applies_to) = applies_to {
            menu.set_value("AppliesTo", &applies_to)
                .with_context(|| "无法写入 NimZip 级联菜单 AppliesTo")?;
        }
        if let Some(icon) = icon {
            menu.set_value("Icon", &icon)
                .with_context(|| "无法写入 NimZip 级联菜单图标")?;
        }
        let (submenu, _) = menu
            .create_subkey("shell")
            .with_context(|| "无法创建 NimZip 级联子菜单")?;
        for spec in verbs {
            let model = if single_select {
                Some("Single")
            } else {
                spec.multi_select
            };
            create_verb(&submenu, spec, exe, icon, model, None)?;
        }
    } else {
        for spec in verbs {
            let model = if single_select {
                Some("Single")
            } else {
                spec.multi_select
            };
            create_verb(parent, spec, exe, icon, model, applies_to)?;
        }
    }
    Ok(())
}

fn create_verb(
    parent: &RegKey,
    spec: &VerbSpec,
    exe: &Path,
    icon: Option<&str>,
    multi_select: Option<&str>,
    applies_to: Option<&str>,
) -> Result<()> {
    let (verb, _) = parent
        .create_subkey(spec.key)
        .with_context(|| format!("无法创建右键菜单项：{}", spec.key))?;
    verb.set_value("MUIVerb", &spec.label)
        .with_context(|| format!("无法写入 MUIVerb：{}", spec.key))?;
    if let Some(icon) = icon {
        verb.set_value("Icon", &icon)
            .with_context(|| format!("无法写入 Icon：{}", spec.key))?;
    }
    if let Some(model) = multi_select {
        verb.set_value("MultiSelectModel", &model)
            .with_context(|| format!("无法写入 MultiSelectModel：{}", spec.key))?;
    }
    if let Some(applies_to) = applies_to {
        verb.set_value("AppliesTo", &applies_to)
            .with_context(|| format!("无法写入 AppliesTo：{}", spec.key))?;
    }
    let (command, _) = verb
        .create_subkey("command")
        .with_context(|| format!("无法创建 command 项：{}", spec.key))?;
    command
        .set_value("", &build_command(exe, spec.args))
        .with_context(|| format!("无法写入命令行：{}", spec.key))?;
    Ok(())
}

pub(crate) fn uninstall(scope: ShellScope) -> Result<()> {
    let root = classes_root(scope);
    let hive = hive_name(scope);
    let classes = match root.open_subkey_with_flags(CLASSES_PATH, KEY_ALL_ACCESS) {
        Ok(key) => key,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(err).with_context(|| {
                format!("无法打开 {hive}\\{CLASSES_PATH}（LocalMachine 需要管理员权限）")
            });
        }
    };
    uninstall_in(&classes)
}

fn uninstall_in(classes: &RegKey) -> Result<()> {
    let _ = crate::context_menu::unregister_context_menu();
    for path in [
        FILE_SHELL_PATH,
        DIRECTORY_SHELL_PATH,
        DIRECTORY_BACKGROUND_SHELL_PATH,
    ] {
        if let Some(shell) = open_optional(classes, path)? {
            delete_prefixed_subkeys(&shell, NIMZIP_PREFIX)?;
        }
    }

    // 各压缩包扩展名下的专门菜单。
    clean_extension_shells(classes)?;

    // ProgID：Software\Classes\NimZip.<ext>
    delete_prefixed_subkeys(classes, NIMZIP_PREFIX)?;
    // OpenWithProgids 中的 NimZip.<ext> 值。
    clean_open_with_progids(classes)
}

/// 清理所有 `.ext\shell` 下的 NimZip 动词，包括已从关联列表移除的扩展名。
fn clean_extension_shells(classes: &RegKey) -> Result<()> {
    let mut extensions = Vec::new();
    for item in classes.enum_keys() {
        let name = item.context("枚举文件扩展名注册表项失败")?;
        if name.starts_with('.') {
            extensions.push(name);
        }
    }

    for extension in extensions {
        let path = format!("{extension}\\shell");
        if let Some(shell) = open_optional(classes, &path)? {
            delete_prefixed_subkeys(&shell, NIMZIP_PREFIX)?;
        }
    }
    Ok(())
}

fn open_optional(parent: &RegKey, path: &str) -> Result<Option<RegKey>> {
    match parent.open_subkey_with_flags(path, KEY_ALL_ACCESS) {
        Ok(key) => Ok(Some(key)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("无法打开注册表项：{path}")),
    }
}

fn delete_prefixed_subkeys(parent: &RegKey, prefix: &str) -> Result<()> {
    let mut names = Vec::new();
    for item in parent.enum_keys() {
        let name = item.context("枚举注册表子项失败")?;
        if name.starts_with(prefix) {
            names.push(name);
        }
    }

    for name in names {
        match parent.delete_subkey_all(&name) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err).with_context(|| format!("无法删除注册表项：{name}"));
            }
        }
    }
    Ok(())
}

fn clean_open_with_progids(classes: &RegKey) -> Result<()> {
    let mut extensions = Vec::new();
    for item in classes.enum_keys() {
        let name = item.context("枚举文件扩展名注册表项失败")?;
        if name.starts_with('.') {
            extensions.push(name);
        }
    }

    for extension in extensions {
        let path = format!("{extension}\\OpenWithProgids");
        let Some(open_with) = open_optional(classes, &path)? else {
            continue;
        };

        let mut values = Vec::new();
        for item in open_with.enum_values() {
            let (name, _) = item.context("枚举 OpenWithProgids 值失败")?;
            if name.starts_with(NIMZIP_PREFIX) {
                values.push(name);
            }
        }
        if values.is_empty() {
            continue;
        }
        for value in values {
            match open_with.delete_value(&value) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("无法删除 OpenWithProgids 值：{value}"));
                }
            }
        }
        drop(open_with);

        // 只清理刚刚移除了 NimZip 值、且已经为空的项，避免影响其他应用。
        delete_key_if_empty(classes, &path)?;
        delete_key_if_empty(classes, &extension)?;
    }
    Ok(())
}

fn delete_key_if_empty(parent: &RegKey, path: &str) -> Result<()> {
    let Some(key) = open_optional(parent, path)? else {
        return Ok(());
    };
    let is_empty = key.enum_values().next().is_none() && key.enum_keys().next().is_none();
    drop(key);
    if !is_empty {
        return Ok(());
    }
    match parent.delete_subkey_all(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("无法删除空注册表项：{path}")),
    }
}

pub(crate) fn is_installed(scope: ShellScope) -> bool {
    let root = classes_root(scope);
    let Ok(classes) = root.open_subkey(CLASSES_PATH) else {
        return false;
    };
    is_installed_in(&classes)
}

fn is_installed_in(classes: &RegKey) -> bool {
    if classes
        .open_subkey(r"*\shellex\ContextMenuHandlers\NimZip")
        .is_ok()
    {
        return true;
    }
    for path in [
        FILE_SHELL_PATH,
        DIRECTORY_SHELL_PATH,
        DIRECTORY_BACKGROUND_SHELL_PATH,
    ] {
        let Ok(shell) = classes.open_subkey(path) else {
            continue;
        };
        if shell
            .enum_keys()
            .flatten()
            .any(|name| name.starts_with(NIMZIP_PREFIX))
        {
            return true;
        }
    }

    for item in classes.enum_keys().flatten() {
        if !item.starts_with('.') {
            continue;
        }
        let Ok(shell) = classes.open_subkey(format!("{item}\\shell")) else {
            continue;
        };
        if shell
            .enum_keys()
            .flatten()
            .any(|name| name.starts_with(NIMZIP_PREFIX))
        {
            return true;
        }
    }
    false
}

pub(crate) fn register_association_with_action(
    scope: ShellScope,
    extension: &str,
    exe: &Path,
    action: &str,
    folder: &str,
) -> Result<()> {
    let extension =
        normalize_extension(extension).with_context(|| format!("不支持的扩展名：{extension}"))?;
    validate_exe(exe)?;

    let root = classes_root(scope);
    let hive = hive_name(scope);
    let (classes, _) = root.create_subkey(CLASSES_PATH).with_context(|| {
        format!("无法写入 {hive}\\{CLASSES_PATH}（LocalMachine 需要管理员权限）")
    })?;

    let our_prog_id = prog_id(&extension);
    let (prog_key, _) = classes
        .create_subkey(&our_prog_id)
        .with_context(|| format!("无法创建 ProgID：{our_prog_id}"))?;
    prog_key
        .set_value("", &association_label(&extension))
        .with_context(|| format!("无法写入 ProgID 描述：{our_prog_id}"))?;
    prog_key
        .set_value("DefaultIcon", &icon_value(exe))
        .with_context(|| format!("无法写入 DefaultIcon：{our_prog_id}"))?;

    let (open_key, _) = prog_key
        .create_subkey(r"shell\open")
        .with_context(|| format!("无法创建 shell\\open：{our_prog_id}"))?;
    let (command_key, _) = open_key
        .create_subkey("command")
        .with_context(|| format!("无法创建 command：{our_prog_id}"))?;
    command_key
        .set_value("", &association_command(exe, action, folder))
        .with_context(|| format!("无法写入打开命令：{our_prog_id}"))?;

    let extension_key_name = extension_key(&extension);
    let (extension_reg_key, _) = classes
        .create_subkey(&extension_key_name)
        .with_context(|| format!("无法创建扩展名项：{extension_key_name}"))?;
    let (open_with, _) = extension_reg_key
        .create_subkey("OpenWithProgids")
        .with_context(|| format!("无法创建 OpenWithProgids：{extension_key_name}"))?;
    open_with
        .set_value(&our_prog_id, &String::new())
        .with_context(|| format!("无法写入 OpenWithProgids：{our_prog_id}"))?;

    Ok(())
}

fn association_command(exe: &Path, action: &str, folder: &str) -> String {
    match action {
        "smart" => build_command(exe, &["--cli", "x", "%1"]),
        "folder" => build_command(exe, &["--cli", "x", "%1", "--folder"]),
        "custom" if !folder.trim().is_empty() => {
            build_command(exe, &["--cli", "x", "%1", "-o", folder])
        }
        _ => build_command(exe, &["--gui", "%1"]),
    }
}

pub(crate) fn unregister_association(scope: ShellScope, extension: &str) -> Result<()> {
    let extension =
        normalize_extension(extension).with_context(|| format!("不支持的扩展名：{extension}"))?;
    let root = classes_root(scope);
    let hive = hive_name(scope);
    let classes = match root.open_subkey_with_flags(CLASSES_PATH, KEY_ALL_ACCESS) {
        Ok(key) => key,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(err).with_context(|| {
                format!("无法打开 {hive}\\{CLASSES_PATH}（LocalMachine 需要管理员权限）")
            });
        }
    };

    let our_prog_id = prog_id(&extension);
    match classes.delete_subkey_all(&our_prog_id) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| format!("无法删除 ProgID：{our_prog_id}"));
        }
    }

    let path = format!("{}\\OpenWithProgids", extension_key(&extension));
    if let Some(open_with) = open_optional(&classes, &path)? {
        match open_with.delete_value(&our_prog_id) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("无法删除 OpenWithProgids：{our_prog_id}"));
            }
        }
        drop(open_with);
        delete_key_if_empty(&classes, &path)?;
        delete_key_if_empty(&classes, &extension_key(&extension))?;
    }
    Ok(())
}

pub(crate) fn is_association_registered(scope: ShellScope, extension: &str) -> bool {
    let Some(extension) = normalize_extension(extension) else {
        return false;
    };
    let root = classes_root(scope);
    let Ok(classes) = root.open_subkey(CLASSES_PATH) else {
        return false;
    };
    classes.open_subkey(prog_id(&extension)).is_ok()
}

pub(crate) fn association_status(scope: ShellScope, extension: &str) -> AssociationStatus {
    let Some(extension) = normalize_extension(extension) else {
        return AssociationStatus::None;
    };
    let our_prog_id = prog_id(&extension);

    // UserChoice 由 Windows 保护，只读取、不写入。
    if let Some(current) = user_choice_prog_id(&extension) {
        return classify_association(&current, &our_prog_id);
    }

    // scope 优先，再回退到另一套注册表配置单元。
    let primary = classes_root(scope);
    let secondary = other_root(scope);
    if let Some(current) =
        default_prog_id(primary, &extension).or_else(|| default_prog_id(secondary, &extension))
    {
        return classify_association(&current, &our_prog_id);
    }

    if open_with_registered(primary, &extension, &our_prog_id)
        || open_with_registered(secondary, &extension, &our_prog_id)
    {
        AssociationStatus::NimZip
    } else {
        AssociationStatus::None
    }
}

fn user_choice_prog_id(extension: &str) -> Option<String> {
    let path = format!(
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\{}\UserChoice",
        extension_key(extension)
    );
    let key = HKCU.open_subkey(path).ok()?;
    non_empty_value(&key, "ProgId")
}

fn default_prog_id(root: &RegKey, extension: &str) -> Option<String> {
    let path = format!("{CLASSES_PATH}\\{}", extension_key(extension));
    let key = root.open_subkey(path).ok()?;
    non_empty_value(&key, "")
}

fn non_empty_value(key: &RegKey, name: &str) -> Option<String> {
    let value: String = key.get_value(name).ok()?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn open_with_registered(root: &RegKey, extension: &str, our_prog_id: &str) -> bool {
    let path = format!(
        "{CLASSES_PATH}\\{}\\OpenWithProgids",
        extension_key(extension)
    );
    root.open_subkey(path)
        .ok()
        .is_some_and(|key| key.get_raw_value(our_prog_id).is_ok())
}

fn classes_root(scope: ShellScope) -> &'static RegKey {
    match scope {
        ShellScope::CurrentUser => HKCU,
        ShellScope::LocalMachine => HKLM,
    }
}

fn other_root(scope: ShellScope) -> &'static RegKey {
    match scope {
        ShellScope::CurrentUser => HKLM,
        ShellScope::LocalMachine => HKCU,
    }
}

fn hive_name(scope: ShellScope) -> &'static str {
    match scope {
        ShellScope::CurrentUser => "HKCU",
        ShellScope::LocalMachine => "HKLM",
    }
}

fn validate_exe(exe: &Path) -> Result<()> {
    if exe.as_os_str().is_empty() {
        bail!("可执行文件路径为空");
    }
    if !exe.is_absolute() {
        bail!("可执行文件路径必须是绝对路径：{}", exe.display());
    }
    Ok(())
}
