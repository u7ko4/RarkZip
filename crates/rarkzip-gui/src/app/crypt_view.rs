use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::{unbounded, Receiver, Sender};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Disableable;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rarkzip_crypt::algorithm::Algorithm;
use rarkzip_crypt::file::{
    decrypt_file, encrypt_file, FileCompression, FileDecryptOptions, FileEncryptOptions,
};
use rarkzip_crypt::format;
use rarkzip_crypt::progress::{Phase, Progress, Reporter};
use rarkzip_crypt::stego::{embed_text_in_jpeg, extract_text_from_jpeg, StegoOptions};
use rarkzip_crypt::text::{decrypt_text, encrypt_text, TextEncoding, TextEncryptOptions};

use crate::components::{self, icon, IconName};
use crate::design;
use crate::i18n::{tr, Language};
use crate::theme::{self, Palette};

use super::{DropdownKind, MainView, ToastKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CryptTab {
    Files,
    Text,
    Stego,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CryptFileMode {
    Encrypt,
    Decrypt,
}

#[derive(Debug, Clone)]
enum CryptFileStatus {
    Pending,
    Done(PathBuf),
    Error(String),
}

#[derive(Debug, Clone)]
pub(crate) struct CryptFileItem {
    path: PathBuf,
    is_dir: bool,
    mode: CryptFileMode,
    status: CryptFileStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CryptJob {
    Files,
    TextEncrypt,
    TextDecrypt,
    StegoEmbed,
    StegoExtract,
}

enum CryptEvent {
    Progress {
        run_id: u64,
        phase: Phase,
        done: u64,
        total: u64,
        current: Option<String>,
    },
    FileDone {
        run_id: u64,
        index: usize,
        result: std::result::Result<PathBuf, String>,
    },
    TextDone {
        run_id: u64,
        result: std::result::Result<(Option<String>, Option<String>), String>,
    },
    StegoDone {
        run_id: u64,
        result: std::result::Result<PathBuf, String>,
    },
    StegoExtracted {
        run_id: u64,
        result: std::result::Result<String, String>,
    },
    Finished {
        run_id: u64,
        error: Option<String>,
    },
}

pub(crate) struct CryptState {
    tab: CryptTab,
    password: Entity<InputState>,
    confirm: Entity<InputState>,
    show_password: bool,
    pub(crate) algorithm: Algorithm,
    pub(crate) encoding: TextEncoding,
    text_compress: bool,
    add_space: u8,
    plaintext: Entity<TextareaState>,
    ciphertext: Entity<TextareaState>,
    pending_plaintext: Option<String>,
    pending_ciphertext: Option<String>,
    files: Vec<CryptFileItem>,
    output_dir: Option<PathBuf>,
    file_compression: FileCompression,
    job: Option<CryptJob>,
    progress_done: u64,
    progress_total: u64,
    status: String,
    error: Option<String>,
    pub(crate) cancel: Arc<AtomicBool>,
    run_id: u64,
    sender: Sender<CryptEvent>,
    receiver: Receiver<CryptEvent>,
    carrier: Option<PathBuf>,
    stego_output: Option<PathBuf>,
    stego_quality: u8,
    stego_max_side: u32,
    stego_result: Option<PathBuf>,
    stego_error: Option<String>,
}

struct CryptReporter {
    sender: Sender<CryptEvent>,
    run_id: u64,
    pub(crate) cancel: Arc<AtomicBool>,
}

impl Reporter for CryptReporter {
    fn progress(&self, progress: Progress) {
        let _ = self.sender.send(CryptEvent::Progress {
            run_id: self.run_id,
            phase: progress.phase,
            done: progress.done,
            total: progress.total,
            current: progress.current,
        });
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

impl MainView {
    pub(crate) fn init_crypt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        sources: Vec<PathBuf>,
    ) {
        let settings = self.settings.crypt.clone();
        let algorithm = Algorithm::from_code(settings.algorithm)
            .filter(|algorithm| Algorithm::UI.contains(algorithm))
            .unwrap_or(Algorithm::Aes256);
        let encoding = match settings.text_encoding {
            1 => TextEncoding::Base32,
            2 => TextEncoding::Base10,
            _ => TextEncoding::Base64,
        };
        let file_compression = match settings.file_compression {
            1 => FileCompression::Always,
            2 => FileCompression::Never,
            _ => FileCompression::Auto,
        };
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(tr(self.language, "密码"))
        });
        let confirm = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(tr(self.language, "确认密码"))
        });
        let plaintext = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(tr(self.language, "输入或粘贴要加密的文本"))
                .soft_wrap(true)
        });
        let ciphertext = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(tr(self.language, "输入或粘贴要解密的文本"))
                .soft_wrap(true)
        });
        let (sender, receiver) = unbounded();
        let mut state = CryptState {
            tab: CryptTab::Files,
            password,
            confirm,
            show_password: false,
            algorithm,
            encoding,
            text_compress: settings.text_compression,
            add_space: settings.add_space_every.min(10),
            plaintext,
            ciphertext,
            pending_plaintext: None,
            pending_ciphertext: None,
            files: Vec::new(),
            output_dir: settings
                .output_dir
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from),
            file_compression,
            job: None,
            progress_done: 0,
            progress_total: 0,
            status: tr(self.language, "就绪").to_string(),
            error: None,
            cancel: Arc::new(AtomicBool::new(false)),
            run_id: 0,
            sender,
            receiver,
            carrier: None,
            stego_output: None,
            stego_quality: settings.stego_quality.clamp(1, 100),
            stego_max_side: settings.stego_max_side,
            stego_result: None,
            stego_error: None,
        };
        for path in sources {
            push_crypt_file(&mut state, path);
        }
        self.crypt_state = Some(state);
        cx.notify();
    }

    pub(crate) fn drain_crypt_events(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(state) = &self.crypt_state else {
            return false;
        };
        let mut events = Vec::new();
        while let Ok(event) = state.receiver.try_recv() {
            events.push(event);
        }
        let handled = !events.is_empty();
        for event in events {
            self.handle_crypt_event(event, cx);
        }
        handled
    }

    fn handle_crypt_event(&mut self, event: CryptEvent, cx: &mut Context<Self>) {
        let Some(state) = &mut self.crypt_state else {
            return;
        };
        match event {
            CryptEvent::Progress {
                run_id,
                phase,
                done,
                total,
                current,
            } => {
                if run_id != state.run_id {
                    return;
                }
                state.progress_done = done;
                state.progress_total = total;
                let mut status = crypt_phase_label(phase).to_string();
                if let Some(current) = current {
                    status.push_str(" · ");
                    status.push_str(&current);
                }
                state.status = status;
            }
            CryptEvent::FileDone {
                run_id,
                index,
                result,
            } => {
                if run_id != state.run_id {
                    return;
                }
                if let Some(item) = state.files.get_mut(index) {
                    item.status = match result {
                        Ok(path) => CryptFileStatus::Done(path),
                        Err(error) => CryptFileStatus::Error(error),
                    };
                }
            }
            CryptEvent::TextDone { run_id, result } => {
                if run_id != state.run_id {
                    return;
                }
                match result {
                    Ok((plaintext, ciphertext)) => {
                        if let Some(value) = plaintext {
                            state.pending_plaintext = Some(value);
                        }
                        if let Some(value) = ciphertext {
                            state.pending_ciphertext = Some(value);
                        }
                        state.status = tr(self.language, "完成").to_string();
                        state.error = None;
                    }
                    Err(error) => {
                        state.status = error.clone();
                        state.error = Some(error);
                    }
                }
            }
            CryptEvent::StegoDone { run_id, result } => {
                if run_id != state.run_id {
                    return;
                }
                match result {
                    Ok(path) => {
                        state.stego_result = Some(path.clone());
                        state.status = tr(self.language, "隐写图已生成").to_string();
                        state.stego_error = None;
                        self.show_toast(
                            tr(self.language, "隐写图已生成").to_string(),
                            ToastKind::Success,
                            cx,
                        );
                    }
                    Err(error) => {
                        state.stego_error = Some(error.clone());
                        state.status = error;
                    }
                }
            }
            CryptEvent::StegoExtracted { run_id, result } => {
                if run_id != state.run_id {
                    return;
                }
                match result {
                    Ok(text) => {
                        state.pending_plaintext = Some(text);
                        state.status = tr(self.language, "已从隐写图读取文本").to_string();
                        state.stego_error = None;
                    }
                    Err(error) => {
                        state.stego_error = Some(error.clone());
                        state.status = error;
                    }
                }
            }
            CryptEvent::Finished { run_id, error } => {
                if run_id != state.run_id {
                    return;
                }
                state.job = None;
                state.cancel.store(false, Ordering::Relaxed);
                if let Some(error) = error {
                    state.error = Some(error.clone());
                    state.status = error.clone();
                    self.show_toast(error, ToastKind::Error, cx);
                } else {
                    state.status = tr(self.language, "完成").to_string();
                    self.show_toast(
                        tr(self.language, "处理完成").to_string(),
                        ToastKind::Success,
                        cx,
                    );
                }
            }
        }
        cx.notify();
    }

    fn crypt_password(&mut self, cx: &mut Context<Self>, require_confirm: bool) -> Option<String> {
        let Some(state) = &self.crypt_state else {
            return None;
        };
        let password = state.password.read(cx).unmask_value().to_string();
        let confirm = state.confirm.read(cx).unmask_value().to_string();
        if password.is_empty() {
            if let Some(state) = &mut self.crypt_state {
                state.error = Some(tr(self.language, "请输入密码").to_string());
            }
            cx.notify();
            return None;
        }
        if require_confirm && password != confirm {
            if let Some(state) = &mut self.crypt_state {
                state.error = Some(tr(self.language, "两次输入的密码不一致").to_string());
            }
            cx.notify();
            return None;
        }
        Some(password)
    }

    fn crypt_begin_job(
        &mut self,
        job: CryptJob,
    ) -> Option<(u64, Sender<CryptEvent>, Arc<AtomicBool>)> {
        let state = self.crypt_state.as_mut()?;
        if state.job.is_some() {
            return None;
        }
        state.run_id = state.run_id.wrapping_add(1);
        state.job = Some(job);
        state.cancel.store(false, Ordering::Relaxed);
        state.error = None;
        state.progress_done = 0;
        state.progress_total = 0;
        Some((state.run_id, state.sender.clone(), state.cancel.clone()))
    }

    fn start_crypt_files(&mut self, cx: &mut Context<Self>) {
        let needs_encrypt = self
            .crypt_state
            .as_ref()
            .map(|state| {
                state
                    .files
                    .iter()
                    .any(|item| item.mode == CryptFileMode::Encrypt)
            })
            .unwrap_or(false);
        let Some(password) = self.crypt_password(cx, needs_encrypt) else {
            return;
        };
        let (algorithm, output_dir, compression, files) = {
            let Some(state) = &self.crypt_state else {
                return;
            };
            if state.files.is_empty() {
                return;
            }
            (
                state.algorithm,
                state.output_dir.clone(),
                state.file_compression,
                state.files.clone(),
            )
        };
        let Some((run_id, sender, cancel)) = self.crypt_begin_job(CryptJob::Files) else {
            return;
        };
        if let Some(state) = &mut self.crypt_state {
            state.progress_total = files.len() as u64;
            state.status = tr(self.language, "开始处理文件").to_string();
            for item in &mut state.files {
                item.status = CryptFileStatus::Pending;
            }
        }
        let reporter = CryptReporter {
            sender: sender.clone(),
            run_id,
            cancel,
        };
        std::thread::spawn(move || {
            let mut first_error = None;
            for (index, item) in files.iter().enumerate() {
                if reporter.cancelled() {
                    break;
                }
                let result = if item.mode == CryptFileMode::Decrypt {
                    decrypt_file(
                        &item.path,
                        &password,
                        &FileDecryptOptions {
                            output_dir: output_dir.clone(),
                        },
                        &reporter,
                    )
                    .map_err(|error| error.to_string())
                } else {
                    encrypt_file(
                        &item.path,
                        &password,
                        &FileEncryptOptions {
                            algorithm,
                            compression,
                            output_dir: output_dir.clone(),
                        },
                        &reporter,
                    )
                    .map_err(|error| error.to_string())
                };
                if let Err(error) = &result {
                    if first_error.is_none() {
                        first_error = Some(error.clone());
                    }
                }
                let _ = reporter.sender.send(CryptEvent::FileDone {
                    run_id,
                    index,
                    result,
                });
                let _ = reporter.sender.send(CryptEvent::Progress {
                    run_id,
                    phase: Phase::Finished,
                    done: (index + 1) as u64,
                    total: files.len() as u64,
                    current: None,
                });
            }
            let _ = reporter.sender.send(CryptEvent::Finished {
                run_id,
                error: first_error,
            });
        });
        cx.notify();
    }

    fn start_crypt_text(&mut self, encrypt: bool, cx: &mut Context<Self>) {
        let Some(password) = self.crypt_password(cx, encrypt) else {
            return;
        };
        let (plaintext, ciphertext, algorithm, encoding, compress, space) = {
            let Some(state) = &self.crypt_state else {
                return;
            };
            (
                state.plaintext.read(cx).value().to_string(),
                state.ciphertext.read(cx).value().to_string(),
                state.algorithm,
                state.encoding,
                state.text_compress,
                state.add_space,
            )
        };
        if encrypt && plaintext.trim().is_empty() {
            if let Some(state) = &mut self.crypt_state {
                state.error = Some(tr(self.language, "没有要加密的文本").to_string());
            }
            cx.notify();
            return;
        }
        if !encrypt && ciphertext.trim().is_empty() {
            if let Some(state) = &mut self.crypt_state {
                state.error = Some(tr(self.language, "没有要解密的文本").to_string());
            }
            cx.notify();
            return;
        }
        let Some((run_id, sender, _cancel)) = self.crypt_begin_job(if encrypt {
            CryptJob::TextEncrypt
        } else {
            CryptJob::TextDecrypt
        }) else {
            return;
        };
        if let Some(state) = &mut self.crypt_state {
            state.status = tr(self.language, "处理中").to_string();
        }
        std::thread::spawn(move || {
            let result = if encrypt {
                encrypt_text(
                    &plaintext,
                    &password,
                    &TextEncryptOptions {
                        algorithm,
                        encoding,
                        compress,
                        space_every: space,
                    },
                )
                .map(|value| (None, Some(value)))
                .map_err(|error| error.to_string())
            } else {
                decrypt_text(&ciphertext, &password)
                    .map(|value| (Some(value), None))
                    .map_err(|error| error.to_string())
            };
            let _ = sender.send(CryptEvent::TextDone { run_id, result });
            let _ = sender.send(CryptEvent::Finished {
                run_id,
                error: None,
            });
        });
        cx.notify();
    }

    fn start_crypt_stego_embed(&mut self, cx: &mut Context<Self>) {
        let Some(password) = self.crypt_password(cx, true) else {
            return;
        };
        let (text, carrier, output, quality, max_side, algorithm) = {
            let Some(state) = &self.crypt_state else {
                return;
            };
            let Some(carrier) = state.carrier.clone() else {
                return;
            };
            let output = state.stego_output.clone().unwrap_or_else(|| {
                let parent = carrier.parent().unwrap_or_else(|| Path::new("."));
                let stem = carrier
                    .file_stem()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "stego".to_string());
                parent.join(format!("{stem}.stego.jpg"))
            });
            (
                state.plaintext.read(cx).value().to_string(),
                carrier,
                output,
                state.stego_quality,
                state.stego_max_side,
                state.algorithm,
            )
        };
        let Some((run_id, sender, _cancel)) = self.crypt_begin_job(CryptJob::StegoEmbed) else {
            return;
        };
        std::thread::spawn(move || {
            let result = embed_text_in_jpeg(
                &text,
                &password,
                &StegoOptions {
                    carrier,
                    output,
                    quality,
                    max_side: if max_side == 0 { None } else { Some(max_side) },
                    algorithm,
                },
            )
            .map_err(|error| error.to_string());
            let _ = sender.send(CryptEvent::StegoDone { run_id, result });
            let _ = sender.send(CryptEvent::Finished {
                run_id,
                error: None,
            });
        });
        cx.notify();
    }

    fn start_crypt_stego_extract(&mut self, cx: &mut Context<Self>) {
        let Some(password) = self.crypt_password(cx, false) else {
            return;
        };
        let Some(carrier) = self
            .crypt_state
            .as_ref()
            .and_then(|state| state.carrier.clone())
        else {
            return;
        };
        let Some((run_id, sender, _cancel)) = self.crypt_begin_job(CryptJob::StegoExtract) else {
            return;
        };
        std::thread::spawn(move || {
            let result =
                extract_text_from_jpeg(&carrier, &password).map_err(|error| error.to_string());
            let _ = sender.send(CryptEvent::StegoExtracted { run_id, result });
            let _ = sender.send(CryptEvent::Finished {
                run_id,
                error: None,
            });
        });
        cx.notify();
    }

    fn crypt_add_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(paths) =
            components::file_dialog_in(window, || rfd::FileDialog::new().pick_files())
        else {
            return;
        };
        self.crypt_add_paths(paths, cx);
    }

    fn crypt_add_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(folder) =
            components::file_dialog_in(window, || rfd::FileDialog::new().pick_folder())
        else {
            return;
        };
        self.crypt_add_paths(vec![folder], cx);
    }

    fn crypt_add_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if let Some(state) = &mut self.crypt_state {
            for path in paths {
                push_crypt_file(state, path);
            }
            state.tab = CryptTab::Files;
            state.error = None;
        }
        cx.notify();
    }

    fn crypt_pick_output_dir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = self
            .crypt_state
            .as_ref()
            .and_then(|state| state.output_dir.clone())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let Some(folder) = components::file_dialog_in(window, move || {
            rfd::FileDialog::new().set_directory(start).pick_folder()
        }) else {
            return;
        };
        if let Some(state) = &mut self.crypt_state {
            state.output_dir = Some(folder.clone());
        }
        self.settings.crypt.output_dir = Some(folder.display().to_string());
        self.save_settings();
        cx.notify();
    }

    fn crypt_pick_carrier(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = self
            .crypt_state
            .as_ref()
            .and_then(|state| state.carrier.clone())
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let Some(path) = components::file_dialog_in(window, move || {
            rfd::FileDialog::new()
                .set_directory(start)
                .add_filter("Images", &["jpg", "jpeg", "png", "bmp"])
                .pick_file()
        }) else {
            return;
        };
        if let Some(state) = &mut self.crypt_state {
            state.carrier = Some(path.clone());
            state.stego_output = None;
            state.stego_error = None;
            state.stego_result = None;
            state.tab = CryptTab::Stego;
        }
        self.settings.crypt.last_stego_dir =
            path.parent().map(|parent| parent.display().to_string());
        self.save_settings();
        cx.notify();
    }

    fn crypt_pick_stego_extract(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = self
            .crypt_state
            .as_ref()
            .and_then(|state| state.carrier.clone())
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let Some(path) = components::file_dialog_in(window, move || {
            rfd::FileDialog::new()
                .set_directory(start)
                .add_filter("JPEG", &["jpg", "jpeg"])
                .pick_file()
        }) else {
            return;
        };
        if let Some(state) = &mut self.crypt_state {
            state.carrier = Some(path.clone());
            state.stego_error = None;
            state.tab = CryptTab::Stego;
        }
        self.settings.crypt.last_stego_dir =
            path.parent().map(|parent| parent.display().to_string());
        self.save_settings();
        self.start_crypt_stego_extract(cx);
    }

    pub(super) fn handle_crypt_drop(
        &mut self,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = window;
        let stego_tab = self
            .crypt_state
            .as_ref()
            .map(|state| state.tab == CryptTab::Stego)
            .unwrap_or(false);
        let mut carrier = None;
        let mut files = Vec::new();
        for path in paths {
            if stego_tab && is_image_path(path) {
                carrier = Some(path.clone());
            } else {
                files.push(path.clone());
            }
        }
        if let Some(carrier) = carrier {
            if let Some(state) = &mut self.crypt_state {
                state.carrier = Some(carrier);
                state.stego_output = None;
                state.stego_result = None;
                state.stego_error = None;
            }
        }
        if !files.is_empty() {
            self.crypt_add_paths(files, cx);
        }
        cx.notify();
    }

    pub(crate) fn render_crypt_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(state) = &mut self.crypt_state {
            if let Some(value) = state.pending_plaintext.take() {
                let entity = state.plaintext.clone();
                entity.update(cx, |input, cx| input.set_value(value, window, cx));
            }
            if let Some(value) = state.pending_ciphertext.take() {
                let entity = state.ciphertext.clone();
                entity.update(cx, |input, cx| input.set_value(value, window, cx));
            }
        }
        let palette = self.palette;
        let language = self.language;
        let Some(state) = self.crypt_state.as_ref() else {
            return div().into_any_element();
        };
        let tab = state.tab;
        let tabs = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(self.crypt_tab_button(
                "crypt-tab-files",
                IconName::FileLock,
                tr(language, "文件"),
                tab == CryptTab::Files,
                cx,
            ))
            .child(self.crypt_tab_button(
                "crypt-tab-text",
                IconName::Type,
                tr(language, "文本"),
                tab == CryptTab::Text,
                cx,
            ))
            .child(self.crypt_tab_button(
                "crypt-tab-stego",
                IconName::Image,
                tr(language, "隐写"),
                tab == CryptTab::Stego,
                cx,
            ));
        let header = div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .pt(px(16.0))
            .px(px(18.0))
            .pb(px(14.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(tabs)
                    .child(
                        div()
                            .text_size(theme::caption())
                            .text_color(palette.muted)
                            .child(tr(language, "本地加密，不联网")),
                    ),
            )
            .child(self.render_crypt_credentials(&palette, language, cx));
        let body = match tab {
            CryptTab::Files => self.render_crypt_files(&palette, language, cx),
            CryptTab::Text => self.render_crypt_text(&palette, language, cx),
            CryptTab::Stego => self.render_crypt_stego(&palette, language, cx),
        };
        div()
            .id("crypt-root")
            .debug_selector(|| "crypt-root".to_string())
            .flex()
            .flex_col()
            .size_full()
            .bg(palette.bg)
            .text_color(palette.text)
            .font_family(theme::ui_font_family())
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(div().px(px(18.0)).pb(px(18.0)).child(body)),
            )
            .into_any_element()
    }

    fn crypt_tab_button(
        &self,
        id: &'static str,
        icon_name: IconName,
        label: &'static str,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let tab = match id {
            "crypt-tab-text" => CryptTab::Text,
            "crypt-tab-stego" => CryptTab::Stego,
            _ => CryptTab::Files,
        };
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .role(Role::Button)
            .aria_label(label)
            .flex()
            .items_center()
            .gap(px(8.0))
            .h(px(38.0))
            .px(px(16.0))
            .rounded(px(99.0))
            .cursor_pointer()
            .bg(if selected {
                palette.accent_soft
            } else {
                palette.panel
            })
            .text_color(if selected {
                palette.accent
            } else {
                palette.sub
            })
            .hover(|style| style.bg(palette.accent_softer))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(state) = &mut this.crypt_state {
                    state.tab = tab;
                }
                cx.notify();
            }))
            .child(icon(
                icon_name,
                if selected {
                    palette.accent
                } else {
                    palette.sub
                },
                16.0,
            ))
            .child(
                div()
                    .text_size(theme::body())
                    .font_weight(if selected {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .child(label),
            )
            .into_any_element()
    }

    fn render_crypt_credentials(
        &self,
        palette: &Palette,
        language: Language,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(state) = self.crypt_state.as_ref() else {
            return div().into_any_element();
        };
        let show_password = state.show_password;
        let algorithm = state.algorithm;
        design::card(palette)
            .p(px(16.0))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap(px(14.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .w(px(260.0))
                            .child(design::field_label(palette, tr(language, "密码")))
                            .child(
                                components::control_height(Input::new(&state.password))
                                    .text_size(theme::body())
                                    .w_full(),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .w(px(260.0))
                            .child(design::field_label(palette, tr(language, "确认密码")))
                            .child(
                                components::control_height(Input::new(&state.confirm))
                                    .text_size(theme::body())
                                    .w_full(),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h(px(components::CONTROL_HEIGHT))
                            .child(design::icon_action(
                                palette,
                                "crypt-toggle-password",
                                if show_password {
                                    IconName::EyeOff
                                } else {
                                    IconName::Eye
                                },
                                palette.sub,
                                false,
                                tr(language, "显示密码"),
                                cx.listener(|this, _, window, cx| {
                                    let (show, password, confirm) = {
                                        let Some(state) = &mut this.crypt_state else {
                                            return;
                                        };
                                        state.show_password = !state.show_password;
                                        (
                                            state.show_password,
                                            state.password.clone(),
                                            state.confirm.clone(),
                                        )
                                    };
                                    password.update(cx, |input, cx| {
                                        input.set_masked(!show, window, cx);
                                    });
                                    confirm.update(cx, |input, cx| {
                                        input.set_masked(!show, window, cx);
                                    });
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .w(px(220.0))
                            .child(design::field_label(palette, tr(language, "算法")))
                            .child(self.crypt_algorithm_trigger(algorithm, palette, language, cx)),
                    ),
            )
            .into_any_element()
    }

    fn crypt_algorithm_trigger(
        &self,
        algorithm: Algorithm,
        palette: &Palette,
        language: Language,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = algorithm.label().to_string();
        self.track_dropdown(
            DropdownKind::CryptAlgorithm,
            div()
                .id("crypt-algorithm")
                .role(Role::Button)
                .aria_label(tr(language, "算法"))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .h(px(components::CONTROL_HEIGHT))
                .px(px(12.0))
                .rounded(px(theme::RADIUS_CONTROL))
                .border_1()
                .border_color(palette.line_strong)
                .bg(palette.card)
                .cursor_pointer()
                .hover(|style| style.border_color(palette.accent))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.open_dropdown(DropdownKind::CryptAlgorithm, cx);
                }))
                .child(
                    div()
                        .truncate()
                        .text_size(theme::body())
                        .text_color(palette.text)
                        .child(selected),
                )
                .child(icon(IconName::ChevronDown, palette.muted, 15.0))
                .into_any_element(),
            cx,
        )
    }

    fn render_crypt_files(
        &self,
        palette: &Palette,
        language: Language,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(state) = self.crypt_state.as_ref() else {
            return div().into_any_element();
        };
        let mut list = div().flex().flex_col().gap(px(8.0));
        if state.files.is_empty() {
            list = list.child(
                div()
                    .py(px(14.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(theme::caption())
                    .text_color(palette.muted)
                    .child(tr(language, "还没有待处理的文件")),
            );
        }
        for (index, item) in state.files.iter().enumerate() {
            let (mode_label, mode_tone) = match item.mode {
                CryptFileMode::Encrypt => (tr(language, "加密"), design::TagTone::Accent),
                CryptFileMode::Decrypt => (tr(language, "解密"), design::TagTone::Ok),
            };
            let (status_label, status_tone) = match &item.status {
                CryptFileStatus::Pending => (tr(language, "等待"), design::TagTone::Neutral),
                CryptFileStatus::Done(_) => (tr(language, "完成"), design::TagTone::Ok),
                CryptFileStatus::Error(_) => (tr(language, "失败"), design::TagTone::Danger),
            };
            let icon_name = if item.is_dir {
                IconName::Folder
            } else {
                IconName::FileText
            };
            let name = item
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| item.path.display().to_string());
            let detail = match &item.status {
                CryptFileStatus::Done(path) => path.display().to_string(),
                CryptFileStatus::Error(error) => error.clone(),
                CryptFileStatus::Pending => item.path.display().to_string(),
            };
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .px(px(12.0))
                    .py(px(10.0))
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(palette.line)
                    .bg(palette.panel)
                    .child(icon(icon_name, palette.sub, 18.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(theme::body())
                                    .text_color(palette.text)
                                    .child(name),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(theme::caption())
                                    .text_color(palette.muted)
                                    .child(detail),
                            ),
                    )
                    .child(design::pill(palette, mode_label, mode_tone))
                    .child(design::pill(palette, status_label, status_tone))
                    .child(design::icon_action(
                        palette,
                        format!("crypt-remove-{index}"),
                        IconName::X,
                        palette.muted,
                        false,
                        tr(language, "移除"),
                        cx.listener(move |this, _, _, cx| {
                            if let Some(state) = &mut this.crypt_state {
                                if index < state.files.len() {
                                    state.files.remove(index);
                                }
                            }
                            cx.notify();
                        }),
                    )),
            );
        }
        let mut compression_row = div().flex().items_center().gap(px(8.0));
        for (value, label) in [
            (FileCompression::Auto, tr(language, "自动")),
            (FileCompression::Always, tr(language, "始终")),
            (FileCompression::Never, tr(language, "不压缩")),
        ] {
            let selected = state.file_compression == value;
            compression_row = compression_row.child(
                Button::new(format!("crypt-compress-{value:?}"))
                    .label(label)
                    .when(selected, |button| button.primary())
                    .when(!selected, |button| button.outline())
                    .h(px(30.0))
                    .px(px(12.0))
                    .text_size(theme::caption())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(state) = &mut this.crypt_state {
                            state.file_compression = value;
                        }
                        this.settings.crypt.file_compression = match value {
                            FileCompression::Auto => 0,
                            FileCompression::Always => 1,
                            FileCompression::Never => 2,
                        };
                        this.save_settings();
                        cx.notify();
                    })),
            );
        }
        let running = state.job.is_some();
        let output_label = state
            .output_dir
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| tr(language, "与源文件相同").to_string());
        let percent = if state.progress_total == 0 {
            0.0
        } else {
            (state.progress_done as f32 / state.progress_total as f32 * 100.0).clamp(0.0, 100.0)
        };
        let status = state.status.clone();
        let error = state.error.clone();
        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(design::dropzone(
                palette,
                IconName::FileLock,
                34.0,
                tr(language, "拖放文件或文件夹到这里"),
                tr(language, "普通文件将加密为 .enc；兼容的 .enc 文件将解密"),
                vec![
                    Button::new("crypt-pick-files")
                        .label(tr(language, "选择文件"))
                        .icon(icon(IconName::Plus, palette.accent, 15.0))
                        .outline()
                        .h(px(32.0))
                        .px(px(12.0))
                        .text_size(theme::caption())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.crypt_add_files(window, cx);
                        }))
                        .into_any_element(),
                    Button::new("crypt-pick-folder")
                        .label(tr(language, "选择文件夹"))
                        .icon(icon(IconName::Folder, palette.accent, 15.0))
                        .outline()
                        .h(px(32.0))
                        .px(px(12.0))
                        .text_size(theme::caption())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.crypt_add_folder(window, cx);
                        }))
                        .into_any_element(),
                ],
                140.0,
            ))
            .child(
                design::card(palette)
                    .p(px(16.0))
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(theme::body())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!(
                                        "{} ({})",
                                        tr(language, "待处理文件"),
                                        state.files.len()
                                    )),
                            )
                            .child(
                                Button::new("crypt-clear-files")
                                    .label(tr(language, "清空"))
                                    .ghost()
                                    .h(px(28.0))
                                    .px(px(10.0))
                                    .text_size(theme::caption())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(state) = &mut this.crypt_state {
                                            state.files.clear();
                                        }
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(list),
            )
            .child(
                design::card(palette)
                    .p(px(16.0))
                    .child(design::field_label(palette, tr(language, "压缩模式")))
                    .child(compression_row)
                    .child(
                        div()
                            .mt(px(14.0))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.0))
                                    .child(
                                        div()
                                            .text_size(theme::body())
                                            .child(tr(language, "输出目录")),
                                    )
                                    .child(
                                        div()
                                            .text_size(theme::caption())
                                            .text_color(palette.muted)
                                            .child(output_label),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .when(state.output_dir.is_some(), |row| {
                                        row.child(
                                            Button::new("crypt-output-reset")
                                                .label(tr(language, "恢复默认"))
                                                .ghost()
                                                .h(px(30.0))
                                                .px(px(10.0))
                                                .text_size(theme::caption())
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    if let Some(state) = &mut this.crypt_state {
                                                        state.output_dir = None;
                                                    }
                                                    this.settings.crypt.output_dir = None;
                                                    this.save_settings();
                                                    cx.notify();
                                                })),
                                        )
                                    })
                                    .child(
                                        Button::new("crypt-output-dir")
                                            .label(tr(language, "选择目录…"))
                                            .outline()
                                            .h(px(30.0))
                                            .px(px(12.0))
                                            .text_size(theme::caption())
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.crypt_pick_output_dir(window, cx);
                                            })),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .debug_selector(|| "crypt-start-files".to_string())
                            .child(
                                Button::new("crypt-start-files")
                                    .label(tr(language, "开始处理"))
                                    .disabled(running)
                                    .icon(icon(IconName::Play, palette.accent_text, 15.0))
                                    .primary()
                                    .h(px(components::CONTROL_HEIGHT))
                                    .px(px(20.0))
                                    .text_size(theme::body())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.start_crypt_files(cx);
                                    })),
                            ),
                    )
                    .when(running, |row| {
                        row.child(
                            Button::new("crypt-stop-files")
                                .label(tr(language, "停止"))
                                .danger()
                                .h(px(components::CONTROL_HEIGHT))
                                .px(px(16.0))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(state) = &this.crypt_state {
                                        state.cancel.store(true, Ordering::Relaxed);
                                    }
                                    cx.notify();
                                })),
                        )
                    })
                    .child(div().flex_1().min_w_0().child(components::progress_bar(
                        percent,
                        palette,
                        palette.accent,
                    )))
                    .child(
                        div()
                            .text_size(theme::caption())
                            .text_color(palette.sub)
                            .child(status),
                    ),
            );
        if let Some(error) = error {
            body.child(crate::design::info_note(palette, error))
        } else {
            body
        }
        .into_any_element()
    }

    fn render_crypt_text(
        &self,
        palette: &Palette,
        language: Language,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(state) = self.crypt_state.as_ref() else {
            return div().into_any_element();
        };
        let running = state.job.is_some();
        let encoding = state.encoding;
        let status = state.status.clone();
        let error = state.error.clone();
        let compression = state.text_compress;
        let add_space = state.add_space;
        let options = design::card(palette)
            .p(px(16.0))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap(px(18.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .w(px(220.0))
                            .child(design::field_label(palette, tr(language, "输出编码")))
                            .child(self.crypt_encoding_trigger(encoding, palette, language, cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h(px(components::CONTROL_HEIGHT))
                            .gap(px(8.0))
                            .child(design::check_box(
                                palette,
                                "crypt-lzma",
                                compression,
                                false,
                                "LZMA",
                                cx.listener(|this, _, _, cx| {
                                    let next = this
                                        .crypt_state
                                        .as_ref()
                                        .map(|state| !state.text_compress)
                                        .unwrap_or(false);
                                    if let Some(state) = &mut this.crypt_state {
                                        state.text_compress = next;
                                    }
                                    this.settings.crypt.text_compression = next;
                                    this.save_settings();
                                    cx.notify();
                                }),
                            ))
                            .child(
                                div()
                                    .text_size(theme::body())
                                    .child(tr(language, "LZMA 压缩")),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h(px(components::CONTROL_HEIGHT))
                            .gap(px(8.0))
                            .child(
                                div()
                                    .text_size(theme::body())
                                    .child(tr(language, "每 N 个字符加空格")),
                            )
                            .child(design::stepper(
                                palette,
                                "crypt-space",
                                if add_space == 0 {
                                    tr(language, "关闭").to_string()
                                } else {
                                    add_space.to_string()
                                },
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = &mut this.crypt_state {
                                        state.add_space = state.add_space.saturating_sub(1).min(10);
                                        this.settings.crypt.add_space_every = state.add_space;
                                    }
                                    this.save_settings();
                                    cx.notify();
                                }),
                                cx.listener(|this, _, _, cx| {
                                    if let Some(state) = &mut this.crypt_state {
                                        state.add_space = (state.add_space + 1).min(10);
                                        this.settings.crypt.add_space_every = state.add_space;
                                    }
                                    this.save_settings();
                                    cx.notify();
                                }),
                            )),
                    ),
            )
            .child(
                div()
                    .mt(px(12.0))
                    .text_size(theme::caption())
                    .text_color(palette.muted)
                    .child(tr(language, "解密时自动识别编码、压缩与算法")),
            );
        let plaintext_count = state.plaintext.read(cx).value().chars().count();
        let ciphertext_count = state.ciphertext.read(cx).value().chars().count();
        let panes = div()
            .flex()
            .gap(px(14.0))
            .child(self.crypt_text_pane(
                "crypt-plaintext",
                tr(language, "明文"),
                palette.ok,
                &state.plaintext,
                plaintext_count,
                palette,
                language,
                false,
                cx,
            ))
            .child(self.crypt_text_pane(
                "crypt-ciphertext",
                tr(language, "密文"),
                palette.accent,
                &state.ciphertext,
                ciphertext_count,
                palette,
                language,
                true,
                cx,
            ));
        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(options)
            .child(panes)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .debug_selector(|| "crypt-encrypt-text".to_string())
                            .child(
                                Button::new("crypt-encrypt-text")
                                    .label(tr(language, "加密"))
                                    .disabled(running)
                                    .icon(icon(IconName::Lock, palette.accent_text, 15.0))
                                    .primary()
                                    .h(px(components::CONTROL_HEIGHT))
                                    .px(px(20.0))
                                    .text_size(theme::body())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.start_crypt_text(true, cx);
                                    })),
                            ),
                    )
                    .child(
                        Button::new("crypt-decrypt-text")
                            .label(tr(language, "解密"))
                            .disabled(running)
                            .icon(icon(IconName::LockKeyholeOpen, palette.accent, 15.0))
                            .outline()
                            .h(px(components::CONTROL_HEIGHT))
                            .px(px(20.0))
                            .text_size(theme::body())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.start_crypt_text(false, cx);
                            })),
                    )
                    .child(
                        Button::new("crypt-clear-text")
                            .label(tr(language, "清空"))
                            .ghost()
                            .h(px(components::CONTROL_HEIGHT))
                            .px(px(16.0))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(state) = &mut this.crypt_state {
                                    let plain = state.plaintext.clone();
                                    let cipher = state.ciphertext.clone();
                                    plain.update(cx, |input, cx| {
                                        input.set_value("", window, cx);
                                    });
                                    cipher.update(cx, |input, cx| {
                                        input.set_value("", window, cx);
                                    });
                                }
                                cx.notify();
                            })),
                    )
                    .when(running, |row| {
                        row.child(
                            div()
                                .text_size(theme::caption())
                                .text_color(palette.accent)
                                .child(tr(language, "处理中…")),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(theme::caption())
                            .text_color(palette.sub)
                            .child(status),
                    ),
            );
        if let Some(error) = error {
            body.child(crate::design::info_note(palette, error))
        } else {
            body
        }
        .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn crypt_text_pane(
        &self,
        id: &'static str,
        title: &'static str,
        accent: Hsla,
        state: &Entity<TextareaState>,
        count: usize,
        palette: &Palette,
        language: Language,
        copy_enabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state_clone = state.clone();
        let copy_state = state.clone();
        design::card(palette)
            .p(px(14.0))
            .flex()
            .flex_col()
            .h(px(300.0))
            .min_w_0()
            .flex_1()
            .border_color(accent.opacity(0.45))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .mb(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(div().w(px(3.0)).h(px(16.0)).rounded(px(2.0)).bg(accent))
                            .child(
                                div()
                                    .text_size(theme::body())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_size(theme::caption())
                                    .text_color(palette.muted)
                                    .child(format!("{count}")),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(design::icon_action(
                                palette,
                                format!("{id}-copy"),
                                IconName::Copy,
                                if copy_enabled {
                                    palette.accent
                                } else {
                                    palette.muted
                                },
                                false,
                                tr(language, "复制"),
                                cx.listener(move |this, _, _, cx| {
                                    let value = copy_state.read(cx).value().to_string();
                                    cx.write_to_clipboard(ClipboardItem::new_string(value));
                                    this.show_toast(
                                        tr(this.language, "已复制").to_string(),
                                        ToastKind::Info,
                                        cx,
                                    );
                                }),
                            ))
                            .child(design::icon_action(
                                palette,
                                format!("{id}-clear"),
                                IconName::Eraser,
                                palette.muted,
                                false,
                                tr(language, "清空"),
                                cx.listener(move |this, _, window, cx| {
                                    state_clone.update(cx, |input, cx| {
                                        input.set_value("", window, cx);
                                    });
                                    cx.notify();
                                    let _ = this;
                                }),
                            )),
                    ),
            )
            .child(Textarea::new(state).h_full().w_full())
            .into_any_element()
    }

    fn crypt_encoding_trigger(
        &self,
        encoding: TextEncoding,
        palette: &Palette,
        language: Language,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = match encoding {
            TextEncoding::Base64 => "Base64".to_string(),
            TextEncoding::Base32 => "Base32".to_string(),
            TextEncoding::Base10 => tr(language, "纯数字").to_string(),
        };
        self.track_dropdown(
            DropdownKind::CryptEncoding,
            div()
                .id("crypt-encoding")
                .role(Role::Button)
                .aria_label(tr(language, "输出编码"))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .h(px(components::CONTROL_HEIGHT))
                .px(px(12.0))
                .rounded(px(theme::RADIUS_CONTROL))
                .border_1()
                .border_color(palette.line_strong)
                .bg(palette.card)
                .cursor_pointer()
                .hover(|style| style.border_color(palette.accent))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.open_dropdown(DropdownKind::CryptEncoding, cx);
                }))
                .child(
                    div()
                        .truncate()
                        .text_size(theme::body())
                        .text_color(palette.text)
                        .child(label),
                )
                .child(icon(IconName::ChevronDown, palette.muted, 15.0))
                .into_any_element(),
            cx,
        )
    }

    fn render_crypt_stego(
        &self,
        palette: &Palette,
        language: Language,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(state) = self.crypt_state.as_ref() else {
            return div().into_any_element();
        };
        let running = state.job.is_some();
        let carrier = state.carrier.clone();
        let output = state.stego_output.clone().or_else(|| {
            carrier.as_ref().map(|carrier| {
                let parent = carrier.parent().unwrap_or_else(|| Path::new("."));
                let stem = carrier
                    .file_stem()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "stego".to_string());
                parent.join(format!("{stem}.stego.jpg"))
            })
        });
        let quality = state.stego_quality;
        let max_side = state.stego_max_side;
        let status = state.status.clone();
        let error = state.stego_error.clone().or_else(|| state.error.clone());
        let result = state.stego_result.clone();
        let carrier_preview = if let Some(carrier) = &carrier {
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .w_full()
                        .h(px(200.0))
                        .rounded(px(12.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(palette.line)
                        .bg(palette.panel)
                        .child(
                            img(carrier.as_path())
                                .w_full()
                                .h_full()
                                .object_fit(ObjectFit::Contain),
                        ),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(theme::caption())
                        .text_color(palette.muted)
                        .child(carrier.display().to_string()),
                )
                .into_any_element()
        } else {
            design::dropzone(
                palette,
                IconName::Image,
                34.0,
                tr(language, "选择或拖入载体图片"),
                tr(language, "支持 JPG / PNG / BMP；输出为 JPEG 隐写图"),
                Vec::new(),
                200.0,
            )
        };
        let plaintext_count = state.plaintext.read(cx).value().chars().count();
        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(
                div()
                    .flex()
                    .gap(px(14.0))
                    .child(
                        design::card(palette)
                            .p(px(16.0))
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .mb(px(10.0))
                                    .child(
                                        div()
                                            .text_size(theme::body())
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(tr(language, "载体图片")),
                                    )
                                    .child(
                                        Button::new("crypt-pick-carrier")
                                            .label(tr(language, "选择图片…"))
                                            .outline()
                                            .h(px(30.0))
                                            .px(px(12.0))
                                            .text_size(theme::caption())
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.crypt_pick_carrier(window, cx);
                                            })),
                                    ),
                            )
                            .child(carrier_preview),
                    )
                    .child(
                        design::card(palette)
                            .p(px(16.0))
                            .flex()
                            .flex_col()
                            .gap(px(14.0))
                            .w(px(340.0))
                            .child(
                                div()
                                    .text_size(theme::body())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(tr(language, "隐写参数")),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_size(theme::caption())
                                            .text_color(palette.sub)
                                            .child(tr(language, "JPEG 质量")),
                                    )
                                    .child(design::stepper(
                                        palette,
                                        "crypt-stego-quality",
                                        quality.to_string(),
                                        cx.listener(|this, _, _, cx| {
                                            if let Some(state) = &mut this.crypt_state {
                                                state.stego_quality =
                                                    state.stego_quality.saturating_sub(5).max(50);
                                                this.settings.crypt.stego_quality =
                                                    state.stego_quality;
                                            }
                                            this.save_settings();
                                            cx.notify();
                                        }),
                                        cx.listener(|this, _, _, cx| {
                                            if let Some(state) = &mut this.crypt_state {
                                                state.stego_quality =
                                                    (state.stego_quality + 5).min(100);
                                                this.settings.crypt.stego_quality =
                                                    state.stego_quality;
                                            }
                                            this.save_settings();
                                            cx.notify();
                                        }),
                                    )),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_size(theme::caption())
                                            .text_color(palette.sub)
                                            .child(tr(language, "最大边长")),
                                    )
                                    .child(design::stepper(
                                        palette,
                                        "crypt-stego-side",
                                        if max_side == 0 {
                                            tr(language, "原图").to_string()
                                        } else {
                                            format!("{max_side}px")
                                        },
                                        cx.listener(|this, _, _, cx| {
                                            if let Some(state) = &mut this.crypt_state {
                                                state.stego_max_side = if state.stego_max_side == 0
                                                {
                                                    0
                                                } else {
                                                    state
                                                        .stego_max_side
                                                        .saturating_sub(200)
                                                        .max(200)
                                                };
                                                this.settings.crypt.stego_max_side =
                                                    state.stego_max_side;
                                            }
                                            this.save_settings();
                                            cx.notify();
                                        }),
                                        cx.listener(|this, _, _, cx| {
                                            if let Some(state) = &mut this.crypt_state {
                                                state.stego_max_side = if state.stego_max_side == 0
                                                {
                                                    1000
                                                } else {
                                                    (state.stego_max_side + 200).min(8000)
                                                };
                                                this.settings.crypt.stego_max_side =
                                                    state.stego_max_side;
                                            }
                                            this.save_settings();
                                            cx.notify();
                                        }),
                                    )),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.0))
                                    .child(
                                        div()
                                            .text_size(theme::caption())
                                            .text_color(palette.sub)
                                            .child(tr(language, "输出文件")),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(theme::caption())
                                            .text_color(palette.muted)
                                            .child(
                                                output
                                                    .as_ref()
                                                    .map(|path| path.display().to_string())
                                                    .unwrap_or_else(|| {
                                                        tr(language, "选择载体后自动生成")
                                                            .to_string()
                                                    }),
                                            ),
                                    ),
                            ),
                    ),
            )
            .child(
                design::card(palette)
                    .p(px(16.0))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .mb(px(8.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(
                                        div()
                                            .text_size(theme::body())
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(tr(language, "要隐藏的文本")),
                                    )
                                    .child(
                                        div()
                                            .text_size(theme::caption())
                                            .text_color(palette.muted)
                                            .child(tr(language, "与文本页共享")),
                                    )
                                    .child(
                                        div()
                                            .text_size(theme::caption())
                                            .text_color(palette.muted)
                                            .child(format!("{plaintext_count}")),
                                    ),
                            ),
                    )
                    .child(Textarea::new(&state.plaintext).h(px(120.0)).w_full()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .debug_selector(|| "crypt-stego-embed".to_string())
                            .child(
                                Button::new("crypt-stego-embed")
                                    .label(tr(language, "制作隐写图"))
                                    .disabled(running)
                                    .icon(icon(IconName::Sparkles, palette.accent_text, 15.0))
                                    .primary()
                                    .h(px(components::CONTROL_HEIGHT))
                                    .px(px(20.0))
                                    .text_size(theme::body())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.start_crypt_stego_embed(cx);
                                    })),
                            ),
                    )
                    .child(
                        Button::new("crypt-stego-extract")
                            .label(tr(language, "从隐写图读取"))
                            .disabled(running)
                            .icon(icon(IconName::Image, palette.accent, 15.0))
                            .outline()
                            .h(px(components::CONTROL_HEIGHT))
                            .px(px(18.0))
                            .text_size(theme::body())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.crypt_pick_stego_extract(window, cx);
                            })),
                    )
                    .when_some(result, |row, result| {
                        row.child(
                            Button::new("crypt-open-stego")
                                .label(tr(language, "打开所在位置"))
                                .ghost()
                                .h(px(components::CONTROL_HEIGHT))
                                .px(px(14.0))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let _ = open::that(&result);
                                    let _ = this;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(theme::caption())
                            .text_color(palette.sub)
                            .child(status),
                    ),
            );
        if let Some(error) = error {
            body.child(crate::design::info_note(palette, error))
        } else {
            body
        }
        .into_any_element()
    }
}

fn push_crypt_file(state: &mut CryptState, path: PathBuf) {
    if state.files.iter().any(|item| item.path == path) {
        return;
    }
    let is_dir = path.is_dir();
    let mode = if !is_dir && format::detect_file(&path).ok().flatten().is_some() {
        CryptFileMode::Decrypt
    } else {
        CryptFileMode::Encrypt
    };
    state.files.push(CryptFileItem {
        path,
        is_dir,
        mode,
        status: CryptFileStatus::Pending,
    });
}

fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "bmp"
            )
        })
        .unwrap_or(false)
}

fn crypt_phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Preparing => "准备",
        Phase::Compressing => "正在压缩",
        Phase::Encrypting => "加密",
        Phase::Verifying => "校验",
        Phase::Decrypting => "解密",
        Phase::Extracting => "正在解压",
        Phase::Steganography => "隐写",
        Phase::Finished => "完成",
    }
}
