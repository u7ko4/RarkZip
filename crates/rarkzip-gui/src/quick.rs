//! 资源管理器右键菜单触发的快速解压 / 快速压缩窗口。
//!
//! 流程与设计稿第 6 页一致：选择解压路径 →（加密时）输入密码 →
//! （同名文件时）冲突确认 → 进度 → 完成。压缩任务直接显示进度。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver, Sender};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::*;

use rarkzip_engine::{
    create_archive, extract_archive, format_size, open_archive, ArchiveFormat, Codepage,
    CreateOptions, EngineError, ExtractMode, ExtractOptions, OpenOptions, PasswordBook,
    PasswordPrompt, ProgressSink, Settings, ZipAlgorithm,
};

use crate::components::*;
use crate::theme::{self, Palette, Skin, ThemeMode};

/// 资源管理器右键菜单的直接解压方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractPreset {
    /// 解压到压缩包所在目录。
    Here,
    /// 按包内结构智能选择目标。
    Smart,
    /// 解压到与压缩包同名的文件夹。
    Folder,
}

#[derive(Debug, Clone)]
pub enum QuickAction {
    /// 解压到其他文件夹…：先选择路径。
    ExtractTo { archive: PathBuf },
    /// 右键“解压到此处 / 智能解压 / 独立文件夹”：直接显示进度。
    ExtractPreset {
        archive: PathBuf,
        preset: ExtractPreset,
        delete: bool,
    },
    /// 快速压缩：一个或多个源文件/文件夹合并为一个压缩包。
    Compress {
        sources: Vec<PathBuf>,
        format: ArchiveFormat,
        delete_sources: bool,
    },
}

