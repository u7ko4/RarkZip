//! 经典资源管理器多选右键菜单（COM `IContextMenu` + `IShellExtInit`）。
//!
//! 静态注册表菜单只能按右键点中的单个对象判断类型；多选混合选择时无法知道
//! 整个选择。这里用 COM 处理器只在 **多选（≥2 项）** 时接管菜单：
//! - 全部为压缩包：显示解压组；
//! - 含普通文件/文件夹：显示压缩组，并生成动态名称。
//!
//! 单选仍由静态注册表菜单处理，避免重复。

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use rarkzip_engine::Settings;
    use windows::core::{implement, ComObject, Error, IUnknown, Interface, GUID, HSTRING, PCWSTR};
    use windows::Win32::Foundation::{E_FAIL, E_INVALIDARG, HMODULE};
    use windows::Win32::Graphics::Gdi::{DeleteObject, HBITMAP, HGDIOBJ};
    use windows::Win32::System::Com::{
        IClassFactory, IClassFactory_Impl, IDataObject, DVASPECT_CONTENT, FORMATETC, STGMEDIUM,
        TYMED_HGLOBAL,
    };
    use windows::Win32::System::LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };
    use windows::Win32::System::Ole::{ReleaseStgMedium, CF_HDROP};
    use windows::Win32::System::Registry::HKEY as RegistryHKEY;
    use windows::Win32::UI::Shell::{
        DragQueryFileW, IContextMenu, IContextMenu_Impl, IShellExtInit, IShellExtInit_Impl,
        SHGetFileInfoW, HDROP, SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, DestroyIcon, DestroyMenu, GetIconInfo, InsertMenuW,
        SetMenuItemInfoW, HMENU, ICONINFO, MENUITEMINFOW, MF_BYPOSITION, MF_POPUP, MF_STRING,
        MIIM_BITMAP,
    };
    use winreg::enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS};
    use winreg::RegKey;

    const CONTEXT_MENU_CLSID: &str = "{6F1B4A54-3C7D-4E21-9A5B-2F8C1D4E6A70}";

    pub fn context_menu_clsid() -> GUID {
        GUID::from_u128(0x6f1b4a54_3c7d_4e21_9a5b_2f8c1d4e6a70)
    }

    pub fn context_menu_clsid_string() -> String {
        CONTEXT_MENU_CLSID.to_string()
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum MenuAction {
        Open,
        Preview,
        Extract(&'static str, bool),
        QuickExtract,
        AdminExtract,
        Test,
        Hash,
        NewArchive,
        QuickCompress(&'static str, bool),
        EachCompress(&'static str),
        AdminCompress,
    }

    #[implement(IShellExtInit, IContextMenu)]
    struct ContextMenuHandler {
        paths: RefCell<Vec<PathBuf>>,
        actions: RefCell<Vec<MenuAction>>,
        icon_bitmaps: RefCell<Vec<HBITMAP>>,
    }

    impl ContextMenuHandler {
        fn new() -> Self {
            Self {
                paths: RefCell::new(Vec::new()),
                actions: RefCell::new(Vec::new()),
                icon_bitmaps: RefCell::new(Vec::new()),
            }
        }
    }

    impl Drop for ContextMenuHandler {
        fn drop(&mut self) {
            for bitmap in self.icon_bitmaps.borrow_mut().drain(..) {
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(bitmap.0));
                }
            }
        }
    }

    fn load_icon_bitmap() -> Option<HBITMAP> {
        let exe = exe_path()?;
        let wide: Vec<u16> = exe
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut file_info = SHFILEINFOW::default();
        let result = unsafe {
            SHGetFileInfoW(
                PCWSTR(wide.as_ptr()),
                Default::default(),
                Some(&mut file_info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_SMALLICON,
            )
        };
        if result == 0 || file_info.hIcon.is_invalid() {
            return None;
        }
        let mut icon_info = ICONINFO::default();
        let result = unsafe { GetIconInfo(file_info.hIcon, &mut icon_info) };
        unsafe {
            let _ = DestroyIcon(file_info.hIcon);
        }
        result.ok()?;
        let bitmap = if !icon_info.hbmColor.is_invalid() {
            if !icon_info.hbmMask.is_invalid() {
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(icon_info.hbmMask.0));
                }
            }
            icon_info.hbmColor
        } else {
            icon_info.hbmMask
        };
        (!bitmap.is_invalid()).then_some(bitmap)
    }

    pub(super) fn is_archive(path: &Path, settings: &Settings) -> bool {
        path.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    settings
                        .integration
                        .associations
                        .iter()
                        .any(|item| item.trim_start_matches('.').eq_ignore_ascii_case(extension))
                })
    }

    fn parent_dir(paths: &[PathBuf]) -> PathBuf {
        paths
            .first()
            .and_then(|path| path.parent())
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// 选中项的基准名：单选用文件/文件夹名，多选用所在文件夹名。
    fn archive_base(paths: &[PathBuf]) -> String {
        let Some(first) = paths.first() else {
            return "archive".to_string();
        };
        if paths.len() == 1 {
            if first.is_dir() {
                return first
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "archive".to_string());
            }
            return rarkzip_engine::paths::archive_stem(first);
        }
        parent_dir(paths)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "archive".to_string())
    }

    /// 返回不冲突的目标文件名，已有同名文件时追加 " (2)"、" (3)"。
    fn unique_target(dir: &Path, base: &str, extension: &str) -> String {
        let mut index = 1;
        loop {
            let name = if index <= 1 {
                format!("{base}.{extension}")
            } else {
                format!("{base} ({index}).{extension}")
            };
            if !dir.join(&name).exists() {
                return name;
            }
            index += 1;
        }
    }

    fn add_action(
        labels: &mut Vec<String>,
        actions: &mut Vec<MenuAction>,
        label: impl Into<String>,
        action: MenuAction,
    ) {
        labels.push(label.into());
        actions.push(action);
    }

    fn enabled(key: &str, keys: &[String]) -> bool {
        keys.iter().any(|item| item == key)
    }

    fn push_extract_actions(
        paths: &[PathBuf],
        settings: &Settings,
        labels: &mut Vec<String>,
        actions: &mut Vec<MenuAction>,
    ) {
        let keys = &settings.integration.extract_verbs;
        let single = paths.len() == 1;
        let stem = paths
            .first()
            .map(|path| rarkzip_engine::paths::archive_stem(path))
            .unwrap_or_else(|| "archive".to_string());
        let (smart_label, smart_delete_label) = if single {
            ("智能解压到此处(&S)", "智能解压到此处并删除")
        } else {
            ("智能解压到独立文件夹(&S)", "智能解压到独立文件夹并删除")
        };
        let (folder_label, folder_delete_label) = if single {
            (
                format!("解压到 {stem}\\(&T)"),
                format!("解压到 {stem}\\ 并删除 (&T)"),
            )
        } else {
            (
                "解压到独立文件夹".to_string(),
                "解压到独立文件夹并删除".to_string(),
            )
        };
        // 多选时智能解压落到各自独立文件夹。
        let smart_flag = if single {
            "--extract-smart"
        } else {
            "--extract-folder"
        };

        if enabled("here", keys) {
            add_action(
                labels,
                actions,
                "解压到此处(&E)",
                MenuAction::Extract("--extract-here", false),
            );
        }
        if enabled("here_delete", keys) {
            add_action(
                labels,
                actions,
                "解压到此处并删除",
                MenuAction::Extract("--extract-here", true),
            );
        }
        if enabled("smart", keys) {
            add_action(
                labels,
                actions,
                smart_label,
                MenuAction::Extract(smart_flag, false),
            );
        }
        if enabled("smart_delete", keys) {
            add_action(
                labels,
                actions,
                smart_delete_label,
                MenuAction::Extract(smart_flag, true),
            );
        }
        if enabled("folder", keys) {
            add_action(
                labels,
                actions,
                folder_label,
                MenuAction::Extract("--extract-folder", false),
            );
        }
        if enabled("folder_delete", keys) {
            add_action(
                labels,
                actions,
                folder_delete_label,
                MenuAction::Extract("--extract-folder", true),
            );
        }
        if enabled("other", keys) {
            add_action(
                labels,
                actions,
                "解压到其他文件夹(&B)...",
                MenuAction::QuickExtract,
            );
        }
        if enabled("admin", keys) {
            add_action(
                labels,
                actions,
                "以管理员身份解压",
                MenuAction::AdminExtract,
            );
        }
        if enabled("test", keys) {
            add_action(labels, actions, "测试压缩文件", MenuAction::Test);
        }
        if enabled("open", keys) {
            add_action(labels, actions, "用 RarkZip 打开", MenuAction::Open);
        }
        if enabled("preview", keys) {
            add_action(labels, actions, "压缩文件预览...", MenuAction::Preview);
        }
    }

    fn push_compress_actions(
        paths: &[PathBuf],
        settings: &Settings,
        labels: &mut Vec<String>,
        actions: &mut Vec<MenuAction>,
    ) {
        let keys = &settings.integration.compress_verbs;
        let single = paths.len() == 1;
        let all_files = paths.iter().all(|path| path.is_file());
        let dir = parent_dir(paths);
        let base = archive_base(paths);
        let target = |extension: &str| unique_target(&dir, &base, extension);
        let zip_name = target("zip");

        if enabled("zip", keys) {
            add_action(
                labels,
                actions,
                format!("压缩为“{}.&zip”", zip_name.trim_end_matches(".zip")),
                MenuAction::QuickCompress("zip", false),
            );
        }
        if enabled("zip_delete", keys) {
            add_action(
                labels,
                actions,
                format!("压缩为“{zip_name}”并删除"),
                MenuAction::QuickCompress("zip", true),
            );
        }
        if enabled("tar", keys) {
            add_action(
                labels,
                actions,
                format!("压缩为“{}”", target("tar")),
                MenuAction::QuickCompress("tar", false),
            );
        }
        if enabled("tgz", keys) {
            add_action(
                labels,
                actions,
                format!("压缩为“{}”", target("tgz")),
                MenuAction::QuickCompress("tgz", false),
            );
        }
        if enabled("sevenz", keys) {
            add_action(
                labels,
                actions,
                format!("压缩为“{}”", target("7z")),
                MenuAction::QuickCompress("7z", false),
            );
        }

        for (key, extension) in [("gz", "gz"), ("xz", "xz"), ("zst", "zst")] {
            if !enabled(key, keys) {
                continue;
            }
            if single && paths[0].is_file() {
                let full_name = paths[0]
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| base.clone());
                add_action(
                    labels,
                    actions,
                    format!("压缩为“{}”", unique_target(&dir, &full_name, extension)),
                    MenuAction::QuickCompress(extension, false),
                );
            } else if !single && all_files {
                add_action(
                    labels,
                    actions,
                    format!("压缩为独立的“文件名.{extension}”"),
                    MenuAction::EachCompress(extension),
                );
            }
        }

        if !single && enabled("each_zip", keys) {
            add_action(
                labels,
                actions,
                "压缩为独立的“压缩文件/文件夹名.zip”(&E)",
                MenuAction::EachCompress("zip"),
            );
        }
        if !single && enabled("each_7z", keys) {
            add_action(
                labels,
                actions,
                "压缩为独立的“压缩文件/文件夹名.7z”",
                MenuAction::EachCompress("7z"),
            );
        }
        if enabled("gui", keys) {
            add_action(
                labels,
                actions,
                "新建压缩包 (RarkZip)(&B)...",
                MenuAction::NewArchive,
            );
        }
        if enabled("admin", keys) {
            add_action(
                labels,
                actions,
                "以管理员身份压缩(&Y)",
                MenuAction::AdminCompress,
            );
        }
    }

    pub(super) fn build_actions(
        paths: &[PathBuf],
        settings: &Settings,
    ) -> (Vec<String>, Vec<MenuAction>) {
        let mut labels = Vec::new();
        let mut actions = Vec::new();
        if settings.integration.disable_menu || paths.is_empty() {
            return (labels, actions);
        }
        let all_archives = paths.iter().all(|path| is_archive(path, settings));
        if all_archives {
            push_extract_actions(paths, settings, &mut labels, &mut actions);
            // 多选压缩包时追加压缩动作；单选压缩包只显示解压动作。
            if paths.len() > 1 {
                push_compress_actions(paths, settings, &mut labels, &mut actions);
            }
        } else {
            push_compress_actions(paths, settings, &mut labels, &mut actions);
        }
        // 哈希对普通文件和压缩包都可用；文件夹不参与。
        if settings.integration.hash_menu && paths.iter().all(|path| path.is_file()) {
            add_action(
                &mut labels,
                &mut actions,
                if paths.len() == 1 {
                    "计算文件哈希…"
                } else {
                    "计算所选文件的哈希…"
                },
                MenuAction::Hash,
            );
        }
        (labels, actions)
    }

    fn module_path() -> Option<PathBuf> {
        let mut module = HMODULE::default();
        unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                PCWSTR(register_from_module as *const u16),
                &mut module,
            )
            .ok()?;
        }
        let mut buffer = vec![0u16; 32768];
        let len = unsafe { GetModuleFileNameW(Some(module), &mut buffer) } as usize;
        if len == 0 {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(&buffer[..len])))
    }

    fn exe_path() -> Option<PathBuf> {
        module_path()?
            .parent()
            .map(|parent| parent.join("rarkzip.exe"))
    }

    fn spawn(args: &[&str]) {
        let Some(exe) = exe_path() else {
            return;
        };
        let _ = Command::new(exe).args(args).spawn();
    }

    fn invoke(action: MenuAction, paths: &[PathBuf]) {
        match action {
            MenuAction::Open | MenuAction::Preview => {
                for path in paths {
                    let path = path.to_string_lossy().to_string();
                    spawn(&["--gui", &path]);
                }
            }
            MenuAction::Extract(flag, delete) => {
                for path in paths {
                    let path = path.to_string_lossy().to_string();
                    if delete {
                        spawn(&[flag, &path, "--delete"]);
                    } else {
                        spawn(&[flag, &path]);
                    }
                }
            }
            MenuAction::QuickExtract => {
                for path in paths {
                    let path = path.to_string_lossy().to_string();
                    spawn(&["--quick-extract", &path]);
                }
            }
            MenuAction::AdminExtract => {
                for path in paths {
                    let path = path.to_string_lossy().to_string();
                    spawn(&["--elevated", "--quick-extract", &path]);
                }
            }
            MenuAction::Test => {
                for path in paths {
                    let path = path.to_string_lossy().to_string();
                    spawn(&["--cli", "t", &path]);
                }
            }
            MenuAction::Hash => {
                let Some(exe) = exe_path() else {
                    return;
                };
                let mut command = Command::new(exe);
                for path in paths {
                    command.arg("--hash").arg(path);
                }
                let _ = command.spawn();
            }
            MenuAction::NewArchive => {
                let Some(exe) = exe_path() else {
                    return;
                };
                let mut command = Command::new(exe);
                command.arg("--gui");
                for path in paths {
                    command.arg("--new").arg(path);
                }
                let _ = command.spawn();
            }
            MenuAction::QuickCompress(format, delete) => {
                for path in paths {
                    let path = path.to_string_lossy().to_string();
                    if delete {
                        spawn(&[
                            "--quick-compress",
                            &path,
                            "--format",
                            format,
                            "--delete-sources",
                        ]);
                    } else {
                        spawn(&["--quick-compress", &path, "--format", format]);
                    }
                }
            }
            MenuAction::EachCompress(format) => {
                let Some(exe) = exe_path() else {
                    return;
                };
                let mut command = Command::new(exe);
                command.args(["--cli", "ce", "-f", format]);
                for path in paths {
                    command.arg(path);
                }
                let _ = command.spawn();
            }
            MenuAction::AdminCompress => {
                for path in paths {
                    let path = path.to_string_lossy().to_string();
                    spawn(&["--elevated", "--quick-compress", &path, "--format", "zip"]);
                }
            }
        }
    }

    impl IShellExtInit_Impl for ContextMenuHandler_Impl {
        fn Initialize(
            &self,
            _pidlfolder: *const windows::Win32::UI::Shell::Common::ITEMIDLIST,
            pdtobj: windows::core::Ref<IDataObject>,
            _hkeyprogid: RegistryHKEY,
        ) -> windows::core::Result<()> {
            let Some(data) = pdtobj.as_ref() else {
                return Ok(());
            };
            let format = FORMATETC {
                cfFormat: CF_HDROP.0,
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
                ..Default::default()
            };
            let mut medium: STGMEDIUM = unsafe { data.GetData(&format)? };
            let drop = HDROP(unsafe { medium.u.hGlobal }.0);
            let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
            let mut paths = Vec::new();
            for index in 0..count {
                let len = unsafe { DragQueryFileW(drop, index, None) } as usize;
                if len == 0 {
                    continue;
                }
                let mut buffer = vec![0u16; len + 1];
                unsafe { DragQueryFileW(drop, index, Some(&mut buffer)) };
                let end = buffer.iter().position(|ch| *ch == 0).unwrap_or(len);
                paths.push(PathBuf::from(String::from_utf16_lossy(&buffer[..end])));
            }
            unsafe { ReleaseStgMedium(&mut medium) };
            *self.paths.borrow_mut() = paths;
            Ok(())
        }
    }

    impl IContextMenu_Impl for ContextMenuHandler_Impl {
        fn QueryContextMenu(
            &self,
            hmenu: HMENU,
            indexmenu: u32,
            idcmdfirst: u32,
            idcmdlast: u32,
            _uflags: u32,
        ) -> windows::core::HRESULT {
            let paths = self.paths.borrow().clone();
            let settings = Settings::load();
            let (labels, actions) = build_actions(&paths, &settings);
            if labels.is_empty() {
                self.actions.borrow_mut().clear();
                return windows::core::HRESULT(0);
            }
            let cascade = settings.integration.cascade;
            let menu = if cascade {
                match unsafe { CreatePopupMenu() } {
                    Ok(menu) => Some(menu),
                    Err(_) => return windows::core::HRESULT(E_FAIL.0),
                }
            } else {
                None
            };
            let mut added = 0usize;
            for (index, label) in labels.iter().enumerate() {
                let id = idcmdfirst as usize + index;
                if id > idcmdlast as usize {
                    break;
                }
                let text = HSTRING::from(label);
                let result = if let Some(submenu) = menu {
                    unsafe { AppendMenuW(submenu, MF_STRING, id, PCWSTR(text.as_ptr())) }
                } else {
                    unsafe {
                        InsertMenuW(
                            hmenu,
                            indexmenu + index as u32,
                            MF_STRING | MF_BYPOSITION,
                            id,
                            PCWSTR(text.as_ptr()),
                        )
                    }
                };
                if result.is_ok() {
                    added += 1;
                }
            }
            if added == 0 {
                if let Some(submenu) = menu {
                    unsafe {
                        let _ = DestroyMenu(submenu);
                    }
                }
                return windows::core::HRESULT(0);
            }
            if let Some(submenu) = menu {
                let title = HSTRING::from("RarkZip (&N)");
                let result = unsafe {
                    InsertMenuW(
                        hmenu,
                        indexmenu,
                        MF_POPUP | MF_BYPOSITION,
                        submenu.0 as usize,
                        PCWSTR(title.as_ptr()),
                    )
                };
                if result.is_err() {
                    unsafe {
                        let _ = DestroyMenu(submenu);
                    }
                    return windows::core::HRESULT(E_FAIL.0);
                }
            }
            if !settings.integration.hide_icon {
                if let Some(bitmap) = load_icon_bitmap() {
                    self.icon_bitmaps.borrow_mut().push(bitmap);
                    let info = MENUITEMINFOW {
                        cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                        fMask: MIIM_BITMAP,
                        hbmpItem: bitmap,
                        ..Default::default()
                    };
                    for index in 0..added {
                        let (target, position) = match menu {
                            Some(submenu) => (submenu, index as u32),
                            None => (hmenu, indexmenu + index as u32),
                        };
                        unsafe {
                            let _ = SetMenuItemInfoW(target, position, true, &info);
                        }
                    }
                    if menu.is_some() {
                        unsafe {
                            let _ = SetMenuItemInfoW(hmenu, indexmenu, true, &info);
                        }
                    }
                }
            }
            *self.actions.borrow_mut() = actions;
            windows::core::HRESULT((added + usize::from(cascade)) as i32)
        }

        fn InvokeCommand(
            &self,
            pici: *const windows::Win32::UI::Shell::CMINVOKECOMMANDINFO,
        ) -> windows::core::Result<()> {
            if pici.is_null() {
                return Ok(());
            }
            let info = unsafe { &*pici };
            let verb = info.lpVerb.0 as usize;
            if verb >> 16 != 0 {
                return Ok(());
            }
            let offset = verb & 0xffff;
            let action = self.actions.borrow().get(offset).copied();
            if let Some(action) = action {
                let paths = self.paths.borrow().clone();
                invoke(action, &paths);
            }
            Ok(())
        }

        fn GetCommandString(
            &self,
            _idcmd: usize,
            _utype: u32,
            _preserved: *const u32,
            _pszname: windows::core::PSTR,
            _cchmax: u32,
        ) -> windows::core::Result<()> {
            Ok(())
        }
    }

    #[implement(IClassFactory)]
    struct ContextMenuFactory;

    impl IClassFactory_Impl for ContextMenuFactory_Impl {
        fn CreateInstance(
            &self,
            _punkouter: windows::core::Ref<IUnknown>,
            riid: *const GUID,
            ppvobject: *mut *mut c_void,
        ) -> windows::core::Result<()> {
            if ppvobject.is_null() {
                return Err(Error::from(E_INVALIDARG));
            }
            let object: IUnknown = ComObject::new(ContextMenuHandler::new()).into_interface();
            unsafe { object.query(riid, ppvobject) }.ok()
        }

        fn LockServer(&self, _flock: windows::core::BOOL) -> windows::core::Result<()> {
            Ok(())
        }
    }

    pub fn factory() -> IClassFactory {
        ComObject::new(ContextMenuFactory).into_interface()
    }

    pub fn register(dll: &Path) -> anyhow::Result<()> {
        let classes = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags("Software\\Classes", KEY_ALL_ACCESS)?;
        let clsid = context_menu_clsid_string();
        let base = format!("CLSID\\{clsid}");
        let (key, _) = classes.create_subkey(&base)?;
        key.set_value("", &"RarkZip Multi-Selection Context Menu")?;
        let (inproc, _) = key.create_subkey("InprocServer32")?;
        inproc.set_value("", &dll.to_string_lossy().to_string())?;
        inproc.set_value("ThreadingModel", &"Apartment")?;
        for path in [
            "*\\shellex\\ContextMenuHandlers\\RarkZip",
            "Directory\\shellex\\ContextMenuHandlers\\RarkZip",
        ] {
            let (key, _) = classes.create_subkey(path)?;
            key.set_value("", &clsid)?;
        }
        Ok(())
    }

    pub fn unregister() -> anyhow::Result<()> {
        let classes = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags("Software\\Classes", KEY_ALL_ACCESS)?;
        for path in [
            "*\\shellex\\ContextMenuHandlers\\RarkZip",
            "Directory\\shellex\\ContextMenuHandlers\\RarkZip",
        ] {
            let _ = classes.delete_subkey_all(path);
        }
        let _ = classes.delete_subkey_all(format!("CLSID\\{}", context_menu_clsid_string()));
        Ok(())
    }

    pub fn installed() -> bool {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey("Software\\Classes\\*\\shellex\\ContextMenuHandlers\\RarkZip")
            .is_ok()
    }

    fn register_from_module() -> windows::core::HRESULT {
        let Some(path) = module_path() else {
            return windows::Win32::Foundation::E_FAIL;
        };
        match register(&path) {
            Ok(()) => windows::Win32::Foundation::S_OK,
            Err(_) => windows::Win32::Foundation::E_FAIL,
        }
    }
}

