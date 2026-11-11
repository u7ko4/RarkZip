//! 手动运行的真实注册表集成测试。
//!
//! 该测试会短暂写入当前用户的 `HKCU\Software\Classes`，因此默认标记为
//! `#[ignore]`。若检测到已有 NimZip 右键菜单，则直接跳过，避免覆盖用户配置。
//! 运行方式：
//!
//! ```text
//! cargo test -p nimzip-shell --test registry_windows -- --ignored --nocapture
//! ```

#![cfg(windows)]

use std::process;

use nimzip_shell::{
    association_status, install, is_association_registered, is_installed, register_association,
    register_association_with_action, uninstall, unregister_association, AssociationStatus,
    ShellOptions, ShellScope,
};

struct Cleanup;

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = uninstall(ShellScope::CurrentUser);
    }
}

#[test]
#[ignore = "写入 HKCU\\Software\\Classes，需在 Windows 测试机上手动运行"]
fn install_register_and_uninstall_roundtrip() {
    if is_installed(ShellScope::CurrentUser) {
        eprintln!("跳过：当前用户已安装 NimZip 右键菜单，避免破坏现有配置");
        return;
    }

    let exe = std::env::current_exe().expect("测试可执行文件路径");
    let pid = process::id();
    let extension = format!("nimzipshelltest{pid}");

    let options = ShellOptions {
        exe: exe.clone(),
        scope: ShellScope::CurrentUser,
        archive_extensions: vec![extension.clone()],
        extract_verbs: vec!["open".into(), "here".into(), "test".into()],
        compress_verbs: vec!["zip".into(), "gui".into()],
        empty_verbs: vec!["open".into(), "new".into()],
        cascade: true,
        hide_icon: true,
        ..Default::default()
    };

    let _cleanup = Cleanup;

    install(&options).expect("安装右键菜单");
    assert!(is_installed(ShellScope::CurrentUser));

    register_association(ShellScope::CurrentUser, &extension, &exe).expect("注册文件关联");
    assert!(is_association_registered(
        ShellScope::CurrentUser,
        &extension
    ));
    assert_eq!(
        association_status(ShellScope::CurrentUser, &extension),
        AssociationStatus::NimZip
    );

    register_association_with_action(ShellScope::CurrentUser, &extension, &exe, "smart", "")
        .expect("更新双击动作");
    assert!(is_association_registered(
        ShellScope::CurrentUser,
        &extension
    ));

    unregister_association(ShellScope::CurrentUser, &extension).expect("移除文件关联");
    assert!(!is_association_registered(
        ShellScope::CurrentUser,
        &extension
    ));
    assert_eq!(
        association_status(ShellScope::CurrentUser, &extension),
        AssociationStatus::None
    );

    uninstall(ShellScope::CurrentUser).expect("卸载");
    assert!(!is_installed(ShellScope::CurrentUser));
}