impl QuickAction {
    pub fn key(&self) -> &'static str {
        match self {
            Self::ExtractTo { .. } => "quick_extract",
            Self::ExtractPreset { .. } => "quick_extract_progress",
            Self::Compress { .. } => "quick_compress",
        }
    }

    pub fn default_size(&self) -> (f32, f32) {
        match self {
            // 选择路径页包含常用位置与 5 个复选项，620×640 刚好铺满。
            Self::ExtractTo { .. } => (620.0, 590.0),
            // 右键直接解压/压缩从进度页开始，去掉完成后关闭选项后进一步收紧。
            Self::ExtractPreset { .. } | Self::Compress { .. } => (640.0, 405.0),
        }
    }

    /// 直接从进度页启动时的窗口高度；随界面字号轻微调整。
    pub fn progress_height(&self, font_size: f32) -> Option<f32> {
        match self {
            Self::ExtractPreset { .. } | Self::Compress { .. } => {
                Some((405.0 + (font_size.clamp(11.0, 17.0) - 13.0) * 18.0).clamp(390.0, 510.0))
            }
            Self::ExtractTo { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Folder,
    Password,
    Conflict,
    Progress,
    Done,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConflictChoice {
    Skip,
    Overwrite,
    Rename,
}

#[derive(Debug, Clone)]
struct ConflictInfo {
    name: String,
    incoming_size: u64,
    existing_size: u64,
}

#[derive(Debug, Clone)]
struct SmartConflictInfo {
    entry_index: usize,
    name: String,
    target_path: PathBuf,
    rename_path: PathBuf,
    incoming_size: u64,
    existing_size: u64,
    incoming_modified: Option<i64>,
    existing_modified: Option<i64>,
}

#[derive(Debug)]
enum QuickEvent {
    NeedPassword,
    PasswordPrompt {
        prompt: PasswordPrompt,
        responder: Sender<Option<String>>,
    },
    Prepared {
        total_entries: usize,
        conflicts: usize,
        first: Option<ConflictInfo>,
        selected: Option<Vec<usize>>,
        password: Option<String>,
    },
    SmartPrepared {
        dest: PathBuf,
        conflicts: Vec<SmartConflictInfo>,
        all_indices: Vec<usize>,
        password: Option<String>,
    },
    Progress {
        percent: f32,
        current: String,
    },
    Extracted {
        dest: PathBuf,
        files: usize,
        bytes: u64,
    },
    Compressed {
        dest: PathBuf,
    },
    Failed(String),
    Cancelled,
}

#[derive(Default)]
struct QuickControl {
    cancelled: AtomicBool,
    paused: AtomicBool,
}

struct QuickSink {
    sender: Sender<QuickEvent>,
    control: Arc<QuickControl>,
    last_percent: Mutex<f32>,
}

impl QuickSink {
    fn new(sender: Sender<QuickEvent>, control: Arc<QuickControl>) -> Self {
        Self {
            sender,
            control,
            last_percent: Mutex::new(-1.0),
        }
    }
}

impl ProgressSink for QuickSink {
    fn update(&self, progress: &rarkzip_engine::Progress) {
        while self.control.paused.load(Ordering::Relaxed)
            && !self.control.cancelled.load(Ordering::Relaxed)
        {
            std::thread::sleep(Duration::from_millis(80));
        }
        let percent = progress.percent();
        let mut last = self.last_percent.lock().unwrap();
        if (percent - *last).abs() >= 0.5 || percent >= 100.0 {
            *last = percent;
            let _ = self.sender.send(QuickEvent::Progress {
                percent,
                current: progress.current.clone(),
            });
        }
    }

    fn is_cancelled(&self) -> bool {
        self.control.cancelled.load(Ordering::Relaxed)
    }

    /// 嵌套层密码错误或首层需要密码时，阻塞工作线程等待窗口输入。
    fn request_password(&self, prompt: &PasswordPrompt) -> Option<String> {
        if self.control.cancelled.load(Ordering::Relaxed) {
            return None;
        }
        let (responder, receiver) = unbounded();
        if self
            .sender
            .send(QuickEvent::PasswordPrompt {
                prompt: prompt.clone(),
                responder,
            })
            .is_err()
        {
            return None;
        }
        loop {
            match receiver.recv_timeout(Duration::from_millis(120)) {
                Ok(value) => return value,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    if self.control.cancelled.load(Ordering::Relaxed) {
                        return None;
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return None,
            }
        }
    }
}

pub struct QuickView {
    focus_handle: FocusHandle,
    palette: Palette,
    settings: Settings,
    action: QuickAction,
    step: Step,
    custom_titlebar: bool,
    dest_input: Entity<InputState>,
    password_input: Entity<InputState>,
    same_name_folder: bool,
    delete_archive: bool,
    open_after: bool,
    remember: bool,
    password_error: Option<String>,
    password_attempted: bool,
    /// 当前窗口创建时加载的密码本，用于密码弹窗中的快速选择。
    password_book: PasswordBook,
    password_picker_open: bool,
    password_reveal: bool,
    /// 引擎阻塞等待中的密码请求；提交后通过它唤醒工作线程。
    pending_password_prompt: Option<(Sender<Option<String>>, PasswordPrompt)>,
    conflict: Option<ConflictInfo>,
    conflict_choice: ConflictChoice,
    smart_dest: Option<PathBuf>,
    smart_conflicts: Vec<SmartConflictInfo>,
    smart_all_indices: Vec<usize>,
    smart_conflict_index: usize,
    smart_conflict_choices: Vec<ConflictChoice>,
    smart_apply_all: bool,
    prepared_selected: Option<Vec<usize>>,
    total_entries: usize,
    percent: f32,
    current: String,
    paused: bool,
    result_text: Option<String>,
    result_open: Option<PathBuf>,
    error: Option<String>,
    control: Arc<QuickControl>,
    sender: Sender<QuickEvent>,
    receiver: Receiver<QuickEvent>,
    password: Option<String>,
    running: bool,
    started: Option<Instant>,
    elapsed_secs: u64,
    active_dest: Option<PathBuf>,
    window_handle: AnyWindowHandle,
    base_size: (f32, f32),
}

impl QuickView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, action: QuickAction) -> Self {
        Self::build(window, cx, action, PasswordBook::load_default())
    }

    #[cfg(test)]
    fn new_with_book(
        window: &mut Window,
        cx: &mut Context<Self>,
        action: QuickAction,
        password_book: PasswordBook,
    ) -> Self {
        Self::build(window, cx, action, password_book)
    }

    fn build(
        window: &mut Window,
        cx: &mut Context<Self>,
        action: QuickAction,
        password_book: PasswordBook,
    ) -> Self {
        let settings = Settings::load();
        let theme_mode = ThemeMode::from_setting(&settings.view.theme);
        let skin = Skin::from_setting(&settings.view.theme_skin);
        let dark = theme_mode.is_dark(window.appearance());
        theme::set_ui_font_size(settings.view.font_size);
        theme::apply(cx, dark, skin);
        let palette = Palette::for_skin(dark, skin);
        let custom_titlebar = settings.view.use_custom_titlebar;

        let remembered_dir = settings.ui.last_extract_dir.trim().to_string();
        let default_dir = match &action {
            QuickAction::ExtractTo { archive } | QuickAction::ExtractPreset { archive, .. } => {
                if remembered_dir.is_empty() {
                    archive
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
                } else {
                    PathBuf::from(remembered_dir)
                }
            }
            QuickAction::Compress { sources, .. } => sources
                .first()
                .and_then(|source| source.parent())
                .map(Path::to_path_buf)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default()),
        };
        let same_name_folder = settings.ui.last_extract_same_folder;
        let delete_archive = settings.ui.last_extract_delete_archive;
        let open_after = settings.extraction.open_folder_after;
        let dest_input = cx
            .new(|cx| InputState::new(window, cx).default_value(default_dir.display().to_string()));
        let password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("输入密码")
        });
        let (sender, receiver) = unbounded();
        let step = match action {
            QuickAction::ExtractTo { .. } => Step::Folder,
            QuickAction::ExtractPreset { .. } | QuickAction::Compress { .. } => Step::Progress,
        };
        let window_handle = window.window_handle();
        let viewport = window.viewport_size();
        let base_size = if viewport.width.as_f32() > 0.0 && viewport.height.as_f32() > 0.0 {
            (viewport.width.as_f32(), viewport.height.as_f32())
        } else {
            action.default_size()
        };
        let mut view = Self {
            focus_handle: cx.focus_handle(),
            palette,
            settings,
            action,
            step,
            custom_titlebar,
            dest_input,
            password_input,
            same_name_folder,
            delete_archive,
            open_after,
            remember: false,
            password_error: None,
            password_attempted: false,
            password_book,
            password_picker_open: false,
            password_reveal: true,
            pending_password_prompt: None,
            conflict: None,
            conflict_choice: ConflictChoice::Overwrite,
            smart_dest: None,
            smart_conflicts: Vec::new(),
            smart_all_indices: Vec::new(),
            smart_conflict_index: 0,
            smart_conflict_choices: Vec::new(),
            smart_apply_all: true,
            prepared_selected: None,
            total_entries: 0,
            percent: 0.0,
            current: String::new(),
            paused: false,
            result_text: None,
            result_open: None,
            error: None,
            control: Arc::new(QuickControl::default()),
            sender,
            receiver,
            password: None,
            running: false,
            started: None,
            elapsed_secs: 0,
            active_dest: None,
            window_handle,
            base_size,
        };

        cx.subscribe(&view.password_input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_password(cx);
            }
        })
        .detach();

        window.focus(&view.focus_handle, cx);
        let weak = cx.entity().downgrade();
        window.on_window_should_close(&*cx, move |window, cx| {
            let _ = weak.update(cx, |view, _| {
                view.control.cancelled.store(true, Ordering::Relaxed);
                view.cancel_pending_prompt();
                view.persist_window_size(window);
            });
            true
        });

        view.start_poll_loop(cx);
        match view.action {
            QuickAction::Compress { .. } => view.start_compress(cx),
            QuickAction::ExtractPreset { .. } => view.start_preset_extract(cx),
            QuickAction::ExtractTo { .. } => {}
        }
        view
    }

    fn start_poll_loop(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |weak, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(80))
                .await;
            let alive = weak
                .update(cx, |view, cx| {
                    view.drain_events(cx);
                    true
                })
                .unwrap_or(false);
            if !alive {
                break;
            }
        })
        .detach();
    }

    fn drain_events(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        while let Ok(event) = self.receiver.try_recv() {
            self.handle_event(event, cx);
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    fn handle_event(&mut self, event: QuickEvent, cx: &mut Context<Self>) {
        match event {
            QuickEvent::NeedPassword => {
                self.running = false;
                self.step = Step::Password;
                self.password_picker_open = false;
                self.password_reveal = true;
                self.password_error = self
                    .password_attempted
                    .then(|| "密码错误，请重试。".to_string());
                self.resize_for_step(cx);
            }
            QuickEvent::PasswordPrompt { prompt, responder } => {
                // 工作线程已在等待回复：此时不能重启解压，提交后直接把
                // 密码送回引擎，让嵌套/首层流程原地继续。
                self.running = false;
                self.password_attempted = prompt.attempt > 1;
                self.password_error =
                    (prompt.attempt > 1).then(|| "密码错误，请重试。".to_string());
                self.password_picker_open = false;
                self.password_reveal = true;
                self.pending_password_prompt = Some((responder, prompt));
                self.step = Step::Password;
                self.resize_for_step(cx);
            }
            QuickEvent::Prepared {
                total_entries,
                conflicts,
                first,
                selected,
                password,
            } => {
                self.running = false;
                if password.is_some() {
                    self.password = password;
                }
                self.total_entries = total_entries;
                self.prepared_selected = selected;
                if conflicts > 0 {
                    self.conflict = first;
                    self.step = Step::Conflict;
                    self.resize_for_step(cx);
                } else {
                    self.start_extract(cx);
                }
            }
            QuickEvent::SmartPrepared {
                dest,
                conflicts,
                all_indices,
                password,
            } => {
                self.running = false;
                if password.is_some() {
                    self.password = password;
                }
                self.smart_dest = Some(dest.clone());
                self.active_dest = Some(dest);
                self.smart_conflicts = conflicts;
                self.smart_all_indices = all_indices;
                self.smart_conflict_index = 0;
                self.smart_conflict_choices.clear();
                self.conflict_choice = ConflictChoice::Overwrite;
                self.smart_apply_all = true;
                if self.smart_conflicts.is_empty() {
                    self.start_smart_extract(cx);
                } else {
                    self.step = Step::Conflict;
                    self.resize_for_step(cx);
                }
            }
            QuickEvent::Progress { percent, current } => {
                self.percent = percent;
                self.elapsed_secs = self.started.map(|t| t.elapsed().as_secs()).unwrap_or(0);
                if !current.is_empty() {
                    self.current = current;
                }
            }
            QuickEvent::Extracted { dest, files, bytes } => {
                self.cancel_pending_prompt();
                self.running = false;
                self.step = Step::Done;
                self.percent = 100.0;
                self.elapsed_secs = self.started.map(|t| t.elapsed().as_secs()).unwrap_or(0);
                // 相对路径的 Here 模式会得到 "."，补全为真实目录；
                // 同时去掉引擎内部使用的 \\?\ 长路径前缀，便于阅读。
                let dest = plain_path(std::fs::canonicalize(&dest).unwrap_or(dest));
                self.result_text = Some(format!(
                    "已解压 {files} 个文件 · {} → {}",
                    format_size(bytes),
                    dest.display()
                ));
                if self.open_after {
                    let _ = open::that(&dest);
                }
                self.active_dest = Some(dest.clone());
                self.result_open = Some(dest);
                self.resize_for_step(cx);
                // 压缩/解压成功后直接关闭弹窗，不再显示完成页。
                let _ = self
                    .window_handle
                    .update(cx, |_, window, _| window.remove_window());
                return;
            }
            QuickEvent::Compressed { dest } => {
                self.running = false;
                self.step = Step::Done;
                self.percent = 100.0;
                self.elapsed_secs = self.started.map(|t| t.elapsed().as_secs()).unwrap_or(0);
                let dest = std::fs::canonicalize(&dest).unwrap_or(dest);
                self.result_text = Some(format!("压缩完成：{}", dest.display()));
                self.active_dest = Some(dest.clone());
                self.result_open = dest.parent().map(Path::to_path_buf);
                self.resize_for_step(cx);
                // 压缩/解压成功后直接关闭弹窗，不再显示完成页。
                let _ = self
                    .window_handle
                    .update(cx, |_, window, _| window.remove_window());
                return;
            }
            QuickEvent::Failed(error) => {
                self.cancel_pending_prompt();
                self.running = false;
                self.step = Step::Failed;
                self.error = Some(error);
                self.resize_for_step(cx);
            }
            QuickEvent::Cancelled => {
                self.cancel_pending_prompt();
                self.running = false;
                self.step = Step::Failed;
                self.error = Some("任务已取消。".to_string());
                self.resize_for_step(cx);
            }
        }
        cx.notify();
    }

    fn target_dir(&self, cx: &mut Context<Self>) -> PathBuf {
        let base = PathBuf::from(self.dest_input.read(cx).value().to_string());
        if !self.same_name_folder {
            return base;
        }
        let QuickAction::ExtractTo { archive } = &self.action else {
            return base;
        };
        let stem = archive
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_else(|| "extract".to_string());
        base.join(stem)
    }

    fn submit_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        let viewport = window.viewport_size();
        if viewport.width.as_f32() > 0.0 && viewport.height.as_f32() > 0.0 {
            self.base_size = (viewport.width.as_f32(), viewport.height.as_f32());
        }
        let dest = self.target_dir(cx);
        if dest.as_os_str().is_empty() {
            self.error = Some("请选择解压目标路径。".to_string());
            cx.notify();
            return;
        }
        if self.remember {
            self.settings.ui.last_extract_dir = self.dest_input.read(cx).value().to_string();
            self.settings.ui.last_extract_same_folder = self.same_name_folder;
            self.settings.ui.last_extract_delete_archive = self.delete_archive;
            self.settings.extraction.open_folder_after = self.open_after;
            self.save_settings();
        }
        self.error = None;
        self.spawn_prepare(dest, cx);
    }

    fn submit_password(&mut self, cx: &mut Context<Self>) {
        let value = self.password_input.read(cx).unmask_value().to_string();
        if value.is_empty() {
            self.password_error = Some("请输入密码。".to_string());
            cx.notify();
            return;
        }
        self.password_picker_open = false;
        // 嵌套层/首层的密码请求：工作线程正阻塞等待，直接回复它。
        if let Some((responder, _)) = self.pending_password_prompt.take() {
            self.password = Some(value.clone());
            self.password_error = None;
            self.password_attempted = true;
            let _ = responder.send(Some(value));
            self.running = true;
            self.step = Step::Progress;
            self.resize_for_step(cx);
            cx.notify();
            return;
        }
        self.password = Some(value);
        self.password_error = None;
        self.password_attempted = true;
        match &self.action {
            QuickAction::ExtractTo { .. } => {
                let dest = self.target_dir(cx);
                self.spawn_prepare(dest, cx);
            }
            QuickAction::ExtractPreset { .. } => self.start_preset_extract(cx),
            QuickAction::Compress { .. } => {}
        }
    }

    /// 密码弹窗：选择密码本条目后填入并立即尝试。
    fn select_book_password(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.password_book.entries().get(index).cloned() else {
            return;
        };
        self.password_input.update(cx, |state, cx| {
            state.set_value(entry.password.clone(), window, cx);
        });
        self.password = Some(entry.password);
        self.password_picker_open = false;
        self.submit_password(cx);
    }

    /// 取消仍阻塞在密码请求上的工作线程。
    fn cancel_pending_prompt(&mut self) {
        if let Some((responder, _)) = self.pending_password_prompt.take() {
            let _ = responder.send(None);
        }
    }

    fn spawn_prepare(&mut self, dest: PathBuf, cx: &mut Context<Self>) {
        let QuickAction::ExtractTo { archive } = self.action.clone() else {
            return;
        };
        self.begin_progress(archive.display().to_string(), cx);
        let sender = self.sender.clone();
        let password = self.password.clone();
        let control = self.control.clone();
        std::thread::spawn(move || {
            let mut resolved_password = password.clone();
            let mut result = open_archive(
                &archive,
                &OpenOptions {
                    password: password.clone(),
                    codepage: Codepage::Auto,
                },
            );
            if matches!(&result, Err(error) if error.is_password_related())
                && password.is_none()
                && rarkzip_engine::supports_quick_password_test(&archive)
            {
                let book = PasswordBook::load_default();
                if let Ok(Some(found)) = book.try_all(&archive) {
                    resolved_password = Some(found.clone());
                    result = open_archive(
                        &archive,
                        &OpenOptions {
                            password: Some(found),
                            codepage: Codepage::Auto,
                        },
                    );
                }
            }
            let info = match result {
                Ok(info) => info,
                Err(error) if error.is_password_related() => {
                    let _ = sender.send(QuickEvent::NeedPassword);
                    return;
                }
                Err(error) => {
                    let _ = sender.send(QuickEvent::Failed(error.to_string()));
                    return;
                }
            };
            // ZIP 的中央目录可以直接读取，密码只在解压内容时校验；
            // 能快速试密的格式先按密码本尝试，命中后把密码传给后续嵌套层。
            if info.encrypted
                && resolved_password.is_none()
                && rarkzip_engine::supports_quick_password_test(&archive)
            {
                let book = PasswordBook::load_default();
                if let Ok(Some(found)) = book.try_all(&archive) {
                    resolved_password = Some(found);
                }
            }
            if info.encrypted && resolved_password.is_none() {
                let _ = sender.send(QuickEvent::NeedPassword);
                return;
            }
            if control.cancelled.load(Ordering::Relaxed) {
                let _ = sender.send(QuickEvent::Cancelled);
                return;
            }
            let mut total_entries = 0usize;
            let mut conflicts = 0usize;
            let mut first = None;
            let mut selected = Vec::new();
            for entry in &info.entries {
                total_entries += 1;
                if entry.is_dir {
                    selected.push(entry.index);
                    continue;
                }
                let target = dest.join(entry.path.replace('/', std::path::MAIN_SEPARATOR_STR));
                if target.exists() {
                    conflicts += 1;
                    if first.is_none() {
                        first = Some(ConflictInfo {
                            name: entry.path.clone(),
                            incoming_size: entry.size,
                            existing_size: target.metadata().map(|m| m.len()).unwrap_or(0),
                        });
                    }
                } else {
                    selected.push(entry.index);
                }
            }
            let _ = sender.send(QuickEvent::Prepared {
                total_entries,
                conflicts,
                first,
                selected: (conflicts > 0).then_some(selected),
                password: resolved_password,
            });
        });
        cx.notify();
    }

    fn spawn_smart_prepare(&mut self, cx: &mut Context<Self>) {
        let QuickAction::ExtractPreset {
            archive,
            preset: ExtractPreset::Smart,
            ..
        } = self.action.clone()
        else {
            return;
        };
        self.begin_progress(archive.display().to_string(), cx);
        let sender = self.sender.clone();
        let password = self.password.clone();
        let control = self.control.clone();

        std::thread::spawn(move || {
            let mut resolved_password = password.clone();
            let mut result = open_archive(
                &archive,
                &OpenOptions {
                    password: password.clone(),
                    codepage: Codepage::Auto,
                },
            );
            if matches!(&result, Err(error) if error.is_password_related())
                && password.is_none()
                && rarkzip_engine::supports_quick_password_test(&archive)
            {
                let book = PasswordBook::load_default();
                if let Ok(Some(found)) = book.try_all(&archive) {
                    resolved_password = Some(found.clone());
                    result = open_archive(
                        &archive,
                        &OpenOptions {
                            password: Some(found),
                            codepage: Codepage::Auto,
                        },
                    );
                }
            }
            let info = match result {
                Ok(info) => info,
                Err(error) if error.is_password_related() => {
                    let _ = sender.send(QuickEvent::NeedPassword);
                    return;
                }
                Err(error) => {
                    let _ = sender.send(QuickEvent::Failed(error.to_string()));
                    return;
                }
            };
            // ZIP 的中央目录可以直接读取，密码只在解压内容时校验；
            // 能快速试密的格式先按密码本尝试，命中后把密码传给后续嵌套层。
            if info.encrypted
                && resolved_password.is_none()
                && rarkzip_engine::supports_quick_password_test(&archive)
            {
                let book = PasswordBook::load_default();
                if let Ok(Some(found)) = book.try_all(&archive) {
                    resolved_password = Some(found);
                }
            }
            if info.encrypted && resolved_password.is_none() {
                let _ = sender.send(QuickEvent::NeedPassword);
                return;
            }
            if control.cancelled.load(Ordering::Relaxed) {
                let _ = sender.send(QuickEvent::Cancelled);
                return;
            }
            let target = rarkzip_engine::smart_target(&archive, &info);
            let all_indices = info
                .entries
                .iter()
                .filter(|entry| !entry.is_dir)
                .map(|entry| entry.index)
                .collect();
            let mut conflicts = Vec::new();
            for entry in info.entries.iter().filter(|entry| !entry.is_dir) {
                let target_path = target
                    .dest
                    .join(entry.path.replace('/', std::path::MAIN_SEPARATOR_STR));
                if !target_path.exists() {
                    continue;
                }
                let existing_size = std::fs::metadata(&target_path)
                    .map(|meta| meta.len())
                    .unwrap_or(0);
                let existing_modified = std::fs::metadata(&target_path)
                    .ok()
                    .and_then(|meta| meta.modified().ok())
                    .and_then(rarkzip_engine::util::system_time_to_unix);
                conflicts.push(SmartConflictInfo {
                    entry_index: entry.index,
                    name: entry.path.clone(),
                    target_path: target_path.clone(),
                    rename_path: conflict_rename_path(&target_path),
                    incoming_size: entry.size,
                    existing_size,
                    incoming_modified: entry.modified,
                    existing_modified,
                });
            }
            let _ = sender.send(QuickEvent::SmartPrepared {
                dest: target.dest,
                conflicts,
                all_indices,
                password: resolved_password,
            });
        });
        cx.notify();
    }

    fn resize_for_step(&mut self, cx: &mut Context<Self>) {
        // 内容区随界面字号缩放；去掉“内容/最近处理”后，进度与结果页按实际内容收紧高度。
        let font_delta = (crate::theme::ui_font_size() - 13.0) * 18.0;
        let (width, height) = match self.step {
            Step::Password => (
                self.base_size.0.clamp(500.0, 640.0),
                if self.password_picker_open {
                    660.0
                } else {
                    440.0
                },
            ),
            Step::Conflict => (self.base_size.0.max(600.0), 480.0),
            Step::Progress => (self.base_size.0, (405.0 + font_delta).clamp(390.0, 510.0)),
            Step::Done | Step::Failed => {
                (self.base_size.0, (375.0 + font_delta).clamp(350.0, 480.0))
            }
            Step::Folder => return,
        };
        let _ = self.window_handle.update(cx, |_, window, _| {
            window.resize(size(px(width), px(height)));
        });
    }

    fn submit_smart_conflict(&mut self, cx: &mut Context<Self>) {
        let total = self.smart_conflicts.len();
        let choice = self.conflict_choice;
        if self.smart_apply_all {
            while self.smart_conflict_choices.len() < total {
                self.smart_conflict_choices.push(choice);
            }
        } else {
            self.smart_conflict_choices.push(choice);
            if self.smart_conflict_choices.len() < total {
                self.smart_conflict_index = self.smart_conflict_choices.len();
                self.conflict_choice = ConflictChoice::Overwrite;
                cx.notify();
                return;
            }
        }
        self.start_smart_extract(cx);
    }

    fn start_smart_extract(&mut self, cx: &mut Context<Self>) {
        let QuickAction::ExtractPreset {
            archive,
            preset: ExtractPreset::Smart,
            delete,
        } = self.action.clone()
        else {
            return;
        };
        let dest = self
            .smart_dest
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let mut selected = self.smart_all_indices.clone();
        let mut rename_jobs = Vec::new();
        for (conflict, choice) in self
            .smart_conflicts
            .iter()
            .zip(self.smart_conflict_choices.iter())
        {
            if matches!(choice, ConflictChoice::Skip | ConflictChoice::Rename) {
                selected.retain(|index| *index != conflict.entry_index);
            }
            if *choice == ConflictChoice::Rename {
                rename_jobs.push(conflict.clone());
            }
        }
        let selected = (selected.len() != self.smart_all_indices.len()).then_some(selected);
        let main_options = self.extract_options(ExtractMode::Smart, delete, true, selected);
        let rename_options = self.extract_options(ExtractMode::Smart, false, true, None);

        self.active_dest = Some(dest);
        self.begin_progress(archive.display().to_string(), cx);
        let control = self.control.clone();
        let sender = self.sender.clone();
        let sink = QuickSink::new(sender.clone(), control.clone());
        std::thread::spawn(move || {
            let mut renamed_files = 0usize;
            let mut renamed_bytes = 0u64;
            for conflict in rename_jobs {
                let temp = std::env::temp_dir().join("RarkZip").join(format!(
                    "rename-{}-{}",
                    std::process::id(),
                    conflict.entry_index
                ));
                let _ = std::fs::remove_dir_all(&temp);
                if let Err(error) = std::fs::create_dir_all(&temp) {
                    let _ = sender.send(QuickEvent::Failed(format!("创建临时目录失败：{error}")));
                    return;
                }
                let mut options = rename_options.clone();
                options.mode = ExtractMode::Custom(temp.clone());
                options.selected = Some(vec![conflict.entry_index]);
                options.nested_depth = 0;
                options.delete_archive = false;
                match extract_archive(&archive, &options, &rarkzip_engine::NoopSink) {
                    Ok(outcome) => {
                        let source =
                            temp.join(conflict.name.replace('/', std::path::MAIN_SEPARATOR_STR));
                        if let Some(parent) = conflict.rename_path.parent() {
                            if let Err(error) = std::fs::create_dir_all(parent) {
                                let _ = std::fs::remove_dir_all(&temp);
                                let _ = sender
                                    .send(QuickEvent::Failed(format!("创建目标目录失败：{error}")));
                                return;
                            }
                        }
                        let moved = std::fs::rename(&source, &conflict.rename_path).or_else(|_| {
                            std::fs::copy(&source, &conflict.rename_path)
                                .map(|_| ())
                                .and_then(|_| std::fs::remove_file(&source))
                        });
                        if let Err(error) = moved {
                            let _ = std::fs::remove_dir_all(&temp);
                            let _ = sender
                                .send(QuickEvent::Failed(format!("重命名解压文件失败：{error}")));
                            return;
                        }
                        renamed_files += 1;
                        renamed_bytes += outcome.bytes;
                    }
                    Err(error) => {
                        let _ = std::fs::remove_dir_all(&temp);
                        let _ = sender.send(QuickEvent::Failed(error.to_string()));
                        return;
                    }
                }
                let _ = std::fs::remove_dir_all(&temp);
            }
            match extract_archive(&archive, &main_options, &sink) {
                Ok(outcome) => {
                    let _ = sender.send(QuickEvent::Extracted {
                        dest: outcome.dest,
                        files: outcome.files + renamed_files,
                        bytes: outcome.bytes + renamed_bytes,
                    });
                }
                Err(error) if error.is_password_related() => {
                    let _ = sender.send(QuickEvent::NeedPassword);
                }
                Err(error) => {
                    let _ = sender.send(QuickEvent::Failed(error.to_string()));
                }
            }
        });
        cx.notify();
    }

    fn begin_progress(&mut self, current: String, cx: &mut Context<Self>) {
        self.running = true;
        self.step = Step::Progress;
        self.percent = 0.0;
        self.current = current;
        self.started = Some(Instant::now());
        self.elapsed_secs = 0;
        self.error = None;
        self.control = Arc::new(QuickControl::default());
        self.resize_for_step(cx);
    }

    fn extract_options(
        &self,
        mode: ExtractMode,
        delete: bool,
        overwrite: bool,
        selected: Option<Vec<usize>>,
    ) -> ExtractOptions {
        ExtractOptions {
            mode,
            password: self.password.clone(),
            passwords: PasswordBook::load_default()
                .entries()
                .iter()
                .map(|entry| entry.password.clone())
                .collect(),
            codepage: Codepage::from_name(&self.settings.extraction.default_codepage),
            selected,
            overwrite,
            strip_root: false,
            propagate_motw: self.settings.security.propagate_motw,
            nested_depth: if self.settings.extraction.nested_enabled {
                self.settings.extraction.nested_depth
            } else {
                0
            },
            delete_archive: delete,
            amsi_scan: self.settings.security.amsi_scan,
            amsi_buffer_kb: self.settings.security.scan_buffer_kb,
        }
    }

    fn start_extract(&mut self, cx: &mut Context<Self>) {
        let QuickAction::ExtractTo { archive } = self.action.clone() else {
            return;
        };
        let dest = self.target_dir(cx);
        let rename = self.conflict_choice == ConflictChoice::Rename;
        let dest = if rename { unique_dir(&dest) } else { dest };
        let overwrite = self.conflict_choice == ConflictChoice::Overwrite;
        let selected = if self.conflict_choice == ConflictChoice::Skip {
            self.prepared_selected.clone()
        } else {
            None
        };
        self.active_dest = Some(dest.clone());
        let options = self.extract_options(
            ExtractMode::Custom(dest),
            self.delete_archive,
            overwrite,
            selected,
        );
        self.begin_progress(archive.display().to_string(), cx);
        let control = self.control.clone();
        let sender = self.sender.clone();
        let sink = QuickSink::new(sender.clone(), control.clone());
        std::thread::spawn(move || match extract_archive(&archive, &options, &sink) {
            Ok(outcome) => {
                let _ = sender.send(QuickEvent::Extracted {
                    dest: outcome.dest,
                    files: outcome.files,
                    bytes: outcome.bytes,
                });
            }
            Err(EngineError::Cancelled) => {
                let _ = sender.send(QuickEvent::Cancelled);
            }
            Err(error) if error.is_password_related() => {
                let _ = sender.send(QuickEvent::NeedPassword);
            }
            Err(error) => {
                let _ = sender.send(QuickEvent::Failed(error.to_string()));
            }
        });
        cx.notify();
    }

    /// 资源管理器右键的直接解压：不需要选择路径，立即显示进度。
    fn start_preset_extract(&mut self, cx: &mut Context<Self>) {
        let QuickAction::ExtractPreset {
            archive,
            preset,
            delete,
        } = self.action.clone()
        else {
            return;
        };
        if preset == ExtractPreset::Smart {
            self.spawn_smart_prepare(cx);
            return;
        }
        let mode = match preset {
            ExtractPreset::Here => ExtractMode::Here,
            ExtractPreset::Smart => ExtractMode::Smart,
            ExtractPreset::Folder => ExtractMode::Folder,
        };
        let parent = archive
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        self.active_dest = Some(match preset {
            ExtractPreset::Here | ExtractPreset::Smart => parent,
            ExtractPreset::Folder => parent.join(
                archive
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().to_string())
                    .unwrap_or_else(|| "extract".to_string()),
            ),
        });
        let options = self.extract_options(mode, delete, true, None);
        self.begin_progress(archive.display().to_string(), cx);
        let control = self.control.clone();
        let sender = self.sender.clone();
        let sink = QuickSink::new(sender.clone(), control.clone());
        std::thread::spawn(move || match extract_archive(&archive, &options, &sink) {
            Ok(outcome) => {
                let _ = sender.send(QuickEvent::Extracted {
                    dest: outcome.dest,
                    files: outcome.files,
                    bytes: outcome.bytes,
                });
            }
            Err(EngineError::Cancelled) => {
                let _ = sender.send(QuickEvent::Cancelled);
            }
            Err(error) if error.is_password_related() => {
                let _ = sender.send(QuickEvent::NeedPassword);
            }
            Err(error) => {
                let _ = sender.send(QuickEvent::Failed(error.to_string()));
            }
        });
        cx.notify();
    }

    fn start_compress(&mut self, cx: &mut Context<Self>) {
        let QuickAction::Compress {
            sources,
            format,
            delete_sources,
        } = self.action.clone()
        else {
            return;
        };
        let Some(first) = sources.first() else {
            return;
        };
        let parent = first
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        // 单选时用文件/文件夹名；多选合并压缩时用所在文件夹名。
        let stem = if sources.len() == 1 {
            let single_stream = format.is_single_stream() && first.is_file();
            let value = if single_stream {
                first.file_name()
            } else {
                first.file_stem()
            };
            value
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_else(|| "archive".to_string())
        } else {
            parent
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "archive".to_string())
        };
        let mut dest = parent.join(format!("{stem}.{}", format.extension()));
        let mut index = 2;
        while dest.exists() {
            dest = parent.join(format!("{stem} ({index}).{}", format.extension()));
            index += 1;
        }
        let options = CreateOptions {
            format,
            level: self.settings.compression.default_level.min(9),
            password: None,
            zip_algorithm: ZipAlgorithm::Deflate,
            include_root: self.settings.compression.include_root,
            exclude: Vec::new(),
            overwrite: false,
            comment: None,
            test_after: self.settings.compression.test_after,
            delete_sources,
            sfx: false,
            volume_size: None,
        };
        self.active_dest = Some(dest.clone());
        let current = if sources.len() == 1 {
            format!("{} → {}", first.display(), dest.display())
        } else {
            format!("{} 个项目 → {}", sources.len(), dest.display())
        };
        self.begin_progress(current, cx);
        let control = self.control.clone();
        let sender = self.sender.clone();
        let sink = QuickSink::new(sender.clone(), control);
        std::thread::spawn(
            move || match create_archive(&sources, &dest, &options, &sink) {
                Ok(_) => {
                    let _ = sender.send(QuickEvent::Compressed { dest });
                }
                Err(EngineError::Cancelled) => {
                    let _ = sender.send(QuickEvent::Cancelled);
                }
                Err(error) => {
                    let _ = sender.send(QuickEvent::Failed(error.to_string()));
                }
            },
        );
        cx.notify();
    }

    /// 保存前合并磁盘上的窗口尺寸，避免旧副本覆盖其他窗口刚保存的尺寸。
    fn save_settings(&mut self) {
        let latest = Settings::load();
        self.settings.ui.window_sizes = latest.ui.window_sizes;
        let _ = self.settings.save();
    }

    fn persist_window_size(&mut self, window: &Window) {
        if let WindowBounds::Windowed(bounds) = window.window_bounds() {
            let mut width = bounds.size.width.as_f32();
            let mut height = bounds.size.height.as_f32();
            // 密码、冲突、完成等步骤会临时调整窗口；关闭时保存不小于基础尺寸的值。
            width = width.max(self.base_size.0);
            height = height.max(self.base_size.1);
            self.settings
                .remember_window_size(self.action.key(), width, height);
            let _ = Settings::persist_window_size(self.action.key(), width, height);
        }
    }

    /// 应用内关闭按钮不会触发 on_window_should_close，关闭前显式保存尺寸。
    fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.control.cancelled.store(true, Ordering::Relaxed);
        self.cancel_pending_prompt();
        self.persist_window_size(window);
        window.remove_window();
        cx.notify();
    }

    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        let paused = !self.paused;
        self.paused = paused;
        self.control.paused.store(paused, Ordering::Relaxed);
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self.control.cancelled.store(true, Ordering::Relaxed);
        self.cancel_pending_prompt();
        cx.notify();
    }

    fn browse_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = PathBuf::from(self.dest_input.read(cx).value().to_string());
        let picked = crate::components::file_dialog_in(window, move || {
            let mut dialog = rfd::FileDialog::new();
            if start.exists() {
                dialog = dialog.set_directory(start);
            }
            dialog.pick_folder()
        });
        if let Some(folder) = picked {
            let value = folder.display().to_string();
            self.dest_input
                .update(cx, |state, cx| state.set_value(value, window, cx));
            cx.notify();
        }
    }

    fn title(&self) -> String {
        match self.step {
            Step::Folder => "选择解压路径".to_string(),
            Step::Password => "输入密码".to_string(),
            Step::Conflict => "重复的文件".to_string(),
            Step::Progress => match self.action {
                QuickAction::ExtractTo { .. } | QuickAction::ExtractPreset { .. } => {
                    format!("{:.0}% 正在解压 {}", self.percent, self.subtitle())
                }
                QuickAction::Compress { .. } => {
                    format!("{:.0}% 正在压缩 {}", self.percent, self.subtitle())
                }
            },
            Step::Done => match self.action {
                QuickAction::ExtractTo { .. } | QuickAction::ExtractPreset { .. } => {
                    format!("已解压 · {}", self.subtitle())
                }
                QuickAction::Compress { .. } => format!("压缩完成 · {}", self.subtitle()),
            },
            Step::Failed => "操作未完成".to_string(),
        }
    }

    fn subtitle(&self) -> String {
        match &self.action {
            QuickAction::ExtractTo { archive } | QuickAction::ExtractPreset { archive, .. } => {
                archive
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default()
            }
            QuickAction::Compress { sources, .. } => match sources.as_slice() {
                [source] => source
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default(),
                _ => format!("{} 个项目", sources.len()),
            },
        }
    }

    fn render_titlebar(&self, window: &Window) -> AnyElement {
        let palette = self.palette;
        let title = self.title();
        let max_icon = if window.is_maximized() {
            IconName::Copy
        } else {
            IconName::Square
        };
        div()
            .flex()
            .items_center()
            .h(px(crate::theme::TITLEBAR_HEIGHT))
            .pl(px(12.0))
            .pr(px(0.0))
            .bg(palette.bar)
            .border_b_1()
            .border_color(palette.line)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag)
                    .child(icon(
                        self.step_icon(),
                        palette.accent,
                        crate::theme::TITLEBAR_ICON_SIZE,
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(crate::theme::font_px(crate::theme::TITLEBAR_FONT_SIZE))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(palette.text)
                            .child(title),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .h_full()
                    .child(self.window_control(
                        "quick-min",
                        IconName::Minus,
                        WindowControlArea::Min,
                        false,
                        crate::theme::TITLEBAR_HEIGHT,
                    ))
                    .child(self.window_control(
                        "quick-max",
                        max_icon,
                        WindowControlArea::Max,
                        false,
                        crate::theme::TITLEBAR_HEIGHT,
                    ))
                    .child(self.window_control(
                        "quick-close",
                        IconName::X,
                        WindowControlArea::Close,
                        true,
                        crate::theme::TITLEBAR_HEIGHT,
                    )),
            )
            .into_any_element()
    }

    fn step_icon(&self) -> IconName {
        match self.step {
            Step::Folder => IconName::FolderOpen,
            Step::Password => IconName::Lock,
            Step::Conflict => IconName::TriangleAlert,
            Step::Progress | Step::Done => match self.action {
                QuickAction::ExtractTo { .. } | QuickAction::ExtractPreset { .. } => {
                    IconName::Download
                }
                QuickAction::Compress { .. } => IconName::Archive,
            },
            Step::Failed => IconName::CircleX,
        }
    }

    fn window_control(
        &self,
        id: &'static str,
        icon_name: IconName,
        area: WindowControlArea,
        danger: bool,
        size: f32,
    ) -> AnyElement {
        let palette = self.palette;
        let control = div()
            .id(id)
            .w(px(size))
            .h(px(size))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(palette.text)
            .when(danger, |this| {
                this.hover(|style| style.bg(palette.bad).text_color(palette.white))
            })
            .when(!danger, |this| this.hover(|style| style.bg(palette.hover)))
            .window_control_area(area)
            .child(Icon::new(icon_name).size(px((size * 0.45).clamp(14.0, 16.0))));

        #[cfg(not(target_os = "windows"))]
        let control = control.on_click(move |_, window, _| match area {
            WindowControlArea::Min => window.minimize_window(),
            WindowControlArea::Max => window.zoom_window(),
            WindowControlArea::Close => window.remove_window(),
            WindowControlArea::Drag => {}
        });
        control.into_any_element()
    }

    fn render_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.step {
            Step::Folder => self.render_folder_step(window, cx),
            Step::Password => self.render_password_step(window, cx),
            Step::Conflict => self.render_conflict_step(cx),
            Step::Progress => self.render_progress_step(cx),
            Step::Done => self.render_result_step(true, cx),
            Step::Failed => self.render_result_step(false, cx),
        }
    }

    fn archive_path(&self) -> Option<&Path> {
        match &self.action {
            QuickAction::ExtractTo { archive } | QuickAction::ExtractPreset { archive, .. } => {
                Some(archive)
            }
            QuickAction::Compress { .. } => None,
        }
    }

    /// 常用位置 + 本机驱动器，用于“选择解压路径”的快速定位列表。
    fn location_entries(&self) -> Vec<(String, PathBuf)> {
        let mut entries: Vec<(String, PathBuf)> = Vec::new();
        let mut push = |name: String, path: PathBuf| {
            if path.is_dir() && !entries.iter().any(|(_, existing)| existing == &path) {
                entries.push((name, path));
            }
        };
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            let profile = PathBuf::from(profile);
            for (label, folder) in [
                ("桌面", "Desktop"),
                ("下载", "Downloads"),
                ("文档", "Documents"),
            ] {
                push(label.to_string(), profile.join(folder));
            }
        }
        if let Some(parent) = self.archive_path().and_then(Path::parent) {
            push("压缩包所在位置".to_string(), parent.to_path_buf());
        }
        for letter in b'A'..=b'Z' {
            let root = PathBuf::from(format!("{}:\\", letter as char));
            if root.is_dir() {
                push(format!("驱动器 {}:", letter as char), root);
            }
        }
        entries
    }

    fn render_folder_step(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let browse = Button::new("quick-browse")
            .label("浏览…")
            .outline()
            .h(px(CONTROL_HEIGHT))
            .px(px(14.0))
            .on_click(cx.listener(|this, _, window, cx| this.browse_folder(window, cx)));
        let current = self
            .dest_input
            .read(cx)
            .value()
            .to_string()
            .trim_end_matches(['\\', '/'])
            .to_ascii_lowercase();
        let mut locations = div()
            .id("quick-locations")
            .h(px(164.0))
            .w_full()
            .rounded(px(8.0))
            .border_1()
            .border_color(palette.line)
            .bg(palette.panel)
            .p(px(5.0))
            .flex()
            .flex_col()
            .gap(px(1.0))
            .overflow_y_scroll();
        for (index, (name, path)) in self.location_entries().into_iter().enumerate() {
            let selected = path
                .to_string_lossy()
                .trim_end_matches(['\\', '/'])
                .eq_ignore_ascii_case(&current);
            let click_path = path.clone();
            let accent = palette.accent;
            let sub = palette.sub;
            locations = locations.child(
                div()
                    .id(("quick-location", index))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(9.0))
                    .py(px(6.0))
                    .rounded(px(crate::theme::RADIUS_CONTROL))
                    .cursor_pointer()
                    .when(selected, |this| this.bg(palette.accent_soft))
                    .hover(|style| style.bg(palette.hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let value = click_path.display().to_string();
                        this.dest_input.update(cx, |state, cx| {
                            state.set_value(value, window, cx);
                        });
                        cx.notify();
                    }))
                    .child(icon(
                        IconName::FolderOpen,
                        if selected { accent } else { sub },
                        14.0,
                    ))
                    .child(
                        div()
                            .text_size(crate::theme::caption())
                            .text_color(palette.text)
                            .child(name),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .max_w(px(240.0))
                            .truncate()
                            .text_size(crate::theme::caption())
                            .text_color(palette.muted)
                            .child(path.display().to_string()),
                    ),
            );
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .p(px(22.0))
            .gap(px(12.0))
            .child(
                div()
                    .text_size(crate::theme::caption())
                    .text_color(palette.sub)
                    .child("选择解压目标位置。默认解压到与压缩包同名的文件夹。"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(7.0))
                    .child(
                        div()
                            .text_size(crate::theme::caption())
                            .text_color(palette.sub)
                            .child("目标路径"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(control_height(Input::new(&self.dest_input))),
                            )
                            .child(browse),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(7.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(crate::theme::caption())
                                    .text_color(palette.sub)
                                    .child("常用位置"),
                            )
                            .child(
                                div()
                                    .text_size(crate::theme::caption())
                                    .text_color(palette.muted)
                                    .child("点击快速定位"),
                            ),
                    )
                    .child(locations),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(self.check_row(
                        "quick-same-folder",
                        "解压到与压缩包同名的文件夹",
                        self.same_name_folder,
                        cx,
                    ))
                    .child(self.check_row(
                        "quick-delete-archive",
                        "解压后删除压缩文件",
                        self.delete_archive,
                        cx,
                    ))
                    .child(self.check_row(
                        "quick-open-after",
                        "解压后打开目标文件夹",
                        self.open_after,
                        cx,
                    ))
                    .child(self.check_row("quick-remember", "记住设置", self.remember, cx)),
            )
            .child(
                div()
                    .text_size(crate::theme::caption())
                    .line_height(relative(1.5))
                    .text_color(palette.muted)
                    .child("勾选「记住设置」后，下次「解压到其他文件夹…」将直接沿用并跳过本窗口。"),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(
                    div()
                        .text_size(crate::theme::caption())
                        .text_color(palette.bad)
                        .child(error),
                )
            })
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(10.0))
                    .child(
                        Button::new("quick-cancel")
                            .label("取消")
                            .outline()
                            .h(px(CONTROL_HEIGHT))
                            .px(px(16.0))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_window(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("quick-ok")
                            .label("确定")
                            .primary()
                            .h(px(CONTROL_HEIGHT))
                            .px(px(18.0))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.submit_folder(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_password_step(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let error = self.password_error.clone();
        let prompt = self
            .pending_password_prompt
            .as_ref()
            .map(|(_, prompt)| prompt.clone());
        let description = prompt
            .as_ref()
            .filter(|prompt| prompt.attempt <= 1)
            .map(|prompt| prompt.message.clone())
            .unwrap_or_else(|| "输入加密文件的密码。".to_string());
        let archive_name = prompt
            .as_ref()
            .and_then(|prompt| prompt.archive.file_name())
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| self.subtitle());

        let book: Vec<(String, String)> = self
            .password_book
            .entries()
            .iter()
            .map(|entry| (entry.note.clone(), entry.password.clone()))
            .collect();
        let book_count = book.len();
        let reveal = self.password_reveal;
        let picker_open = self.password_picker_open;
        let mut picker = div().flex().flex_col().gap(px(7.0)).child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .debug_selector(|| "quick-pwd-book-toggle".to_string())
                        .child(
                            Button::new("quick-pwd-book")
                                .label(format!("从密码本选择（{book_count}）"))
                                .icon(icon(IconName::BookOpen, palette.accent, 15.0))
                                .outline()
                                .h(px(30.0))
                                .px(px(12.0))
                                .text_size(crate::theme::caption())
                                .disabled(book_count == 0)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.password_picker_open = !this.password_picker_open;
                                    this.resize_for_step(cx);
                                    cx.notify();
                                })),
                        ),
                )
                .child(crate::design::icon_action(
                    &palette,
                    "quick-pwd-reveal",
                    if reveal {
                        IconName::EyeOff
                    } else {
                        IconName::Eye
                    },
                    palette.sub,
                    book_count == 0,
                    if reveal {
                        "隐藏密码"
                    } else {
                        "显示密码"
                    },
                    cx.listener(|this, _, _, cx| {
                        this.password_reveal = !this.password_reveal;
                        cx.notify();
                    }),
                )),
        );
        if book_count == 0 {
            picker = picker.child(
                div()
                    .text_size(crate::theme::caption())
                    .text_color(palette.muted)
                    .child("密码本为空，可先在主窗口的密码本页面添加常用密码。"),
            );
        } else if picker_open {
            let mut list = div()
                .id("quick-password-book-list")
                .flex()
                .flex_col()
                .max_h(px(150.0))
                .overflow_y_scroll()
                .rounded(px(crate::theme::RADIUS_CONTROL))
                .border_1()
                .border_color(palette.line)
                .bg(palette.panel);
            for (index, (note, password)) in book.iter().enumerate() {
                let label = if note.is_empty() {
                    format!("密码 {}", index + 1)
                } else {
                    note.clone()
                };
                let shown = if reveal {
                    password.clone()
                } else {
                    "••••••••".to_string()
                };
                list = list.child(
                    div()
                        .id(format!("quick-password-book-{index}"))
                        .debug_selector(|| format!("quick-password-book-{index}"))
                        .role(Role::Button)
                        .aria_label(label.clone())
                        .cursor_pointer()
                        .h(px(32.0))
                        .px(px(10.0))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .when(index + 1 < book_count, |this| {
                            this.border_b_1().border_color(palette.line)
                        })
                        .hover(|style| style.bg(palette.hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_book_password(index, window, cx);
                        }))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(crate::theme::caption())
                                .text_color(palette.text)
                                .child(label),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .font_family("Cascadia Mono")
                                .text_size(crate::theme::caption())
                                .text_color(palette.muted)
                                .child(shown),
                        ),
                );
            }
            picker = picker.child(list).child(
                div()
                    .text_size(crate::theme::caption())
                    .text_color(palette.muted)
                    .child("点击密码即可直接尝试。"),
            );
        }

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .p(px(22.0))
            .gap(px(14.0))
            .child(
                div()
                    .text_size(crate::theme::body())
                    .text_color(palette.text)
                    .child(description),
            )
            .when_some(error, |this, error| {
                this.child(
                    div()
                        .text_size(crate::theme::caption())
                        .text_color(palette.bad)
                        .child(error),
                )
            })
            .when(!picker_open, |this| {
                this.child(
                    div()
                        .rounded(px(crate::theme::RADIUS_OVERLAY))
                        .border_1()
                        .border_color(palette.line)
                        .bg(palette.panel)
                        .p(px(12.0))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .child(icon(IconName::Lock, palette.accent, 18.0))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.0))
                                .child(
                                    div()
                                        .text_size(crate::theme::caption())
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(palette.text)
                                        .child("该压缩包已加密"),
                                )
                                .child(
                                    div()
                                        .text_size(crate::theme::caption())
                                        .text_color(palette.muted)
                                        .child("输入密码后才能继续解压，密码只用于本次操作。"),
                                ),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .w(px(70.0))
                            .text_size(crate::theme::body())
                            .text_color(palette.sub)
                            .child("文件名"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family("Cascadia Mono")
                            .text_size(crate::theme::caption())
                            .text_color(palette.text)
                            .child(archive_name),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(7.0))
                    .child(
                        div()
                            .text_size(crate::theme::caption())
                            .text_color(palette.sub)
                            .child("密码"),
                    )
                    .child(control_height(Input::new(&self.password_input))),
            )
            .child(picker)
            .child(
                div()
                    .text_size(crate::theme::caption())
                    .text_color(palette.muted)
                    .child("提示：输入错误会停留在本窗口，可重新输入或取消。"),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        Button::new("quick-pwd-cancel")
                            .label("取消")
                            .outline()
                            .h(px(CONTROL_HEIGHT))
                            .px(px(16.0))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_window(window, cx)),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("quick-pwd-ok")
                            .label("确定")
                            .primary()
                            .h(px(CONTROL_HEIGHT))
                            .px(px(18.0))
                            .on_click(cx.listener(|this, _, _, cx| this.submit_password(cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_smart_conflict_step(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let index = self.smart_conflict_index;
        let total = self.smart_conflicts.len();
        let Some(conflict) = self.smart_conflicts.get(index).cloned() else {
            return div().into_any_element();
        };
        let choice = self.conflict_choice;
        let time_panel = |title: &'static str, size: u64, modified: Option<i64>| {
            div()
                .flex_1()
                .min_w_0()
                .p(px(12.0))
                .rounded(px(crate::theme::RADIUS_CONTROL))
                .border_1()
                .border_color(palette.line)
                .bg(palette.panel)
                .flex()
                .flex_col()
                .gap(px(7.0))
                .child(
                    div()
                        .text_size(crate::theme::caption())
                        .text_color(palette.muted)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(crate::theme::body())
                        .text_color(palette.text)
                        .child(format_size(size)),
                )
                .child(
                    div()
                        .text_size(crate::theme::caption())
                        .text_color(palette.sub)
                        .child(conflict_time(modified)),
                )
        };
        let rename_label = format!(
            "重命名(R)：{}",
            conflict
                .rename_path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| conflict.name.clone())
        );
        let apply_entity = cx.entity();
        let checkbox = div()
            .id("smart-apply-all")
            .role(Role::CheckBox)
            .aria_label("应用到全部文件(A)")
            .aria_toggled(if self.smart_apply_all {
                Toggled::True
            } else {
                Toggled::False
            })
            .flex()
            .items_center()
            .gap(px(9.0))
            .cursor_pointer()
            .text_size(crate::theme::body())
            .text_color(palette.text)
            .on_click(move |_, _, app| {
                apply_entity.update(app, |this, cx| {
                    this.smart_apply_all = !this.smart_apply_all;
                    cx.notify();
                });
            })
            .child(crate::design::check_box_visual(
                &palette,
                self.smart_apply_all,
                false,
            ))
            .child("应用到全部文件(A)");

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .p(px(22.0))
            .gap(px(12.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(crate::theme::body())
                            .text_color(palette.sub)
                            .child("存在相同的文件"),
                    )
                    .when(total > 1, |this| {
                        this.child(
                            div()
                                .text_size(crate::theme::caption())
                                .text_color(palette.muted)
                                .child(format!("{} / {}", index + 1, total)),
                        )
                    }),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .truncate()
                    .font_family("Cascadia Mono")
                    .text_size(crate::theme::caption())
                    .text_color(palette.accent)
                    .child(conflict.target_path.display().to_string()),
            )
            .child(
                div()
                    .flex()
                    .gap(px(12.0))
                    .child(time_panel(
                        "现有文件",
                        conflict.existing_size,
                        conflict.existing_modified,
                    ))
                    .child(time_panel(
                        "要解压的文件",
                        conflict.incoming_size,
                        conflict.incoming_modified,
                    )),
            )
            .child(
                div()
                    .text_size(crate::theme::caption())
                    .text_color(palette.sub)
                    .child("选择以下选项："),
            )
            .child(self.smart_choice_row(
                "smart-skip",
                "跳过(S)".to_string(),
                ConflictChoice::Skip,
                choice == ConflictChoice::Skip,
                cx,
            ))
            .child(self.smart_choice_row(
                "smart-overwrite",
                "覆盖(O)".to_string(),
                ConflictChoice::Overwrite,
                choice == ConflictChoice::Overwrite,
                cx,
            ))
            .child(self.smart_choice_row(
                "smart-rename",
                rename_label,
                ConflictChoice::Rename,
                choice == ConflictChoice::Rename,
                cx,
            ))
            .child(checkbox)
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(10.0))
                    .child(
                        Button::new("smart-conflict-cancel")
                            .label("取消")
                            .outline()
                            .h(px(CONTROL_HEIGHT))
                            .px(px(16.0))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_window(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("smart-conflict-ok")
                            .label("确定")
                            .primary()
                            .h(px(CONTROL_HEIGHT))
                            .px(px(18.0))
                            .on_click(cx.listener(|this, _, _, cx| this.submit_smart_conflict(cx))),
                    ),
            )
            .into_any_element()
    }

    fn smart_choice_row(
        &self,
        id: &'static str,
        label: String,
        choice: ConflictChoice,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let entity = cx.entity();
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label.clone())
            .flex()
            .items_center()
            .gap(px(8.0))
            .cursor_pointer()
            .text_size(crate::theme::body())
            .text_color(if selected {
                palette.accent
            } else {
                palette.text
            })
            .on_click(move |_, _, app| {
                entity.update(app, |this, cx| {
                    this.conflict_choice = choice;
                    cx.notify();
                });
            })
            .child(
                div()
                    .w(px(17.0))
                    .h(px(17.0))
                    .flex_none()
                    .rounded(px(99.0))
                    .border_1()
                    .border_color(if selected {
                        palette.accent
                    } else {
                        palette.line_strong
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |this| {
                        this.child(
                            div()
                                .w(px(9.0))
                                .h(px(9.0))
                                .rounded(px(99.0))
                                .bg(palette.accent),
                        )
                    }),
            )
            .child(label)
            .into_any_element()
    }

    fn render_conflict_step(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if !self.smart_conflicts.is_empty() {
            return self.render_smart_conflict_step(cx);
        }
        let palette = self.palette;
        let conflict = self.conflict.clone();
        let choice = self.conflict_choice;
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .p(px(22.0))
            .gap(px(14.0))
            .child(
                div()
                    .text_size(crate::theme::body())
                    .text_color(palette.text)
                    .child("目标文件夹中已存在同名文件。"),
            )
            .when_some(conflict, |this, conflict| {
                this.child(
                    div()
                        .px(px(12.0))
                        .py(px(9.0))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(palette.line)
                        .bg(palette.panel)
                        .font_family("Cascadia Mono")
                        .text_size(crate::theme::caption())
                        .text_color(palette.sub)
                        .child(conflict.name.clone()),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(12.0))
                        .child(
                            div()
                                .flex_1()
                                .p(px(12.0))
                                .rounded(px(crate::theme::RADIUS_CONTROL))
                                .border_1()
                                .border_color(palette.line)
                                .bg(palette.panel)
                                .child(div().text_size(crate::theme::caption()).text_color(palette.muted).child(
                                    format!("现有文件 · {}", format_size(conflict.existing_size)),
                                )),
                        )
                        .child(
                            div()
                                .flex_1()
                                .p(px(12.0))
                                .rounded(px(crate::theme::RADIUS_CONTROL))
                                .border_1()
                                .border_color(palette.line)
                                .bg(palette.panel)
                                .child(div().text_size(crate::theme::caption()).text_color(palette.muted).child(
                                    format!(
                                        "压缩包中的文件 · {}",
                                        format_size(conflict.incoming_size)
                                    ),
                                )),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .gap(px(18.0))
                    .child(self.radio_row("quick-skip", "跳过", choice == ConflictChoice::Skip, cx))
                    .child(self.radio_row(
                        "quick-overwrite",
                        "覆盖",
                        choice == ConflictChoice::Overwrite,
                        cx,
                    ))
                    .child(self.radio_row(
                        "quick-rename",
                        "重命名",
                        choice == ConflictChoice::Rename,
                        cx,
                    )),
            )
            .child(
                div()
                    .text_size(crate::theme::caption())
                    .line_height(relative(1.5))
                    .text_color(palette.muted)
                    .child("跳过：只解压不冲突的文件；覆盖：用压缩包内文件替换；重命名：解压到新的目标文件夹。"),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(10.0))
                    .child(
                        Button::new("quick-conflict-cancel")
                            .label("取消")
                            .outline()
                            .h(px(CONTROL_HEIGHT))
                            .px(px(16.0))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_window(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("quick-conflict-ok")
                            .label("确定")
                            .primary()
                            .h(px(CONTROL_HEIGHT))
                            .px(px(18.0))
                            .on_click(cx.listener(|this, _, _, cx| this.start_extract(cx))),
                    ),
            )
            .into_any_element()
    }

    fn open_icon_button(
        &self,
        id: &'static str,
        icon_name: IconName,
        path: Option<PathBuf>,
    ) -> AnyElement {
        let palette = self.palette;
        let enabled = path.is_some();
        div()
            .id(id)
            .w(px(32.0))
            .h(px(32.0))
            .rounded(px(8.0))
            .flex()
            .items_center()
            .justify_center()
            .when(enabled, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(palette.hover))
                    .on_click(move |_, _, _| {
                        if let Some(path) = path.clone() {
                            let _ = open::that(path);
                        }
                    })
            })
            .child(icon(
                icon_name,
                if enabled { palette.sub } else { palette.muted },
                16.0,
            ))
            .into_any_element()
    }

    fn render_progress_step(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let percent = self.percent;
        let current = self.current.clone();
        let paused = self.paused;
        let elapsed = format_elapsed(self.elapsed_secs);
        let archive_name = self.subtitle();
        let archive_dir = match &self.action {
            QuickAction::ExtractTo { archive } | QuickAction::ExtractPreset { archive, .. } => {
                archive.parent().map(Path::to_path_buf)
            }
            QuickAction::Compress { sources, .. } => sources
                .first()
                .and_then(|source| source.parent())
                .map(Path::to_path_buf),
        };
        let dest = self.active_dest.clone();
        let total_entries = self.total_entries;
        let source_count = match &self.action {
            QuickAction::Compress { sources, .. } => sources.len(),
            _ => 0,
        };
        let is_extract = matches!(
            self.action,
            QuickAction::ExtractTo { .. } | QuickAction::ExtractPreset { .. }
        );
        let bar_color = if is_extract {
            palette.ok
        } else {
            palette.accent
        };
        let status = if paused {
            "已暂停"
        } else if is_extract {
            "正在解压"
        } else {
            "正在压缩"
        };
        let item_count = if is_extract {
            total_entries
        } else {
            source_count
        };
        let footer = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(20.0))
            .py(px(12.0))
            .border_t_1()
            .border_color(palette.line)
            .bg(palette.bar)
            .child(self.open_icon_button(
                "quick-open-archive",
                IconName::Archive,
                archive_dir.clone(),
            ))
            .child(self.open_icon_button("quick-open-dest", IconName::FolderOpen, dest.clone()))
            .child(div().flex_1())
            .child(
                Button::new("quick-pause")
                    .label(if paused { "继续" } else { "暂停" })
                    .outline()
                    .h(px(CONTROL_HEIGHT))
                    .px(px(16.0))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx))),
            )
            .child(
                Button::new("quick-stop")
                    .label("停止")
                    .outline()
                    .h(px(CONTROL_HEIGHT))
                    .px(px(16.0))
                    .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
            );
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .id("quick-progress-body")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(20.0))
                    .gap(px(11.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(crate::theme::caption())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(palette.sub)
                                    .child("总计"),
                            )
                            .child(
                                div()
                                    .text_size(crate::theme::caption())
                                    .text_color(palette.muted)
                                    .child(format!("已用 {elapsed}")),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(10.0))
                            .child(
                                div()
                                    .text_size(crate::theme::title())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(palette.text)
                                    .child(format!("{percent:.0}%")),
                            )
                            .child(
                                div()
                                    .text_size(crate::theme::caption())
                                    .text_color(palette.sub)
                                    .child(status),
                            ),
                    )
                    .child(thick_bar(percent, 18.0, bar_color, &palette))
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_between()
                            .mt(px(4.0))
                            .child(
                                div()
                                    .text_size(crate::theme::caption())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(palette.sub)
                                    .child("当前文件"),
                            )
                            .child(
                                div()
                                    .text_size(crate::theme::caption())
                                    .text_color(palette.muted)
                                    .child(format!("{item_count} 项")),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .px(px(12.0))
                            .py(px(9.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(palette.line)
                            .bg(palette.panel)
                            .font_family("Cascadia Mono")
                            .text_size(crate::theme::caption())
                            .text_color(palette.sub)
                            .truncate()
                            .child(current),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .rounded(px(crate::theme::RADIUS_OVERLAY))
                            .border_1()
                            .border_color(palette.line)
                            .p(px(11.0))
                            .flex()
                            .flex_col()
                            .gap(px(5.0))
                            .child(detail_row(
                                &palette,
                                if is_extract { "压缩包" } else { "源文件" },
                                archive_name,
                            ))
                            .child(detail_row(
                                &palette,
                                "输出位置",
                                dest.as_ref()
                                    .map(|path| path.display().to_string())
                                    .unwrap_or_else(|| "—".to_string()),
                            )),
                    ),
            )
            .child(footer)
            .into_any_element()
    }

    fn render_result_step(&mut self, success: bool, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let title = if success {
            match self.action {
                QuickAction::ExtractTo { .. } | QuickAction::ExtractPreset { .. } => "解压完成",
                QuickAction::Compress { .. } => "压缩完成",
            }
        } else {
            "操作未完成"
        };
        let (color, soft, icon_name) = if success {
            (palette.ok, palette.ok_soft, IconName::CircleCheck)
        } else {
            (palette.bad, palette.bad_soft, IconName::CircleX)
        };
        let detail = self
            .result_text
            .clone()
            .or_else(|| self.error.clone())
            .unwrap_or_default();
        let open = self.result_open.clone();
        let elapsed = format_elapsed(self.elapsed_secs);
        let footer = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(22.0))
            .py(px(12.0))
            .border_t_1()
            .border_color(palette.line)
            .bg(palette.bar)
            .child(div().flex_1())
            .when_some(open.clone(), |this, path| {
                this.child(
                    Button::new("quick-open-folder")
                        .label("打开所在位置")
                        .outline()
                        .h(px(CONTROL_HEIGHT))
                        .px(px(16.0))
                        .on_click(cx.listener(move |_, _, _, _| {
                            let _ = open::that(&path);
                        })),
                )
            })
            .child(
                Button::new("quick-done-close")
                    .label(if success { "完成" } else { "关闭" })
                    .primary()
                    .h(px(CONTROL_HEIGHT))
                    .px(px(18.0))
                    .on_click(cx.listener(|this, _, window, cx| this.close_window(window, cx))),
            );

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .id("quick-result-body")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(22.0))
                    .gap(px(14.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .child(
                                div()
                                    .w(px(48.0))
                                    .h(px(48.0))
                                    .flex_none()
                                    .rounded(px(crate::theme::RADIUS_OVERLAY))
                                    .bg(soft)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon(icon_name, color, 24.0)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.0))
                                    .child(
                                        div()
                                            .text_size(crate::theme::subtitle())
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(palette.text)
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .text_size(crate::theme::caption())
                                            .text_color(palette.sub)
                                            .truncate()
                                            .child(self.subtitle()),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .rounded(px(crate::theme::RADIUS_OVERLAY))
                            .border_1()
                            .border_color(palette.line)
                            .bg(if success {
                                palette.ok_soft
                            } else {
                                palette.bad_soft
                            })
                            .px(px(13.0))
                            .py(px(10.0))
                            .flex()
                            .items_center()
                            .gap(px(9.0))
                            .child(icon(icon_name, color, 16.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(crate::theme::caption())
                                    .text_color(if success { palette.ok } else { palette.bad })
                                    .child(detail),
                            ),
                    )
                    .child(
                        div()
                            .rounded(px(crate::theme::RADIUS_OVERLAY))
                            .border_1()
                            .border_color(palette.line)
                            .p(px(11.0))
                            .flex()
                            .flex_col()
                            .gap(px(5.0))
                            .child(detail_row(&palette, "用时", elapsed))
                            .child(detail_row(
                                &palette,
                                "位置",
                                open.as_ref()
                                    .map(|path| path.display().to_string())
                                    .unwrap_or_else(|| "—".to_string()),
                            )),
                    ),
            )
            .child(footer)
            .into_any_element()
    }

    fn check_row(
        &self,
        id: &'static str,
        label: &'static str,
        checked: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let entity = cx.entity();
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .flex()
            .items_center()
            .gap(px(9.0))
            .cursor_pointer()
            .text_size(crate::theme::body())
            .text_color(palette.text)
            .on_click(move |_, _, app| {
                entity.update(app, |this, cx| {
                    match id {
                        "quick-same-folder" => this.same_name_folder = !this.same_name_folder,
                        "quick-delete-archive" => this.delete_archive = !this.delete_archive,
                        "quick-open-after" => this.open_after = !this.open_after,
                        "quick-remember" => this.remember = !this.remember,
                        _ => {}
                    }
                    cx.notify();
                });
            })
            .child(crate::design::check_box_visual(&palette, checked, false))
            .child(label)
            .into_any_element()
    }

    fn radio_row(
        &self,
        id: &'static str,
        label: &'static str,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let entity = cx.entity();
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .flex()
            .items_center()
            .gap(px(8.0))
            .cursor_pointer()
            .text_size(crate::theme::body())
            .text_color(if selected {
                palette.accent
            } else {
                palette.text
            })
            .on_click(move |_, _, app| {
                entity.update(app, |this, cx| {
                    this.conflict_choice = match id {
                        "quick-skip" => ConflictChoice::Skip,
                        "quick-rename" => ConflictChoice::Rename,
                        _ => ConflictChoice::Overwrite,
                    };
                    cx.notify();
                });
            })
            .child(
                div()
                    .w(px(17.0))
                    .h(px(17.0))
                    .flex_none()
                    .rounded(px(99.0))
                    .border_1()
                    .border_color(if selected {
                        palette.accent
                    } else {
                        palette.line_strong
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |this| {
                        this.child(
                            div()
                                .w(px(9.0))
                                .h(px(9.0))
                                .rounded(px(99.0))
                                .bg(palette.accent),
                        )
                    }),
            )
            .child(label)
            .into_any_element()
    }
}

