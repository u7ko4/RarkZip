use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use rarkzip_engine::{
    create_archive, extract_archive, verify_archive, ArchiveFormat, CreateOptions, ExtractMode,
    ExtractOptions, HashAlgorithm, PasswordBook, ZipAlgorithm,
};

use super::*;
use crate::design;

impl MainView {
    // ------------------------------------------------------------------
    // Keyboard
    // ------------------------------------------------------------------
    pub(super) fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.to_lowercase();
        let control = event.keystroke.modifiers.control;
        if self.feature_window.is_some() {
            if key == "escape" {
                if self.hash_context_menu.is_some() {
                    self.hash_context_menu = None;
                    cx.notify();
                } else if self.dropdown.is_some() {
                    self.dropdown = None;
                    cx.notify();
                } else if self.modal.is_some() {
                    self.close_modal(cx);
                } else {
                    if let Some(feature) = self.feature_window {
                        self.save_feature_window_state(window, feature);
                    }
                    window.remove_window();
                }
            }
            return;
        }
        eprintln!("RarkZip key: {key:?} ctrl={control} page={:?}", self.page);
        let search_focused = self.search.read(cx).focus_handle(cx).is_focused(window);

        if key == "escape" {
            if self.archive_menu.is_some() {
                self.archive_menu = None;
                cx.notify();
                return;
            }
            if self.dropdown.is_some() {
                self.dropdown = None;
                cx.notify();
                return;
            }
            if self.menu_open {
                self.close_app_menu();
                cx.notify();
                return;
            }
            if self.modal.is_some() {
                self.close_modal(cx);
                window.focus(&self.focus_handle, cx);
                return;
            }
            if self.page != Page::Main {
                self.page = Page::Main;
                cx.notify();
                return;
            }
            if self.settings.general.esc_closes_window {
                self.save_window_state(window);
                window.remove_window();
            }
            return;
        }

        if self.modal.is_some() {
            return;
        }

        if control {
            if event.keystroke.modifiers.shift && key == "n" {
                self.open_new_window(window, cx);
                return;
            }
            if event.keystroke.modifiers.shift && key == "w" {
                self.save_window_state(window);
                window.remove_window();
                return;
            }
            match key.as_str() {
                "a" => self.select_all_visible(cx),
                "c" => self.copy_selected_entries(cx),
                "v" => self.paste_clipboard_entries(cx),
                "w" => self.close_current_archive(cx),
                "o" => self.open_file_dialog(window, cx),
                "n" => self.open_new_archive_modal(window, cx),
                "e" => self.start_extract_smart(window, cx),
                "f" => self.search.read(cx).focus_handle(cx).focus(window, cx),
                "t" => self.start_verify(window, cx),
                "h" => self.open_hash_modal(cx),
                "i" => self.scan_current_archive(cx),
                _ => {}
            }
            return;
        }

        if search_focused {
            return;
        }