#[cfg(windows)]
pub use imp::{
    context_menu_clsid, context_menu_clsid_string, factory as context_menu_factory,
    installed as context_menu_installed, register as register_context_menu,
    unregister as unregister_context_menu,
};

#[cfg(all(windows, test))]
mod tests {
    use super::imp::{build_actions, is_archive, MenuAction};
    use rarkzip_engine::Settings;
    use std::path::PathBuf;

    fn settings() -> Settings {
        let mut settings = Settings::default();
        settings.integration.associations = vec!["zip".to_string(), "7z".to_string()];
        settings.integration.extract_verbs = [
            "open",
            "smart",
            "folder",
            "here",
            "other",
            "here_delete",
            "smart_delete",
            "folder_delete",
            "admin",
            "test",
            "preview",
        ]
        .into_iter()
        .map(ToString::to_string)
        .collect();
        settings.integration.hash_menu = true;
        settings.integration.compress_verbs = [
            "zip",
            "sevenz",
            "tar",
            "each_zip",
            "each_7z",
            "zip_delete",
            "gui",
            "admin",
        ]
        .into_iter()
        .map(ToString::to_string)
        .collect();
        settings
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rarkzip-cm-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn single_archive_menu_order() {
        let dir = temp_dir("single-archive");
        let archive = dir.join("a.zip");
        let _ = std::fs::write(&archive, b"a");
        let (labels, _) = build_actions(&[archive], &settings());
        assert_eq!(
            labels,
            vec![
                "解压到此处(&E)",
                "解压到此处并删除",
                "智能解压到此处(&S)",
                "智能解压到此处并删除",
                "解压到 a\\(&T)",
                "解压到 a\\ 并删除 (&T)",
                "解压到其他文件夹(&B)...",
                "以管理员身份解压",
                "测试压缩文件",
                "用 RarkZip 打开",
                "压缩文件预览...",
                "计算文件哈希…",
            ]
        );
    }

    #[test]
    fn multi_archive_menu_uses_independent_folders_and_adds_compression() {
        let dir = temp_dir("multi-archive");
        let a = dir.join("a.zip");
        let b = dir.join("b.7z");
        let _ = std::fs::write(&a, b"a");
        let _ = std::fs::write(&b, b"b");
        assert!(is_archive(&a, &settings()));
        let (labels, _) = build_actions(&[a, b], &settings());
        assert_eq!(labels[0], "解压到此处(&E)");
        assert!(labels
            .iter()
            .any(|label| label == "智能解压到独立文件夹(&S)"));
        assert!(labels.iter().any(|label| label == "解压到独立文件夹"));
        assert!(labels.iter().any(|label| label.contains("独立的")));
        assert!(labels.iter().any(|label| label.contains("压缩为“")));
        assert!(!labels.iter().any(|label| label.contains("(2)")));
        assert!(labels.iter().any(|label| label == "计算所选文件的哈希…"));
    }

    #[test]
    fn single_text_compression_uses_unique_dynamic_names() {
        let dir = temp_dir("single-text");
        let text = dir.join("a.txt");
        let _ = std::fs::write(&text, b"text");
        let _ = std::fs::write(dir.join("a.zip"), b"existing");
        let (labels, _) = build_actions(&[text], &settings());
        assert_eq!(labels[0], "压缩为“a (2).&zip”");
        assert_eq!(labels[1], "压缩为“a (2).zip”并删除");
        assert!(labels.iter().any(|label| label == "压缩为“a.tar”"));
        assert!(labels.iter().any(|label| label == "压缩为“a.7z”"));
        assert!(!labels.iter().any(|label| label.contains("独立的")));
        let new_index = labels
            .iter()
            .position(|label| label.starts_with("新建压缩包"))
            .unwrap();
        let admin_index = labels
            .iter()
            .position(|label| label.starts_with("以管理员身份压缩"))
            .unwrap();
        assert_eq!(new_index + 1, admin_index);
    }

    #[test]
    fn mixed_selection_builds_compression_menu_only() {
        let dir = temp_dir("mixed");
        let archive = dir.join("测试.zip");
        let pdf = dir.join("文档.pdf");
        let csv = dir.join("数据.csv");
        for path in [&archive, &pdf, &csv] {
            let _ = std::fs::write(path, b"x");
        }
        let (labels, actions) = build_actions(&[archive, pdf, csv], &settings());
        assert_eq!(actions.len(), labels.len());
        assert!(labels[0].contains("压缩为“"), "{labels:?}");
        assert!(labels.iter().any(|label| label.contains("独立的")));
        assert!(!labels.iter().any(|label| label.contains("解压到")));
        assert!(labels.iter().any(|label| label == "计算所选文件的哈希…"));
    }

    #[test]
    fn hash_action_handles_single_and_multiple_files() {
        let dir = temp_dir("hash-files");
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        let _ = std::fs::write(&a, b"a");
        let _ = std::fs::write(&b, b"b");
        let (single, actions) = build_actions(std::slice::from_ref(&a), &settings());
        assert_eq!(single.last().unwrap(), "计算文件哈希…");
        assert_eq!(actions.last().unwrap(), &MenuAction::Hash);
        let (multi, actions) = build_actions(&[a, b], &settings());
        assert_eq!(multi.last().unwrap(), "计算所选文件的哈希…");
        assert_eq!(actions.last().unwrap(), &MenuAction::Hash);
    }

    #[test]
    fn hash_action_is_hidden_for_directories_and_when_disabled() {
        let dir = temp_dir("hash-dirs");
        let mut settings = settings();
        let (labels, _) = build_actions(std::slice::from_ref(&dir), &settings);
        assert!(!labels.iter().any(|label| label.contains("哈希")));
        settings.integration.hash_menu = false;
        let file = dir.join("a.txt");
        let _ = std::fs::write(&file, b"a");
        let (labels, _) = build_actions(&[file], &settings);
        assert!(!labels.iter().any(|label| label.contains("哈希")));
    }
}
