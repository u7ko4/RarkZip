use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, Utc};
use crossbeam_channel::{unbounded, Receiver, Sender};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Disableable;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rarkzip_engine::openpgp::{
    self, format_fingerprint, is_clearsigned, is_signature_file, KeyInfo, Keyring, SecretKeyInfo,
    SecretKeyring, SignReport, SignatureState as SignatureVerdict, VerificationReport,
    VerifiedSignature,
};
use rarkzip_engine::paths::unique_path;

use crate::components::{self, icon, IconName};
use crate::design;
use crate::i18n::{tr, Language};
use crate::theme::{self, Palette};

use super::{DropdownKind, MainView, ToastKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SignatureTab {
    Verify,
    Sign,
}

#[derive(Debug)]
enum SignatureEvent {
    Verified {
        run_id: u64,
        result: std::result::Result<VerificationReport, String>,
    },
    Signed {
        run_id: u64,
        result: std::result::Result<SignReport, String>,
    },
}

pub(super) struct SignatureState {
    tab: SignatureTab,
    keyring: Keyring,
    secret_keyring: SecretKeyring,
    signing_key: Option<String>,
    signature: Option<PathBuf>,
    data: Option<PathBuf>,
    clearsigned: bool,
    running: bool,
    report: Option<VerificationReport>,
    error: Option<String>,
    key_error: Option<String>,
    secret_key_error: Option<String>,
    status: String,
    run_id: u64,
    sign_file: Option<PathBuf>,
    sign_output: Option<PathBuf>,
    sign_output_custom: bool,
    armor: bool,
    passphrase: Entity<InputState>,
    show_passphrase: bool,
    sign_running: bool,
    sign_report: Option<SignReport>,
    sign_error: Option<String>,
    sign_run_id: u64,
    sender: Sender<SignatureEvent>,
    receiver: Receiver<SignatureEvent>,
}

impl MainView {
    pub(crate) fn init_signature(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        sources: Vec<PathBuf>,
    ) {
        let (sender, receiver) = unbounded();
        let secret_keyring = SecretKeyring::load();
        let signing_key = secret_keyring
            .keys()
            .first()
            .map(|key| key.fingerprint.clone());
        let passphrase = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(tr(self.language, "私钥口令"))
        });
        let mut state = SignatureState {
            tab: SignatureTab::Verify,
            keyring: Keyring::load(),
            secret_keyring,
            signing_key,
            signature: None,
            data: None,
            clearsigned: false,
            running: false,
            report: None,
            error: None,
            key_error: None,
            secret_key_error: None,
            status: tr(self.language, "就绪").to_string(),
            run_id: 0,
            sign_file: None,
            sign_output: None,
            sign_output_custom: false,
            armor: self.settings.signature.sign_armor,
            passphrase,
            show_passphrase: false,
            sign_running: false,
            sign_report: None,
            sign_error: None,
            sign_run_id: 0,
            sender,
            receiver,
        };
        if !sources.is_empty() {
            apply_signature_paths(&mut state, &sources);
        }
        self.signature_state = Some(state);
        cx.notify();
    }

    pub(super) fn handle_signature_drop(
        &mut self,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = window;
        let Some(state) = &mut self.signature_state else {
            return;
        };
        if state.tab == SignatureTab::Sign {
            if let Some(path) = paths.iter().find(|path| path.is_file()).cloned() {
                state.set_sign_file(path);
            }
        } else {
            apply_signature_paths(state, paths);
            state.report = None;
            state.error = None;
            state.status = tr(self.language, "就绪").to_string();
        }
        cx.notify();
    }

    fn signature_pick_signature(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = self
            .signature_state
            .as_ref()
            .and_then(|state| state.signature.clone())
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .or_else(|| {
                self.settings
                    .signature
                    .last_signature_dir
                    .as_ref()
                    .map(PathBuf::from)
            })
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let Some(path) = components::file_dialog_in(window, move || {
            rfd::FileDialog::new()
                .set_directory(start)
                .add_filter("OpenPGP signatures", &["sig", "asc", "sign", "gpg", "pgp"])
                .pick_file()
        }) else {
            return;
        };
        if let Some(state) = &mut self.signature_state {
            state.set_signature(path.clone());
        }
        self.settings.signature.last_signature_dir =
            path.parent().map(|parent| parent.display().to_string());
        self.save_settings();
        cx.notify();
    }

    fn signature_pick_data(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = self
            .signature_state
            .as_ref()
            .and_then(|state| state.data.clone())
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .or_else(|| {
                self.settings
                    .signature
                    .last_data_dir
                    .as_ref()
                    .map(PathBuf::from)
            })
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let Some(path) = components::file_dialog_in(window, move || {
            rfd::FileDialog::new()
                .set_directory(start)
                .add_filter("All files", &["*"])
                .pick_file()
        }) else {
            return;
        };
        if let Some(state) = &mut self.signature_state {
            state.set_data(path.clone());
        }
        self.settings.signature.last_data_dir =
            path.parent().map(|parent| parent.display().to_string());
        self.save_settings();
        cx.notify();
    }

    fn signature_import_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = self
            .settings
            .signature
            .last_key_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let Some(path) = components::file_dialog_in(window, move || {
            rfd::FileDialog::new()
                .set_directory(start)
                .add_filter("OpenPGP public keys", &["asc", "gpg", "pgp", "key", "pub"])
                .pick_file()
        }) else {
            return;
        };
        let mut toast = None;
        if let Some(state) = &mut self.signature_state {
            match state.keyring.import(&path) {
                Ok(report) => {
                    state.key_error = (!report.errors.is_empty()).then(|| report.errors.join("；"));
                    toast = Some((
                        format!(
                            "{}: {}",
                            tr(self.language, "已导入公钥"),
                            report.imported_count()
                        ),
                        if report.errors.is_empty() {
                            ToastKind::Info
                        } else {
                            ToastKind::Error
                        },
                    ));
                }
                Err(error) => {
                    state.key_error = Some(error.to_string());
                    toast = Some((error.to_string(), ToastKind::Error));
                }
            }
        }
        self.settings.signature.last_key_dir =
            path.parent().map(|parent| parent.display().to_string());
        self.save_settings();
        if let Some((message, kind)) = toast {
            self.show_toast(message, kind, cx);
        }
        cx.notify();
    }

    fn signature_remove_key(&mut self, fingerprint: String, cx: &mut Context<Self>) {
        let mut toast = None;
        if let Some(state) = &mut self.signature_state {
            match state.keyring.remove(&fingerprint) {
                Ok(true) => {
                    toast = Some((tr(self.language, "已移除公钥").to_string(), ToastKind::Info))
                }
                Ok(false) => {}
                Err(error) => {
                    state.key_error = Some(error.to_string());
                    toast = Some((error.to_string(), ToastKind::Error));
                }
            }
        }
        if let Some((message, kind)) = toast {
            self.show_toast(message, kind, cx);
        }
        cx.notify();
    }

    fn signature_open_key_dir(&mut self, cx: &mut Context<Self>) {
        let path = self
            .signature_state
            .as_ref()
            .map(|state| state.keyring.dir().to_path_buf())
            .unwrap_or_else(openpgp::keys_dir);
        if let Err(error) = std::fs::create_dir_all(&path) {
            self.show_toast(error.to_string(), ToastKind::Error, cx);
            return;
        }
        if let Err(error) = open::that(&path) {
            self.show_toast(error.to_string(), ToastKind::Error, cx);
        }
        cx.notify();
    }

    fn signature_clear(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = &mut self.signature_state {
            if state.running {
                return;
            }
            state.signature = None;
            state.data = None;
            state.clearsigned = false;
            state.report = None;
            state.error = None;
            state.status = tr(self.language, "就绪").to_string();
        }
        cx.notify();
    }

    fn signature_pick_sign_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = self
            .signature_state
            .as_ref()
            .and_then(|state| state.sign_file.clone())
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .or_else(|| {
                self.settings
                    .signature
                    .last_data_dir
                    .as_ref()
                    .map(PathBuf::from)
            })
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let Some(path) = components::file_dialog_in(window, move || {
            rfd::FileDialog::new().set_directory(start).pick_file()
        }) else {
            return;
        };
        if let Some(state) = &mut self.signature_state {
            state.set_sign_file(path.clone());
        }
        self.settings.signature.last_data_dir =
            path.parent().map(|parent| parent.display().to_string());
        self.save_settings();
        cx.notify();
    }

    fn signature_pick_sign_output(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.signature_state.as_ref() else {
            return;
        };
        let Some(file) = state.sign_file.clone() else {
            return;
        };
        let default_output = state
            .sign_output
            .clone()
            .unwrap_or_else(|| default_sign_output(&file, state.armor));
        let start = default_output
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let name = default_output
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "signature.sig".to_string());
        let Some(output) = components::file_dialog_in(window, move || {
            rfd::FileDialog::new()
                .set_directory(start)
                .set_file_name(name)
                .save_file()
        }) else {
            return;
        };
        let parent = output_parent(&output).map(|parent| parent.display().to_string());
        if let Some(state) = &mut self.signature_state {
            state.set_sign_output(output);
        }
        self.settings.signature.last_data_dir = parent;
        self.save_settings();
        cx.notify();
    }

    fn signature_import_secret_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = self
            .settings
            .signature
            .last_key_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let Some(path) = components::file_dialog_in(window, move || {
            rfd::FileDialog::new()
                .set_directory(start)
                .add_filter(
                    "OpenPGP private keys",
                    &["asc", "gpg", "pgp", "key", "sec", "skr", "private"],
                )
                .pick_file()
        }) else {
            return;
        };
        let mut toast = None;
        if let Some(state) = &mut self.signature_state {
            match state.secret_keyring.import(&path) {
                Ok(report) => {
                    state.secret_key_error =
                        (!report.errors.is_empty()).then(|| report.errors.join("；"));
                    if !report.imported.is_empty() {
                        let keys = state.secret_keyring.keys();
                        let missing = state.signing_key.as_ref().is_none_or(|fingerprint| {
                            !keys.iter().any(|key| key.fingerprint == *fingerprint)
                        });
                        if missing {
                            state.signing_key = keys.first().map(|key| key.fingerprint.clone());
                        }
                    }
                    toast = Some((
                        format!(
                            "{}: {}",
                            tr(self.language, "已导入私钥"),
                            report.imported_count()
                        ),
                        if report.errors.is_empty() {
                            ToastKind::Info
                        } else {
                            ToastKind::Error
                        },
                    ));
                }
                Err(error) => {
                    state.secret_key_error = Some(error.to_string());
                    toast = Some((error.to_string(), ToastKind::Error));
                }
            }
        }
        self.settings.signature.last_key_dir =
            path.parent().map(|parent| parent.display().to_string());
        self.save_settings();
        if let Some((message, kind)) = toast {
            self.show_toast(message, kind, cx);
        }
        cx.notify();
    }

    fn signature_remove_secret_key(&mut self, fingerprint: String, cx: &mut Context<Self>) {
        let mut toast = None;
        if let Some(state) = &mut self.signature_state {
            match state.secret_keyring.remove(&fingerprint) {
                Ok(true) => {
                    if state.signing_key.as_deref() == Some(fingerprint.as_str()) {
                        state.signing_key = state
                            .secret_keyring
                            .keys()
                            .first()
                            .map(|key| key.fingerprint.clone());
                    }
                    toast = Some((tr(self.language, "已移除私钥").to_string(), ToastKind::Info));
                }
                Ok(false) => {}
                Err(error) => {
                    state.secret_key_error = Some(error.to_string());
                    toast = Some((error.to_string(), ToastKind::Error));
                }
            }
        }
        if let Some((message, kind)) = toast {
            self.show_toast(message, kind, cx);
        }
        cx.notify();
    }

    fn signature_open_sign_output_dir(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self
            .signature_state
            .as_ref()
            .and_then(|state| state.sign_report.as_ref())
            .and_then(|report| output_parent(&report.output))
        else {
            return;
        };
        if let Err(error) = open::that(path) {
            self.show_toast(error.to_string(), ToastKind::Error, cx);
        }
        cx.notify();
    }

    fn signature_sign_clear(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = &mut self.signature_state {
            if state.sign_running {
                return;
            }
            state.sign_file = None;
            state.sign_output = None;
            state.sign_output_custom = false;
            state.sign_report = None;
            state.sign_error = None;
        }
        cx.notify();
    }

    fn start_signature_sign(&mut self, cx: &mut Context<Self>) {
        let Some(state) = &mut self.signature_state else {
            return;
        };
        if state.sign_running {
            return;
        }
        let Some(file) = state.sign_file.clone() else {
            state.sign_error = Some(tr(self.language, "请选择要签名的文件").to_string());
            return;
        };
        let Some(output) = state.sign_output.clone() else {
            state.sign_error = Some(tr(self.language, "请选择签名输出文件").to_string());
            return;
        };
        let Some(fingerprint) = state.signing_key.clone() else {
            state.sign_error = Some(tr(self.language, "请选择签名私钥").to_string());
            return;
        };
        let passphrase = state.passphrase.read(cx).value().to_string();
        if passphrase.is_empty() {
            state.sign_error = Some(tr(self.language, "请输入私钥口令").to_string());
            return;
        }
        state.sign_running = true;
        state.sign_error = None;
        state.sign_report = None;
        state.sign_run_id += 1;
        let run_id = state.sign_run_id;
        let armored = state.armor;
        let keyring = state.secret_keyring.clone();
        let sender = state.sender.clone();
        std::thread::spawn(move || {
            let result = keyring
                .sign(&file, &output, &fingerprint, &passphrase, armored)
                .map_err(|error| error.to_string());
            let _ = sender.send(SignatureEvent::Signed { run_id, result });
        });
        cx.notify();
    }

    fn start_signature_verify(&mut self, cx: &mut Context<Self>) {
        let Some(state) = &mut self.signature_state else {
            return;
        };
        if state.running {
            return;
        }
        let Some(signature) = state.signature.clone() else {
            state.error = Some(tr(self.language, "请选择签名文件").to_string());
            return;
        };
        if !state.clearsigned && state.data.is_none() {
            state.error = Some(tr(self.language, "分离签名需要选择被签名的原始文件").to_string());
            return;
        }
        state.running = true;
        state.error = None;
        state.report = None;
        state.status = tr(self.language, "正在校验…").to_string();
        state.run_id += 1;
        let run_id = state.run_id;
        let keyring = state.keyring.clone();
        let data = state.data.clone();
        let sender = state.sender.clone();
        std::thread::spawn(move || {
            let result = keyring
                .verify(&signature, data.as_deref())
                .map_err(|error| error.to_string());
            let _ = sender.send(SignatureEvent::Verified { run_id, result });
        });
        cx.notify();
    }

    pub(crate) fn drain_signature_events(&mut self, _cx: &mut Context<Self>) -> bool {
        let Some(state) = &mut self.signature_state else {
            return false;
        };
        let mut changed = false;
        while let Ok(event) = state.receiver.try_recv() {
            match event {
                SignatureEvent::Verified { run_id, result } => {
                    if run_id != state.run_id {
                        continue;
                    }
                    state.running = false;
                    match result {
                        Ok(report) => {
                            state.report = Some(report);
                            state.status = tr(self.language, "校验完成").to_string();
                        }
                        Err(error) => {
                            state.error = Some(error);
                            state.status = tr(self.language, "校验失败").to_string();
                        }
                    }
                }
                SignatureEvent::Signed { run_id, result } => {
                    if run_id != state.sign_run_id {
                        continue;
                    }
                    state.sign_running = false;
                    match result {
                        Ok(report) => state.sign_report = Some(report),
                        Err(error) => state.sign_error = Some(error),
                    }
                }
            }
            changed = true;
        }
        changed
    }

    pub(crate) fn render_signature_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _ = window;
        let palette = self.palette;
        let language = self.language;
        let Some(state) = self.signature_state.as_ref() else {
            return div().into_any_element();
        };
        if state.tab == SignatureTab::Sign {
            return self.render_signature_sign_page(window, cx);
        }
        let keys = state.keyring.keys();
        let secret_count = state.secret_keyring.len();
        let warnings = state.keyring.warnings().to_vec();
        let signature = state.signature.clone();
        let data = state.data.clone();
        let clearsigned = state.clearsigned;
        let running = state.running;
        let report = state.report.clone();
        let error = state.error.clone();
        let key_error = state.key_error.clone();
        let status = state.status.clone();
        let incomplete = signature.is_none() || (!clearsigned && data.is_none());

        let key_list = if keys.is_empty() {
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .p(px(12.0))
                .rounded(px(10.0))
                .bg(palette.panel)
                .child(icon(IconName::KeyRound, palette.muted, 18.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(theme::body())
                                .text_color(palette.sub)
                                .child(tr(language, "尚未导入公钥")),
                        )
                        .child(
                            div()
                                .text_size(theme::caption())
                                .text_color(palette.muted)
                                .child(tr(
                                    language,
                                    "导入签名者的公钥（.asc / .gpg）后即可离线校验。",
                                )),
                        ),
                )
                .into_any_element()
        } else {
            let mut list = div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .max_h(px(152.0))
                .overflow_y_scrollbar();
            for key in &keys {
                list = list.child(render_key_row(key, &palette, language, cx));
            }
            list.into_any_element()
        };

        let keyring_card = design::card(&palette)
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
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(design::panel_title(
                                &palette,
                                IconName::KeyRound,
                                tr(language, "本地公钥环"),
                            ))
                            .child(design::pill(
                                &palette,
                                format!("{}", keys.len()),
                                design::TagTone::Neutral,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                Button::new("signature-open-key-dir")
                                    .label(tr(language, "打开目录"))
                                    .ghost()
                                    .h(px(30.0))
                                    .px(px(10.0))
                                    .text_size(theme::caption())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.signature_open_key_dir(cx);
                                    })),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "signature-import-key".to_string())
                                    .child(
                                        Button::new("signature-import-key")
                                            .label(tr(language, "导入公钥…"))
                                            .outline()
                                            .h(px(30.0))
                                            .px(px(12.0))
                                            .text_size(theme::caption())
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.signature_import_key(window, cx);
                                            })),
                                    ),
                            ),
                    ),
            )
            .child(key_list)
            .when_some(key_error, |card, error| {
                card.child(design::info_note(&palette, error))
            })
            .when(!warnings.is_empty(), |card| {
                card.child(
                    div()
                        .text_size(theme::caption())
                        .text_color(palette.warn)
                        .child(warnings.join("；")),
                )
            });

        let signature_body = if let Some(path) = &signature {
            selected_file_row(
                path,
                IconName::Signature,
                &palette,
                language,
                IconName::RefreshCw,
                cx.listener(|this, _, window, cx| {
                    this.signature_pick_signature(window, cx);
                }),
                cx.listener(|this, _, _, cx| {
                    if let Some(state) = &mut this.signature_state {
                        state.signature = None;
                        state.data = None;
                        state.clearsigned = false;
                        state.report = None;
                        state.error = None;
                    }
                    cx.notify();
                }),
            )
        } else {
            design::dropzone(
                &palette,
                IconName::Signature,
                28.0,
                tr(language, "选择或拖入签名文件"),
                tr(language, "支持 .sig / .asc / .gpg 与明文签名"),
                vec![div()
                    .debug_selector(|| "signature-pick-signature".to_string())
                    .child(
                        Button::new("signature-pick-signature")
                            .label(tr(language, "选择签名…"))
                            .outline()
                            .h(px(30.0))
                            .px(px(12.0))
                            .text_size(theme::caption())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.signature_pick_signature(window, cx);
                            })),
                    )
                    .into_any_element()],
                110.0,
            )
        };

        let signature_card = design::card(&palette)
            .p(px(16.0))
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(10.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(design::panel_title(
                        &palette,
                        IconName::Signature,
                        tr(language, "签名文件"),
                    ))
                    .when(clearsigned, |row| {
                        row.child(design::pill(
                            &palette,
                            tr(language, "明文签名"),
                            design::TagTone::Ok,
                        ))
                    }),
            )
            .child(signature_body)
            .when(clearsigned, |card| {
                card.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .text_size(theme::caption())
                        .text_color(palette.ok)
                        .child(icon(IconName::FileCheck, palette.ok, 15.0))
                        .child(tr(language, "签名与文本在同一文件中，无需原始文件")),
                )
            });

        let data_body = if clearsigned {
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .p(px(12.0))
                .rounded(px(10.0))
                .bg(palette.ok_soft)
                .child(icon(IconName::FileCheck, palette.ok, 20.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(theme::body())
                                .text_color(palette.text)
                                .child(tr(language, "明文签名")),
                        )
                        .child(
                            div()
                                .text_size(theme::caption())
                                .text_color(palette.sub)
                                .child(tr(language, "签名与文本在同一文件中，无需原始文件")),
                        ),
                )
                .into_any_element()
        } else if let Some(path) = &data {
            selected_file_row(
                path,
                IconName::FileText,
                &palette,
                language,
                IconName::RefreshCw,
                cx.listener(|this, _, window, cx| {
                    this.signature_pick_data(window, cx);
                }),
                cx.listener(|this, _, _, cx| {
                    if let Some(state) = &mut this.signature_state {
                        state.data = None;
                        state.report = None;
                        state.error = None;
                    }
                    cx.notify();
                }),
            )
        } else {
            design::dropzone(
                &palette,
                IconName::FileText,
                28.0,
                tr(language, "选择或拖入原始文件"),
                tr(language, "分离签名需要原始文件；明文签名可留空"),
                vec![div()
                    .debug_selector(|| "signature-pick-data".to_string())
                    .child(
                        Button::new("signature-pick-data")
                            .label(tr(language, "选择文件…"))
                            .outline()
                            .h(px(30.0))
                            .px(px(12.0))
                            .text_size(theme::caption())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.signature_pick_data(window, cx);
                            })),
                    )
                    .into_any_element()],
                110.0,
            )
        };

        let data_card = design::card(&palette)
            .p(px(16.0))
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(10.0))
            .child(design::panel_title(
                &palette,
                IconName::FileText,
                tr(language, "被签名文件"),
            ))
            .child(data_body);

        let selection_row = div()
            .flex()
            .gap(px(14.0))
            .child(signature_card)
            .child(data_card);

        let actions = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(
                div()
                    .debug_selector(|| "signature-verify".to_string())
                    .child(
                        Button::new("signature-verify")
                            .label(tr(language, "校验签名"))
                            .icon(icon(IconName::ShieldCheck, palette.accent_text, 15.0))
                            .primary()
                            .disabled(running || incomplete)
                            .h(px(components::CONTROL_HEIGHT))
                            .px(px(20.0))
                            .text_size(theme::body())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.start_signature_verify(cx);
                            })),
                    ),
            )
            .child(
                Button::new("signature-clear")
                    .label(tr(language, "清空"))
                    .ghost()
                    .disabled(running)
                    .h(px(components::CONTROL_HEIGHT))
                    .px(px(14.0))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.signature_clear(cx);
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_size(theme::caption())
                    .text_color(if incomplete {
                        palette.muted
                    } else {
                        palette.sub
                    })
                    .child(if incomplete {
                        tr(language, "请选择签名文件和原始文件").to_string()
                    } else {
                        status
                    }),
            );

        let result_card = render_result_card(&palette, language, running, &report, &error);

        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(keyring_card)
            .child(selection_row)
            .child(actions)
            .child(result_card);

        let header = self.render_signature_header(
            &palette,
            language,
            SignatureTab::Verify,
            keys.len(),
            secret_count,
            cx,
        );

        div()
            .id("signature-root")
            .debug_selector(|| "signature-root".to_string())
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
                    .child(div().px(px(20.0)).pb(px(20.0)).child(body)),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_signature_header(
        &self,
        palette: &Palette,
        language: Language,
        tab: SignatureTab,
        public_count: usize,
        secret_count: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tabs = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(self.signature_tab_button(
                "signature-tab-verify",
                IconName::ShieldCheck,
                tr(language, "校验"),
                tab == SignatureTab::Verify,
                SignatureTab::Verify,
                cx,
            ))
            .child(self.signature_tab_button(
                "signature-tab-sign",
                IconName::Signature,
                tr(language, "签名"),
                tab == SignatureTab::Sign,
                SignatureTab::Sign,
                cx,
            ));
        div()
            .flex()
            .items_center()
            .justify_between()
            .pt(px(16.0))
            .px(px(20.0))
            .pb(px(12.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .w(px(38.0))
                            .h(px(38.0))
                            .rounded(px(10.0))
                            .bg(palette.accent_soft)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon(IconName::Signature, palette.accent, 20.0)),
                    )
                    .child(
                        div()
                            .text_size(theme::font_px(17.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(tr(language, "GnuPG 签名工具")),
                    ),
            )
            .child(tabs)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(design::pill(
                        palette,
                        format!("{} {}", tr(language, "公钥"), public_count),
                        design::TagTone::Neutral,
                    ))
                    .child(design::pill(
                        palette,
                        format!("{} {}", tr(language, "私钥"), secret_count),
                        design::TagTone::Neutral,
                    )),
            )
            .into_any_element()
    }

    fn signature_tab_button(
        &self,
        id: &'static str,
        icon_name: IconName,
        label: &'static str,
        selected: bool,
        tab: SignatureTab,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .role(Role::Button)
            .aria_label(label)
            .flex()
            .items_center()
            .gap(px(7.0))
            .h(px(34.0))
            .px(px(14.0))
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
                if let Some(state) = &mut this.signature_state {
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
                15.0,
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

    fn render_signature_sign_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let language = self.language;
        let Some(state) = self.signature_state.as_ref() else {
            return div().into_any_element();
        };
        let keys = state.secret_keyring.keys();
        let warnings = state.secret_keyring.warnings().to_vec();
        let public_count = state.keyring.len();
        let secret_count = keys.len();
        let signing_key_label = state.selected_signing_key_label();
        let sign_file = state.sign_file.clone();
        let sign_output = state.sign_output.clone();
        let armor = state.armor;
        let show_passphrase = state.show_passphrase;
        let passphrase = state.passphrase.read(cx).value().to_string();
        let sign_running = state.sign_running;
        let sign_report = state.sign_report.clone();
        let sign_error = state.sign_error.clone();
        let secret_key_error = state.secret_key_error.clone();
        let incomplete =
            sign_file.is_none() || sign_output.is_none() || signing_key_label.is_none();

        let key_list = if keys.is_empty() {
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .p(px(12.0))
                .rounded(px(10.0))
                .bg(palette.panel)
                .child(icon(IconName::LockKeyhole, palette.muted, 18.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(theme::body())
                                .text_color(palette.sub)
                                .child(tr(language, "尚未导入私钥")),
                        )
                        .child(
                            div()
                                .text_size(theme::caption())
                                .text_color(palette.muted)
                                .child(tr(language, "仅支持带口令的 OpenPGP 私钥。")),
                        ),
                )
                .into_any_element()
        } else {
            let mut list = div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .max_h(px(152.0))
                .overflow_y_scrollbar();
            for key in &keys {
                list = list.child(render_secret_key_row(key, &palette, language, cx));
            }
            list.into_any_element()
        };

        let secret_keyring_card = design::card(&palette)
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
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(design::panel_title(
                                &palette,
                                IconName::LockKeyhole,
                                tr(language, "签名私钥"),
                            ))
                            .child(design::pill(
                                &palette,
                                format!("{}", keys.len()),
                                design::TagTone::Neutral,
                            )),
                    )
                    .child(
                        div()
                            .debug_selector(|| "signature-secret-import".to_string())
                            .child(
                                Button::new("signature-secret-import")
                                    .label(tr(language, "导入私钥…"))
                                    .outline()
                                    .h(px(30.0))
                                    .px(px(12.0))
                                    .text_size(theme::caption())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.signature_import_secret_key(window, cx);
                                    })),
                            ),
                    ),
            )
            .child(key_list)
            .when_some(secret_key_error, |card, error| {
                card.child(design::info_note(&palette, error))
            })
            .when(!warnings.is_empty(), |card| {
                card.child(
                    div()
                        .text_size(theme::caption())
                        .text_color(palette.warn)
                        .child(warnings.join("；")),
                )
            });

        let sign_file_body = if let Some(path) = &sign_file {
            selected_file_row(
                path,
                IconName::FileText,
                &palette,
                language,
                IconName::RefreshCw,
                cx.listener(|this, _, window, cx| {
                    this.signature_pick_sign_file(window, cx);
                }),
                cx.listener(|this, _, _, cx| {
                    if let Some(state) = &mut this.signature_state {
                        state.sign_file = None;
                        state.sign_output = None;
                        state.sign_output_custom = false;
                        state.sign_report = None;
                        state.sign_error = None;
                    }
                    cx.notify();
                }),
            )
        } else {
            design::dropzone(
                &palette,
                IconName::FileText,
                28.0,
                tr(language, "选择或拖入要签名的文件"),
                tr(language, "支持任意文件类型"),
                vec![div()
                    .debug_selector(|| "signature-pick-sign-file".to_string())
                    .child(
                        Button::new("signature-pick-sign-file")
                            .label(tr(language, "选择文件…"))
                            .outline()
                            .h(px(30.0))
                            .px(px(12.0))
                            .text_size(theme::caption())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.signature_pick_sign_file(window, cx);
                            })),
                    )
                    .into_any_element()],
                130.0,
            )
        };

        let sign_file_card = design::card(&palette)
            .p(px(16.0))
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(10.0))
            .child(design::panel_title(
                &palette,
                IconName::FileText,
                tr(language, "要签名的文件"),
            ))
            .child(sign_file_body)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .pt(px(2.0))
                    .text_size(theme::caption())
                    .text_color(palette.muted)
                    .child(icon(IconName::Info, palette.muted, 14.0))
                    .child(tr(
                        language,
                        "原文件不会被修改，签名会写入单独的 .sig / .asc 文件。",
                    )),
            );

        let output_label = sign_output
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| tr(language, "选择文件后自动生成").to_string());
        let passphrase_input = components::control_height(Input::new(&state.passphrase))
            .text_size(theme::body())
            .w_full();
        let options_card = design::card(&palette)
            .p(px(16.0))
            .flex()
            .flex_col()
            .w(px(430.0))
            .gap(px(12.0))
            .child(design::panel_title(
                &palette,
                IconName::Signature,
                tr(language, "签名选项"),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(design::field_label(&palette, tr(language, "签名私钥")))
                    .child(self.signature_signing_key_trigger(
                        signing_key_label.as_deref(),
                        keys.is_empty(),
                        &palette,
                        language,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(design::field_label(&palette, tr(language, "输出文件")))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(theme::caption())
                                    .text_color(palette.sub)
                                    .child(output_label),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "signature-pick-sign-output".to_string())
                                    .child(
                                        Button::new("signature-pick-sign-output")
                                            .label(tr(language, "另存为…"))
                                            .outline()
                                            .h(px(30.0))
                                            .px(px(10.0))
                                            .text_size(theme::caption())
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.signature_pick_sign_output(window, cx);
                                            })),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(design::check_box(
                        &palette,
                        "signature-armor",
                        armor,
                        sign_running,
                        "ASCII armor (.asc)",
                        cx.listener(|this, _, _, cx| {
                            let next = this
                                .signature_state
                                .as_ref()
                                .map(|state| !state.armor)
                                .unwrap_or(true);
                            if let Some(state) = &mut this.signature_state {
                                state.set_armor(next);
                            }
                            this.settings.signature.sign_armor = next;
                            this.save_settings();
                            cx.notify();
                        }),
                    ))
                    .child(
                        div()
                            .text_size(theme::body())
                            .child(tr(language, "ASCII armor (.asc)")),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(theme::caption())
                            .text_color(palette.muted)
                            .child("SHA-256"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(design::field_label(&palette, tr(language, "私钥口令")))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(passphrase_input)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .h(px(components::CONTROL_HEIGHT))
                                    .child(design::icon_action(
                                        &palette,
                                        "signature-toggle-passphrase",
                                        if show_passphrase {
                                            IconName::EyeOff
                                        } else {
                                            IconName::Eye
                                        },
                                        palette.sub,
                                        false,
                                        tr(language, "显示口令"),
                                        cx.listener(|this, _, window, cx| {
                                            let (show, passphrase) = {
                                                let Some(state) = &mut this.signature_state else {
                                                    return;
                                                };
                                                state.show_passphrase = !state.show_passphrase;
                                                (state.show_passphrase, state.passphrase.clone())
                                            };
                                            passphrase.update(cx, |input, cx| {
                                                input.set_masked(!show, window, cx);
                                            });
                                            cx.notify();
                                        }),
                                    )),
                            ),
                    ),
            );

        let form_row = div()
            .flex()
            .items_stretch()
            .gap(px(14.0))
            .child(sign_file_card)
            .child(options_card);

        let sign_status = if sign_running {
            tr(language, "正在签名…").to_string()
        } else if sign_report.is_some() {
            tr(language, "签名完成").to_string()
        } else if sign_error.is_some() {
            tr(language, "签名失败").to_string()
        } else {
            tr(language, "就绪").to_string()
        };
        let actions = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(
                div().debug_selector(|| "signature-sign".to_string()).child(
                    Button::new("signature-sign")
                        .label(tr(language, "生成签名"))
                        .icon(icon(IconName::Signature, palette.accent_text, 15.0))
                        .primary()
                        .disabled(sign_running || incomplete)
                        .h(px(components::CONTROL_HEIGHT))
                        .px(px(20.0))
                        .text_size(theme::body())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.start_signature_sign(cx);
                        })),
                ),
            )
            .child(
                Button::new("signature-sign-clear")
                    .label(tr(language, "清空"))
                    .ghost()
                    .disabled(sign_running)
                    .h(px(components::CONTROL_HEIGHT))
                    .px(px(14.0))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.signature_sign_clear(cx);
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_size(theme::caption())
                    .text_color(if incomplete && !sign_running {
                        palette.muted
                    } else {
                        palette.sub
                    })
                    .child(if passphrase.is_empty() && !incomplete && !sign_running {
                        tr(language, "请输入私钥口令").to_string()
                    } else if incomplete {
                        tr(language, "请选择文件、私钥和输出位置").to_string()
                    } else {
                        sign_status
                    }),
            );

        let result_card = render_sign_result_card(
            &palette,
            language,
            sign_running,
            &sign_report,
            &sign_error,
            cx,
        );

        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(secret_keyring_card)
            .child(form_row)
            .child(actions)
            .child(result_card);
        let header = self.render_signature_header(
            &palette,
            language,
            SignatureTab::Sign,
            public_count,
            secret_count,
            cx,
        );
        let _ = window;
        div()
            .id("signature-root")
            .debug_selector(|| "signature-root".to_string())
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
                    .child(div().px(px(20.0)).pb(px(20.0)).child(body)),
            )
            .into_any_element()
    }

    fn signature_signing_key_trigger(
        &self,
        selected: Option<&str>,
        empty: bool,
        palette: &Palette,
        language: Language,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = selected
            .map(ToString::to_string)
            .unwrap_or_else(|| tr(language, "未选择私钥").to_string());
        if empty {
            return div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .h(px(components::CONTROL_HEIGHT))
                .px(px(12.0))
                .rounded(px(theme::RADIUS_CONTROL))
                .border_1()
                .border_color(palette.line_strong)
                .bg(palette.panel)
                .child(
                    div()
                        .truncate()
                        .text_size(theme::body())
                        .text_color(palette.muted)
                        .child(label),
                )
                .child(icon(IconName::ChevronDown, palette.muted, 15.0))
                .into_any_element();
        }
        self.track_dropdown(
            DropdownKind::SignatureSigningKey,
            div()
                .id("signature-signing-key")
                .debug_selector(|| "signature-signing-key".to_string())
                .role(Role::Button)
                .aria_label(tr(language, "签名私钥"))
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
                    this.open_dropdown(DropdownKind::SignatureSigningKey, cx);
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
}

impl SignatureState {
    fn set_signature(&mut self, path: PathBuf) {
        self.clearsigned = is_clearsigned(&path);
        if self.clearsigned {
            self.data = None;
        } else if let Some(paired) = openpgp::paired_data_path(&path) {
            self.data = Some(paired);
        }
        self.signature = Some(path);
        self.report = None;
        self.error = None;
    }

    fn set_data(&mut self, path: PathBuf) {
        if self.signature.is_none() {
            self.signature = openpgp::paired_signature_path(&path);
            if let Some(signature) = &self.signature {
                self.clearsigned = is_clearsigned(signature);
            }
        }
        self.data = Some(path);
        self.report = None;
        self.error = None;
    }

    fn set_sign_file(&mut self, path: PathBuf) {
        self.sign_file = Some(path.clone());
        if !self.sign_output_custom || self.sign_output.is_none() {
            self.sign_output = Some(default_sign_output(&path, self.armor));
        }
        self.sign_report = None;
        self.sign_error = None;
    }

    fn set_sign_output(&mut self, path: PathBuf) {
        self.sign_output = Some(path);
        self.sign_output_custom = true;
        self.sign_report = None;
        self.sign_error = None;
    }

    fn set_armor(&mut self, armor: bool) {
        self.armor = armor;
        if !self.sign_output_custom {
            if let Some(file) = &self.sign_file {
                self.sign_output = Some(default_sign_output(file, armor));
            }
        }
        self.sign_report = None;
        self.sign_error = None;
    }

    pub(crate) fn signing_key_options(&self) -> Vec<(String, String)> {
        self.secret_keyring
            .keys()
            .into_iter()
            .map(|key| {
                (
                    key.fingerprint.clone(),
                    format!("{} · {}", key.user_id, short_fingerprint(&key.fingerprint)),
                )
            })
            .collect()
    }

    pub(crate) fn selected_signing_key(&self) -> Option<String> {
        self.signing_key.clone()
    }

    pub(crate) fn selected_signing_key_label(&self) -> Option<String> {
        self.signing_key_options()
            .into_iter()
            .find(|(fingerprint, _)| Some(fingerprint) == self.signing_key.as_ref())
            .map(|(_, label)| label)
    }

    pub(crate) fn select_signing_key(&mut self, fingerprint: String) {
        if self
            .signing_key_options()
            .iter()
            .any(|(candidate, _)| *candidate == fingerprint)
        {
            self.signing_key = Some(fingerprint);
            self.sign_report = None;
            self.sign_error = None;
        }
    }
}

fn default_sign_output(input: &Path, armored: bool) -> PathBuf {
    let parent = input
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = input
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "signature".to_string());
    unique_path(parent, &name, Some(if armored { "asc" } else { "sig" }))
}