        match key.as_str() {
            "f5" => {
                self.page = Page::Settings;
                cx.notify();
            }
            "enter" => {
                if let Some(index) = self.selected {
                    self.open_entry(index, window, cx);
                }
            }
            "f2" => self.open_rename_modal(window, cx),
            "delete" => self.delete_selected_entry(cx),
            "insert" => self.add_files_to_archive(cx),
            "backspace" => self.go_up(cx),
            _ => {}
        }
    }

    // ------------------------------------------------------------------
    // New archive draft
    // ------------------------------------------------------------------

    pub(super) fn open_password_manager(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.password_book = PasswordBook::load_default();
        self.password_new
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.password_note
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.password_reveal = false;
        self.page = Page::PasswordManager;
        self.modal = None;
        cx.notify();
    }

    pub(super) fn open_password_recovery(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.feature_window == Some(FeatureWindow::PasswordRecovery) {
            self.init_password_recovery(cx);
            return;
        }
        let source = self.archive.as_ref().map(|archive| archive.path.clone());
        self.open_feature_window(
            FeatureWindow::PasswordRecovery,
            source.into_iter().collect(),
            None,
            cx,
        );
    }

    pub(super) fn init_password_recovery(&mut self, cx: &mut Context<Self>) {
        if self.recovery_running {
            self.recovery_control.cancel();
        }
        self.recovery_running = false;
        self.recovery_paused = false;
        self.recovery_result = None;
        self.recovery_error = None;
        self.recovery_attempts = 0;
        self.recovery_total = 0;
        self.recovery_path = self
            .archive
            .as_ref()
            .map(|archive| archive.path.clone())
            .or_else(|| self.settings.ui.recent_files.first().map(PathBuf::from));
        self.page = Page::PasswordRecovery;
        self.modal = None;
        cx.notify();
    }

    pub(super) fn start_password_recovery(&mut self, cx: &mut Context<Self>) {
        if self.recovery_running {
            return;
        }
        let Some(path) = self.recovery_path.clone() else {
            self.show_toast("请先选择要恢复的压缩包", ToastKind::Error, cx);
            return;
        };
        let mode = match self.recovery_mode {
            RecoveryUiMode::Dictionary => {
                if self.recovery_dictionary.is_empty() {
                    self.show_toast("请先导入字典文件", ToastKind::Error, cx);
                    return;
                }
                RecoveryMode::Dictionary(self.recovery_dictionary.clone())
            }
            RecoveryUiMode::Mask => {
                RecoveryMode::Mask(self.recovery_mask.read(cx).value().to_string())
            }
            RecoveryUiMode::Brute => RecoveryMode::Brute {
                charset: self.recovery_charset.read(cx).value().to_string(),
                min_len: self.recovery_min as usize,
                max_len: self.recovery_max as usize,
            },
        };
        self.recovery_run_id += 1;
        let run_id = self.recovery_run_id;
        let control = RecoveryControl::new();
        self.recovery_control = control.clone();
        self.recovery_running = true;
        self.recovery_paused = false;
        self.recovery_attempts = 0;
        self.recovery_total = 0;
        self.recovery_result = None;
        self.recovery_error = None;
        let sender = self.recovery_sender.clone();
        let options = RecoveryOptions {
            mode,
            threads: self.recovery_threads,
        };
        std::thread::spawn(move || {
            let progress_sender = sender.clone();
            let result = rarkzip_engine::recover_password(
                &path,
                &options,
                &control,
                move |attempts, total| {
                    let _ = progress_sender.send(RecoveryEvent::Progress {
                        run_id,
                        attempts,
                        total,
                    });
                },
            )
            .map_err(|error| error.to_string());
            let _ = sender.send(RecoveryEvent::Done { run_id, result });
        });
        cx.notify();
    }

    pub(super) fn handle_recovery_event(&mut self, event: RecoveryEvent, cx: &mut Context<Self>) {
        match event {
            RecoveryEvent::Progress {
                run_id,
                attempts,
                total,
            } => {
                if run_id == self.recovery_run_id && self.recovery_running {
                    self.recovery_attempts = attempts;
                    self.recovery_total = total;
                }
            }
            RecoveryEvent::Done { run_id, result } => {
                if run_id != self.recovery_run_id {
                    return;
                }
                self.recovery_running = false;
                self.recovery_paused = false;
                match result {
                    Ok(outcome) => {
                        self.recovery_attempts = outcome.attempts;
                        self.recovery_total = outcome.total;
                        if let Some(password) = outcome.password {
                            let mut book = PasswordBook::load_default();
                            let _ = book.remember(&password);
                            self.password_book = PasswordBook::load_default();
                            self.recovery_result = Some(password.clone());
                            self.recovery_error = None;
                            self.show_toast(
                                format!("已找到密码并记入密码本：{password}"),
                                ToastKind::Success,
                                cx,
                            );
                        } else if outcome.cancelled {
                            self.recovery_error = Some("已取消".to_string());
                        } else {
                            self.recovery_error =
                                Some(format!("未找到匹配密码（已尝试 {} 次）", outcome.attempts));
                        }
                    }
                    Err(error) => {
                        self.recovery_error = Some(error);
                    }
                }
            }
        }
        cx.notify();
    }

    pub(super) fn scan_current_archive(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.archive.as_ref().map(|archive| archive.path.clone()) else {
            self.show_toast("请先打开压缩包", ToastKind::Info, cx);
            return;
        };
        let buffer = self.settings.security.scan_buffer_kb as u64 * 1024;
        match rarkzip_engine::scan_file(&path, buffer) {
            Ok(rarkzip_engine::AmsiVerdict::Detected) => self.show_toast(
                "AMSI 检测到恶意内容，请勿解压或运行该文件",
                ToastKind::Error,
                cx,
            ),
            Ok(rarkzip_engine::AmsiVerdict::Clean) => self.show_toast(
                format!(
                    "AMSI 扫描完成：未检测到威胁（缓冲区 {} KB）",
                    self.settings.security.scan_buffer_kb
                ),
                ToastKind::Success,
                cx,
            ),
            Ok(rarkzip_engine::AmsiVerdict::Unavailable(error)) => {
                self.show_toast(format!("AMSI 不可用：{error}"), ToastKind::Info, cx)
            }
            Err(error) => self.show_toast(format!("扫描失败：{error}"), ToastKind::Error, cx),
        }
    }

    pub(super) fn repair_zip_dialog(&mut self, cx: &mut Context<Self>) {
        let source = self
            .archive
            .as_ref()
            .filter(|archive| archive.info.format.is_zip())
            .map(|archive| archive.path.clone())
            .or_else(|| {
                crate::components::file_dialog(|| {
                    rfd::FileDialog::new()
                        .add_filter("ZIP 归档", &["zip", "zipx"])
                        .pick_file()
                })
            });
        let Some(source) = source else {
            self.show_toast("请选择要修复的 ZIP 文件", ToastKind::Info, cx);
            return;
        };
        let stem = source
            .file_stem()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| "archive".to_string());
        let suggested = source.with_file_name(format!("{stem}.repaired.zip"));
        let suggested_name = suggested
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| "repaired.zip".to_string());
        let Some(dest) = crate::components::file_dialog(move || {
            rfd::FileDialog::new()
                .add_filter("ZIP 归档", &["zip"])
                .set_file_name(suggested_name)
                .save_file()
        }) else {
            return;
        };
        match rarkzip_engine::repair_zip(&source, &dest, &rarkzip_engine::NoopSink) {
            Ok(report) => self.show_toast(
                format!(
                    "修复完成：恢复 {}/{} 项 · 跳过 {} 项 → {}",
                    report.recovered,
                    report.entries,
                    report.skipped,
                    dest.display()
                ),
                ToastKind::Success,
                cx,
            ),
            Err(error) => self.show_toast(format!("修复失败：{error}"), ToastKind::Error, cx),
        }
    }

    pub(super) fn add_files_to_archive(&mut self, cx: &mut Context<Self>) {
        let Some(archive) = &self.archive else {
            return;
        };
        if !archive.info.format.is_zip() {
            self.show_toast("当前仅支持向 ZIP 归档添加文件", ToastKind::Info, cx);
            return;
        }
        let path = archive.path.clone();
        let password = archive.password.clone();
        let Some(files) = crate::components::file_dialog(|| rfd::FileDialog::new().pick_files())
        else {
            return;
        };
        if files.is_empty() {
            return;
        }
        match rarkzip_engine::add_zip_entries(&path, &files, password.as_deref()) {
            Ok(report) => {
                self.show_toast(
                    format!("已添加 {} 个条目", report.added),
                    ToastKind::Success,
                    cx,
                );
                self.reload_archive(cx);
            }
            Err(error) => self.show_toast(format!("添加失败：{error}"), ToastKind::Error, cx),
        }
    }

    pub(super) fn delete_selected_entry(&mut self, cx: &mut Context<Self>) {
        let Some(archive) = &self.archive else {
            return;
        };
        if !archive.info.format.is_zip() {
            self.show_toast("当前仅支持删除 ZIP 条目", ToastKind::Info, cx);
            return;
        }
        let selected = self.selected_indices();
        if selected.is_empty() {
            self.show_toast("请先选择一个条目", ToastKind::Info, cx);
            return;
        }
        let mut delete = std::collections::HashSet::new();
        for index in &selected {
            let Some(entry) = archive.info.find(*index) else {
                continue;
            };
            delete.insert(*index);
            if entry.is_dir {
                let prefix = format!("{}/", entry.path.trim_end_matches('/'));
                for candidate in &archive.info.entries {
                    if candidate.path.starts_with(&prefix) {
                        delete.insert(candidate.index);
                    }
                }
            }
        }
        let delete: Vec<usize> = delete.into_iter().collect();
        let description = if selected.len() == 1 {
            let name = archive
                .info
                .find(selected[0])
                .map(|entry| entry.path.clone())
                .unwrap_or_default();
            format!("确定从压缩包删除「{name}」吗？")
        } else {
            format!("确定从压缩包删除选中的 {} 个条目吗？", selected.len())
        };
        let result = crate::components::file_dialog(move || {
            rfd::MessageDialog::new()
                .set_title("删除条目")
                .set_description(description)
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
        });
        if result != rfd::MessageDialogResult::Yes {
            return;
        }
        let path = archive.path.clone();
        let password = archive.password.clone();
        match rarkzip_engine::edit_zip(
            &path,
            &path,
            password.as_deref(),
            &rarkzip_engine::ZipEditOptions {
                delete,
                ..Default::default()
            },
        ) {
            Ok(report) => {
                self.show_toast(
                    format!("已删除条目，重写 {} 项", report.written),
                    ToastKind::Success,
                    cx,
                );
                self.selected = None;
                self.multi_selected.clear();
                self.archive_menu = None;
                self.reload_archive(cx);
            }
            Err(error) => self.show_toast(format!("删除失败：{error}"), ToastKind::Error, cx),
        }
    }

    pub(super) fn reload_archive(&mut self, cx: &mut Context<Self>) {
        let Some(archive) = &self.archive else {
            return;
        };
        let path = archive.path.clone();
        let password = archive.password.clone();
        let current_dir = self.current_dir.clone();
        match rarkzip_engine::open_archive(
            &path,
            &rarkzip_engine::OpenOptions {
                password: password.clone(),
                codepage: Codepage::from_name(&self.settings.extraction.default_codepage),
            },
        ) {
            Ok(info) => {
                self.install_archive(path, info, password, cx);
                if !current_dir.is_empty()
                    && self.archive.as_ref().is_some_and(|archive| {
                        archive
                            .info
                            .entries
                            .iter()
                            .any(|entry| entry.is_dir && entry.path == current_dir)
                    })
                {
                    self.current_dir = current_dir;
                    self.rebuild_rows();
                }
            }
            Err(error) => {
                self.show_toast(format!("重新读取归档失败：{error}"), ToastKind::Error, cx)
            }
        }
    }

    pub(super) fn open_rename_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(archive) = &self.archive else {
            return;
        };
        if !archive.info.format.is_zip() {
            self.show_toast("当前仅支持重命名 ZIP 条目", ToastKind::Info, cx);
            return;
        }
        let Some(selected) = self.selected else {
            self.show_toast("请先选择一个条目", ToastKind::Info, cx);
            return;
        };
        let Some(entry) = archive.info.find(selected) else {
            return;
        };
        let current = entry.path.clone();
        self.rename_input.update(cx, |state, cx| {
            state.set_value(current, window, cx);
        });
        self.modal = Some(Modal::Rename { index: selected });
        cx.notify();
    }

    pub(super) fn open_comment_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(archive) = &self.archive else {
            return;
        };
        if !archive.info.format.is_zip() {
            self.show_toast("当前仅支持编辑 ZIP 注释", ToastKind::Info, cx);
            return;
        }
        let comment = archive.info.comment.clone().unwrap_or_default();
        self.comment_input.update(cx, |state, cx| {
            state.set_value(comment, window, cx);
        });
        self.modal = Some(Modal::Comment);
        cx.notify();
    }

    pub(super) fn open_new_folder_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(archive) = &self.archive else {
            return;
        };
        if !archive.info.format.is_zip() {
            self.show_toast("当前仅支持在 ZIP 中新建文件夹", ToastKind::Info, cx);
            return;
        }
        self.rename_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        self.modal = Some(Modal::NewFolder {
            parent: self.current_dir.clone(),
        });
        cx.notify();
    }

    pub(super) fn submit_new_folder(&mut self, cx: &mut Context<Self>) {
        let Some(Modal::NewFolder { parent }) = self.modal.clone() else {
            return;
        };
        let Some(archive) = &self.archive else {
            return;
        };
        let name = self.rename_input.read(cx).value().to_string();
        if name.trim().is_empty() {
            self.show_toast("文件夹名称不能为空", ToastKind::Error, cx);
            return;
        }
        let path = if parent.is_empty() {
            name.trim().to_string()
        } else {
            format!("{parent}/{}", name.trim())
        };
        let archive_path = archive.path.clone();
        let password = archive.password.clone();
        match rarkzip_engine::add_zip_directory(&archive_path, &path, password.as_deref()) {
            Ok(_) => {
                self.modal = None;
                self.selected = None;
                self.show_toast("文件夹已创建", ToastKind::Success, cx);
                self.reload_archive(cx);
            }
            Err(error) => self.show_toast(format!("新建文件夹失败：{error}"), ToastKind::Error, cx),
        }
    }

    pub(super) fn copy_selected_entries(&mut self, cx: &mut Context<Self>) {
        let Some(archive) = &self.archive else {
            return;
        };
        let selected = self.selected_indices();
        if selected.is_empty() {
            self.show_toast("请先选择要复制的条目", ToastKind::Info, cx);
            return;
        }
        if archive.info.encrypted && archive.password.is_none() {
            self.show_toast(
                "当前归档需要密码，请先输入密码后再复制",
                ToastKind::Info,
                cx,
            );
            return;
        }
        let mut names: Vec<String> = selected
            .iter()
            .filter_map(|index| archive.info.find(*index))
            .map(|entry| entry.path.clone())
            .collect();
        names.sort_by_key(String::len);
        let mut roots: Vec<String> = Vec::new();
        for name in names {
            if !roots
                .iter()
                .any(|root| name.starts_with(&format!("{root}/")))
            {
                roots.push(name);
            }
        }
        if roots.is_empty() {
            return;
        }

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir()
            .join("RarkZip")
            .join(format!("clipboard-{}-{stamp}", std::process::id()));
        if let Err(error) = std::fs::create_dir_all(&dir) {
            self.show_toast(format!("创建临时目录失败：{error}"), ToastKind::Error, cx);
            return;
        }
        let options = ExtractOptions {
            mode: ExtractMode::Custom(dir.clone()),
            password: archive.password.clone(),
            passwords: Vec::new(),
            codepage: Codepage::from_name(&self.settings.extraction.default_codepage),
            selected: Some(selected),
            overwrite: true,
            strip_root: false,
            propagate_motw: false,
            nested_depth: 0,
            delete_archive: false,
            amsi_scan: false,
            amsi_buffer_kb: 0,
        };
        let archive_path = archive.path.clone();
        match extract_archive(&archive_path, &options, &rarkzip_engine::NoopSink) {
            Ok(_) => {}
            Err(error) => {
                self.show_toast(format!("复制前解压失败：{error}"), ToastKind::Error, cx);
                return;
            }
        }
        let files: Vec<PathBuf> = roots
            .iter()
            .map(|name| dir.join(name.replace('/', std::path::MAIN_SEPARATOR_STR)))
            .filter(|path| path.exists())
            .collect();
        if files.is_empty() {
            self.show_toast("选中条目没有可复制的文件", ToastKind::Info, cx);
            return;
        }
        match crate::clipboard::set_file_list(&files) {
            Ok(()) => self.show_toast(
                format!("已复制 {} 个条目到剪贴板", files.len()),
                ToastKind::Success,
                cx,
            ),
            Err(error) => self.show_toast(format!("复制失败：{error}"), ToastKind::Error, cx),
        }
    }

    pub(super) fn paste_clipboard_entries(&mut self, cx: &mut Context<Self>) {
        let Some(archive) = &self.archive else {
            return;
        };
        if !archive.info.format.is_zip() {
            self.show_toast("当前仅支持向 ZIP 粘贴条目", ToastKind::Info, cx);
            return;
        }
        let files = match crate::clipboard::file_list() {
            Ok(files) => files,
            Err(error) => {
                self.show_toast(format!("读取剪贴板失败：{error}"), ToastKind::Error, cx);
                return;
            }
        };
        if files.is_empty() {
            self.show_toast("剪贴板中没有文件", ToastKind::Info, cx);
            return;
        }
        let path = archive.path.clone();
        let password = archive.password.clone();
        match rarkzip_engine::add_zip_entries(&path, &files, password.as_deref()) {
            Ok(report) => {
                self.show_toast(
                    format!("已粘贴 {} 个条目", report.added),
                    ToastKind::Success,
                    cx,
                );
                self.reload_archive(cx);
            }
            Err(error) => self.show_toast(format!("粘贴失败：{error}"), ToastKind::Error, cx),
        }
    }

    pub(super) fn submit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(Modal::Rename { index }) = self.modal.clone() else {
            return;
        };
        let Some(archive) = &self.archive else {
            return;
        };
        let new_name = self.rename_input.read(cx).value().to_string();
        if new_name.trim().is_empty() {
            self.show_toast("条目路径不能为空", ToastKind::Error, cx);
            return;
        }
        let path = archive.path.clone();
        let password = archive.password.clone();
        let mut rename = std::collections::HashMap::new();
        rename.insert(index, new_name.clone());
        match rarkzip_engine::edit_zip(
            &path,
            &path,
            password.as_deref(),
            &rarkzip_engine::ZipEditOptions {
                rename,
                ..Default::default()
            },
        ) {
            Ok(_) => {
                self.modal = None;
                self.selected = None;
                self.show_toast("条目已重命名", ToastKind::Success, cx);
                self.reload_archive(cx);
            }
            Err(error) => self.show_toast(format!("重命名失败：{error}"), ToastKind::Error, cx),
        }
    }

    pub(super) fn submit_comment(&mut self, cx: &mut Context<Self>) {
        let Some(archive) = &self.archive else {
            return;
        };
        let comment = self.comment_input.read(cx).value().to_string();
        let path = archive.path.clone();
        let password = archive.password.clone();
        match rarkzip_engine::edit_zip(
            &path,
            &path,
            password.as_deref(),
            &rarkzip_engine::ZipEditOptions {
                comment: Some(comment),
                ..Default::default()
            },
        ) {
            Ok(_) => {
                self.modal = None;
                self.show_toast("注释已保存", ToastKind::Success, cx);
                self.reload_archive(cx);
            }
            Err(error) => self.show_toast(format!("保存注释失败：{error}"), ToastKind::Error, cx),
        }
    }

    pub(super) fn handle_external_drop(
        &mut self,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        // 独立功能窗口的拖放目标不同：新建压缩包加入源文件，哈希加入待计算列表，
        // 密码恢复设置目标压缩包。
        match self.feature_window {
            Some(FeatureWindow::NewArchive) => {
                let mut suggested_name = None;
                if let Some(draft) = &mut self.new_draft {
                    let was_empty = draft.sources.is_empty();
                    for path in paths {
                        if !draft.sources.contains(path) {
                            draft.sources.push(path.clone());
                        }
                    }
                    draft.source_bytes = sum_source_bytes(&draft.sources);
                    draft.error = None;
                    if was_empty {
                        if let Some(first) = draft.sources.first() {
                            suggested_name = Some(default_archive_name(Some(first), draft.format));
                        }
                    }
                }
                if let Some(name) = suggested_name {
                    self.new_name.update(cx, |state, cx| {
                        state.set_value(name, window, cx);
                    });
                }
                self.show_toast(
                    format!("已添加 {} 个拖放源文件", paths.len()),
                    ToastKind::Info,
                    cx,
                );
                cx.notify();
                return;
            }
            Some(FeatureWindow::Hash) => {
                for path in paths {
                    if !self.hash_batch_paths.contains(path) {
                        self.hash_batch_paths.push(path.clone());
                    }
                }
                self.hash_batch_results.clear();
                self.hash_batch_error = None;
                self.hash_selected_row = None;
                self.hash_context_menu = None;
                self.show_toast(
                    format!("已添加 {} 个文件到哈希列表", paths.len()),
                    ToastKind::Info,
                    cx,
                );
                cx.notify();
                return;
            }
            Some(FeatureWindow::PasswordRecovery) => {
                if let Some(path) = paths.iter().find(|path| path.is_file()) {
                    self.recovery_path = Some(path.clone());
                    self.recovery_result = None;
                    self.recovery_error = None;
                    self.recovery_attempts = 0;
                    self.recovery_total = 0;
                    cx.notify();
                }
                return;
            }
            None => {}
        }
        if self.archive.is_none() {
            if paths.len() == 1 && paths[0].is_file() {
                self.open_path(paths[0].clone(), None, cx);
                return;
            }
            self.open_new_archive_modal(window, cx);
            if let Some(draft) = &mut self.new_draft {
                draft.sources = paths.to_vec();
                draft.source_bytes = sum_source_bytes(paths);
                draft.error = None;
            }
            if let Some(first) = paths.first() {
                let name = default_archive_name(Some(first), ArchiveFormat::Zip);
                self.new_name.update(cx, |state, cx| {
                    state.set_value(name, window, cx);
                });
            }
            self.show_toast(
                format!("已添加 {} 个拖放源文件", paths.len()),
                ToastKind::Info,
                cx,
            );
            cx.notify();
            return;
        }
        let Some(archive) = &self.archive else {
            return;
        };
        if !archive.info.format.is_zip() {
            self.show_toast("当前仅支持向 ZIP 归档拖放添加", ToastKind::Info, cx);
            return;
        }
        let path = archive.path.clone();
        let password = archive.password.clone();
        match rarkzip_engine::add_zip_entries(&path, paths, password.as_deref()) {
            Ok(report) => {
                self.show_toast(
                    format!("已添加 {} 个拖放条目", report.added),
                    ToastKind::Success,
                    cx,
                );
                self.reload_archive(cx);
            }
            Err(error) => self.show_toast(format!("拖放添加失败：{error}"), ToastKind::Error, cx),
        }
    }

    pub(super) fn open_hash_batch(&mut self, cx: &mut Context<Self>) {
        self.open_hash_modal(cx);
    }

    pub(super) fn add_hash_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(paths) =
            crate::components::file_dialog_in(window, || rfd::FileDialog::new().pick_files())
        else {
            return;
        };
        for path in paths {
            if !self.hash_batch_paths.contains(&path) {
                self.hash_batch_paths.push(path);
            }
        }
        self.hash_batch_results.clear();
        self.hash_selected_row = None;
        self.hash_context_menu = None;
        cx.notify();
    }

    pub(super) fn add_hash_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(folder) =
            crate::components::file_dialog_in(window, || rfd::FileDialog::new().pick_folder())
        else {
            return;
        };
        for entry in walkdir::WalkDir::new(&folder).follow_links(false) {
            let Ok(entry) = entry else {
                continue;
            };
            if entry.file_type().is_file() {
                let path = entry.path().to_path_buf();
                if !self.hash_batch_paths.contains(&path) {
                    self.hash_batch_paths.push(path);
                }
            }
        }
        self.hash_batch_results.clear();
        self.hash_selected_row = None;
        self.hash_context_menu = None;
        cx.notify();
    }

    pub(super) fn toggle_hash_algorithm(
        &mut self,
        algorithm: HashAlgorithm,
        cx: &mut Context<Self>,
    ) {
        if self.hash_batch_running {
            return;
        }
        if let Some(position) = self
            .hash_batch_algorithms
            .iter()
            .position(|candidate| *candidate == algorithm)
        {
            if self.hash_batch_algorithms.len() <= 1 {
                self.show_toast("至少保留一个哈希算法", ToastKind::Info, cx);
                return;
            }
            self.hash_batch_algorithms.remove(position);
        } else {
            self.hash_batch_algorithms.push(algorithm);
        }
        self.hash_batch_algorithms.sort_by_key(|candidate| {
            HashAlgorithm::ALL
                .iter()
                .position(|item| item == candidate)
                .unwrap_or(usize::MAX)
        });
        self.hash_batch_results.clear();
        self.hash_batch_error = None;
        self.hash_batch_progress = (0, 0);
        self.hash_selected_row = None;
        self.hash_context_menu = None;
        self.hash_table_scroll.set_offset(point(px(0.0), px(0.0)));
        self.settings.hash.algorithms = self
            .hash_batch_algorithms
            .iter()
            .map(|algorithm| algorithm.name().to_string())
            .collect();
        self.save_settings();
        cx.notify();
    }

    pub(super) fn start_hash_batch(&mut self, cx: &mut Context<Self>) {
        if self.hash_batch_running || self.hash_batch_paths.is_empty() {
            return;
        }
        self.hash_batch_run_id += 1;
        let run_id = self.hash_batch_run_id;
        let paths = self.hash_batch_paths.clone();
        let algorithms = self.hash_batch_algorithms.clone();
        let total = paths.len();
        self.hash_batch_running = true;
        self.hash_batch_results.clear();
        self.hash_batch_error = None;
        self.hash_selected_row = None;
        self.hash_context_menu = None;
        self.hash_batch_progress = (0, total);
        let sender = self.hash_batch_sender.clone();
        std::thread::spawn(move || {
            let mut results = Vec::with_capacity(paths.len());
            for (index, path) in paths.iter().enumerate() {
                let result = hash_file_many(&algorithms, path, &rarkzip_engine::NoopSink);
                results.push(match result {
                    Ok(hashes) => HashBatchItem {
                        path: path.clone(),
                        hashes,
                        error: None,
                    },
                    Err(error) => HashBatchItem {
                        path: path.clone(),
                        hashes: Vec::new(),
                        error: Some(error.to_string()),
                    },
                });
                let _ = sender.send(HashBatchEvent::Progress {
                    run_id,
                    done: index + 1,
                    total,
                });
            }
            let _ = sender.send(HashBatchEvent::Done { run_id, results });
        });
        cx.notify();
    }

    pub(super) fn handle_hash_batch_event(
        &mut self,
        event: HashBatchEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            HashBatchEvent::Progress {
                run_id,
                done,
                total,
            } => {
                if run_id == self.hash_batch_run_id {
                    self.hash_batch_progress = (done, total);
                }
            }
            HashBatchEvent::Done { run_id, results } => {
                if run_id == self.hash_batch_run_id {
                    self.hash_batch_running = false;
                    self.hash_batch_results = results;
                }
            }
        }
        cx.notify();
    }

    pub(super) fn open_columns(&mut self, cx: &mut Context<Self>) {
        self.modal = Some(Modal::Columns);
        cx.notify();
    }

    pub(super) fn open_toolbar_settings(&mut self, cx: &mut Context<Self>) {
        self.modal = Some(Modal::Toolbar);
        cx.notify();
    }

    pub(super) fn open_new_archive_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.feature_window == Some(FeatureWindow::NewArchive) {
            self.init_new_archive_draft(Vec::new(), None, window, cx);
            return;
        }
        self.open_feature_window(FeatureWindow::NewArchive, Vec::new(), None, cx);
    }

    /// 资源管理器右键菜单入口：打开新建压缩包窗口并预填源文件/输出目录。
    pub(super) fn open_new_archive_with(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        sources: Vec<PathBuf>,
        output_dir: Option<PathBuf>,
    ) {
        if self.feature_window == Some(FeatureWindow::NewArchive) {
            self.init_new_archive_draft(sources, output_dir, window, cx);
        } else {
            self.open_feature_window(FeatureWindow::NewArchive, sources, output_dir, cx);
        }
    }

    pub(super) fn init_new_archive_draft(
        &mut self,
        sources: Vec<PathBuf>,
        output_dir: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let format = ArchiveFormat::from_name(&self.settings.compression.default_format)
            .filter(|format| format.can_create())
            .unwrap_or(ArchiveFormat::Zip);
        let level = self.settings.compression.default_level.min(9);
        self.new_password
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.new_comment
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.new_advanced_open = false;
        self.new_advanced_base_width = None;
        self.new_name.update(cx, |state, cx| {
            state.set_value(
                default_archive_name(sources.first().map(PathBuf::as_path), format),
                window,
                cx,
            )
        });
        self.level_slider
            .update(cx, |state, cx| state.set_value(level as f32, window, cx));
        self.exclude_input.update(cx, |state, cx| {
            state.set_value(self.settings.compression.exclude.clone(), window, cx)
        });
        let source_bytes = sum_source_bytes(&sources);
        self.new_draft = Some(NewArchiveDraft {
            sources,
            source_bytes,
            format,
            level,
            include_root: self.settings.compression.include_root,
            test_after: self.settings.compression.test_after,
            delete_sources: false,
            output_dir,
            zip_algorithm: ZipAlgorithm::Deflate,
            sfx: false,
            volume_size_mb: 0,
            error: None,
        });
        self.feature_result = None;
        self.page = Page::NewArchive;
        self.modal = None;
        cx.notify();
    }

    pub(super) fn toggle_new_advanced(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_advanced_open = !self.new_advanced_open;
        if self.feature_window == Some(FeatureWindow::NewArchive) {
            let height = window.viewport_size().height.as_f32().max(560.0);
            let current = window.viewport_size().width.as_f32();
            let width = if self.new_advanced_open {
                self.new_advanced_base_width = Some(current);
                (current + 200.0).max(1180.0)
            } else {
                self.new_advanced_base_width
                    .take()
                    .unwrap_or_else(|| (current - 200.0).max(980.0))
            };
            window.resize(size(px(width), px(height)));
        }
        cx.notify();
    }

    fn add_sources(&mut self, folder: bool, window: &mut Window, cx: &mut Context<Self>) {
        let picked: Vec<PathBuf> = if folder {
            crate::components::file_dialog_in(window, || {
                rfd::FileDialog::new().pick_folder().into_iter().collect()
            })
        } else {
            crate::components::file_dialog_in(window, || {
                rfd::FileDialog::new().pick_files().unwrap_or_default()
            })
        };
        if picked.is_empty() {
            return;
        }
        let mut suggested_name = None;
        if let Some(draft) = &mut self.new_draft {
            let was_empty = draft.sources.is_empty();
            for path in picked {
                if !draft.sources.contains(&path) {
                    draft.sources.push(path);
                }
            }
            draft.source_bytes = sum_source_bytes(&draft.sources);
            draft.error = None;
            if was_empty {
                if let Some(first) = draft.sources.first() {
                    suggested_name = Some(default_archive_name(Some(first), draft.format));
                }
            }
        }
        if let Some(name) = suggested_name {
            self.new_name.update(cx, |state, cx| {
                state.set_value(name, window, cx);
            });
        }
        cx.notify();
    }

    fn pick_create_dest(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picked) =
            crate::components::file_dialog_in(window, || rfd::FileDialog::new().pick_folder())
        else {
            return;
        };
        if let Some(draft) = &mut self.new_draft {
            draft.output_dir = Some(picked);
            draft.error = None;
        }
        cx.notify();
    }

    fn start_create(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.new_draft.as_ref() else {
            return;
        };
        let sources = draft.sources.clone();
        if sources.is_empty() {
            if let Some(draft) = &mut self.new_draft {
                draft.error = Some("请先选择要压缩的文件或文件夹".to_string());
            }
            cx.notify();
            return;
        }
        let sfx = draft.sfx;
        let format = if sfx {
            ArchiveFormat::Zip
        } else {
            draft.format
        };
        let level = draft.level;
        let include_root = draft.include_root;
        let test_after = draft.test_after;
        let delete_sources = draft.delete_sources;
        let zip_algorithm = draft.zip_algorithm;
        let volume_size_mb = if sfx { 0 } else { draft.volume_size_mb };
        let output_dir = draft
            .output_dir
            .clone()
            .unwrap_or_else(|| default_output_dir(draft));
        let name = self.new_name.read(cx).value().to_string();
        if name.trim().is_empty() {
            if let Some(draft) = &mut self.new_draft {
                draft.error = Some("请输入压缩包名称".to_string());
            }
            cx.notify();
            return;
        }
        if name
            .chars()
            .any(|ch| matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        {
            if let Some(draft) = &mut self.new_draft {
                draft.error = Some("压缩包名称不能包含 \\ / : * ? \" < > |".to_string());
            }
            cx.notify();
            return;
        }
        let file_name = if sfx {
            ensure_sfx_name(&name)
        } else {
            ensure_archive_name(&name, format)
        };
        let password = self.new_password.read(cx).unmask_value().to_string();
        let comment = self.new_comment.read(cx).value().trim().to_string();
        if sfx && !password.is_empty() {
            if let Some(draft) = &mut self.new_draft {
                draft.error = Some("SFX 暂不支持密码载荷".to_string());
            }
            cx.notify();
            return;
        }
        if !password.is_empty() && !(format.is_zip() || matches!(format, ArchiveFormat::SevenZ)) {
            if let Some(draft) = &mut self.new_draft {
                draft.error = Some("加密写入仅支持 ZIP(AES-256) 与 7Z(AES-256)".to_string());
            }
            cx.notify();
            return;
        }
        let dest = unique_if_exists(output_dir.join(file_name));
        let exclude_value = self.exclude_input.read(cx).value();
        let exclude: Vec<String> = exclude_value
            .split([';', ',', '\n', '\r'])
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect();
        let options = CreateOptions {
            format,
            level,
            password: (!password.is_empty()).then_some(password),
            zip_algorithm,
            include_root,
            exclude,
            overwrite: false,
            comment: (!comment.is_empty()).then_some(comment),
            test_after: test_after || delete_sources,
            delete_sources,
            sfx,
            volume_size: (volume_size_mb > 0).then(|| volume_size_mb as u64 * 1024 * 1024),
        };
        self.modal = None;
        self.new_draft = None;
        self.feature_result = None;
        if self.feature_window.is_none() {
            self.page = Page::Main;
        }
        self.spawn_create(sources, dest, options, cx);
    }

    fn spawn_create(
        &mut self,
        sources: Vec<PathBuf>,
        dest: PathBuf,
        options: CreateOptions,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_task_id;
        self.next_task_id += 1;
        let title = format!(
            "压缩 {}",
            dest.file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default()
        );
        let mut task = TaskItem::new(
            id,
            title,
            Some(RetrySpec::Create {
                sources: sources.clone(),
                dest: dest.clone(),
                options: options.clone(),
            }),
        );
        task.detail = format!("{} 个源 · {}", sources.len(), dest.display());
        let cancel = task.cancel.clone();
        let dest_existed = dest.exists();
        self.tasks.insert(0, task);
        let sender = self.task_sender.clone();
        let password = options.password.clone();
        std::thread::spawn(move || {
            let sink = GuiSink::new(sender.clone(), id, cancel);
            let result = create_archive(&sources, &dest, &options, &sink);
            if result.is_err() && !dest_existed {
                let _ = std::fs::remove_file(&dest);
            }
            let _ = sender.send(TaskEvent::Finished {
                id,
                result: JobResult::Create { password, result },
            });
        });
        self.show_toast("压缩任务已开始", ToastKind::Info, cx);
        cx.notify();
    }

    // ------------------------------------------------------------------
    // Extraction / verification
    // ------------------------------------------------------------------
    fn extract_options(&self, mode: ExtractMode, selected: Option<Vec<usize>>) -> ExtractOptions {
        ExtractOptions {
            mode,
            password: self
                .archive
                .as_ref()
                .and_then(|archive| archive.password.clone()),
            passwords: PasswordBook::load_default()
                .entries()
                .iter()
                .map(|entry| entry.password.clone())
                .collect(),
            codepage: Codepage::from_name(&self.settings.extraction.default_codepage),
            selected,
            overwrite: true,
            strip_root: false,
            propagate_motw: self.settings.security.propagate_motw,
            nested_depth: if self.settings.extraction.nested_enabled {
                self.settings.extraction.nested_depth
            } else {
                0
            },
            delete_archive: false,
            amsi_scan: self.settings.security.amsi_scan,
            amsi_buffer_kb: self.settings.security.scan_buffer_kb,
        }
    }

    pub(super) fn start_extract_smart(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(archive) = &self.archive else {
            return;
        };
        let selected = self.selected.map(|index| vec![index]);
        let mode = if self.settings.extraction.smart {
            ExtractMode::Smart
        } else {
            ExtractMode::Here
        };
        let options = self.extract_options(mode, selected);
        let path = archive.path.clone();
        if archive.info.encrypted && options.password.is_none() {
            self.ensure_password(PendingAction::Extract { path, options }, cx);
        } else {
            self.spawn_extract(path, options, cx);
        }
    }

    /// 包内右键：解压全部 / 选定的文件到指定文件夹。
    pub(super) fn context_extract_to(
        &mut self,
        selected: Option<Vec<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = crate::components::file_dialog(|| rfd::FileDialog::new().pick_folder())
        else {
            return;
        };
        self.extract_with_mode(ExtractMode::Custom(target), selected, cx);
    }

    /// 包内右键：全部文件解压到压缩包所在目录。
    pub(super) fn context_extract_here(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.extract_with_mode(ExtractMode::Here, None, cx);
    }

    fn extract_with_mode(
        &mut self,
        mode: ExtractMode,
        selected: Option<Vec<usize>>,
        cx: &mut Context<Self>,
    ) {
        let Some(archive) = &self.archive else {
            return;
        };
        let options = self.extract_options(mode, selected);
        let path = archive.path.clone();
        if archive.info.encrypted && options.password.is_none() {
            self.ensure_password(PendingAction::Extract { path, options }, cx);
        } else {
            self.spawn_extract(path, options, cx);
        }
    }

    /// 包内右键：复制选中条目的包内完整路径。
    pub(super) fn copy_selected_entry_paths(&mut self, cx: &mut Context<Self>) {
        let Some(archive) = &self.archive else {
            return;
        };
        let indices = self.selected_indices();
        let text = indices
            .iter()
            .filter_map(|index| archive.info.find(*index))
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if text.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.show_toast(
            format!("已复制 {} 个条目的完整路径", indices.len()),
            ToastKind::Success,
            cx,
        );
    }

    /// 包内右键：在 RarkZip 预览面板打开（图片）或使用系统程序打开。
    pub(super) fn context_open_rarkzip(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.selected else {
            return;
        };
        let Some(entry) = self
            .archive
            .as_ref()
            .and_then(|archive| archive.info.find(index))
            .cloned()
        else {
            return;
        };
        if entry.is_dir {
            self.navigate_to(entry.path, cx);
            return;
        }
        if is_image_name(&entry.path) {
            self.selected = Some(index);
            self.refresh_preview(cx);
            self.show_toast("已在右侧预览面板打开", ToastKind::Info, cx);
            cx.notify();
        } else {
            self.open_entry(index, window, cx);
        }
    }

    pub(super) fn pick_extract_target(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(archive) = &self.archive else {
            return;
        };
        let Some(target) = crate::components::file_dialog(|| rfd::FileDialog::new().pick_folder())
        else {
            return;
        };
        let selected = self.selected.map(|index| vec![index]);
        let options = self.extract_options(ExtractMode::Custom(target), selected);
        let path = archive.path.clone();
        self.spawn_extract(path, options, cx);
    }

    fn spawn_extract(&mut self, path: PathBuf, options: ExtractOptions, cx: &mut Context<Self>) {
        let id = self.next_task_id;
        self.next_task_id += 1;
        let title = format!(
            "解压 {}",
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default()
        );
        let mut task = TaskItem::new(
            id,
            title,
            Some(RetrySpec::Extract {
                path: path.clone(),
                options: options.clone(),
            }),
        );
        task.detail = describe_extract_mode(&options, &path);
        let cancel = task.cancel.clone();
        self.tasks.insert(0, task);
        let sender = self.task_sender.clone();
        std::thread::spawn(move || {
            let sink = GuiSink::new(sender.clone(), id, cancel);
            let result = extract_archive(&path, &options, &sink);
            let _ = sender.send(TaskEvent::Finished {
                id,
                result: JobResult::Extract { path, result },
            });
        });
        cx.notify();
    }

    pub(super) fn start_verify(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(archive) = &self.archive else {
            return;
        };
        let path = archive.path.clone();
        let password = archive.password.clone();
        if archive.info.encrypted && password.is_none() {
            self.ensure_password(PendingAction::Verify { path }, cx);
        } else {
            self.spawn_verify(path, password, cx);
        }
    }

    fn spawn_verify(&mut self, path: PathBuf, password: Option<String>, cx: &mut Context<Self>) {
        let id = self.next_task_id;
        self.next_task_id += 1;
        let title = format!(
            "校验 {}",
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default()
        );
        let mut task = TaskItem::new(
            id,
            title,
            Some(RetrySpec::Verify {
                path: path.clone(),
                password: password.clone(),
            }),
        );
        task.detail = "流式读取全部条目并校验 CRC".to_string();
        let cancel = task.cancel.clone();
        self.tasks.insert(0, task);
        let sender = self.task_sender.clone();
        std::thread::spawn(move || {
            let sink = GuiSink::new(sender.clone(), id, cancel);
            let result = verify_archive(&path, password.as_deref(), &sink);
            let _ = sender.send(TaskEvent::Finished {
                id,
                result: JobResult::Verify { path, result },
            });
        });
        cx.notify();
    }

    // ------------------------------------------------------------------
    // Hash
    // ------------------------------------------------------------------
    pub(super) fn open_hash_modal(&mut self, cx: &mut Context<Self>) {
        if self.feature_window == Some(FeatureWindow::Hash) {
            self.init_hash(cx);
            return;
        }
        let source = self.archive.as_ref().map(|archive| archive.path.clone());
        self.open_feature_window(FeatureWindow::Hash, source.into_iter().collect(), None, cx);
    }

    pub(super) fn init_hash(&mut self, cx: &mut Context<Self>) {
        // 与旧单文件哈希一致：已打开归档时预填当前归档，仍可继续添加其它文件。
        self.hash_batch_paths = self
            .archive
            .as_ref()
            .map(|archive| vec![archive.path.clone()])
            .unwrap_or_default();
        self.hash_batch_results.clear();
        self.hash_batch_error = None;
        self.hash_batch_running = false;
        self.hash_batch_progress = (0, 0);
        self.hash_selected_row = None;
        self.hash_context_menu = None;
        self.page = Page::Hash;
        self.modal = None;
        cx.notify();
    }

    fn spawn_hash_task(
        &mut self,
        path: PathBuf,
        algorithm: HashAlgorithm,
        cx: &mut Context<Self>,
    ) -> u64 {
        let id = self.next_task_id;
        self.next_task_id += 1;
        let title = format!(
            "哈希 {}",
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default()
        );
        let mut task = TaskItem::new(
            id,
            title,
            Some(RetrySpec::Hash {
                path: path.clone(),
                algorithm,
            }),
        );
        task.detail = algorithm.label().to_string();
        let cancel = task.cancel.clone();
        self.tasks.insert(0, task);
        let sender = self.task_sender.clone();
        std::thread::spawn(move || {
            let sink = GuiSink::new(sender.clone(), id, cancel);
            let result = hash_file_with_progress(algorithm, &path, &sink);
            let _ = sender.send(TaskEvent::Finished {
                id,
                result: JobResult::Hash {
                    path,
                    algorithm,
                    result,
                },
            });
        });
        cx.notify();
        id
    }

    // ------------------------------------------------------------------
    // Open selected entry with system program
    // ------------------------------------------------------------------
    pub(super) fn open_entry(
        &mut self,
        index: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(archive) = &self.archive else {
            return;
        };
        let Some(entry) = archive.info.find(index).cloned() else {
            return;
        };
        if entry.is_dir {
            self.navigate_to(entry.path, cx);
            return;
        }
        if let Some(path) = self.preview.cache.get(&index).filter(|path| path.exists()) {
            match open::that(path) {
                Ok(()) => self.show_toast(
                    format!("已用系统程序打开：{}", entry.name()),
                    ToastKind::Info,
                    cx,
                ),
                Err(error) => self.show_toast(format!("打开失败：{error}"), ToastKind::Error, cx),
            }
            return;
        }
        let archive_path = archive.path.clone();
        let password = archive.password.clone();
        let pending = PendingAction::OpenEntry {
            archive: archive_path.clone(),
            index,
            entry: entry.path.clone(),
        };
        if archive.info.encrypted && password.is_none() {
            self.ensure_password(pending, cx);
        } else {
            self.execute_pending(pending, password, cx);
        }
    }

    fn spawn_open_entry(
        &mut self,
        archive: PathBuf,
        index: usize,
        entry: String,
        password: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let dir = std::env::temp_dir()
            .join("RarkZip")
            .join("open")
            .join(format!("{}-{}", std::process::id(), index));
        let sender = self.open_sender.clone();
        let name = entry.rsplit('/').next().unwrap_or(&entry).to_string();
        std::thread::spawn(move || {
            let options = ExtractOptions {
                mode: ExtractMode::Custom(dir.clone()),
                password,
                selected: Some(vec![index]),
                nested_depth: 0,
                propagate_motw: false,
                overwrite: true,
                ..Default::default()
            };
            let result = extract_archive(&archive, &options, &rarkzip_engine::NoopSink)
                .map_err(|error| error.to_string())
                .and_then(|_| {
                    let target = dir.join(entry.replace('/', std::path::MAIN_SEPARATOR_STR));
                    open::that(target).map_err(|error| error.to_string())
                });
            let _ = sender.send(OpenEntryEvent { name, result });
        });
        let _ = cx;
    }

    // ------------------------------------------------------------------
    // Password flow
    // ------------------------------------------------------------------
    fn ensure_password(&mut self, pending: PendingAction, cx: &mut Context<Self>) {
        let archive = pending_path(&pending);
        // RAR 等格式无法快速校验密码，自动逐个尝试会表现为长时间无响应；
        // 直接弹出密码框，由用户输入一次后解压。
        if !rarkzip_engine::supports_quick_password_test(&archive) {
            self.show_password_modal(archive, pending, None, cx);
            cx.notify();
            return;
        }
        let book = PasswordBook::load_default();
        match book.try_all(&archive) {
            Ok(Some(password)) => {
                self.set_archive_password(&archive, &password);
                self.execute_pending(pending, Some(password), cx);
            }
            Ok(None) => {
                self.show_password_modal(archive, pending, None, cx);
                cx.notify();
            }
            Err(error) => {
                self.show_toast(format!("密码本试密失败：{error}"), ToastKind::Error, cx);
                self.show_password_modal(archive, pending, None, cx);
                cx.notify();
            }
        }
    }

    fn set_archive_password(&mut self, archive: &Path, password: &str) {
        if let Some(state) = &mut self.archive {
            if state.path == archive {
                state.password = Some(password.to_string());
            }
        }
    }

    fn execute_pending(
        &mut self,
        pending: PendingAction,
        password: Option<String>,
        cx: &mut Context<Self>,
    ) {
        match pending {
            PendingAction::Open { path } => {
                self.open_path_with_password(path, password, cx);
            }
            PendingAction::Extract { path, mut options } => {
                options.password = password;
                self.spawn_extract(path, options, cx);
            }
            PendingAction::Verify { path } => {
                self.spawn_verify(path, password, cx);
            }
            PendingAction::OpenEntry {
                archive,
                index,
                entry,
            } => {
                self.spawn_open_entry(archive, index, entry, password, cx);
            }
            PendingAction::Prompt { responder, .. } => {
                let _ = responder.send(password);
            }
        }
    }

    pub(super) fn submit_password(&mut self, cx: &mut Context<Self>) {
        let Some(Modal::Password {
            archive, pending, ..
        }) = self.modal.clone()
        else {
            return;
        };
        let password = self.password_input.read(cx).unmask_value().to_string();
        if password.is_empty() {
            if let Some(Modal::Password { error, .. }) = &mut self.modal {
                *error = Some("请输入密码".to_string());
            }
            cx.notify();
            return;
        }
        // 无法快速校验的格式（如部分 RAR）在 UI 线程验证密码会长时间无响应；
        // 交给后台任务在解压时验证，错误时引擎会再次请求密码。
        let quick_test = rarkzip_engine::supports_quick_password_test(&archive);
        if !quick_test || matches!(pending, PendingAction::Prompt { .. }) {
            self.modal = None;
            self.password_picker_open = false;
            self.execute_pending(pending, Some(password), cx);
            return;
        }
        match rarkzip_engine::test_password(&archive, &password) {
            Ok(true) => {
                let mut book = PasswordBook::load_default();
                if let Err(error) = book.remember(&password) {
                    self.show_toast(
                        format!("密码正确，但写入密码本失败：{error}"),
                        ToastKind::Info,
                        cx,
                    );
                }
                self.set_archive_password(&archive, &password);
                self.modal = None;
                self.password_picker_open = false;
                self.execute_pending(pending, Some(password), cx);
            }
            Ok(false) => {
                if let Some(Modal::Password { error, .. }) = &mut self.modal {
                    *error = Some("密码错误，请重试".to_string());
                }
                cx.notify();
            }
            Err(error) => {
                if let Some(Modal::Password { error: slot, .. }) = &mut self.modal {
                    *slot = Some(error.to_string());
                }
                cx.notify();
            }
        }
    }

    /// 密码弹窗：点击密码本条目后填入并立即尝试。
    pub(super) fn select_book_password(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.password_book.entries().get(index).cloned() else {
            return;
        };
        self.password_input.update(cx, |state, cx| {
            state.set_value(entry.password, window, cx);
        });
        self.password_picker_open = false;
        self.submit_password(cx);
    }

    // ------------------------------------------------------------------
    // Task center
    // ------------------------------------------------------------------
    pub(super) fn render_tasks(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let running = self.tasks.iter().filter(|task| task.is_running()).count();
        let completed = self
            .tasks
            .iter()
            .filter(|task| matches!(&task.status, TaskStatus::Completed))
            .count();
        let failed = self
            .tasks
            .iter()
            .filter(|task| matches!(&task.status, TaskStatus::Failed))
            .count();
        let tasks: Vec<TaskSnapshot> = self
            .tasks
            .iter()
            .map(|task| {
                let archive_path = match &task.retry {
                    Some(RetrySpec::Open { path, .. })
                    | Some(RetrySpec::Extract { path, .. })
                    | Some(RetrySpec::Verify { path, .. }) => Some(path.clone()),
                    _ => None,
                };
                let tag = match &task.retry {
                    Some(RetrySpec::Extract { options, .. })
                        if options.mode == ExtractMode::Smart =>
                    {
                        Some("智能解压")
                    }
                    _ => None,
                };
                TaskSnapshot {
                    id: task.id,
                    title: task.title.clone(),
                    detail: task.detail.clone(),
                    status: task.status.clone(),
                    progress: task.progress,
                    phase: task.phase.clone(),
                    dest: task.dest.clone(),
                    has_retry: task.retry.is_some(),
                    archive_path,
                    tag,
                }
            })
            .collect();

        let stat_chip = |count: usize, label: &'static str, color: Hsla| {
            div()
                .h(px(26.0))
                .px(px(12.0))
                .rounded(px(99.0))
                .border_1()
                .border_color(palette.line)
                .bg(palette.panel)
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_size(crate::theme::font_px(12.5))
                .text_color(palette.sub)
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(color)
                        .child(count.to_string()),
                )
                .child(label)
                .into_any_element()
        };

        let clear = design::link_action(
            "tasks-clear-all",
            "全部清除",
            &palette,
            cx.listener(|this, _, _, cx| {
                for task in &this.tasks {
                    task.cancel.store(true, Ordering::Relaxed);
                }
                this.tasks.clear();
                cx.notify();
            }),
        );
        let retry_failed = (failed > 0).then(|| {
            design::link_action(
                "tasks-retry-failed",
                "重试失败项",
                &palette,
                cx.listener(|this, _, _, cx| this.retry_failed(cx)),
            )
        });
        let close = Button::new("tasks-close")
            .label("关闭")
            .icon(icon(IconName::X, palette.text, 14.0))
            .outline()
            .h(px(30.0))
            .px(px(12.0))
            .text_size(crate::theme::font_px(12.5))
            .on_click(cx.listener(|this, _, _, cx| {
                this.page = Page::Main;
                cx.notify();
            }));

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .flex_wrap()
                    .child(icon(IconName::Clock, palette.accent, 16.0))
                    .child(
                        div()
                            .text_size(crate::theme::font_px(13.5))
                            .text_color(palette.sub)
                            .child("任务在后台执行，不阻塞主窗口"),
                    )
                    .child(clear)
                    .when_some(retry_failed, |this, action| this.child(action)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(stat_chip(running, "运行中", palette.accent))
                    .child(stat_chip(completed, "完成", palette.ok))
                    .child(stat_chip(failed, "失败", palette.bad))
                    .child(div().ml(px(6.0)).child(close)),
            );

        let mut list = div().flex().flex_col().gap(px(12.0));
        if tasks.is_empty() {
            list = list.child(
                div()
                    .h(px(300.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(crate::theme::font_px(13.0))
                    .text_color(palette.muted)
                    .child("拖入或双击压缩包后，任务会出现在这里"),
            );
        } else {
            for snapshot in tasks {
                list = list.child(self.render_task_card(snapshot, cx));
            }
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(palette.bg)
            .child(
                div()
                    .id("tasks-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div().w_full().flex().justify_center().child(
                            div()
                                .w_full()
                                .max_w(px(design::PAGE_MAX_WIDTH))
                                .px(px(design::PAGE_PAD_X))
                                .pt(px(24.0))
                                .pb(px(28.0))
                                .flex()
                                .flex_col()
                                .gap(px(18.0))
                                .child(header)
                                .child(list),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn render_task_card(&mut self, task: TaskSnapshot, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let failed = matches!(&task.status, TaskStatus::Failed);
        let password_wait = task.is_running() && task.phase.contains("密码");
        let (status_text, badge_bg, badge_fg, task_icon) = match &task.status {
            TaskStatus::Running if password_wait => (
                "试密中".to_string(),
                palette.accent_soft,
                palette.accent,
                IconName::Key,
            ),
            TaskStatus::Running => (
                format!("进行中 {:.0}%", task.progress),
                palette.accent_soft,
                palette.accent,
                IconName::LoaderCircle,
            ),
            TaskStatus::Completed => (
                "完成".to_string(),
                palette.ok_soft,
                palette.ok,
                IconName::Check,
            ),
            TaskStatus::Failed => (
                "失败".to_string(),
                palette.bad_soft,
                palette.bad,
                IconName::X,
            ),
            TaskStatus::Cancelled => (
                "已取消".to_string(),
                palette.warn_soft,
                palette.warn,
                IconName::Ban,
            ),
        };
        let id = task.id;
        let dest = task.dest.clone();
        let archive_path = task.archive_path.clone();

        let (verb, rest) = {
            let title = task.title.clone();
            title
                .split_once(' ')
                .map(|(verb, rest)| (verb.to_string(), rest.to_string()))
                .unwrap_or_else(|| (String::new(), title.clone()))
        };

        let cancel = (task.is_running() && !password_wait).then(|| {
            Button::new(format!("task-cancel-{id}"))
                .label("取消")
                .icon(icon(IconName::Ban, palette.bad, 13.0))
                .outline()
                .h(px(28.0))
                .px(px(10.0))
                .text_size(crate::theme::font_px(12.0))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(task) = this.tasks.iter_mut().find(|task| task.id == id) {
                        task.cancel.store(true, Ordering::Relaxed);
                        task.detail = "正在取消…".to_string();
                    }
                    cx.notify();
                }))
        });
        let password = (password_wait && archive_path.is_some()).then(|| {
            let path = archive_path.clone().unwrap_or_default();
            let modal_path = path.clone();
            Button::new(format!("task-password-{id}"))
                .label("输入密码")
                .icon(icon(IconName::Key, palette.accent, 13.0))
                .outline()
                .h(px(28.0))
                .px(px(10.0))
                .text_size(crate::theme::font_px(12.0))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(task) = this.tasks.iter_mut().find(|task| task.id == id) {
                        task.cancel.store(true, Ordering::Relaxed);
                        task.detail = "已取消自动试密，等待手动输入".to_string();
                    }
                    this.show_password_modal(
                        modal_path.clone(),
                        PendingAction::Open {
                            path: modal_path.clone(),
                        },
                        Some("已取消自动尝试密码，请输入密码".to_string()),
                        cx,
                    );
                }))
        });
        let retry = (failed && task.has_retry).then(|| {
            Button::new(format!("task-retry-{id}"))
                .label("重试")
                .icon(icon(IconName::RotateCcw, palette.accent, 13.0))
                .outline()
                .h(px(28.0))
                .px(px(10.0))
                .text_size(crate::theme::font_px(12.0))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.retry_task(id, cx);
                }))
        });
        let open_target = dest.clone().filter(|dest| dest.exists()).or_else(|| {
            archive_path
                .as_ref()
                .and_then(|path| path.parent())
                .filter(|parent| parent.exists())
                .map(Path::to_path_buf)
        });
        let open_dest = open_target.map(|dest| {
            design::icon_action(
                &palette,
                format!("task-open-{id}"),
                IconName::FolderOpen,
                palette.sub,
                false,
                "打开所在位置",
                move |_, _, _| {
                    let _ = open::that(&dest);
                },
            )
        });
        let remove = design::icon_action(
            &palette,
            format!("task-remove-{id}"),
            IconName::X,
            palette.muted,
            false,
            "移除记录",
            cx.listener(move |this, _, _, cx| {
                this.tasks.retain(|task| task.id != id);
                cx.notify();
            }),
        );

        div()
            .id(format!("task-{id}"))
            .w_full()
            .rounded(px(12.0))
            .border_1()
            .border_color(if failed {
                palette.bad_soft
            } else {
                palette.line
            })
            .bg(palette.bg)
            .shadow_sm()
            .px(px(20.0))
            .py(px(16.0))
            .flex()
            .items_center()
            .gap(px(16.0))
            .hover(|style| style.border_color(palette.line_strong).shadow_md())
            .child(
                div()
                    .w(px(42.0))
                    .h(px(42.0))
                    .flex_none()
                    .rounded(px(11.0))
                    .bg(badge_bg)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(task_icon, badge_fg, 21.0)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .min_w_0()
                            .when(!verb.is_empty(), |this| {
                                this.child(
                                    div()
                                        .flex_none()
                                        .text_size(crate::theme::font_px(14.0))
                                        .text_color(palette.sub)
                                        .child(verb),
                                )
                            })
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(crate::theme::font_px(14.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(palette.text)
                                    .child(rest),
                            )
                            .child(design::tag(
                                &palette,
                                status_text,
                                if failed {
                                    design::TagTone::Danger
                                } else if matches!(&task.status, TaskStatus::Completed) {
                                    design::TagTone::Ok
                                } else if matches!(&task.status, TaskStatus::Cancelled) {
                                    design::TagTone::Manual
                                } else {
                                    design::TagTone::Accent
                                },
                            ))
                            .when_some(task.tag, |this, tag| {
                                this.child(design::tag(&palette, tag, design::TagTone::Neutral))
                            }),
                    )
                    .child(
                        div()
                            .mt(px(4.0))
                            .text_size(crate::theme::font_px(12.5))
                            .text_color(palette.muted)
                            .truncate()
                            .child(task.detail.clone()),
                    )
                    .when(task.is_running() && !password_wait, |this| {
                        this.child(div().mt(px(10.0)).child(progress_bar(
                            task.progress,
                            &palette,
                            palette.accent,
                        )))
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .when_some(password, |this, action| this.child(action))
                    .when_some(cancel, |this, action| this.child(action))
                    .when_some(retry, |this, action| this.child(action))
                    .when_some(open_dest, |this, action| this.child(action))
                    .child(remove),
            )
            .into_any_element()
    }

    fn retry_task(&mut self, id: u64, cx: &mut Context<Self>) {
        let spec = self
            .tasks
            .iter()
            .find(|task| task.id == id)
            .and_then(|task| task.retry.clone());
        if let Some(spec) = spec {
            self.execute_retry(spec, cx);
        }
    }

    fn retry_failed(&mut self, cx: &mut Context<Self>) {
        let specs: Vec<RetrySpec> = self
            .tasks
            .iter()
            .filter(|task| matches!(task.status, TaskStatus::Failed))
            .filter_map(|task| task.retry.clone())
            .collect();
        for spec in specs {
            self.execute_retry(spec, cx);
        }
    }

    fn execute_retry(&mut self, spec: RetrySpec, cx: &mut Context<Self>) {
        match spec {
            RetrySpec::Open { path, password } => {
                self.open_path_with_password(path, password, cx);
            }
            RetrySpec::Extract { path, options } => self.spawn_extract(path, options, cx),
            RetrySpec::Create {
                sources,
                dest,
                options,
            } => self.spawn_create(sources, dest, options, cx),
            RetrySpec::Verify { path, password } => self.spawn_verify(path, password, cx),
            RetrySpec::Hash { path, algorithm } => {
                self.spawn_hash_task(path, algorithm, cx);
            }
        }
    }
}