impl Render for QuickView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette;
        let title = self.title();
        window.set_window_title(&title);
        let titlebar = self.custom_titlebar.then(|| self.render_titlebar(window));
        div()
            .id("quick-root")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key.eq_ignore_ascii_case("escape") {
                    if matches!(this.step, Step::Progress) {
                        this.control.cancelled.store(true, Ordering::Relaxed);
                    } else {
                        this.close_window(window, cx);
                    }
                }
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(palette.bg)
            .text_color(palette.text)
            .font_family(crate::theme::ui_font_family())
            .when_some(titlebar, |this, titlebar| this.child(titlebar))
            .child(self.render_body(window, cx))
            .into_any_element()
    }
}

fn conflict_rename_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let extension = path.extension().and_then(|extension| extension.to_str());
    rarkzip_engine::paths::unique_path(parent, &stem, extension)
}

fn conflict_time(timestamp: Option<i64>) -> String {
    use chrono::{Local, TimeZone};
    match timestamp {
        Some(ts) if ts > 0 => Local
            .timestamp_opt(ts, 0)
            .single()
            .map(|date| date.format("%Y/%m/%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "—".to_string()),
        _ => "—".to_string(),
    }
}

fn plain_path(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

fn format_elapsed(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

fn detail_row(palette: &Palette, label: &'static str, value: String) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .child(
            div()
                .w(px(64.0))
                .flex_none()
                .text_size(crate::theme::caption())
                .text_color(palette.muted)
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(crate::theme::caption())
                .text_color(palette.sub)
                .child(value),
        )
        .into_any_element()
}

fn thick_bar(percent: f32, height: f32, color: Hsla, palette: &Palette) -> AnyElement {
    let width = (percent / 100.0).clamp(0.0, 1.0);
    div()
        .flex_shrink_0()
        .h(px(height))
        .w_full()
        .rounded(px(crate::theme::RADIUS_CONTROL))
        .bg(palette.panel)
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .w(relative(width))
                .rounded(px(crate::theme::RADIUS_CONTROL))
                .bg(color),
        )
        .into_any_element()
}

fn unique_dir(dir: &Path) -> PathBuf {
    if !dir.exists() {
        return dir.to_path_buf();
    }
    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "extract".to_string());
    let parent = dir.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut index = 2;
    loop {
        let candidate = parent.join(format!("{name} ({index})"));
        if !candidate.exists() {
            return candidate;
        }
        index += 1;
    }
}

#[cfg(test)]
mod ui_tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use gpui_kit::{point, AppContext, Modifiers, TestAppContext, VisualTestContext};
    use rarkzip_engine::PasswordBook;

    use super::{ExtractPreset, QuickAction, QuickView, Step};

    struct LocalAppDataGuard {
        previous: Option<std::ffi::OsString>,
    }

    impl LocalAppDataGuard {
        fn set(path: &Path) -> Self {
            let previous = std::env::var_os("LOCALAPPDATA");
            std::env::set_var("LOCALAPPDATA", path);
            Self { previous }
        }
    }

    impl Drop for LocalAppDataGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var("LOCALAPPDATA", value),
                None => std::env::remove_var("LOCALAPPDATA"),
            }
        }
    }

    fn pump(visual: &mut VisualTestContext) {
        visual.executor().advance_clock(Duration::from_millis(100));
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
    }

    /// 真实密码弹窗 UI：展开密码本列表，点击条目后自动提交并完成解压。
    #[cfg(debug_assertions)]
    #[gpui_kit::test]
    fn password_prompt_book_picker_selects_entry(cx: &mut TestAppContext) {
        let _serial = crate::UI_TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempfile::tempdir().unwrap();
        let _env = LocalAppDataGuard::set(temp.path());
        cx.update(gpui_kit::init);

        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../rarkzip-engine/tests/fixtures/crypted.rar");
        let archive = temp.path().join("crypted.rar");
        std::fs::copy(&fixture, &archive).unwrap();

        let mut book = PasswordBook::load(temp.path().join("passwords.json"));
        book.add("unrar", "测试密码");
        book.add("wrong-pass", "备用密码");

        let action = QuickAction::ExtractPreset {
            archive: archive.clone(),
            preset: ExtractPreset::Here,
            delete: false,
        };
        let (view, visual) = cx
            .add_window_view(move |window, cx| QuickView::new_with_book(window, cx, action, book));

        // 等后台解压线程发现 RAR 需要密码并切到密码步骤。
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            pump(visual);
            if visual.read_entity(&view, |view, _| view.step) == Step::Password {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "等待密码弹窗超时，当前步骤：{:?}",
                visual.read_entity(&view, |view, _| view.step)
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            visual.read_entity(&view, |view, _| view.password_reveal),
            "密码本列表应默认显示明文"
        );

        // 展开“从密码本选择”。
        let toggle = visual
            .debug_bounds("quick-pwd-book-toggle")
            .expect("密码本选择按钮未渲染");
        visual.simulate_click(
            point(toggle.center().x, toggle.center().y),
            Modifiers::default(),
        );
        pump(visual);
        assert!(
            visual.read_entity(&view, |view, _| view.password_picker_open),
            "密码本列表未展开"
        );

        // 点击第一条密码（测试密码本中为 unrar），应自动填入并提交。
        let row = visual
            .debug_bounds("quick-password-book-0")
            .expect("密码本条目未渲染");
        visual.simulate_click(point(row.center().x, row.center().y), Modifiers::default());
        pump(visual);
        assert_eq!(
            visual.read_entity(&view, |view, _| view.password.clone()),
            Some("unrar".to_string()),
            "选择密码后未填入并提交密码"
        );

        // 密码被引擎接受后，crypted.rar 中的 .gitignore 应解压到临时目录。
        let output = temp.path().join(".gitignore");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !output.exists() {
            assert!(
                Instant::now() < deadline,
                "选择密码后解压超时，skipped：{:?}",
                visual.read_entity(&view, |view, _| view.error.clone())
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(output.is_file());
    }
}