fn output_parent(path: &Path) -> Option<&Path> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
}

fn short_fingerprint(fingerprint: &str) -> String {
    let upper = fingerprint.to_ascii_uppercase();
    let start = upper.len().saturating_sub(8);
    format_fingerprint(&upper[start..])
}

fn apply_signature_paths(state: &mut SignatureState, paths: &[PathBuf]) {
    let mut signature = None;
    let mut data = None;
    for path in paths {
        if !path.is_file() {
            continue;
        }
        if is_clearsigned(path) || is_signature_file(path) {
            signature.get_or_insert(path.clone());
        } else {
            data.get_or_insert(path.clone());
        }
    }
    if let Some(signature) = signature {
        state.set_signature(signature);
        if !state.clearsigned {
            if let Some(data) = data {
                state.data = Some(data);
            }
        }
    } else if let Some(data) = data {
        state.set_data(data);
    }
}

fn render_key_row(
    key: &KeyInfo,
    palette: &Palette,
    language: Language,
    cx: &mut Context<MainView>,
) -> AnyElement {
    let fingerprint = key.fingerprint.clone();
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .px(px(10.0))
        .py(px(8.0))
        .rounded(px(9.0))
        .border_1()
        .border_color(palette.line)
        .bg(palette.panel)
        .child(icon(IconName::KeyRound, palette.accent, 16.0))
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
                        .child(key.user_id.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(theme::font_px(11.0))
                        .text_color(palette.muted)
                        .child(format_fingerprint(&key.fingerprint)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .items_end()
                .gap(px(2.0))
                .child(
                    div()
                        .text_size(theme::caption())
                        .text_color(palette.sub)
                        .child(key.algorithm.clone()),
                )
                .when_some(key.expires_at, |column, expires_at| {
                    column.child(
                        div()
                            .text_size(theme::font_px(11.0))
                            .text_color(palette.muted)
                            .child(format!(
                                "{} {}",
                                tr(language, "至"),
                                format_datetime(expires_at)
                            )),
                    )
                }),
        )
        .child(design::icon_action(
            palette,
            format!("signature-remove-key-{fingerprint}"),
            IconName::X,
            palette.muted,
            false,
            tr(language, "移除公钥"),
            cx.listener(move |this, _, _, cx| {
                this.signature_remove_key(fingerprint.clone(), cx);
            }),
        ))
        .into_any_element()
}

fn render_secret_key_row(
    key: &SecretKeyInfo,
    palette: &Palette,
    language: Language,
    cx: &mut Context<MainView>,
) -> AnyElement {
    let fingerprint = key.fingerprint.clone();
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .px(px(10.0))
        .py(px(8.0))
        .rounded(px(9.0))
        .border_1()
        .border_color(palette.line)
        .bg(palette.panel)
        .child(icon(IconName::LockKeyhole, palette.accent, 16.0))
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
                        .child(key.user_id.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(theme::font_px(11.0))
                        .text_color(palette.muted)
                        .child(format_fingerprint(&key.fingerprint)),
                ),
        )
        .when(key.revoked, |row| {
            row.child(design::pill(
                palette,
                tr(language, "已吊销"),
                design::TagTone::Danger,
            ))
        })
        .child(
            div()
                .text_size(theme::caption())
                .text_color(palette.sub)
                .child(key.algorithm.clone()),
        )
        .child(design::icon_action(
            palette,
            format!("signature-remove-secret-{fingerprint}"),
            IconName::X,
            palette.muted,
            false,
            tr(language, "移除私钥"),
            cx.listener(move |this, _, _, cx| {
                this.signature_remove_secret_key(fingerprint.clone(), cx);
            }),
        ))
        .into_any_element()
}

