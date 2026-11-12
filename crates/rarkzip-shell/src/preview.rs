//! Windows Shell 归档预览处理器（COM in-proc server）。
//!
//! 以只读多行 EDIT 控件展示归档摘要与前 5000 个条目；注册到常见归档扩展名的
//! `ShellEx\{8895b1c6-...}`。DLL 同时导出标准 DllGetClassObject 等入口。

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::path::{Path, PathBuf};

    use rarkzip_engine::{format_size, open_archive, OpenOptions};
    use windows::core::{
        implement, ComObject, Error, IUnknown, Interface, GUID, HRESULT, HSTRING, PCWSTR,
    };
    use windows::Win32::Foundation::{
        CLASS_E_CLASSNOTAVAILABLE, E_FAIL, E_INVALIDARG, E_NOTIMPL, HWND, RECT, S_FALSE, S_OK,
    };
    use windows::Win32::System::Com::{CoTaskMemAlloc, IClassFactory, IClassFactory_Impl};
    use windows::Win32::System::LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };
    use windows::Win32::System::Ole::{IObjectWithSite, IObjectWithSite_Impl};
    use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows::Win32::UI::Shell::PropertiesSystem::{
        IInitializeWithFile, IInitializeWithFile_Impl,
    };
    use windows::Win32::UI::Shell::{
        IExplorerCommand, IExplorerCommand_Impl, IPreviewHandler, IPreviewHandler_Impl,
        IShellItemArray, SIGDN_FILESYSPATH,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, MoveWindow, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY,
        WS_BORDER, WS_CHILD, WS_EX_CLIENTEDGE, WS_VISIBLE, WS_VSCROLL,
    };
    use winreg::enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS};
    use winreg::RegKey;

    const PREVIEW_HANDLER_IID: &str = "{8895b1c6-b41f-4c1c-a562-0d564250836f}";

    pub fn preview_clsid() -> GUID {
        GUID::from_u128(0x6f1b4a52_3c7d_4e21_9a5b_2f8c1d4e6a70)
    }

    pub fn preview_clsid_string() -> String {
        "{6F1B4A52-3C7D-4E21-9A5B-2F8C1D4E6A70}".to_string()
    }

    #[derive(Default)]
    struct PreviewState {
        path: Option<PathBuf>,
        parent: HWND,
        rect: RECT,
        edit: HWND,
    }

    #[implement(
        IPreviewHandler,
        IInitializeWithFile,
        IObjectWithSite,
        IExplorerCommand
    )]
    struct PreviewHandler {
        state: RefCell<PreviewState>,
    }

    impl PreviewHandler {
        fn new() -> Self {
            Self {
                state: RefCell::new(PreviewState::default()),
            }
        }

        fn ensure_edit(&self) -> windows::core::Result<HWND> {
            let mut state = self.state.borrow_mut();
            if !state.edit.is_invalid() {
                return Ok(state.edit);
            }
            if state.parent.is_invalid() {
                return Err(Error::from(E_FAIL));
            }
            let text = build_preview_text(state.path.as_deref());
            let edit = unsafe {
                CreateWindowExW(
                    WS_EX_CLIENTEDGE,
                    windows::core::w!("EDIT"),
                    &HSTRING::from(text),
                    WS_CHILD
                        | WS_VISIBLE
                        | WS_VSCROLL
                        | WS_BORDER
                        | windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
                            (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32,
                        ),
                    0,
                    0,
                    (state.rect.right - state.rect.left).max(1),
                    (state.rect.bottom - state.rect.top).max(1),
                    Some(state.parent),
                    None,
                    None,
                    None,
                )?
            };
            state.edit = edit;
            Ok(edit)
        }
    }

    impl IInitializeWithFile_Impl for PreviewHandler_Impl {
        fn Initialize(&self, pszfilepath: &PCWSTR, _grfmode: u32) -> windows::core::Result<()> {
            let path = unsafe { pszfilepath.to_string() }.map_err(|_| Error::from(E_INVALIDARG))?;
            if path.is_empty() {
                return Err(Error::from(E_INVALIDARG));
            }
            self.state.borrow_mut().path = Some(PathBuf::from(path));
            Ok(())
        }
    }

    impl IPreviewHandler_Impl for PreviewHandler_Impl {
        fn SetWindow(&self, hwnd: HWND, prc: *const RECT) -> windows::core::Result<()> {
            let mut state = self.state.borrow_mut();
            state.parent = hwnd;
            if !prc.is_null() {
                state.rect = unsafe { *prc };
            }
            Ok(())
        }

        fn SetRect(&self, prc: *const RECT) -> windows::core::Result<()> {
            if prc.is_null() {
                return Ok(());
            }
            let rect = unsafe { *prc };
            let edit = {
                let mut state = self.state.borrow_mut();
                state.rect = rect;
                state.edit
            };
            if !edit.is_invalid() {
                unsafe {
                    MoveWindow(
                        edit,
                        0,
                        0,
                        (rect.right - rect.left).max(1),
                        (rect.bottom - rect.top).max(1),
                        true,
                    )?;
                }
            }
            Ok(())
        }

        fn DoPreview(&self) -> windows::core::Result<()> {
            self.ensure_edit()?;
            Ok(())
        }

        fn Unload(&self) -> windows::core::Result<()> {
            let edit = {
                let mut state = self.state.borrow_mut();
                let edit = state.edit;
                state.edit = HWND::default();
                edit
            };
            if !edit.is_invalid() {
                unsafe {
                    let _ = DestroyWindow(edit);
                }
            }
            Ok(())
        }

        fn SetFocus(&self) -> windows::core::Result<()> {
            let edit = self.ensure_edit()?;
            unsafe {
                let _ = SetFocus(Some(edit));
            }
            Ok(())
        }

        fn QueryFocus(&self) -> windows::core::Result<HWND> {
            Ok(self.state.borrow().edit)
        }

        fn TranslateAccelerator(
            &self,
            _pmsg: *const windows::Win32::UI::WindowsAndMessaging::MSG,
        ) -> windows::core::Result<()> {
            Ok(())
        }
    }

    impl IObjectWithSite_Impl for PreviewHandler_Impl {
        fn SetSite(&self, _punksite: windows::core::Ref<IUnknown>) -> windows::core::Result<()> {
            Ok(())
        }

        fn GetSite(
            &self,
            _riid: *const GUID,
            ppvsite: *mut *mut c_void,
        ) -> windows::core::Result<()> {
            if !ppvsite.is_null() {
                unsafe { *ppvsite = std::ptr::null_mut() };
            }
            Ok(())
        }
    }

    impl IExplorerCommand_Impl for PreviewHandler_Impl {
        fn GetTitle(
            &self,
            _psiitemarray: windows::core::Ref<IShellItemArray>,
        ) -> windows::core::Result<windows::core::PWSTR> {
            Ok(alloc_pwstr("RarkZip"))
        }

        fn GetIcon(
            &self,
            _psiitemarray: windows::core::Ref<IShellItemArray>,
        ) -> windows::core::Result<windows::core::PWSTR> {
            Ok(alloc_pwstr(
                &module_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            ))
        }

        fn GetToolTip(
            &self,
            _psiitemarray: windows::core::Ref<IShellItemArray>,
        ) -> windows::core::Result<windows::core::PWSTR> {
            Ok(alloc_pwstr("用 RarkZip 打开/解压"))
        }

        fn GetCanonicalName(&self) -> windows::core::Result<GUID> {
            Ok(preview_clsid())
        }

        fn GetState(
            &self,
            _psiitemarray: windows::core::Ref<IShellItemArray>,
            _foktobeslow: windows::core::BOOL,
        ) -> windows::core::Result<u32> {
            Ok(0)
        }

        fn Invoke(
            &self,
            psiitemarray: windows::core::Ref<IShellItemArray>,
            _pbc: windows::core::Ref<windows::Win32::System::Com::IBindCtx>,
        ) -> windows::core::Result<()> {
            let Some(array) = psiitemarray.as_ref() else {
                return Ok(());
            };
            let item = unsafe { array.GetItemAt(0) }?;
            let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }?;
            let path = unsafe { name.to_string() }.unwrap_or_default();
            unsafe {
                windows::Win32::System::Com::CoTaskMemFree(Some(name.0 as *const c_void));
            }
            if path.is_empty() {
                return Ok(());
            }
            let Some(dir) = module_path().and_then(|p| p.parent().map(|p| p.to_path_buf())) else {
                return Ok(());
            };
            let exe = dir.join("rarkzip.exe");
            let _ = std::process::Command::new(exe)
                .arg("--gui")
                .arg(path)
                .spawn();
            Ok(())
        }

        fn GetFlags(&self) -> windows::core::Result<u32> {
            Ok(0)
        }

        fn EnumSubCommands(
            &self,
        ) -> windows::core::Result<windows::Win32::UI::Shell::IEnumExplorerCommand> {
            Err(Error::from(E_NOTIMPL))
        }
    }

    fn alloc_pwstr(value: &str) -> windows::core::PWSTR {
        let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = wide.len() * std::mem::size_of::<u16>();
        unsafe {
            let memory = CoTaskMemAlloc(bytes) as *mut u16;
            std::ptr::copy_nonoverlapping(wide.as_ptr(), memory, wide.len());
            windows::core::PWSTR(memory)
        }
    }

    #[implement(IClassFactory)]
    struct PreviewFactory;

    impl IClassFactory_Impl for PreviewFactory_Impl {
        fn CreateInstance(
            &self,
            _punkouter: windows::core::Ref<IUnknown>,
            riid: *const GUID,
            ppvobject: *mut *mut c_void,
        ) -> windows::core::Result<()> {
            if ppvobject.is_null() {
                return Err(Error::from(E_INVALIDARG));
            }
            let object: IUnknown = ComObject::new(PreviewHandler::new()).into_interface();
            unsafe { object.query(riid, ppvobject) }.ok()
        }

        fn LockServer(&self, _flock: windows::core::BOOL) -> windows::core::Result<()> {
            Ok(())
        }
    }

    fn build_preview_text(path: Option<&Path>) -> String {
        let Some(path) = path else {
            return "RarkZip 预览\n\n未初始化文件".to_string();
        };
        match open_archive(path, &OpenOptions::default()) {
            Ok(info) => {
                let mut text = format!(
                    "RarkZip 归档预览\n\n文件：{}\n格式：{} · {} 项 · 原始 {}\n{}\n\n",
                    path.display(),
                    info.format.label(),
                    info.file_count(),
                    format_size(info.total_size()),
                    if info.encrypted {
                        "加密：是"
                    } else {
                        "加密：否"
                    }
                );
                for entry in info.entries.iter().take(5000) {
                    text.push_str(if entry.is_dir { "[目录] " } else { "       " });
                    text.push_str(&entry.path);
                    if !entry.is_dir {
                        text.push_str("  ");
                        text.push_str(&format_size(entry.size));
                    }
                    text.push('\n');
                }
                if info.entries.len() > 5000 {
                    text.push_str("\n… 条目过多，仅显示前 5000 项\n");
                }
                text
            }
            Err(error) if error.is_password_related() => {
                format!(
                    "RarkZip 归档预览\n\n文件：{}\n\n需要密码，无法预览内容。",
                    path.display()
                )
            }
            Err(error) => format!(
                "RarkZip 归档预览\n\n文件：{}\n\n无法读取：{error}",
                path.display()
            ),
        }
    }

    fn module_path() -> Option<PathBuf> {
        let mut module = windows::Win32::Foundation::HMODULE::default();
        unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                windows::core::PCWSTR(register_from_module as *const u16),
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

    pub fn register_preview_handler(dll: &Path) -> anyhow::Result<()> {
        let classes = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags("Software\\Classes", KEY_ALL_ACCESS)?;
        let clsid = preview_clsid_string();
        let base = format!("CLSID\\{clsid}");
        let (key, _) = classes.create_subkey(&base)?;
        key.set_value("", &"RarkZip Archive Preview")?;
        let (inproc, _) = key.create_subkey("InprocServer32")?;
        inproc.set_value("", &dll.to_string_lossy().to_string())?;
        inproc.set_value("ThreadingModel", &"Apartment")?;
        for ext in [".zip", ".7z", ".rar", ".tar", ".gz", ".tgz", ".xz", ".zst"] {
            let path = format!("{ext}\\ShellEx\\{PREVIEW_HANDLER_IID}");
            let (key, _) = classes.create_subkey(&path)?;
            key.set_value("", &clsid)?;
        }
        Ok(())
    }

    pub fn unregister_preview_handler() -> anyhow::Result<()> {
        let classes = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags("Software\\Classes", KEY_ALL_ACCESS)?;
        let clsid = preview_clsid_string();
        for ext in [".zip", ".7z", ".rar", ".tar", ".gz", ".tgz", ".xz", ".zst"] {
            let path = format!("{ext}\\ShellEx\\{PREVIEW_HANDLER_IID}");
            let _ = classes.delete_subkey_all(&path);
        }
        let _ = classes.delete_subkey_all(format!("CLSID\\{clsid}"));
        Ok(())
    }

    pub fn preview_handler_installed() -> bool {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey("Software\\Classes\\.zip\\ShellEx\\{8895b1c6-b41f-4c1c-a562-0d564250836f}")
            .is_ok()
    }

    fn register_from_module() -> HRESULT {
        let Some(path) = module_path() else {
            return E_FAIL;
        };
        match register_preview_handler(&path) {
            Ok(()) => S_OK,
            Err(_) => E_FAIL,
        }
    }

    fn unregister_from_module() -> HRESULT {
        match unregister_preview_handler() {
            Ok(()) => S_OK,
            Err(_) => E_FAIL,
        }
    }

    #[no_mangle]
    pub extern "system" fn DllGetClassObject(
        rclsid: *const GUID,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> HRESULT {
        if rclsid.is_null() || riid.is_null() || ppv.is_null() {
            return E_INVALIDARG;
        }
        if unsafe { *rclsid } == crate::context_menu::context_menu_clsid() {
            let factory = crate::context_menu::context_menu_factory();
            return unsafe { factory.query(riid, ppv) };
        }
        if unsafe { *rclsid } != preview_clsid() {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = ComObject::new(PreviewFactory).into_interface();
        unsafe { factory.query(riid, ppv) }
    }

    #[no_mangle]
    pub extern "system" fn DllCanUnloadNow() -> HRESULT {
        S_FALSE
    }

    #[no_mangle]
    pub extern "system" fn DllRegisterServer() -> HRESULT {
        register_from_module()
    }

    #[no_mangle]
    pub extern "system" fn DllUnregisterServer() -> HRESULT {
        unregister_from_module()
    }
}

#[cfg(windows)]
pub use imp::{
    preview_clsid, preview_clsid_string, preview_handler_installed, register_preview_handler,
    unregister_preview_handler,
};

#[cfg(all(windows, test))]
mod tests {
    use super::*;
    use windows::core::Interface;
    use windows::Win32::System::Com::IClassFactory;

    #[test]
    fn dll_get_class_object_creates_factory() {
        let mut ppv = std::ptr::null_mut();
        let hr = super::imp::DllGetClassObject(&preview_clsid(), &IClassFactory::IID, &mut ppv);
        assert!(hr.is_ok(), "DllGetClassObject failed: {hr:?}");
        assert!(!ppv.is_null());
        unsafe {
            let factory = IClassFactory::from_raw(ppv);
            let handler: windows::Win32::UI::Shell::IPreviewHandler =
                factory.CreateInstance(None).unwrap();
            let command: windows::Win32::UI::Shell::IExplorerCommand = handler.cast().unwrap();
            drop(command);
            drop(handler);
            drop(factory);
        }
    }

    #[test]
    fn dll_get_class_object_creates_context_menu() {
        let mut ppv = std::ptr::null_mut();
        let hr = super::imp::DllGetClassObject(
            &crate::context_menu::context_menu_clsid(),
            &IClassFactory::IID,
            &mut ppv,
        );
        assert!(hr.is_ok(), "DllGetClassObject failed: {hr:?}");
        assert!(!ppv.is_null());
        unsafe {
            let factory = IClassFactory::from_raw(ppv);
            let menu: windows::Win32::UI::Shell::IContextMenu =
                factory.CreateInstance(None).unwrap();
            let hmenu = windows::Win32::UI::WindowsAndMessaging::CreatePopupMenu().unwrap();
            // 未初始化选择时不会插入菜单。
            let result = menu.QueryContextMenu(hmenu, 0, 1, 100, 0);
            assert_eq!(result.0 & 0xffff, 0);
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyMenu(hmenu);
            drop(menu);
            drop(factory);
        }
    }
}