struct TaskSnapshot {
    id: u64,
    title: String,
    detail: String,
    status: TaskStatus,
    progress: f32,
    phase: String,
    dest: Option<PathBuf>,
    has_retry: bool,
    archive_path: Option<PathBuf>,
    tag: Option<&'static str>,
}

impl TaskSnapshot {
    fn is_running(&self) -> bool {
        matches!(self.status, TaskStatus::Running)
    }
}

fn pending_path(pending: &PendingAction) -> PathBuf {
    match pending {
        PendingAction::Open { path }
        | PendingAction::Extract { path, .. }
        | PendingAction::Verify { path } => path.clone(),
        PendingAction::OpenEntry { archive, .. } => archive.clone(),
        // 运行中任务的密码请求没有独立的归档路径；调用方不会对它调用本函数。
        PendingAction::Prompt { .. } => PathBuf::new(),
    }
}

fn describe_extract_mode(options: &ExtractOptions, archive: &Path) -> String {
    match &options.mode {
        ExtractMode::Smart => "智能解压".to_string(),
        ExtractMode::Here => archive
            .parent()
            .map(|parent| parent.display().to_string())
            .unwrap_or_default(),
        ExtractMode::Folder => archive
            .parent()
            .map(|parent| {
                parent
                    .join(rarkzip_engine::paths::archive_stem(archive))
                    .display()
                    .to_string()
            })
            .unwrap_or_default(),
        ExtractMode::Custom(path) => path.display().to_string(),
    }
}