fn render_sign_result_card(
    palette: &Palette,
    language: Language,
    running: bool,
    report: &Option<SignReport>,
    error: &Option<String>,
    cx: &mut Context<MainView>,
) -> AnyElement {
    let mut card = design::card(palette)
        .p(px(16.0))
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(design::panel_title(
                    palette,
                    IconName::Signature,
                    tr(language, "签名结果"),
                ))
                .when_some(report.as_ref(), |row, report| {
                    row.child(design::pill(
                        palette,
                        if report.armored {
                            tr(language, "ASCII armor")
                        } else {
                            tr(language, "二进制签名")
                        },
                        design::TagTone::Neutral,
                    ))
                }),
        );
    if running {
        return card
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(icon(IconName::Signature, palette.accent, 22.0))
                    .child(
                        div()
                            .text_size(theme::body())
                            .text_color(palette.sub)
                            .child(tr(language, "正在签名…")),
                    ),
            )
            .into_any_element();
    }
    if let Some(error) = error {
        return card
            .child(design::info_note(palette, error.clone()))
            .into_any_element();
    }
    let Some(report) = report else {
        return card
            .child(
                div()
                    .py(px(20.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.0))
                    .child(icon(IconName::Signature, palette.muted, 34.0))
                    .child(
                        div()
                            .text_size(theme::body())
                            .text_color(palette.sub)
                            .child(tr(language, "尚未生成签名")),
                    )
                    .child(
                        div()
                            .text_size(theme::caption())
                            .text_color(palette.muted)
                            .child(tr(language, "选择文件、私钥并输入口令后生成签名")),
                    ),
            )
            .into_any_element();
    };

    let mut details = div().flex().flex_col().gap(px(5.0));
    details = details.child(detail_row(
        tr(language, "签名者"),
        report.user_id.clone(),
        palette,
    ));
    details = details.child(detail_row(
        tr(language, "密钥 ID"),
        report.key_id.clone(),
        palette,
    ));
    details = details.child(detail_row(
        tr(language, "指纹"),
        format_fingerprint(&report.fingerprint),
        palette,
    ));
    details = details.child(detail_row(
        tr(language, "输出文件"),
        report.output.display().to_string(),
        palette,
    ));
    details = details.child(detail_row(
        tr(language, "格式"),
        if report.armored {
            tr(language, "ASCII armor").to_string()
        } else {
            tr(language, "二进制签名").to_string()
        },
        palette,
    ));
    details = details.child(detail_row(
        tr(language, "大小"),
        rarkzip_engine::format_size(report.bytes),
        palette,
    ));

    card = card.child(
        div()
            .flex()
            .items_start()
            .gap(px(10.0))
            .child(design::pill(
                palette,
                tr(language, "签名已生成"),
                design::TagTone::Ok,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(theme::caption())
                    .text_color(palette.sub)
                    .child(tr(language, "签名文件已写入磁盘。")),
            )
            .child(
                Button::new("signature-open-sign-output")
                    .label(tr(language, "打开所在位置"))
                    .ghost()
                    .h(px(28.0))
                    .px(px(10.0))
                    .text_size(theme::caption())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.signature_open_sign_output_dir(cx);
                    })),
            ),
    );
    card.child(details).into_any_element()
}

fn selected_file_row(
    path: &Path,
    icon_name: IconName,
    palette: &Palette,
    language: Language,
    replace_icon: IconName,
    on_replace: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_clear: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .p(px(12.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(palette.line)
        .bg(palette.panel)
        .child(icon(icon_name, palette.accent, 20.0))
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
                        .child(path.display().to_string()),
                ),
        )
        .child(
            Button::new(format!("signature-replace-{}", path.display()))
                .label(tr(language, "更换…"))
                .icon(icon(replace_icon, palette.accent, 14.0))
                .outline()
                .h(px(28.0))
                .px(px(10.0))
                .text_size(theme::caption())
                .on_click(on_replace),
        )
        .child(design::icon_action(
            palette,
            format!("signature-clear-{}", path.display()),
            IconName::X,
            palette.muted,
            false,
            tr(language, "清除"),
            on_clear,
        ))
        .into_any_element()
}

fn render_result_card(
    palette: &Palette,
    language: Language,
    running: bool,
    report: &Option<VerificationReport>,
    error: &Option<String>,
) -> AnyElement {
    let mut card = design::card(palette)
        .p(px(16.0))
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(design::panel_title(
                    palette,
                    IconName::ShieldCheck,
                    tr(language, "校验结果"),
                ))
                .when_some(report.as_ref(), |row, report| {
                    row.child(design::pill(
                        palette,
                        if report.cleartext {
                            tr(language, "明文签名")
                        } else {
                            tr(language, "分离签名")
                        },
                        design::TagTone::Neutral,
                    ))
                }),
        );

    if running {
        return card
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(icon(IconName::ShieldCheck, palette.accent, 22.0))
                    .child(
                        div()
                            .text_size(theme::body())
                            .text_color(palette.sub)
                            .child(tr(language, "正在校验…")),
                    ),
            )
            .into_any_element();
    }
    if let Some(error) = error {
        return card
            .child(design::info_note(palette, error.clone()))
            .into_any_element();
    }
    let Some(report) = report else {
        return card
            .child(
                div()
                    .py(px(20.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.0))
                    .child(icon(IconName::ShieldCheck, palette.muted, 34.0))
                    .child(
                        div()
                            .text_size(theme::body())
                            .text_color(palette.sub)
                            .child(tr(language, "尚未校验")),
                    )
                    .child(
                        div()
                            .text_size(theme::caption())
                            .text_color(palette.muted)
                            .child(tr(language, "选择签名文件和被签名文件后开始校验")),
                    ),
            )
            .into_any_element();
    };

    card = card.child(
        div()
            .flex()
            .items_start()
            .gap(px(10.0))
            .child(
                div()
                    .debug_selector(move || format!("signature-result-{:?}", report.overall))
                    .child(design::pill(
                        palette,
                        signature_state_label(report.overall, language),
                        signature_state_tone(report.overall),
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(theme::caption())
                    .text_color(palette.sub)
                    .child(signature_state_description(report.overall, language)),
            ),
    );

    for signature in &report.signatures {
        card = card.child(render_signature_row(signature, palette, language));
    }
    card.into_any_element()
}

fn render_signature_row(
    signature: &VerifiedSignature,
    palette: &Palette,
    language: Language,
) -> AnyElement {
    let tone = signature_state_tone(signature.state);
    let accent = match tone {
        design::TagTone::Ok => palette.ok,
        design::TagTone::Danger => palette.bad,
        design::TagTone::Manual => palette.warn,
        design::TagTone::Accent => palette.accent,
        design::TagTone::Neutral => palette.sub,
    };
    let title = signature
        .user_id
        .clone()
        .or_else(|| signature.key_id.clone())
        .unwrap_or_else(|| tr(language, "未知签名者").to_string());
    let mut details = div().flex().flex_col().gap(px(5.0));
    if let Some(user_id) = &signature.user_id {
        details = details.child(detail_row(tr(language, "签名者"), user_id.clone(), palette));
    }
    if let Some(key_id) = &signature.key_id {
        details = details.child(detail_row(tr(language, "密钥 ID"), key_id.clone(), palette));
    }
    if let Some(fingerprint) = &signature.fingerprint {
        details = details.child(detail_row(
            tr(language, "指纹"),
            format_fingerprint(fingerprint),
            palette,
        ));
    }
    if let Some(signed_at) = signature.signed_at {
        details = details.child(detail_row(
            tr(language, "签名时间"),
            format_datetime(signed_at),
            palette,
        ));
    }
    if let Some(created_at) = signature.key_created_at {
        details = details.child(detail_row(
            tr(language, "密钥创建"),
            format_datetime(created_at),
            palette,
        ));
    }
    if let Some(expires_at) = signature.key_expires_at {
        details = details.child(detail_row(
            tr(language, "密钥过期"),
            format_datetime(expires_at),
            palette,
        ));
    }
    let mut algorithm = signature.public_key_algorithm.clone().unwrap_or_default();
    if let Some(hash) = &signature.hash_algorithm {
        if !algorithm.is_empty() {
            algorithm.push_str(" / ");
        }
        algorithm.push_str(hash);
    }
    if !algorithm.is_empty() {
        details = details.child(detail_row(tr(language, "算法"), algorithm, palette));
    }
    if let Some(detail) = &signature.detail {
        details = details.child(detail_row(tr(language, "详情"), detail.clone(), palette));
    }

    div()
        .rounded(px(10.0))
        .border_1()
        .border_color(accent.opacity(0.35))
        .bg(palette.panel)
        .p(px(12.0))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .min_w_0()
                        .child(icon(IconName::Signature, accent, 17.0))
                        .child(
                            div()
                                .truncate()
                                .text_size(theme::body())
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(palette.text)
                                .child(title),
                        ),
                )
                .child(design::pill(
                    palette,
                    signature_state_label(signature.state, language),
                    tone,
                )),
        )
        .child(details)
        .into_any_element()
}