fn default_output_dir(draft: &NewArchiveDraft) -> PathBuf {
    draft
        .output_dir
        .clone()
        .or_else(|| {
            draft
                .sources
                .first()
                .and_then(|path| path.parent())
                .filter(|parent| !parent.as_os_str().is_empty())
                .map(Path::to_path_buf)
        })
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn default_archive_name(source: Option<&Path>, format: ArchiveFormat) -> String {
    let base = source
        .and_then(|path| {
            if path.is_dir() {
                path.file_name()
                    .map(|name| name.to_string_lossy().to_string())
            } else {
                path.file_stem()
                    .map(|name| name.to_string_lossy().to_string())
            }
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "archive".to_string());
    format!(
        "{base} {}.{}",
        chrono::Local::now().format("%Y-%m-%d"),
        format.extension()
    )
}

fn ensure_archive_name(name: &str, format: ArchiveFormat) -> String {
    let name = Path::new(name.trim())
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| name.trim().to_string());
    let name = name.trim_end_matches([' ', '.']).to_string();
    if name.is_empty() {
        return format!("archive.{}", format.extension());
    }
    let extension = format.extension();
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(&format!(".{extension}")) {
        return name;
    }
    for suffix in [
        "tar.gz", "tar.bz2", "tar.xz", "tar.zst", "tgz", "tbz2", "tbz", "txz", "tzst", "zip", "7z",
        "tar", "gz", "bz2", "xz", "zst", "exe",
    ] {
        if lower.ends_with(&format!(".{suffix}")) {
            let stem = &name[..name.len() - suffix.len() - 1];
            return format!("{stem}.{extension}");
        }
    }
    format!("{name}.{extension}")
}

pub(crate) fn ensure_sfx_name(name: &str) -> String {
    let name = Path::new(name.trim())
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| name.trim().to_string());
    let mut base = name.trim_end_matches([' ', '.']).to_string();
    let lower = base.to_ascii_lowercase();
    for suffix in [
        "tar.gz", "tar.bz2", "tar.xz", "tar.zst", "tgz", "tbz2", "tbz", "txz", "tzst", "zip", "7z",
        "tar", "gz", "bz2", "xz", "zst", "exe",
    ] {
        if lower.ends_with(&format!(".{suffix}")) {
            base.truncate(base.len() - suffix.len() - 1);
            break;
        }
    }
    if base.is_empty() {
        return "archive.exe".to_string();
    }
    format!("{base}.exe")
}

fn sum_source_bytes(paths: &[PathBuf]) -> u64 {
    let mut total = 0u64;
    let mut seen = 0usize;
    for path in paths {
        if path.is_file() {
            if let Ok(metadata) = path.metadata() {
                total += metadata.len();
            }
            seen += 1;
            continue;
        }
        if !path.is_dir() {
            continue;
        }
        for entry in walkdir::WalkDir::new(path).follow_links(false) {
            if seen >= 20_000 {
                return total;
            }
            let Ok(entry) = entry else {
                continue;
            };
            if entry.file_type().is_file() {
                if let Ok(metadata) = entry.metadata() {
                    total += metadata.len();
                }
                seen += 1;
            }
        }
    }
    total
}

fn unique_if_exists(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let (stem, extension) = match name.split_once('.') {
        Some((stem, extension)) => (stem.to_string(), Some(extension.to_string())),
        None => (name, None),
    };
    rarkzip_engine::paths::unique_path(parent, &stem, extension.as_deref())
}

pub(crate) mod ui;