fn detail_row(label: &'static str, value: String, palette: &Palette) -> AnyElement {
    div()
        .flex()
        .items_start()
        .gap(px(12.0))
        .child(
            div()
                .w(px(72.0))
                .flex_none()
                .text_size(theme::caption())
                .text_color(palette.muted)
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(theme::caption())
                .text_color(palette.sub)
                .child(value),
        )
        .into_any_element()
}

fn signature_state_label(state: SignatureVerdict, language: Language) -> &'static str {
    match state {
        SignatureVerdict::Valid => tr(language, "签名有效"),
        SignatureVerdict::Invalid => tr(language, "签名无效"),
        SignatureVerdict::MissingKey => tr(language, "缺少公钥"),
        SignatureVerdict::ExpiredSignature => tr(language, "签名已过期"),
        SignatureVerdict::ExpiredKey => tr(language, "公钥已过期"),
        SignatureVerdict::RevokedKey => tr(language, "公钥已吊销"),
        SignatureVerdict::InvalidKey => tr(language, "公钥无效"),
        SignatureVerdict::Error => tr(language, "校验失败"),
    }
}

fn signature_state_tone(state: SignatureVerdict) -> design::TagTone {
    match state {
        SignatureVerdict::Valid => design::TagTone::Ok,
        SignatureVerdict::Invalid | SignatureVerdict::RevokedKey | SignatureVerdict::InvalidKey => {
            design::TagTone::Danger
        }
        SignatureVerdict::MissingKey
        | SignatureVerdict::ExpiredSignature
        | SignatureVerdict::ExpiredKey
        | SignatureVerdict::Error => design::TagTone::Manual,
    }
}

fn signature_state_description(state: SignatureVerdict, language: Language) -> &'static str {
    match state {
        SignatureVerdict::Valid => tr(language, "签名与文件内容匹配，且公钥位于本地公钥环中。"),
        SignatureVerdict::Invalid => tr(language, "签名与文件内容不匹配，文件可能已被修改。"),
        SignatureVerdict::MissingKey => {
            tr(language, "本地公钥环中没有签发此签名的公钥，请先导入。")
        }
        SignatureVerdict::ExpiredSignature => tr(language, "签名已超过有效期。"),
        SignatureVerdict::ExpiredKey => tr(language, "签名本身有效，但签名密钥已过期。"),
        SignatureVerdict::RevokedKey => tr(language, "签名密钥已被吊销，不应信任。"),
        SignatureVerdict::InvalidKey => tr(language, "公钥未通过自签名校验。"),
        SignatureVerdict::Error => tr(language, "无法完成校验。"),
    }
}

fn format_datetime(value: DateTime<Utc>) -> String {
    value
        .with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

#[cfg(test)]
mod ui_tests {
    use gpui_kit::{point, AppContext, Modifiers, TestAppContext, VisualTestContext};

    use super::super::{FeatureWindow, MainView};

    struct LocalAppDataGuard {
        previous: Option<std::ffi::OsString>,
    }

    impl LocalAppDataGuard {
        fn set(path: &std::path::Path) -> Self {
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
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
    }

    #[cfg(debug_assertions)]
    #[gpui_kit::test]
    fn signature_window_renders(cx: &mut TestAppContext) {
        let _serial = crate::UI_TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempfile::tempdir().unwrap();
        let _env = LocalAppDataGuard::set(temp.path());
        cx.update(gpui_kit::init);

        let (_view, visual) = cx.add_window_view(|window, cx| {
            MainView::new_feature(window, cx, FeatureWindow::Signature, Vec::new(), None)
        });
        pump(visual);
        for selector in [
            "signature-root",
            "signature-import-key",
            "signature-pick-signature",
            "signature-pick-data",
            "signature-verify",
        ] {
            assert!(visual.debug_bounds(selector).is_some(), "未渲染 {selector}");
        }

        let tab = visual
            .debug_bounds("signature-tab-sign")
            .expect("签名页签未渲染");
        visual.simulate_click(point(tab.center().x, tab.center().y), Modifiers::default());
        pump(visual);
        for selector in [
            "signature-secret-import",
            "signature-pick-sign-file",
            "signature-sign",
        ] {
            assert!(visual.debug_bounds(selector).is_some(), "未渲染 {selector}");
        }
    }

    #[cfg(debug_assertions)]
    #[gpui_kit::test]
    fn signature_window_verifies_detached_signature(cx: &mut TestAppContext) {
        use std::time::{Duration, Instant};

        use rarkzip_engine::openpgp::SignatureState as SignatureVerdict;

        let _serial = crate::UI_TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempfile::tempdir().unwrap();
        let _env = LocalAppDataGuard::set(temp.path());
        let keys = temp.path().join("RarkZip").join("signature-keys");
        std::fs::create_dir_all(&keys).unwrap();
        std::fs::copy(fixture("public.asc"), keys.join("public.asc")).unwrap();

        cx.update(gpui_kit::init);
        let (view, visual) = cx.add_window_view(|window, cx| {
            MainView::new_feature(
                window,
                cx,
                FeatureWindow::Signature,
                vec![fixture("data.txt.asc"), fixture("data.txt")],
                None,
            )
        });
        pump(visual);

        visual.update_entity(&view, |view, cx| {
            view.start_signature_verify(cx);
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut overall = None;
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            let current = visual.update_entity(&view, |view, cx| {
                view.drain_signature_events(cx);
                view.signature_state
                    .as_ref()
                    .and_then(|state| state.report.as_ref())
                    .map(|report| report.overall)
            });
            if current.is_some() {
                overall = current;
                break;
            }
        }
        assert_eq!(overall, Some(SignatureVerdict::Valid));
        pump(visual);
        assert!(
            visual.debug_bounds("signature-result-Valid").is_some(),
            "未渲染有效签名结果"
        );
    }

    #[cfg(debug_assertions)]
    #[gpui_kit::test]
    fn signature_window_signs_file(cx: &mut TestAppContext) {
        use std::time::{Duration, Instant};

        use super::SignatureTab;

        let _serial = crate::UI_TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempfile::tempdir().unwrap();
        let _env = LocalAppDataGuard::set(temp.path());
        let keys = temp.path().join("RarkZip").join("signature-secret-keys");
        std::fs::create_dir_all(&keys).unwrap();
        std::fs::copy(fixture("signer-secret.asc"), keys.join("signer-secret.asc")).unwrap();

        cx.update(gpui_kit::init);
        let (view, visual) = cx.add_window_view(|window, cx| {
            MainView::new_feature(window, cx, FeatureWindow::Signature, Vec::new(), None)
        });
        pump(visual);

        let output = temp.path().join("sign-data.sig");
        visual.update(|window, app| {
            view.update(app, |view, cx| {
                let passphrase = {
                    let state = view.signature_state.as_mut().unwrap();
                    state.tab = SignatureTab::Sign;
                    state.set_sign_file(fixture("sign-data.txt"));
                    state.set_sign_output(output.clone());
                    state.set_armor(false);
                    state.passphrase.clone()
                };
                passphrase.update(cx, |input, cx| {
                    input.set_value("test-passphrase", window, cx);
                });
                view.start_signature_sign(cx);
            });
        });

        let deadline = Instant::now() + Duration::from_secs(15);
        let mut signed = false;
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            signed = visual.update_entity(&view, |view, cx| {
                view.drain_signature_events(cx);
                view.signature_state
                    .as_ref()
                    .is_some_and(|state| state.sign_report.is_some())
            });
            if signed {
                break;
            }
        }
        assert!(signed, "签名未完成");
        assert!(output.exists(), "签名输出文件不存在");
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../rarkzip-engine/tests/fixtures/openpgp")
            .join(name)
    }
}
