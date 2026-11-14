use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
use gpui_kit::component::slider::Slider;
use gpui_kit::component::Disableable;
use rarkzip_engine::{ArchiveFormat, HashAlgorithm};

use crate::app::impls::default_output_dir;
use crate::app::*;
use crate::design;
use crate::i18n::tr;

type ClickCallback = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HashRowStatus {
    Pending,
    Done,
    Match,
    Insecure,
    Mismatch,
    Unchecked,
    Error,
}

fn expected_hash_for<'a>(
    path: &std::path::Path,
    expected: &'a std::collections::HashMap<String, String>,
) -> Option<&'a str> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    expected
        .get(&name)
        .or_else(|| expected.get("*"))
        .map(String::as_str)
}

fn hash_row_status(
    item: &HashBatchItem,
    expected: &std::collections::HashMap<String, String>,
) -> HashRowStatus {
    if item.error.is_some() {
        return HashRowStatus::Error;
    }
    if item.hashes.is_empty() {
        return HashRowStatus::Pending;
    }
    if expected.is_empty() {
        return HashRowStatus::Done;
    }
    let Some(expected) = expected_hash_for(&item.path, expected) else {
        return HashRowStatus::Unchecked;
    };
    let matches: Vec<HashAlgorithm> = item
        .hashes
        .iter()
        .filter(|(_, hash)| hash.eq_ignore_ascii_case(expected))
        .map(|(algorithm, _)| *algorithm)
        .collect();
    if matches.is_empty() {
        HashRowStatus::Mismatch
    } else if matches
        .iter()
        .all(|algorithm| matches!(algorithm, HashAlgorithm::Md5 | HashAlgorithm::Sha1))
    {
        HashRowStatus::Insecure
    } else {
        HashRowStatus::Match
    }
}

fn hash_export_contents(
    items: &[HashBatchItem],
    format: HashExportFormat,
    hash_settings: &rarkzip_engine::HashSettings,
) -> String {
    let algorithm = match format {
        HashExportFormat::Sha256Sum => HashAlgorithm::Sha256,
        HashExportFormat::Md5Sum => HashAlgorithm::Md5,
        HashExportFormat::Sfv => HashAlgorithm::Crc32,
    };
    let mut lines = Vec::new();
    for item in items {
        if item.error.is_some() {
            continue;
        }
        let Some((_, hash)) = item
            .hashes
            .iter()
            .find(|(candidate, _)| *candidate == algorithm)
        else {
            continue;
        };
        let name = item
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| item.path.display().to_string());
        let hash = if hash_settings.export_uppercase || format == HashExportFormat::Sfv {
            hash.to_ascii_uppercase()
        } else {
            hash.clone()
        };
        let separator = if hash_settings.export_star {
            " *"
        } else {
            "  "
        };
        if format == HashExportFormat::Sfv {
            lines.push(format!("{name}{separator}{hash}"));
        } else {
            lines.push(format!("{hash}{separator}{name}"));
        }
    }
    lines.join(if hash_settings.export_lf {
        "\n"
    } else {
        "\r\n"
    })
}

#[derive(Debug, Clone)]
struct HashDisplayRow {
    path: PathBuf,
    algorithm: Option<HashAlgorithm>,
    hash: Option<String>,
    error: Option<String>,
    matched: bool,
    insecure: bool,
}

fn hash_hex_len(algorithm: HashAlgorithm) -> usize {
    match algorithm {
        HashAlgorithm::Md5 => 32,
        HashAlgorithm::Sha1 => 40,
        HashAlgorithm::Sha256 | HashAlgorithm::Sha3 | HashAlgorithm::Blake3 => 64,
        HashAlgorithm::Sha512 => 128,
        HashAlgorithm::XxHash64 => 16,
        HashAlgorithm::Crc32 => 8,
    }
}

fn hash_display_rows(
    paths: &[PathBuf],
    results: &[HashBatchItem],
    algorithms: &[HashAlgorithm],
    expected: &std::collections::HashMap<String, String>,
    uppercase: bool,
) -> Vec<HashDisplayRow> {
    let mut rows = Vec::new();
    for path in paths {
        let result = results.iter().find(|item| &item.path == path);
        if let Some(error) = result.and_then(|item| item.error.clone()) {
            rows.push(HashDisplayRow {
                path: path.clone(),
                algorithm: None,
                hash: None,
                error: Some(error),
                matched: false,
                insecure: false,
            });
            continue;
        }
        let expected_hash = expected_hash_for(path, expected);
        for algorithm in algorithms {
            let hash = result
                .and_then(|item| {
                    item.hashes
                        .iter()
                        .find(|(candidate, _)| candidate == algorithm)
                })
                .map(|(_, hash)| {
                    if uppercase {
                        hash.to_ascii_uppercase()
                    } else {
                        hash.clone()
                    }
                });
            let matched = hash
                .as_deref()
                .zip(expected_hash)
                .is_some_and(|(hash, want)| hash.eq_ignore_ascii_case(want));
            rows.push(HashDisplayRow {
                path: path.clone(),
                algorithm: Some(*algorithm),
                hash,
                error: None,
                matched,
                insecure: matched && matches!(algorithm, HashAlgorithm::Md5 | HashAlgorithm::Sha1),
            });
        }
    }
    rows
}

fn hash_row_name(row: &HashDisplayRow) -> String {
    row.path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| row.path.display().to_string())
}

fn hash_row_text(row: &HashDisplayRow) -> String {
    format!(
        "{}	{}	{}",
        hash_row_name(row),
        row.algorithm.map(HashAlgorithm::label).unwrap_or("—"),
        row.hash.as_deref().unwrap_or("—"),
    )
}

impl MainView {
    pub(in crate::app) fn render_modal(
        &mut self,
        modal: Modal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        // 上下各预留 20px，确保弹窗底部与圆角不会被窗口边缘裁掉；
        // 内容超过可用高度时由 modal_shell 的滚动区承担。
        let max_height = (window.viewport_size().height.as_f32() - 40.0).clamp(240.0, 600.0);
        let card = match modal {
            Modal::Password {
                archive,
                pending,
                error,
                message,
            } => self.render_password_modal(archive, pending, error, message, max_height, cx),
            Modal::Rename { index } => self.render_rename_modal(index, max_height, cx),
            Modal::NewFolder { parent } => self.render_new_folder_modal(&parent, max_height, cx),
            Modal::Comment => self.render_comment_modal(max_height, cx),
            Modal::Columns => self.render_columns_modal(max_height, cx),
            Modal::Toolbar => self.render_toolbar_modal(max_height, cx),
            Modal::ConfirmDeletePassword { index } => {
                self.render_delete_password_modal(index, max_height, cx)
            }
        };
        div()
            .id("modal-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(palette.overlay)
            // 弹窗打开时阻断背后列表/文件的滚轮与鼠标交互，避免背景跟着滚动。
            .occlude()
            .on_scroll(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close_modal(cx)),
            )
            .child(
                div()
                    .id("modal-card")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    )
                    .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
                    .child(card),
            )
            .into_any_element()
    }

    fn render_password_modal(
        &mut self,
        archive: PathBuf,
        pending: PendingAction,
        error: Option<String>,
        message: Option<String>,
        max_height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let action = match &pending {
            PendingAction::Open { .. } => self.localized("打开归档"),
            PendingAction::Extract { .. } => self.localized("解压"),
            PendingAction::Verify { .. } => self.localized("校验"),
            PendingAction::OpenEntry { .. } => self.localized("打开文件"),
            PendingAction::Prompt { .. } => self.localized("继续解压"),
        };
        let archive_name = archive
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| archive.display().to_string());
        let quick_test = rarkzip_engine::supports_quick_password_test(&archive);
        let hint = match &pending {
            PendingAction::Prompt { .. } => {
                self.localized("嵌套/首层压缩包需要密码；输入正确后继续当前解压任务。")
            }
            _ if quick_test => {
                self.localized("密码本自动尝试失败后才会弹窗；手动输入成功后会自动记入密码本末尾。")
            }
            _ => self.localized(
                "该格式无法快速校验密码，不会自动尝试密码本；输入成功后会记入密码本末尾。",
            ),
        };
        let mut body = vec![
            div()
                .text_size(crate::theme::body())
                .text_color(palette.sub)
                .child(message.unwrap_or_else(|| {
                    crate::i18n::tr_fmt(self.language, "{}需要密码：{}", &[&action, &archive_name])
                }))
                .into_any_element(),
            crate::components::control_height(Input::new(&self.password_input))
                .text_size(crate::theme::body())
                .w_full()
                .into_any_element(),
            div()
                .text_size(crate::theme::caption())
                .text_color(palette.muted)
                .child(hint)
                .into_any_element(),
        ];

        // 从密码本中选择：列表默认打码，点击条目直接使用该密码。
        let book: Vec<(String, String)> = self
            .password_book
            .entries()
            .iter()
            .map(|entry| (entry.note.clone(), entry.password.clone()))
            .collect();
        let reveal = self.password_picker_reveal;
        if book.is_empty() {
            body.push(
                div()
                    .text_size(crate::theme::caption())
                    .text_color(palette.muted)
                    .child(self.localized("密码本为空，可先在密码本页面添加常用密码。"))
                    .into_any_element(),
            );
        } else {
            body.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .debug_selector(|| "password-book-toggle".to_string())
                            .child(
                                Button::new("password-book-toggle")
                                    .label(crate::i18n::tr_fmt(
                                        self.language,
                                        "从密码本选择（{}）",
                                        &[&book.len()],
                                    ))
                                    .icon(icon(IconName::BookOpen, palette.accent, 15.0))
                                    .outline()
                                    .h(px(30.0))
                                    .px(px(12.0))
                                    .text_size(crate::theme::caption())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.password_picker_open = !this.password_picker_open;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(design::icon_action(
                        &palette,
                        "password-book-reveal",
                        if reveal {
                            IconName::EyeOff
                        } else {
                            IconName::Eye
                        },
                        palette.sub,
                        false,
                        if reveal {
                            self.localized("隐藏密码")
                        } else {
                            self.localized("显示密码")
                        },
                        cx.listener(|this, _, _, cx| {
                            this.password_picker_reveal = !this.password_picker_reveal;
                            cx.notify();
                        }),
                    ))
                    .into_any_element(),
            );
            if self.password_picker_open {
                let count = book.len();
                let mut list = div()
                    .id("password-book-list")
                    .flex()
                    .flex_col()
                    .max_h(px(176.0))
                    .overflow_y_scroll()
                    .rounded(px(crate::theme::RADIUS_CONTROL))
                    .border_1()
                    .border_color(palette.line)
                    .bg(palette.panel);
                for (index, (note, password)) in book.iter().enumerate() {
                    let label = if note.is_empty() {
                        crate::i18n::tr_fmt(self.language, "密码 {}", &[&(index + 1)])
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
                            .id(format!("password-book-{index}"))
                            .debug_selector(|| format!("password-book-{index}"))
                            .role(Role::Button)
                            .aria_label(label.clone())
                            .cursor_pointer()
                            .h(px(34.0))
                            .px(px(10.0))
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .when(index + 1 < count, |this| {
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
                body.push(list.into_any_element());
                body.push(
                    div()
                        .text_size(crate::theme::caption())
                        .text_color(palette.muted)
                        .child(self.localized("点击密码即可直接尝试，无需再次点“解锁”。"))
                        .into_any_element(),
                );
            }
        }
        if let Some(error) = error {
            body.push(
                div()
                    .text_size(crate::theme::caption())
                    .text_color(palette.bad)
                    .child(error)
                    .into_any_element(),
            );
        }
        let footer = vec![
            design::secondary_action(
                "password-cancel",
                self.localized("取消"),
                cx.listener(|this, _, _, cx| this.close_modal(cx)),
            )
            .into_any_element(),
            design::primary_icon_action(
                "password-ok",
                self.localized("解锁"),
                IconName::Lock,
                &palette,
                false,
                cx.listener(|this, _, _, cx| this.submit_password(cx)),
            )
            .into_any_element(),
        ];
        design::modal_shell(
            &palette,
            430.0,
            max_height,
            self.localized("输入密码"),
            Some(if matches!(pending, PendingAction::Prompt { .. }) {
                self.localized("输入密码后继续当前解压任务。").to_string()
            } else if quick_test {
                self.localized("按密码本顺序全部失败，请手动输入。")
                    .to_string()
            } else {
                self.localized("该格式无法快速试密，请输入密码。")
                    .to_string()
            }),
            None,
            body,
            footer,
            cx.listener(|this, _, _, cx| this.close_modal(cx)),
        )
    }

    fn render_rename_modal(
        &mut self,
        index: usize,
        max_height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let old = self
            .archive
            .as_ref()
            .and_then(|archive| archive.info.find(index))
            .map(|entry| entry.path.clone())
            .unwrap_or_default();
        let body = vec![
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(design::field_label(&palette, self.localized("原路径")))
                .child(
                    div()
                        .px(px(12.0))
                        .py(px(9.0))
                        .rounded(px(crate::theme::RADIUS_CONTROL))
                        .bg(palette.panel)
                        .font_family("Cascadia Mono")
                        .text_size(crate::theme::caption())
                        .text_color(palette.sub)
                        .child(old),
                )
                .into_any_element(),
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(design::field_label(&palette, self.localized("新路径")))
                .child(
                    crate::components::control_height(Input::new(&self.rename_input))
                        .text_size(crate::theme::body())
                        .w_full(),
                )
                .into_any_element(),
        ];
        let footer = vec![
            design::secondary_action(
                "rename-cancel",
                self.localized("取消"),
                cx.listener(|this, _, _, cx| this.close_modal(cx)),
            )
            .into_any_element(),
            design::primary_icon_action(
                "rename-save",
                self.localized("保存"),
                IconName::Check,
                &palette,
                false,
                cx.listener(|this, _, _, cx| this.submit_rename(cx)),
            )
            .into_any_element(),
        ];
        design::modal_shell(
            &palette,
            460.0,
            max_height,
            self.localized("重命名条目"),
            Some(
                self.localized("路径使用 / 分隔；修改会重写 ZIP 归档。")
                    .to_string(),
            ),
            None,
            body,
            footer,
            cx.listener(|this, _, _, cx| this.close_modal(cx)),
        )
    }

    fn render_new_folder_modal(
        &mut self,
        parent: &str,
        max_height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let location = if parent.is_empty() {
            self.localized("根目录").to_string()
        } else {
            parent.to_string()
        };
        let body = vec![
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(design::field_label(&palette, self.localized("位置")))
                .child(
                    div()
                        .px(px(12.0))
                        .py(px(9.0))
                        .rounded(px(crate::theme::RADIUS_CONTROL))
                        .bg(palette.panel)
                        .font_family("Cascadia Mono")
                        .text_size(crate::theme::caption())
                        .text_color(palette.sub)
                        .child(location),
                )
                .into_any_element(),
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(design::field_label(&palette, self.localized("文件夹名称")))
                .child(
                    crate::components::control_height(Input::new(&self.rename_input))
                        .text_size(crate::theme::body())
                        .w_full(),
                )
                .into_any_element(),
        ];
        let footer = vec![
            design::secondary_action(
                "new-folder-cancel",
                self.localized("取消"),
                cx.listener(|this, _, _, cx| this.close_modal(cx)),
            )
            .into_any_element(),
            design::primary_icon_action(
                "new-folder-create",
                self.localized("创建"),
                IconName::FolderPlus,
                &palette,
                false,
                cx.listener(|this, _, _, cx| this.submit_new_folder(cx)),
            )
            .into_any_element(),
        ];
        design::modal_shell(
            &palette,
            460.0,
            max_height,
            self.localized("新建文件夹"),
            Some(
                self.localized("在当前目录创建一个空文件夹条目。")
                    .to_string(),
            ),
            None,
            body,
            footer,
            cx.listener(|this, _, _, cx| this.close_modal(cx)),
        )
    }

    fn render_comment_modal(&mut self, max_height: f32, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let body = vec![
            div()
                .font_family(crate::theme::ui_font_family())
                .text_size(crate::theme::caption())
                .text_color(palette.muted)
                .child(self.localized("ZIP 归档注释；留空表示清除注释"))
                .into_any_element(),
            crate::components::control_height(Input::new(&self.comment_input))
                .font_family(crate::theme::ui_font_family())
                .text_size(crate::theme::body())
                .w_full()
                .into_any_element(),
        ];
        let footer = vec![
            design::secondary_action(
                "comment-cancel",
                self.localized("取消"),
                cx.listener(|this, _, _, cx| this.close_modal(cx)),
            )
            .into_any_element(),
            design::primary_icon_action(
                "comment-save",
                self.localized("保存"),
                IconName::Check,
                &palette,
                false,
                cx.listener(|this, _, _, cx| this.submit_comment(cx)),
            )
            .into_any_element(),
        ];
        design::modal_shell(
            &palette,
            480.0,
            max_height,
            self.localized("编辑注释"),
            Some(self.localized("兼容 ZIP 标准注释字段。").to_string()),
            None,
            body,
            footer,
            cx.listener(|this, _, _, cx| this.close_modal(cx)),
        )
    }

    pub(in crate::app) fn render_password_manager_page(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let reveal = self.password_reveal;
        let count = self.password_book.len();

        let count_badge = div()
            .h(px(24.0))
            .px(px(11.0))
            .rounded(px(99.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .bg(palette.accent_soft)
            .text_size(crate::theme::caption())
            .font_weight(FontWeight::MEDIUM)
            .text_color(palette.accent)
            .child(icon(IconName::Lock, palette.accent, 13.0))
            .child(crate::i18n::tr_fmt(self.language, "{} 条密码", &[&count]));
        let meta_row = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .text_size(crate::theme::body())
                    .text_color(palette.sub)
                    .child(count_badge)
                    .child(self.localized("Windows DPAPI 加密保存")),
            )
            .child(design::switch(
                &palette,
                "pw-reveal",
                reveal,
                if reveal {
                    self.localized("隐藏密码")
                } else {
                    self.localized("显示密码")
                },
                cx.listener(|this, _, _, cx| {
                    this.password_reveal = !this.password_reveal;
                    cx.notify();
                }),
            ));

        let add_row = div()
            .flex()
            .items_end()
            .gap(px(12.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(design::field_label(&palette, self.localized("新密码")))
                    .child(
                        crate::components::control_height(Input::new(&self.password_new))
                            .text_size(crate::theme::body())
                            .w_full(),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(design::field_label(&palette, self.localized("备注")))
                    .child(
                        crate::components::control_height(Input::new(&self.password_note))
                            .text_size(crate::theme::body())
                            .w_full(),
                    ),
            )
            .child(
                Button::new("pw-add")
                    .label(self.localized("添加"))
                    .icon(icon(IconName::Plus, palette.accent_text, 15.0))
                    .primary()
                    .h(px(crate::components::CONTROL_HEIGHT))
                    .px(px(16.0))
                    .text_size(crate::theme::body())
                    .on_click(cx.listener(|this, _, window, cx| {
                        let password = this.password_new.read(cx).value().to_string();
                        if password.is_empty() {
                            this.show_toast(this.localized("密码不能为空"), ToastKind::Error, cx);
                            return;
                        }
                        if this
                            .password_book
                            .entries()
                            .iter()
                            .any(|entry| entry.password == password)
                        {
                            this.show_toast(this.localized("该密码已存在"), ToastKind::Info, cx);
                            return;
                        }
                        let note = this.password_note.read(cx).value().trim().to_string();
                        this.password_book.add(password, note);
                        if let Err(error) = this.password_book.save() {
                            this.show_toast(
                                crate::i18n::tr_fmt(this.language, "保存密码本失败：{}", &[&error]),
                                ToastKind::Error,
                                cx,
                            );
                        } else {
                            this.password_new.update(cx, |state, cx| {
                                state.set_value("", window, cx);
                            });
                            this.password_note.update(cx, |state, cx| {
                                state.set_value("", window, cx);
                            });
                            this.show_toast(this.localized("已添加密码"), ToastKind::Success, cx);
                        }
                        cx.notify();
                    })),
            );

        let header_cell = |text: &'static str, width: Option<f32>, flex: bool, align_end: bool| {
            div()
                .text_size(crate::theme::caption())
                .text_color(palette.muted)
                .font_weight(FontWeight::MEDIUM)
                .when(flex, |this| this.flex_1().min_w_0())
                .when_some(width, |this, width| this.w(px(width)).flex_shrink_0())
                .when(align_end, |this| this.text_right())
                .child(text)
        };
        let mut table = div().w_full().flex().flex_col();
        table = table.child(
            div()
                .flex()
                .items_center()
                .h(px(38.0))
                .px(px(16.0))
                .bg(palette.panel)
                .border_b_1()
                .border_color(palette.line)
                .child(header_cell("#", Some(42.0), false, false))
                .child(header_cell(self.localized("密码"), None, true, false))
                .child(header_cell(
                    self.localized("备注"),
                    Some(180.0),
                    false,
                    false,
                ))
                .child(header_cell(
                    self.localized("操作"),
                    Some(130.0),
                    false,
                    true,
                )),
        );

        if count == 0 {
            table = table.child(
                div()
                    .h(px(120.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(crate::theme::body())
                    .text_color(palette.muted)
                    .child(self.localized("密码本为空")),
            );
        } else {
            for index in 0..count {
                let entry = self.password_book.entries()[index].clone();
                let shown = if reveal {
                    entry.password.clone()
                } else {
                    "••••••••".to_string()
                };
                let note = entry.note.clone();
                let has_note = !note.is_empty();
                table = table.child(
                    div()
                        .id(format!("pw-row-{index}"))
                        .flex()
                        .items_center()
                        .h(px(42.0))
                        .px(px(16.0))
                        .when(index + 1 < count, |this| {
                            this.border_b_1().border_color(palette.line)
                        })
                        .hover(|style| style.bg(palette.accent_softer))
                        .child(
                            div()
                                .w(px(42.0))
                                .flex_shrink_0()
                                .text_size(crate::theme::caption())
                                .text_color(palette.muted)
                                .child(format!("{}", index + 1)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family("Cascadia Mono")
                                .text_size(crate::theme::body())
                                .text_color(palette.text)
                                .child(shown),
                        )
                        .child(
                            div()
                                .w(px(180.0))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .when(has_note, |this| {
                                    this.child(design::tag(&palette, note, design::TagTone::Manual))
                                }),
                        )
                        .child(
                            div()
                                .w(px(130.0))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_end()
                                .gap(px(2.0))
                                .child(design::icon_action(
                                    &palette,
                                    format!("pw-copy-{index}"),
                                    IconName::Copy,
                                    palette.sub,
                                    false,
                                    self.localized("复制密码"),
                                    cx.listener({
                                        let password = entry.password.clone();
                                        move |this, _, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                password.clone(),
                                            ));
                                            this.show_toast(
                                                this.localized("已复制密码"),
                                                ToastKind::Success,
                                                cx,
                                            );
                                        }
                                    }),
                                ))
                                .child(design::icon_action(
                                    &palette,
                                    format!("pw-up-{index}"),
                                    IconName::ArrowUp,
                                    palette.sub,
                                    index == 0,
                                    self.localized("上移"),
                                    cx.listener(move |this, _, _, cx| {
                                        this.password_book.move_up(index);
                                        let _ = this.password_book.save();
                                        cx.notify();
                                    }),
                                ))
                                .child(design::icon_action(
                                    &palette,
                                    format!("pw-down-{index}"),
                                    IconName::ArrowDown,
                                    palette.sub,
                                    index + 1 >= count,
                                    self.localized("下移"),
                                    cx.listener(move |this, _, _, cx| {
                                        this.password_book.move_down(index);
                                        let _ = this.password_book.save();
                                        cx.notify();
                                    }),
                                ))
                                .child(design::danger_icon_action(
                                    &palette,
                                    format!("pw-remove-{index}"),
                                    IconName::Trash,
                                    false,
                                    self.localized("删除"),
                                    cx.listener(move |this, _, _, cx| {
                                        this.modal = Some(Modal::ConfirmDeletePassword { index });
                                        cx.notify();
                                    }),
                                )),
                        ),
                );
            }
        }
        let table_card = design::card(&palette)
            .w_full()
            .overflow_hidden()
            .child(table);

        let note = design::info_note(
            &palette,
            self.localized("删除、上移或下移后立即保存；自动试密严格按上表顺序执行。"),
        );

        let password_text_filter = self.localized("文本密码本");
        let import = design::secondary_icon_action(
            "pw-import",
            self.localized("导入"),
            IconName::Upload,
            palette.text,
            cx.listener(move |this, _, _, cx| {
                let Some(path) = crate::components::file_dialog(move || {
                    rfd::FileDialog::new()
                        .add_filter(password_text_filter, &["txt", "csv"])
                        .pick_file()
                }) else {
                    return;
                };
                let Ok(text) = std::fs::read_to_string(&path) else {
                    this.show_toast(this.localized("读取密码文件失败"), ToastKind::Error, cx);
                    return;
                };
                let mut added = 0usize;
                for line in text.lines() {
                    let line = line.trim().trim_start_matches('\u{feff}');
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    let (password, note) = line
                        .split_once('\t')
                        .map(|(password, note)| (password.trim(), note.trim()))
                        .unwrap_or((line, this.localized("导入")));
                    if password.is_empty()
                        || this
                            .password_book
                            .entries()
                            .iter()
                            .any(|entry| entry.password == password)
                    {
                        continue;
                    }
                    this.password_book.add(password, note);
                    added += 1;
                }
                let _ = this.password_book.save();
                this.show_toast(
                    crate::i18n::tr_fmt(this.language, "已导入 {} 条密码", &[&added]),
                    ToastKind::Success,
                    cx,
                );
                cx.notify();
            }),
        );
        let export = design::secondary_icon_action(
            "pw-export",
            self.localized("导出"),
            IconName::Download,
            palette.text,
            cx.listener(move |this, _, _, cx| {
                let Some(path) = crate::components::file_dialog(move || {
                    rfd::FileDialog::new()
                        .add_filter(password_text_filter, &["txt"])
                        .set_file_name("rarkzip-passwords.txt")
                        .save_file()
                }) else {
                    return;
                };
                let text = this
                    .password_book
                    .entries()
                    .iter()
                    .map(|entry| {
                        if entry.note.is_empty() {
                            entry.password.clone()
                        } else {
                            format!("{}\t{}", entry.password, entry.note)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\r\n");
                match std::fs::write(&path, text) {
                    Ok(()) => this.show_toast(
                        crate::i18n::tr_fmt(this.language, "密码本已导出：{}", &[&path.display()]),
                        ToastKind::Success,
                        cx,
                    ),
                    Err(error) => this.show_toast(
                        crate::i18n::tr_fmt(this.language, "导出失败：{}", &[&error]),
                        ToastKind::Error,
                        cx,
                    ),
                }
            }),
        );

        design::page_shell(
            &palette,
            self.localized("密码管理器"),
            self.localized("自动试密将按下方顺序执行；手动输入成功的密码会自动追加到末尾"),
            design::done_button(
                &palette,
                self.language,
                cx.listener(|this, _, window, cx| this.close_page(window, cx)),
            ),
            vec![
                meta_row.into_any_element(),
                add_row.into_any_element(),
                table_card.into_any_element(),
            ],
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.0))
                    .child(note)
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(import)
                            .child(export),
                    )
                    .into_any_element(),
            ),
        )
    }

    fn render_delete_password_modal(
        &mut self,
        index: usize,
        max_height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let note = self
            .password_book
            .entries()
            .get(index)
            .map(|entry| entry.note.clone())
            .unwrap_or_default();
        let mut body = vec![div()
            .text_size(crate::theme::body())
            .text_color(palette.sub)
            .child(self.localized("确定从密码本删除这条密码吗？删除后自动试密将不再使用它。"))
            .into_any_element()];
        if !note.is_empty() {
            body.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(crate::theme::caption())
                    .text_color(palette.muted)
                    .child(self.localized("备注"))
                    .child(design::tag(&palette, note, design::TagTone::Manual))
                    .into_any_element(),
            );
        }
        let footer = vec![
            design::secondary_action(
                "pw-delete-cancel",
                self.localized("取消"),
                cx.listener(|this, _, _, cx| this.close_modal(cx)),
            )
            .into_any_element(),
            div()
                .id("pw-delete-confirm")
                .role(Role::Button)
                .aria_label(self.localized("删除"))
                .h(px(crate::components::CONTROL_HEIGHT))
                .px(px(16.0))
                .rounded(px(crate::theme::RADIUS_CONTROL))
                .flex()
                .items_center()
                .justify_center()
                .bg(palette.bad)
                .text_color(palette.white)
                .text_size(crate::theme::body())
                .cursor_pointer()
                .hover(|style| style.bg(palette.bad))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.password_book.remove(index);
                    if let Err(error) = this.password_book.save() {
                        this.show_toast(
                            crate::i18n::tr_fmt(this.language, "保存密码本失败：{}", &[&error]),
                            ToastKind::Error,
                            cx,
                        );
                    } else {
                        this.show_toast(this.localized("已删除密码"), ToastKind::Success, cx);
                    }
                    this.modal = None;
                    cx.notify();
                }))
                .child(self.localized("删除"))
                .into_any_element(),
        ];
        design::modal_shell(
            &palette,
            440.0,
            max_height,
            self.localized("删除密码"),
            Some(self.localized("此操作无法撤销。").to_string()),
            None,
            body,
            footer,
            cx.listener(|this, _, _, cx| this.close_modal(cx)),
        )
    }

    pub(in crate::app) fn render_password_recovery_page(
        &mut self,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let archive_filter = self.localized("压缩包");
        let running = self.recovery_running;
        let paused = self.recovery_paused;
        let mode = self.recovery_mode;
        let step = if running {
            4
        } else if self.recovery_result.is_some() || self.recovery_error.is_some() {
            5
        } else if self.recovery_path.is_some() {
            2
        } else {
            1
        };

        let mut stepper = div().flex().items_center();
        for (index, label) in [
            self.localized("选择目标"),
            self.localized("模式"),
            self.localized("参数"),
            self.localized("恢复"),
            self.localized("完成"),
        ]
        .into_iter()
        .enumerate()
        {
            let number = index + 1;
            let state = if number < step {
                design::StepState::Done
            } else if number == step {
                design::StepState::Active
            } else {
                design::StepState::Pending
            };
            stepper = stepper.child(design::step_dot(&palette, number, label, state));
            if number < 5 {
                stepper = stepper.child(design::step_line(&palette));
            }
        }

        let path_display = self
            .recovery_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| self.localized("尚未选择压缩包").to_string());
        let target_panel = design::panel(&palette)
            .flex()
            .flex_col()
            .child(design::panel_title(
                &palette,
                IconName::Target,
                self.localized("恢复目标"),
            ))
            .child(
                div()
                    .mt(px(14.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .h(px(36.0))
                            .px(px(13.0))
                            .flex()
                            .items_center()
                            .rounded(px(crate::theme::RADIUS_CONTROL))
                            .border_1()
                            .border_color(palette.line_strong)
                            .bg(palette.panel)
                            .font_family("Cascadia Mono")
                            .text_size(crate::theme::caption())
                            .text_color(if self.recovery_path.is_some() {
                                palette.sub
                            } else {
                                palette.muted
                            })
                            .child(path_display),
                    )
                    .child(design::secondary_icon_action(
                        "recovery-pick",
                        self.localized("选择压缩包…"),
                        IconName::FolderOpen,
                        palette.text,
                        cx.listener(move |this, _, window, cx| {
                            let Some(path) = crate::components::file_dialog_in(window, move || {
                                rfd::FileDialog::new()
                                    .add_filter(
                                        archive_filter,
                                        &["zip", "7z", "rar", "tar", "gz", "tgz", "xz", "zst"],
                                    )
                                    .pick_file()
                            }) else {
                                return;
                            };
                            this.recovery_path = Some(path);
                            this.recovery_result = None;
                            this.recovery_error = None;
                            this.recovery_attempts = 0;
                            this.recovery_total = 0;
                            cx.notify();
                        }),
                    )),
            );

        let thread_stepper = design::stepper(
            &palette,
            "recovery-threads",
            self.recovery_threads.to_string(),
            cx.listener(|this, _, _, cx| {
                this.recovery_threads = this.recovery_threads.saturating_sub(1).max(1);
                cx.notify();
            }),
            cx.listener(|this, _, _, cx| {
                this.recovery_threads = (this.recovery_threads + 1).min(32);
                cx.notify();
            }),
        );

        let mode_card = |icon_name: IconName,
                         title: &'static str,
                         description: &'static str,
                         selected: bool,
                         value: RecoveryUiMode| {
            let entity = cx.entity();
            let mut card = div()
                .id(format!("recovery-mode-{title}"))
                .role(Role::Button)
                .aria_label(title)
                .relative()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .px(px(16.0))
                .py(px(14.0))
                .rounded(px(crate::theme::RADIUS_OVERLAY))
                .border(px(1.5))
                .border_color(if selected {
                    palette.accent
                } else {
                    palette.line
                })
                .bg(if selected {
                    palette.accent_softer
                } else {
                    palette.bg
                })
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.recovery_mode = value;
                    this.recovery_result = None;
                    this.recovery_error = None;
                    cx.notify();
                }))
                .on_a11y_action(AccessibleAction::Click, move |_, _, app| {
                    entity.update(app, |this, cx| {
                        this.recovery_mode = value;
                        this.recovery_result = None;
                        this.recovery_error = None;
                        cx.notify();
                    });
                })
                .when(!selected, |this| {
                    this.hover(|style| style.border_color(palette.accent))
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .text_size(crate::theme::body())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.text)
                        .child(icon(icon_name, palette.accent, 16.0))
                        .child(title),
                )
                .child(
                    div()
                        .text_size(crate::theme::caption())
                        .line_height(relative(1.5))
                        .text_color(palette.muted)
                        .child(description),
                );
            if selected {
                card = card.child(
                    div()
                        .absolute()
                        .top(px(12.0))
                        .right(px(12.0))
                        .w(px(16.0))
                        .h(px(16.0))
                        .rounded(px(99.0))
                        .bg(palette.accent)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(IconName::Check, palette.white, 10.0)),
                );
            }
            card.into_any_element()
        };

        let mode_cards = div()
            .flex()
            .items_stretch()
            .gap(px(12.0))
            .child(mode_card(
                IconName::ClipboardList,
                self.localized("字典"),
                self.localized("从字典文件逐行读取候选密码，速度快、命中率高"),
                mode == RecoveryUiMode::Dictionary,
                RecoveryUiMode::Dictionary,
            ))
            .child(mode_card(
                IconName::SquareFunction,
                self.localized("掩码"),
                self.localized("按已知字符结构生成组合，如已知前缀或长度"),
                mode == RecoveryUiMode::Mask,
                RecoveryUiMode::Mask,
            ))
            .child(mode_card(
                IconName::Zap,
                self.localized("暴力"),
                self.localized("遍历指定字符集的全部排列组合，耗时最长"),
                mode == RecoveryUiMode::Brute,
                RecoveryUiMode::Brute,
            ));

        let params: AnyElement = match mode {
            RecoveryUiMode::Dictionary => {
                let count = self.recovery_dictionary.len();
                let file_name = self
                    .recovery_dictionary_path
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().to_string());
                let status = match (&file_name, count) {
                    (Some(name), count) => div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .min_w_0()
                        .font_family("Cascadia Mono")
                        .text_size(crate::theme::caption())
                        .text_color(palette.text)
                        .child(icon(IconName::FileText, palette.accent, 16.0))
                        .child(div().truncate().child(name.clone()))
                        .child(
                            div()
                                .flex_none()
                                .text_size(crate::theme::caption())
                                .text_color(palette.muted)
                                .child(crate::i18n::tr_fmt(self.language, "· {} 条", &[&count])),
                        )
                        .into_any_element(),
                    _ => div()
                        .text_size(crate::theme::body())
                        .text_color(palette.muted)
                        .child(self.localized("尚未导入字典文件"))
                        .into_any_element(),
                };
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.0))
                    .px(px(16.0))
                    .py(px(14.0))
                    .rounded(px(crate::theme::RADIUS_OVERLAY))
                    .border(px(1.5))
                    .border_dashed()
                    .border_color(palette.line_strong)
                    .child(status)
                    .child({
                        let dictionary_filter = self.localized("字典/密码本");
                        design::secondary_icon_action(
                            "recovery-dict",
                            self.localized("选择字典…"),
                            IconName::FileText,
                            palette.text,
                            cx.listener(move |this, _, window, cx| {
                                let Some(path) =
                                    crate::components::file_dialog_in(window, move || {
                                        rfd::FileDialog::new()
                                            .add_filter(
                                                dictionary_filter,
                                                &["txt", "lst", "dict", "csv"],
                                            )
                                            .pick_file()
                                    })
                                else {
                                    return;
                                };
                                match std::fs::read_to_string(&path) {
                                    Ok(text) => {
                                        let words: Vec<String> = text
                                            .lines()
                                            .map(|line| line.trim().to_string())
                                            .filter(|line| !line.is_empty())
                                            .collect();
                                        let count = words.len();
                                        this.recovery_dictionary = words;
                                        this.recovery_dictionary_path = Some(path);
                                        this.show_toast(
                                            crate::i18n::tr_fmt(
                                                this.language,
                                                "已加载 {} 条候选密码",
                                                &[&count],
                                            ),
                                            ToastKind::Success,
                                            cx,
                                        );
                                    }
                                    Err(error) => this.show_toast(
                                        crate::i18n::tr_fmt(
                                            this.language,
                                            "读取字典失败：{}",
                                            &[&error],
                                        ),
                                        ToastKind::Error,
                                        cx,
                                    ),
                                }
                                cx.notify();
                            }),
                        )
                        .into_any_element()
                    })
                    .into_any_element()
            }
            RecoveryUiMode::Mask => div()
                .flex()
                .flex_col()
                .gap(px(5.0))
                .child(
                    div()
                        .text_size(crate::theme::caption())
                        .text_color(palette.muted)
                        .child(self.localized(
                            "掩码：?l 小写  ?u 大写  ?d 数字  ?s 符号  ?a 全部  ?? 问号",
                        )),
                )
                .child(
                    crate::components::control_height(Input::new(&self.recovery_mask))
                        .text_size(crate::theme::body())
                        .w_full(),
                )
                .into_any_element(),
            RecoveryUiMode::Brute => {
                let min_stepper = design::stepper(
                    &palette,
                    "recovery-min",
                    self.recovery_min.to_string(),
                    cx.listener(|this, _, _, cx| {
                        this.recovery_min = this.recovery_min.saturating_sub(1).max(1);
                        this.recovery_max = this.recovery_max.max(this.recovery_min);
                        cx.notify();
                    }),
                    cx.listener(|this, _, _, cx| {
                        this.recovery_min = (this.recovery_min + 1).min(this.recovery_max);
                        cx.notify();
                    }),
                );
                let max_stepper = design::stepper(
                    &palette,
                    "recovery-max",
                    self.recovery_max.to_string(),
                    cx.listener(|this, _, _, cx| {
                        this.recovery_max =
                            this.recovery_max.saturating_sub(1).max(this.recovery_min);
                        cx.notify();
                    }),
                    cx.listener(|this, _, _, cx| {
                        this.recovery_max = (this.recovery_max + 1).min(12);
                        cx.notify();
                    }),
                );
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(
                        crate::components::control_height(Input::new(&self.recovery_charset))
                            .text_size(crate::theme::body())
                            .w_full(),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(20.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .text_size(crate::theme::caption())
                                    .text_color(palette.sub)
                                    .child(self.localized("最短长度"))
                                    .child(min_stepper),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .text_size(crate::theme::caption())
                                    .text_color(palette.sub)
                                    .child(self.localized("最长长度"))
                                    .child(max_stepper),
                            ),
                    )
                    .into_any_element()
            }
        };

        let mode_panel = design::panel(&palette)
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.0))
                    .child(design::panel_title(
                        &palette,
                        IconName::Key,
                        self.localized("恢复模式"),
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .text_size(crate::theme::body())
                            .text_color(palette.sub)
                            .child(self.localized("线程"))
                            .child(thread_stepper),
                    ),
            )
            .child(mode_cards)
            .child(params);

        let percent = if self.recovery_total > 0 {
            (self.recovery_attempts as f64 / self.recovery_total as f64 * 100.0) as f32
        } else {
            0.0
        };
        let status: Option<AnyElement> = if running {
            let pause = design::secondary_icon_action(
                "recovery-pause",
                if paused {
                    self.localized("继续")
                } else {
                    self.localized("暂停")
                },
                if paused {
                    IconName::Play
                } else {
                    IconName::Pause
                },
                palette.text,
                cx.listener(|this, _, _, cx| {
                    this.recovery_paused = !this.recovery_paused;
                    this.recovery_control.set_paused(this.recovery_paused);
                    cx.notify();
                }),
            );
            let cancel = design::secondary_icon_action(
                "recovery-cancel",
                self.localized("取消"),
                IconName::X,
                palette.bad,
                cx.listener(|this, _, _, cx| {
                    this.recovery_control.cancel();
                    this.show_toast(this.localized("正在停止恢复…"), ToastKind::Info, cx);
                }),
            );
            Some(
                design::panel(&palette)
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
                                    .text_size(crate::theme::body())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(if paused { palette.warn } else { palette.accent })
                                    .child(if paused {
                                        self.localized("已暂停")
                                    } else {
                                        self.localized("正在恢复密码…")
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(crate::theme::caption())
                                    .text_color(palette.muted)
                                    .child(crate::i18n::tr_fmt(
                                        self.language,
                                        "已尝试 {} / {}",
                                        &[&self.recovery_attempts, &self.recovery_total],
                                    )),
                            ),
                    )
                    .child(progress_bar(percent, &palette, palette.accent))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(pause)
                            .child(cancel),
                    )
                    .into_any_element(),
            )
        } else if let Some(password) = &self.recovery_result {
            let copy = password.clone();
            Some(
                design::panel(&palette)
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(crate::theme::caption())
                            .text_color(palette.ok)
                            .child(self.localized("已找到密码，并已追加到密码本")),
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
                                    .truncate()
                                    .px(px(12.0))
                                    .py(px(9.0))
                                    .rounded(px(crate::theme::RADIUS_CONTROL))
                                    .bg(palette.ok_soft)
                                    .font_family("Cascadia Mono")
                                    .text_size(crate::theme::body())
                                    .text_color(palette.ok)
                                    .child(password.clone()),
                            )
                            .child(design::secondary_icon_action(
                                "recovery-copy",
                                self.localized("复制"),
                                IconName::Copy,
                                palette.text,
                                cx.listener(move |_, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                                }),
                            )),
                    )
                    .into_any_element(),
            )
        } else {
            self.recovery_error.as_ref().map(|error| {
                div()
                    .text_size(crate::theme::caption())
                    .text_color(palette.bad)
                    .child(error.clone())
                    .into_any_element()
            })
        };

        let can_start = !running && self.recovery_path.is_some();
        let start_button = {
            let mut button = div()
                .id("recovery-start")
                .role(Role::Button)
                .aria_label(self.localized("开始恢复"))
                .w_full()
                .h(px(44.0))
                .rounded(px(crate::theme::RADIUS_OVERLAY))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .text_size(crate::theme::body())
                .font_weight(FontWeight::MEDIUM)
                .bg(if can_start {
                    palette.accent
                } else {
                    palette.line
                })
                .text_color(if can_start {
                    palette.accent_text
                } else {
                    palette.muted
                })
                .child(icon(
                    IconName::Play,
                    if can_start {
                        palette.accent_text
                    } else {
                        palette.muted
                    },
                    17.0,
                ))
                .child(if running {
                    self.localized("恢复中…")
                } else {
                    self.localized("开始恢复")
                });
            if can_start {
                button = button
                    .cursor_pointer()
                    .hover(|style| style.bg(palette.accent_hover))
                    .on_click(cx.listener(|this, _, _, cx| this.start_password_recovery(cx)));
            }
            button.into_any_element()
        };

        let note = design::info_note(
            &palette,
            self.localized("恢复只做快速试密，不会整包解压；找到的密码会写入密码本。"),
        );

        let mut body = vec![
            div()
                .flex()
                .items_center()
                .child(stepper)
                .into_any_element(),
            target_panel.into_any_element(),
            mode_panel.into_any_element(),
        ];
        if let Some(status) = status {
            body.push(status);
        }

        design::page_shell_bare(
            &palette,
            body,
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(note)
                    .child(start_button)
                    .into_any_element(),
            ),
        )
    }

    fn render_toolbar_modal(&mut self, max_height: f32, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let hidden = self.settings.general.hidden_toolbar_buttons.clone();
        let entries: [(&str, &str, IconName); 9] = [
            ("new", self.localized("新建压缩包"), IconName::PackagePlus),
            ("open", self.localized("打开压缩包"), IconName::FolderOpen),
            ("extract", self.localized("解压"), IconName::Download),
            ("add", self.localized("添加文件"), IconName::FilePlus),
            ("test", self.localized("测试完整性"), IconName::TestTube),
            ("scan", self.localized("AMSI 扫描"), IconName::ShieldCheck),
            ("hash", self.localized("计算哈希"), IconName::Hash),
            ("delete", self.localized("删除条目"), IconName::Trash),
            ("encoding", self.localized("编码"), IconName::Globe),
        ];
        let total = entries.len() + 1;
        let mut rows = design::row_list(&palette)
            .id("toolbar-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        for (index, (key, label, icon_name)) in entries.iter().enumerate() {
            let key_owned = key.to_string();
            let visible = !hidden.iter().any(|hidden| hidden == key);
            let toggle_key = key_owned.clone();
            rows = rows.child(
                design::row_list_row(&palette, index + 1 == total, false)
                    .child(design::check_box(
                        &palette,
                        format!("toolbar-{key}"),
                        visible,
                        false,
                        *label,
                        cx.listener(move |this, _, _, cx| {
                            if this
                                .settings
                                .general
                                .hidden_toolbar_buttons
                                .iter()
                                .any(|hidden| hidden == &toggle_key)
                            {
                                this.settings
                                    .general
                                    .hidden_toolbar_buttons
                                    .retain(|hidden| hidden != &toggle_key);
                            } else {
                                this.settings
                                    .general
                                    .hidden_toolbar_buttons
                                    .push(toggle_key.clone());
                            }
                            this.save_settings();
                            cx.notify();
                        }),
                    ))
                    .child(icon(*icon_name, palette.muted, 16.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(crate::theme::body())
                            .text_color(palette.text)
                            .child(*label),
                    ),
            );
        }
        rows = rows.child(
            design::row_list_row(&palette, true, true)
                .child(design::check_box(
                    &palette,
                    "toolbar-settings",
                    true,
                    true,
                    self.localized("设置"),
                    |_, _, _| {},
                ))
                .child(icon(IconName::Settings, palette.muted, 16.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .flex_1()
                        .text_size(crate::theme::body())
                        .text_color(palette.muted)
                        .child(self.localized("设置"))
                        .child(design::fixed_tag(&palette, self.localized("固定显示"))),
                ),
        );

        let body = vec![rows.into_any_element()];
        let footer = vec![
            design::secondary_icon_action(
                "toolbar-reset",
                self.localized("全部显示"),
                IconName::RotateCcw,
                palette.text,
                cx.listener(|this, _, _, cx| {
                    this.settings.general.hidden_toolbar_buttons.clear();
                    this.save_settings();
                    cx.notify();
                }),
            )
            .into_any_element(),
            design::primary_icon_action(
                "toolbar-done",
                self.localized("完成"),
                IconName::Check,
                &palette,
                false,
                cx.listener(|this, _, _, cx| this.close_modal(cx)),
            )
            .into_any_element(),
        ];
        design::modal_shell_fill(
            &palette,
            560.0,
            max_height,
            self.localized("工具栏按钮"),
            Some(
                self.localized("隐藏不常用的按钮，保留核心工作流。")
                    .to_string(),
            ),
            Some(
                self.localized("选择要显示的工具栏按钮；菜单与窗口控制固定保留。")
                    .to_string(),
            ),
            body,
            footer,
            cx.listener(|this, _, _, cx| this.close_modal(cx)),
        )
    }

    fn render_columns_modal(&mut self, max_height: f32, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let mut order = self.settings.view.column_order.clone();
        for column in [
            SortColumn::Name,
            SortColumn::Original,
            SortColumn::Compressed,
            SortColumn::Type,
            SortColumn::Modified,
            SortColumn::Method,
            SortColumn::Encryption,
            SortColumn::Crc,
            SortColumn::Attributes,
            SortColumn::Comment,
            SortColumn::Extra,
        ] {
            if !order.iter().any(|key| key == column.key()) {
                order.push(column.key().to_string());
            }
        }
        let hidden = self.settings.view.hidden_columns.clone();
        let widths = self.settings.view.column_widths.clone();
        let count = order.len();
        let mut rows = div()
            .id("columns-list")
            .w_full()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .rounded(px(crate::theme::RADIUS_OVERLAY))
            .border_1()
            .border_color(palette.line);
        for (index, key) in order.iter().enumerate() {
            let Some(column) = SortColumn::from_key(key) else {
                continue;
            };
            let is_name = column == SortColumn::Name;
            let visible = is_name || !hidden.iter().any(|hidden| hidden == key);
            let width = widths
                .get(key)
                .copied()
                .unwrap_or_else(|| crate::app::default_column_width(key));
            let toggle_key = key.clone();
            let up_key = key.clone();
            let down_order_key = key.clone();
            let wide_key = key.clone();
            let narrow_key = key.clone();
            let is_last = index + 1 == count;

            let width_control = (!is_name).then(|| {
                let step = |id: String, label: &'static str, onclick: ClickCallback| {
                    div()
                        .id(id)
                        .role(Role::Button)
                        .aria_label(label)
                        .w(px(26.0))
                        .h(px(26.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(crate::theme::RADIUS_CONTROL))
                        .cursor_pointer()
                        .text_size(crate::theme::body())
                        .text_color(palette.sub)
                        .hover(|style| style.bg(palette.accent_soft).text_color(palette.accent))
                        .on_click(move |event, window, cx| onclick(event, window, cx))
                        .child(label)
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .child(step(
                        format!("col-width-down-{key}"),
                        "−",
                        Box::new(cx.listener(move |this, _, _, cx| {
                            let current = this
                                .settings
                                .view
                                .column_widths
                                .get(&narrow_key)
                                .copied()
                                .unwrap_or_else(|| crate::app::default_column_width(&narrow_key));
                            this.settings
                                .view
                                .column_widths
                                .insert(narrow_key.clone(), (current - 8.0).max(40.0));
                            this.save_settings();
                            cx.notify();
                        })),
                    ))
                    .child(
                        div()
                            .w(px(54.0))
                            .flex_shrink_0()
                            .text_center()
                            .font_family("Cascadia Mono")
                            .text_size(crate::theme::caption())
                            .text_color(palette.sub)
                            .child(format!("{width:.0}px")),
                    )
                    .child(step(
                        format!("col-width-up-{key}"),
                        "＋",
                        Box::new(cx.listener(move |this, _, _, cx| {
                            let current = this
                                .settings
                                .view
                                .column_widths
                                .get(&wide_key)
                                .copied()
                                .unwrap_or_else(|| crate::app::default_column_width(&wide_key));
                            this.settings
                                .view
                                .column_widths
                                .insert(wide_key.clone(), (current + 8.0).min(240.0));
                            this.save_settings();
                            cx.notify();
                        })),
                    ))
            });

            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .min_h(px(44.0))
                    .px(px(16.0))
                    .py(px(10.0))
                    .when(is_name, |this| this.bg(palette.panel))
                    .when(!is_last, |this| {
                        this.border_b_1().border_color(palette.line)
                    })
                    .when(!is_name, |this| {
                        this.hover(|style| style.bg(palette.accent_softer))
                    })
                    .child(design::check_box(
                        &palette,
                        format!("col-visible-{key}"),
                        visible,
                        is_name,
                        crate::i18n::tr(self.language, column.label()),
                        cx.listener(move |this, _, _, cx| {
                            if this
                                .settings
                                .view
                                .hidden_columns
                                .iter()
                                .any(|hidden| hidden == &toggle_key)
                            {
                                this.settings
                                    .view
                                    .hidden_columns
                                    .retain(|hidden| hidden != &toggle_key);
                            } else {
                                this.settings.view.hidden_columns.push(toggle_key.clone());
                            }
                            this.save_settings();
                            cx.notify();
                        }),
                    ))
                    .when(is_name, |this| {
                        this.child(icon(IconName::FileText, palette.muted, 16.0))
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .flex_1()
                            .min_w_0()
                            .text_size(crate::theme::body())
                            .text_color(palette.text)
                            .child(crate::i18n::tr(self.language, column.label()))
                            .when(is_name, |this| {
                                this.child(design::fixed_tag(&palette, self.localized("固定显示")))
                            }),
                    )
                    .when_some(width_control, |this, control| this.child(control))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.0))
                            .child(design::icon_action(
                                &palette,
                                format!("col-up-{key}"),
                                IconName::ArrowUp,
                                palette.sub,
                                index == 0,
                                self.localized("上移"),
                                cx.listener(move |this, _, _, cx| {
                                    if let Some(position) = this
                                        .settings
                                        .view
                                        .column_order
                                        .iter()
                                        .position(|key| key == &up_key)
                                    {
                                        if position > 0 {
                                            this.settings
                                                .view
                                                .column_order
                                                .swap(position, position - 1);
                                            this.save_settings();
                                            cx.notify();
                                        }
                                    }
                                }),
                            ))
                            .child(design::icon_action(
                                &palette,
                                format!("col-down-{key}"),
                                IconName::ArrowDown,
                                palette.sub,
                                index + 1 >= count,
                                self.localized("下移"),
                                cx.listener(move |this, _, _, cx| {
                                    if let Some(position) = this
                                        .settings
                                        .view
                                        .column_order
                                        .iter()
                                        .position(|key| key == &down_order_key)
                                    {
                                        if position + 1 < this.settings.view.column_order.len() {
                                            this.settings
                                                .view
                                                .column_order
                                                .swap(position, position + 1);
                                            this.save_settings();
                                            cx.notify();
                                        }
                                    }
                                }),
                            )),
                    ),
            );
        }

        let body = vec![rows.into_any_element()];
        let footer = vec![
            design::secondary_icon_action(
                "columns-reset",
                self.localized("恢复默认"),
                IconName::RotateCcw,
                palette.text,
                cx.listener(|this, _, _, cx| {
                    let defaults = rarkzip_engine::ViewSettings::default();
                    this.settings.view.column_order = defaults.column_order;
                    this.settings.view.hidden_columns = defaults.hidden_columns;
                    this.settings.view.column_widths = defaults.column_widths;
                    this.save_settings();
                    cx.notify();
                }),
            )
            .into_any_element(),
            design::primary_icon_action(
                "columns-done",
                self.localized("完成"),
                IconName::Check,
                &palette,
                false,
                cx.listener(|this, _, _, cx| this.close_modal(cx)),
            )
            .into_any_element(),
        ];
        design::modal_shell_fill(
            &palette,
            660.0,
            max_height,
            self.localized("列设置"),
            Some(
                self.localized("选择显示列、调整列宽与顺序；设置会自动保存。")
                    .to_string(),
            ),
            Some(
                self.localized("拖拽表头可调整列顺序；此处也可用 ↑/↓、显隐与宽度。")
                    .to_string(),
            ),
            body,
            footer,
            cx.listener(|this, _, _, cx| this.close_modal(cx)),
        )
    }

    pub(in crate::app) fn render_hash_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let language = self.language;
        let running = self.hash_batch_running;
        let count = self.hash_batch_paths.len();
        let algorithms = self.hash_batch_algorithms.clone();
        let expected = self.hash_batch_expected.clone();
        let expected_present = !expected.is_empty();
        let has_results = !self.hash_batch_results.is_empty();
        let (done, total) = self.hash_batch_progress;
        let uppercase_display = self.settings.hash.uppercase_display;
        let monospace = self.settings.hash.monospace;

        let mut matched = 0usize;
        let mut insecure = 0usize;
        let mut mismatched = 0usize;
        let mut unchecked = 0usize;
        let mut errors = 0usize;
        if expected_present {
            for item in &self.hash_batch_results {
                match hash_row_status(item, &expected) {
                    HashRowStatus::Match => matched += 1,
                    HashRowStatus::Insecure => insecure += 1,
                    HashRowStatus::Mismatch => mismatched += 1,
                    HashRowStatus::Unchecked => unchecked += 1,
                    HashRowStatus::Error => errors += 1,
                    HashRowStatus::Pending | HashRowStatus::Done => {}
                }
            }
        }

        let (state_icon, state_color, state_text) = if running {
            (
                IconName::Hash,
                palette.accent,
                format!("{} {done} / {total}", tr(language, "正在计算…")),
            )
        } else if count == 0 {
            (
                IconName::Hash,
                palette.muted,
                tr(language, "尚未添加文件").to_string(),
            )
        } else if has_results {
            (
                IconName::CircleCheck,
                palette.ok,
                format!(
                    "{} · {count} {}",
                    tr(language, "计算完成"),
                    tr(language, "个文件")
                ),
            )
        } else {
            (
                IconName::Hash,
                palette.muted,
                tr(language, "等待开始").to_string(),
            )
        };
        let recalc = Button::new("hash-recalc")
            .label(if has_results {
                tr(language, "重新计算")
            } else {
                tr(language, "开始计算")
            })
            .icon(icon(IconName::RotateCcw, palette.text, 14.0))
            .outline()
            .h(px(32.0))
            .px(px(12.0))
            .text_size(crate::theme::caption())
            .disabled(running || count == 0)
            .opacity(if running || count == 0 { 0.45 } else { 1.0 })
            .on_click(cx.listener(|this, _, _, cx| this.start_hash_batch(cx)));

        let status_row = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(icon(state_icon, state_color, 16.0))
            .child(
                div()
                    .text_size(crate::theme::body())
                    .text_color(if running {
                        palette.accent
                    } else {
                        palette.text
                    })
                    .child(state_text),
            )
            .when(running, |this| {
                let percent = if total == 0 {
                    0.0
                } else {
                    done as f32 / total as f32 * 100.0
                };
                this.child(div().w(px(150.0)).child(progress_bar(
                    percent,
                    &palette,
                    palette.accent,
                )))
            })
            .when(expected_present && has_results, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(design::pill(
                            &palette,
                            format!("{} {matched}", tr(language, "匹配")),
                            design::TagTone::Ok,
                        ))
                        .when(insecure > 0, |this| {
                            this.child(design::pill(
                                &palette,
                                format!("{} {insecure}", tr(language, "不安全")),
                                design::TagTone::Manual,
                            ))
                        })
                        .when(mismatched > 0, |this| {
                            this.child(design::pill(
                                &palette,
                                format!("{} {mismatched}", tr(language, "不匹配")),
                                design::TagTone::Danger,
                            ))
                        })
                        .when(unchecked > 0, |this| {
                            this.child(design::pill(
                                &palette,
                                format!("{} {unchecked}", tr(language, "未校验")),
                                design::TagTone::Neutral,
                            ))
                        })
                        .when(errors > 0, |this| {
                            this.child(design::pill(
                                &palette,
                                format!("{} {errors}", tr(language, "错误")),
                                design::TagTone::Danger,
                            ))
                        }),
                )
            })
            .child(div().flex_1())
            .child(recalc);

        let verify_hint = if expected.contains_key("*") {
            tr(language, "已识别粘贴哈希").to_string()
        } else if expected_present {
            format!("{} ({})", tr(language, "已载入期望"), expected.len())
        } else {
            tr(language, "未设置").to_string()
        };
        let verify_row = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .flex_none()
                    .text_size(crate::theme::caption())
                    .text_color(palette.sub)
                    .child(icon(IconName::Target, palette.sub, 14.0))
                    .child(tr(language, "校验")),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(crate::components::control_height(Input::new(
                        &self.hash_verify_input,
                    ))),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(crate::theme::caption())
                    .text_color(if expected_present {
                        palette.accent
                    } else {
                        palette.muted
                    })
                    .child(verify_hint),
            );

        let mut column_row = div().flex().items_center().gap(px(6.0)).child(
            div()
                .flex_none()
                .text_size(crate::theme::caption())
                .text_color(palette.sub)
                .child(tr(language, "列")),
        );
        for algorithm in HashAlgorithm::ALL {
            let selected = algorithms.contains(&algorithm);
            column_row = column_row.child(
                div()
                    .id(format!("hash-alg-{}", algorithm.name()))
                    .role(Role::Button)
                    .aria_label(algorithm.label())
                    .h(px(26.0))
                    .px(px(8.0))
                    .rounded(px(5.0))
                    .border_1()
                    .border_color(if selected {
                        palette.accent_soft
                    } else {
                        palette.line
                    })
                    .bg(if selected {
                        palette.accent_soft
                    } else {
                        palette.bg
                    })
                    .text_size(crate::theme::caption())
                    .text_color(if selected {
                        palette.accent
                    } else {
                        palette.sub
                    })
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .hover(|style| style.border_color(palette.line_strong))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_hash_algorithm(algorithm, cx);
                    }))
                    .child(algorithm.label()),
            );
        }

        let add_file = || {
            design::secondary_icon_action(
                "hashbatch-files",
                tr(language, "添加文件…"),
                IconName::Plus,
                palette.text,
                cx.listener(|this, _, window, cx| this.add_hash_files(window, cx)),
            )
        };
        let add_folder = || {
            design::secondary_icon_action(
                "hashbatch-folder",
                tr(language, "添加文件夹…"),
                IconName::FolderOpen,
                palette.text,
                cx.listener(|this, _, window, cx| this.add_hash_folder(window, cx)),
            )
        };
        let checksum_filter = tr(language, "校验文件");
        let load_checksum = || {
            design::secondary_icon_action(
                "hashbatch-verify-file",
                tr(language, "载入校验文件…"),
                IconName::Upload,
                palette.text,
                cx.listener(move |this, _, window, cx| {
                    let Some(path) = crate::components::file_dialog_in(window, move || {
                        rfd::FileDialog::new()
                            .add_filter(
                                checksum_filter,
                                &["sha256", "sha1", "md5", "sha512", "txt", "sfv"],
                            )
                            .pick_file()
                    }) else {
                        return;
                    };
                    match rarkzip_engine::load_checksum_file(&path) {
                        Ok(entries) => {
                            let count = entries.len();
                            if let Some(parent) = path.parent() {
                                for entry in &entries {
                                    let candidate = parent.join(
                                        entry
                                            .file_name
                                            .replace(['/', '\\'], std::path::MAIN_SEPARATOR_STR),
                                    );
                                    if candidate.is_file()
                                        && !this.hash_batch_paths.contains(&candidate)
                                    {
                                        this.hash_batch_paths.push(candidate);
                                    }
                                }
                            }
                            this.hash_batch_expected =
                                rarkzip_engine::checksum_expectations(&entries);
                            this.hash_verify_input.update(cx, |state, cx| {
                                state.set_value("", window, cx);
                            });
                            this.hash_batch_results.clear();
                            this.hash_selected_row = None;
                            this.hash_context_menu = None;
                            this.show_toast(
                                format!("{} ({count})", tr(language, "已载入期望哈希")),
                                ToastKind::Success,
                                cx,
                            );
                        }
                        Err(error) => this.show_toast(
                            format!("{}: {error}", tr(language, "读取校验文件失败")),
                            ToastKind::Error,
                            cx,
                        ),
                    }
                    cx.notify();
                }),
            )
        };

        let mut body: Vec<AnyElement> = vec![
            status_row.into_any_element(),
            verify_row.into_any_element(),
            column_row.into_any_element(),
        ];
        if let Some(error) = self.hash_batch_error.clone() {
            body.push(
                div()
                    .text_size(crate::theme::caption())
                    .text_color(palette.bad)
                    .child(error)
                    .into_any_element(),
            );
        }

        if count == 0 {
            body.push(design::dropzone(
                &palette,
                IconName::Hash,
                40.0,
                tr(language, "尚未添加文件 · 可添加多个文件或整个文件夹"),
                tr(language, "也可以直接把文件拖到这里"),
                vec![
                    add_file().into_any_element(),
                    add_folder().into_any_element(),
                    load_checksum().into_any_element(),
                ],
                250.0,
            ));
        } else {
            let clear = (!running).then(|| {
                design::link_action(
                    "hashbatch-clear",
                    tr(language, "清空"),
                    &palette,
                    cx.listener(|this, _, _, cx| {
                        if !this.hash_batch_running {
                            this.hash_batch_paths.clear();
                            this.hash_batch_results.clear();
                            this.hash_batch_progress = (0, 0);
                            this.hash_selected_row = None;
                            this.hash_context_menu = None;
                            cx.notify();
                        }
                    }),
                )
            });
            let toolbar = div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.0))
                        .text_size(crate::theme::body())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.text)
                        .child(format!("{count} {}", tr(language, "个文件"))),
                )
                .when_some(clear, |this, clear| this.child(clear))
                .child(div().flex_1())
                .child(add_file())
                .child(add_folder())
                .child(load_checksum());

            let rows = hash_display_rows(
                &self.hash_batch_paths,
                &self.hash_batch_results,
                &algorithms,
                &expected,
                uppercase_display,
            );
            let file_width = 240.0;
            let algorithm_width = 96.0;
            let char_width = crate::theme::caption().as_f32() * 0.66;
            let min_hash_width = algorithms
                .iter()
                .map(|algorithm| hash_hex_len(*algorithm))
                .max()
                .unwrap_or(32) as f32
                * char_width
                + 32.0;
            let page_width = (window
                .viewport_size()
                .width
                .as_f32()
                .min(design::PAGE_MAX_WIDTH)
                - design::PAGE_PAD_X * 2.0
                - 18.0)
                .max(360.0);
            let table_width = page_width.max(28.0 + file_width + algorithm_width + min_hash_width);
            let hash_width = table_width - 28.0 - file_width - algorithm_width;
            let row_height = 32.0;
            let needs_h_scroll = table_width > page_width + 0.5;
            let scrollbar_inset = if needs_h_scroll { 10.0 } else { 0.0 };
            let table_height = 36.0 + rows.len() as f32 * row_height + scrollbar_inset;

            let table_header = div()
                .flex()
                .items_center()
                .h(px(36.0))
                .w(px(table_width))
                .px(px(14.0))
                .bg(palette.panel)
                .border_b_1()
                .border_color(palette.line)
                .child(
                    div()
                        .w(px(file_width))
                        .flex_shrink_0()
                        .pl(px(23.0))
                        .text_size(crate::theme::caption())
                        .text_color(palette.muted)
                        .child(tr(language, "文件名")),
                )
                .child(
                    div()
                        .w(px(algorithm_width))
                        .flex_shrink_0()
                        .px(px(8.0))
                        .text_size(crate::theme::caption())
                        .text_color(palette.muted)
                        .child(tr(language, "算法")),
                )
                .child(
                    div()
                        .w(px(hash_width))
                        .flex_shrink_0()
                        .px(px(8.0))
                        .text_size(crate::theme::caption())
                        .text_color(palette.muted)
                        .child(tr(language, "哈希")),
                );

            let mut table = div()
                .w(px(table_width))
                .flex()
                .flex_col()
                .child(table_header);

            for (row_index, row) in rows.iter().enumerate() {
                let selected = self.hash_selected_row == Some(row_index);
                let hash = row.hash.clone();
                let hash_display = hash.clone().unwrap_or_else(|| {
                    if let Some(error) = &row.error {
                        format!("{}: {error}", tr(language, "错误"))
                    } else {
                        "—".to_string()
                    }
                });
                let hash_color = if row.error.is_some() {
                    palette.bad
                } else if row.matched {
                    if row.insecure {
                        palette.warn
                    } else {
                        palette.ok
                    }
                } else if hash.is_some() {
                    palette.sub
                } else {
                    palette.muted
                };
                let menu_file = row.path.clone();
                let menu_algorithm = row.algorithm;
                let menu_hash = hash.clone();
                let row_id = format!("hashbatch-row-{row_index}");

                let table_row = div()
                    .id(row_id)
                    .flex()
                    .items_center()
                    .h(px(row_height))
                    .w(px(table_width))
                    .px(px(14.0))
                    .when(row_index + 1 < rows.len(), |this| {
                        this.border_b_1().border_color(palette.line)
                    })
                    .when(selected, |this| this.bg(palette.accent_soft))
                    .when(!selected, |this| {
                        this.hover(|style| style.bg(palette.accent_softer))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.hash_selected_row = Some(row_index);
                        this.hash_context_menu = None;
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.hash_selected_row = Some(row_index);
                            this.hash_context_menu = Some(HashContextMenu {
                                x: event.position.x.as_f32(),
                                y: event.position.y.as_f32(),
                                file: menu_file.clone(),
                                algorithm: menu_algorithm,
                                hash: menu_hash.clone(),
                            });
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .w(px(file_width))
                            .flex_shrink_0()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(icon(IconName::FileText, palette.muted, 15.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(crate::theme::caption())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(palette.text)
                                    .child(hash_row_name(row)),
                            ),
                    )
                    .child(
                        div()
                            .w(px(algorithm_width))
                            .flex_shrink_0()
                            .px(px(8.0))
                            .truncate()
                            .text_size(crate::theme::caption())
                            .text_color(palette.sub)
                            .child(
                                row.algorithm
                                    .map(HashAlgorithm::label)
                                    .unwrap_or("—")
                                    .to_string(),
                            ),
                    )
                    .child(
                        div()
                            .w(px(hash_width))
                            .flex_shrink_0()
                            .px(px(8.0))
                            .text_size(crate::theme::caption())
                            .text_color(hash_color)
                            .when(hash.is_some() && monospace, |this| {
                                this.font_family("Cascadia Mono")
                            })
                            .whitespace_nowrap()
                            .child(hash_display),
                    );
                table = table.child(table_row);
            }

            let table = table.pb(px(scrollbar_inset));
            let table_scroll = div()
                .id("hash-table-scroll")
                .w_full()
                .h(px(table_height))
                .flex_none()
                .overflow_x_scroll()
                .restrict_scroll_to_axis()
                .track_scroll(&self.hash_table_scroll)
                .child(table)
                .when(needs_h_scroll, |this| {
                    this.child(
                        div()
                            .absolute()
                            .left(px(14.0))
                            .right(px(14.0))
                            .bottom_0()
                            .h(px(10.0))
                            .child(
                                Scrollbar::horizontal(&self.hash_table_scroll)
                                    .id("hash-table-scrollbar")
                                    .mode(ScrollbarMode::Always),
                            ),
                    )
                });

            body.push(toolbar.into_any_element());
            body.push(
                design::card(&palette)
                    .w_full()
                    .h(px(table_height))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(table_scroll)
                    .into_any_element(),
            );
        }

        let export_format = self.hash_export_format;
        let mut format_row = div().flex().items_center().gap(px(4.0)).child(
            div()
                .text_size(crate::theme::caption())
                .text_color(palette.muted)
                .child(tr(language, "导出格式")),
        );
        for format in HashExportFormat::ALL {
            let selected = format == export_format;
            format_row = format_row.child(
                div()
                    .id(format!("hash-format-{}", format.extension()))
                    .role(Role::Button)
                    .aria_label(format.label())
                    .h(px(26.0))
                    .px(px(7.0))
                    .rounded(px(5.0))
                    .bg(if selected {
                        palette.accent_soft
                    } else {
                        palette.bg
                    })
                    .text_size(crate::theme::caption())
                    .text_color(if selected {
                        palette.accent
                    } else {
                        palette.sub
                    })
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.hash_export_format = format;
                        cx.notify();
                    }))
                    .child(format.label()),
            );
        }

        let export_text =
            hash_export_contents(&self.hash_batch_results, export_format, &self.settings.hash);
        let export_value = export_text.clone();
        let export = Button::new("hash-export")
            .label(tr(language, "导出…"))
            .icon(icon(IconName::Download, palette.text, 14.0))
            .outline()
            .h(px(32.0))
            .px(px(12.0))
            .text_size(crate::theme::caption())
            .disabled(export_text.is_empty())
            .opacity(if export_text.is_empty() { 0.45 } else { 1.0 })
            .on_click(cx.listener(move |this, _, window, cx| {
                if export_value.is_empty() {
                    this.show_toast(tr(language, "没有可导出的哈希"), ToastKind::Info, cx);
                    return;
                }
                let extension = this.hash_export_format.extension();
                let Some(path) = crate::components::file_dialog_in(window, move || {
                    rfd::FileDialog::new()
                        .add_filter(tr(language, "校验文件"), &[extension])
                        .set_file_name(format!("checksums.{extension}"))
                        .save_file()
                }) else {
                    return;
                };
                match std::fs::write(&path, &export_value) {
                    Ok(()) => this.show_toast(
                        format!("{}: {}", tr(language, "校验文件已保存"), path.display()),
                        ToastKind::Success,
                        cx,
                    ),
                    Err(error) => this.show_toast(
                        format!("{}: {error}", tr(language, "保存失败")),
                        ToastKind::Error,
                        cx,
                    ),
                }
            }));
        let copy_value = export_text;
        let copy_all = Button::new("hash-copy-all")
            .label(tr(language, "复制全部"))
            .icon(icon(IconName::Copy, palette.text, 14.0))
            .outline()
            .h(px(32.0))
            .px(px(12.0))
            .text_size(crate::theme::caption())
            .disabled(copy_value.is_empty())
            .opacity(if copy_value.is_empty() { 0.45 } else { 1.0 })
            .on_click(cx.listener(move |this, _, _, cx| {
                if copy_value.is_empty() {
                    this.show_toast(tr(language, "没有可复制的哈希"), ToastKind::Info, cx);
                    return;
                }
                cx.write_to_clipboard(ClipboardItem::new_string(copy_value.clone()));
                this.show_toast(tr(language, "哈希结果已复制"), ToastKind::Success, cx);
            }));

        let footer = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(format_row)
            .child(div().flex_1())
            .child(copy_all)
            .child(export)
            .into_any_element();
        design::page_shell_bare(&palette, body, Some(footer))
    }

    fn hash_rows_text(&self) -> String {
        hash_display_rows(
            &self.hash_batch_paths,
            &self.hash_batch_results,
            &self.hash_batch_algorithms,
            &self.hash_batch_expected,
            self.settings.hash.uppercase_display,
        )
        .iter()
        .filter(|row| row.hash.is_some())
        .map(hash_row_text)
        .collect::<Vec<_>>()
        .join("\n")
    }

    fn hash_menu_item<F>(
        &self,
        id: &'static str,
        label: &'static str,
        enabled: bool,
        action: F,
        cx: &mut Context<Self>,
    ) -> AnyElement
    where
        F: Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    {
        let palette = self.palette;
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .h(px(30.0))
            .px(px(9.0))
            .rounded(px(crate::theme::RADIUS_CONTROL))
            .flex()
            .items_center()
            .text_size(crate::theme::body())
            .when(enabled, |this| {
                this.cursor_pointer()
                    .text_color(palette.text)
                    .hover(|style| style.bg(palette.accent_soft).text_color(palette.accent))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.hash_context_menu = None;
                        action(this, window, cx);
                        cx.notify();
                    }))
            })
            .when(!enabled, |this| this.text_color(palette.muted).opacity(0.6))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .into_any_element()
    }

    pub(in crate::app) fn render_hash_context_menu(
        &mut self,
        menu: HashContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let language = self.language;
        let viewport = window.viewport_size();
        let width = 188.0;
        let estimated_height = 4.0 * 30.0 + 12.0;
        let left = menu
            .x
            .clamp(8.0, (viewport.width.as_f32() - width - 8.0).max(8.0));
        let top = menu.y.clamp(
            8.0,
            (viewport.height.as_f32() - estimated_height - 8.0).max(8.0),
        );

        let hash = menu.hash.clone();
        let row_text = hash_row_text(&HashDisplayRow {
            path: menu.file.clone(),
            algorithm: menu.algorithm,
            hash: menu.hash.clone(),
            error: None,
            matched: false,
            insecure: false,
        });
        let file_name = menu
            .file
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| menu.file.display().to_string());
        let all_text = self.hash_rows_text();

        let copy_hash_value = hash.clone();
        let copy_hash = self.hash_menu_item(
            "hash-ctx-copy-hash",
            tr(language, "复制哈希值"),
            hash.is_some(),
            move |this, _, cx| {
                let Some(value) = copy_hash_value.clone() else {
                    return;
                };
                cx.write_to_clipboard(ClipboardItem::new_string(value));
                this.show_toast(tr(language, "已复制哈希值"), ToastKind::Info, cx);
            },
            cx,
        );
        let copy_row_value = row_text;
        let copy_row = self.hash_menu_item(
            "hash-ctx-copy-row",
            tr(language, "复制整行"),
            hash.is_some(),
            move |this, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy_row_value.clone()));
                this.show_toast(tr(language, "已复制整行"), ToastKind::Info, cx);
            },
            cx,
        );
        let copy_name = self.hash_menu_item(
            "hash-ctx-copy-name",
            tr(language, "复制文件名"),
            true,
            move |this, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(file_name.clone()));
                this.show_toast(tr(language, "已复制文件名"), ToastKind::Info, cx);
            },
            cx,
        );
        let all_enabled = !all_text.is_empty();
        let copy_all = self.hash_menu_item(
            "hash-ctx-copy-all",
            tr(language, "复制全部"),
            all_enabled,
            move |this, _, cx| {
                if all_text.is_empty() {
                    return;
                }
                cx.write_to_clipboard(ClipboardItem::new_string(all_text.clone()));
                this.show_toast(tr(language, "已复制全部哈希"), ToastKind::Info, cx);
            },
            cx,
        );

        div()
            .id("hash-context-layer")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                div()
                    .id("hash-context-backdrop")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(Hsla::transparent_black())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.hash_context_menu = None;
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, _, _, cx| {
                            this.hash_context_menu = None;
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .id("hash-context-menu")
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(width))
                    .p(px(5.0))
                    .rounded(px(crate::theme::RADIUS_OVERLAY))
                    .bg(palette.bg)
                    .border_1()
                    .border_color(palette.line_strong)
                    .shadow_lg()
                    .occlude()
                    .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(copy_hash)
                            .child(copy_row)
                            .child(copy_name)
                            .child(copy_all),
                    ),
            )
            .into_any_element()
    }

    pub(in crate::app) fn render_new_archive_page(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let Some(draft) = self.new_draft.as_ref() else {
            return div().into_any_element();
        };
        let sources = draft.sources.clone();
        let format = draft.format;
        let level = draft.level;
        let test_after = draft.test_after;
        let delete_sources = draft.delete_sources;
        let sfx = draft.sfx;
        let error = draft.error.clone();
        let source_bytes = draft.source_bytes;
        let output_dir = default_output_dir(draft);
        let include_root = draft.include_root;
        let zip_algorithm = draft.zip_algorithm;
        let volume_size_mb = draft.volume_size_mb;
        let password = self.new_password.read(cx).unmask_value().to_string();
        let name_value = self.new_name.read(cx).value().to_string();
        let advanced_open = self.new_advanced_open;
        let comment_value = self.new_comment.read(cx).value().to_string();

        let add_files = design::secondary_icon_action(
            "new-add-files",
            self.localized("添加文件"),
            IconName::FileText,
            palette.text,
            cx.listener(|this, _, window, cx| {
                this.add_sources(false, window, cx);
            }),
        );
        let add_folder = design::secondary_icon_action(
            "new-add-folder",
            self.localized("添加文件夹"),
            IconName::FolderOpen,
            palette.text,
            cx.listener(|this, _, window, cx| {
                this.add_sources(true, window, cx);
            }),
        );
        let clear_sources = (!sources.is_empty()).then(|| {
            design::link_action(
                "new-clear-sources",
                self.localized("清空"),
                &palette,
                cx.listener(|this, _, _, cx| {
                    if let Some(draft) = &mut this.new_draft {
                        draft.sources.clear();
                        draft.source_bytes = 0;
                        draft.output_dir = None;
                    }
                    cx.notify();
                }),
            )
        });

        let src_head = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(crate::theme::body())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(palette.text)
                    .child(self.localized("源文件"))
                    .child(design::pill(
                        &palette,
                        crate::i18n::tr_fmt(self.language, "{} 项", &[&sources.len()]),
                        design::TagTone::Accent,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .when_some(clear_sources, |this, clear| this.child(clear))
                    .child(add_files)
                    .child(add_folder),
            );

        let sources_box: AnyElement = if sources.is_empty() {
            design::dropzone(
                &palette,
                IconName::FolderOpen,
                34.0,
                self.localized("尚未选择文件或文件夹 · 拖放即可添加"),
                "",
                Vec::new(),
                120.0,
            )
        } else {
            let mut list = div()
                .id("new-source-list")
                .flex()
                .flex_col()
                .max_h(px(140.0))
                .overflow_y_scroll();
            let total = sources.len();
            for (index, path) in sources.iter().enumerate() {
                let is_dir = path.is_dir();
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                let parent = path
                    .parent()
                    .map(|parent| parent.display().to_string())
                    .unwrap_or_default();
                let remove_path = path.clone();
                list = list.child(
                    div()
                        .id(format!("new-source-{index}"))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .h(px(40.0))
                        .px(px(14.0))
                        .when(index + 1 < total, |this| {
                            this.border_b_1().border_color(palette.line)
                        })
                        .hover(|style| style.bg(palette.accent_softer))
                        .child(icon(
                            if is_dir {
                                IconName::Folder
                            } else {
                                IconName::FileText
                            },
                            if is_dir {
                                palette.accent
                            } else {
                                palette.muted
                            },
                            15.0,
                        ))
                        .child(
                            div()
                                .text_size(crate::theme::body())
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.text)
                                .truncate()
                                .child(name),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family("Cascadia Mono")
                                .text_size(crate::theme::caption())
                                .text_color(palette.muted)
                                .child(parent),
                        )
                        .child(design::icon_action(
                            &palette,
                            format!("new-remove-source-{index}"),
                            IconName::X,
                            palette.muted,
                            false,
                            self.localized("移除源文件"),
                            cx.listener(move |this, _, _, cx| {
                                if let Some(draft) = &mut this.new_draft {
                                    draft.sources.retain(|path| path != &remove_path);
                                    draft.source_bytes =
                                        crate::app::impls::sum_source_bytes(&draft.sources);
                                }
                                cx.notify();
                            }),
                        )),
                );
            }
            div()
                .w_full()
                .rounded(px(crate::theme::RADIUS_OVERLAY))
                .border_1()
                .border_color(palette.line)
                .bg(palette.bg)
                .overflow_hidden()
                .child(list)
                .into_any_element()
        };

        let format_label = if sfx {
            self.localized("ZIP · 自解压 EXE").to_string()
        } else {
            format.label().to_string()
        };
        let entity = cx.entity();
        let format_control = div()
            .id("new-format-select")
            .role(Role::Button)
            .aria_label(format_label.clone())
            .h(px(36.0))
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .px(px(13.0))
            .rounded(px(crate::theme::RADIUS_CONTROL))
            .border_1()
            .border_color(palette.line_strong)
            .bg(palette.bg)
            .cursor_pointer()
            .text_size(crate::theme::body())
            .hover(|style| style.border_color(palette.accent))
            .on_click(cx.listener(|this, _, _, cx| {
                this.open_dropdown(DropdownKind::NewFormat, cx);
            }))
            .on_a11y_action(AccessibleAction::Click, move |_, _, app| {
                entity.update(app, |this, cx| {
                    this.open_dropdown(DropdownKind::NewFormat, cx);
                });
            })
            .child(div().truncate().child(format_label))
            .child(icon(IconName::ChevronDown, palette.muted, 14.0));

        let name_row = div()
            .flex()
            .gap(px(16.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(design::field_label(&palette, self.localized("压缩包名")))
                    .child(
                        crate::components::control_height(Input::new(&self.new_name))
                            .text_size(crate::theme::body())
                            .w_full(),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(design::field_label(&palette, self.localized("压缩格式")))
                    .child(self.track_dropdown(
                        DropdownKind::NewFormat,
                        format_control.into_any_element(),
                        cx,
                    )),
            );

        let level_name = self.localized(compression_level_name(level));
        let level_block = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .justify_between()
                    .child(design::field_label(&palette, self.localized("压缩级别")))
                    .child(
                        div()
                            .text_size(crate::theme::body())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(palette.accent)
                            .child(level_name),
                    ),
            )
            .child(
                div().h(px(24.0)).w_full().child(
                    Slider::new(&self.level_slider)
                        .w_full()
                        .h(px(24.0))
                        .accessibility_label(self.localized("压缩级别")),
                ),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_size(crate::theme::caption())
                    .text_color(palette.muted)
                    .child(self.localized("仅存储 0"))
                    .child(self.localized("极限 9")),
            );

        let volume_label = if volume_size_mb == 0 {
            self.localized("不分卷").to_string()
        } else {
            crate::app::dropdown::volume_size_label(volume_size_mb)
        };
        let volume_control: AnyElement = if sfx {
            div()
                .h(px(36.0))
                .w_full()
                .flex()
                .items_center()
                .px(px(13.0))
                .rounded(px(crate::theme::RADIUS_CONTROL))
                .border_1()
                .border_color(palette.line)
                .bg(palette.panel)
                .text_size(crate::theme::body())
                .text_color(palette.muted)
                .child(self.localized("SFX 不支持分卷"))
                .into_any_element()
        } else {
            let entity = cx.entity();
            let control = div()
                .id("new-volume-size")
                .debug_selector(|| "new-volume-size".to_string())
                .role(Role::Button)
                .aria_label(volume_label.clone())
                .h(px(36.0))
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .px(px(13.0))
                .rounded(px(crate::theme::RADIUS_CONTROL))
                .border_1()
                .border_color(palette.line_strong)
                .bg(palette.bg)
                .cursor_pointer()
                .text_size(crate::theme::body())
                .hover(|style| style.border_color(palette.accent))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.open_dropdown(DropdownKind::NewVolumeSize, cx);
                }))
                .on_a11y_action(AccessibleAction::Click, move |_, _, app| {
                    entity.update(app, |this, cx| {
                        this.open_dropdown(DropdownKind::NewVolumeSize, cx);
                    });
                })
                .child(div().truncate().child(volume_label.clone()))
                .child(icon(IconName::ChevronDown, palette.muted, 14.0));
            self.track_dropdown(DropdownKind::NewVolumeSize, control.into_any_element(), cx)
        };
        let volume_block = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(design::field_label(&palette, self.localized("分卷大小")))
            .child(volume_control)
            .child(
                div()
                    .text_size(crate::theme::caption())
                    .text_color(palette.muted)
                    .child(if sfx {
                        self.localized("自解压 EXE 只能输出单个文件").to_string()
                    } else if volume_size_mb > 0 {
                        self.localized("输出为 .001、.002…，最后一卷可能更小")
                            .to_string()
                    } else {
                        self.localized("不分卷；超过所选大小时自动切分").to_string()
                    }),
            );
        let level_and_volume = div()
            .flex()
            .gap(px(16.0))
            .child(div().flex_1().min_w_0().child(level_block))
            .child(div().w(px(220.0)).flex_none().child(volume_block));

        let mut password_column = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(design::field_label(
                &palette,
                self.localized("密码（留空不加密）"),
            ))
            .child(
                crate::components::control_height(Input::new(&self.new_password))
                    .text_size(crate::theme::body())
                    .w_full(),
            );
        if sfx && !password.is_empty() {
            password_column = password_column.child(
                div()
                    .mt(px(5.0))
                    .text_size(crate::theme::caption())
                    .text_color(palette.bad)
                    .child(self.localized("SFX 不支持密码；请清除密码或取消自解压")),
            );
        } else if !password.is_empty() && format.is_zip() {
            password_column = password_column.child(
                div()
                    .mt(px(5.0))
                    .text_size(crate::theme::caption())
                    .text_color(palette.accent)
                    .child(self.localized("已自动启用 AES-256")),
            );
        } else if !password.is_empty() && format == ArchiveFormat::SevenZ {
            password_column = password_column.child(
                div()
                    .mt(px(5.0))
                    .text_size(crate::theme::caption())
                    .text_color(palette.accent)
                    .child(self.localized("已启用 AES-256 内容加密 + 文件名加密")),
            );
        } else if !password.is_empty() {
            password_column = password_column.child(
                div()
                    .mt(px(5.0))
                    .text_size(crate::theme::caption())
                    .text_color(palette.bad)
                    .child(self.localized("当前格式不支持密码；请选择 ZIP/ZIPX 或 7Z")),
            );
        }
        let credentials_row = div().flex().gap(px(16.0)).child(password_column).child(
            div()
                .flex_1()
                .min_w_0()
                .child(design::field_label(&palette, self.localized("排除过滤器")))
                .child(
                    crate::components::control_height(Input::new(&self.exclude_input))
                        .text_size(crate::theme::body())
                        .w_full(),
                ),
        );

        let checks = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(10.0))
            .child(design::check_chip(
                &palette,
                "new-include-root",
                self.localized("包含根文件夹"),
                include_root,
                false,
                cx.listener(|this, _, _, cx| {
                    if let Some(draft) = &mut this.new_draft {
                        draft.include_root = !draft.include_root;
                    }
                    cx.notify();
                }),
            ))
            .child(design::check_chip(
                &palette,
                "new-delete",
                self.localized("压缩后删除源文件"),
                delete_sources,
                false,
                cx.listener(|this, _, _, cx| {
                    if let Some(draft) = &mut this.new_draft {
                        draft.delete_sources = !draft.delete_sources;
                    }
                    cx.notify();
                }),
            ))
            .child(design::check_chip(
                &palette,
                "new-test",
                self.localized("压缩后测试"),
                test_after,
                false,
                cx.listener(|this, _, _, cx| {
                    if let Some(draft) = &mut this.new_draft {
                        draft.test_after = !draft.test_after;
                    }
                    cx.notify();
                }),
            ))
            .child(design::check_chip(
                &palette,
                "new-sfx",
                self.localized("创建自解压 EXE（SFX）"),
                sfx,
                false,
                cx.listener(|this, _, window, cx| {
                    let mut enabled = false;
                    let mut disabled_format = None;
                    if let Some(draft) = &mut this.new_draft {
                        draft.sfx = !draft.sfx;
                        if draft.sfx {
                            draft.format = ArchiveFormat::Zip;
                            draft.volume_size_mb = 0;
                        } else {
                            disabled_format = Some(draft.format);
                        }
                        enabled = draft.sfx;
                    }
                    if enabled || disabled_format.is_some() {
                        let name = this.new_name.read(cx).value().to_string();
                        let updated = if enabled {
                            crate::app::impls::ensure_sfx_name(&name)
                        } else {
                            crate::app::impls::ensure_archive_name(
                                &name,
                                disabled_format.unwrap_or(ArchiveFormat::Zip),
                            )
                        };
                        this.new_name.update(cx, |state, cx| {
                            state.set_value(updated, window, cx);
                        });
                    }
                    cx.notify();
                }),
            ));

        let output_row = div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .px(px(16.0))
            .py(px(12.0))
            .rounded(px(crate::theme::RADIUS_OVERLAY))
            .border_1()
            .border_color(palette.line)
            .bg(palette.panel)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(7.0))
                    .text_size(crate::theme::body())
                    .text_color(palette.sub)
                    .child(icon(IconName::Folder, palette.accent, 15.0))
                    .child(self.localized("输出到")),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family("Cascadia Mono")
                    .text_size(crate::theme::caption())
                    .text_color(palette.text)
                    .child(output_dir.display().to_string()),
            )
            .child(design::link_action(
                "new-dest",
                self.localized("更改"),
                &palette,
                cx.listener(|this, _, window, cx| this.pick_create_dest(window, cx)),
            ));

        let ratio = estimate_ratio(if sfx { ArchiveFormat::Zip } else { format });
        let split_note = if !sfx && volume_size_mb > 0 {
            crate::i18n::tr_fmt(self.language, " · 按 {} 分卷", &[&volume_label])
        } else {
            String::new()
        };
        let estimate = if source_bytes > 0 {
            crate::i18n::tr_fmt(
                self.language,
                "{}（压缩后约占 {}%）{}",
                &[
                    &format_size((source_bytes as f64 * ratio) as u64),
                    &format!("{:.0}", ratio * 100.0),
                    &split_note,
                ],
            )
        } else {
            self.localized("选择源文件后显示预估").to_string()
        };
        let can_create = !sources.is_empty()
            && !name_value.trim().is_empty()
            && (password.is_empty() || format.is_zip() || format == ArchiveFormat::SevenZ)
            && (!sfx || password.is_empty());

        // ZIP 高级设置下拉。
        let algorithm_entity = cx.entity();
        let algorithm_control = div()
            .id("new-zip-algorithm")
            .role(Role::Button)
            .aria_label(zip_algorithm.label())
            .h(px(36.0))
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .px(px(13.0))
            .rounded(px(crate::theme::RADIUS_CONTROL))
            .border_1()
            .border_color(palette.line_strong)
            .bg(palette.bg)
            .cursor_pointer()
            .text_size(crate::theme::body())
            .hover(|style| style.border_color(palette.accent))
            .on_click(cx.listener(|this, _, _, cx| {
                this.open_dropdown(DropdownKind::NewZipAlgorithm, cx);
            }))
            .on_a11y_action(AccessibleAction::Click, move |_, _, app| {
                algorithm_entity.update(app, |this, cx| {
                    this.open_dropdown(DropdownKind::NewZipAlgorithm, cx);
                });
            })
            .child(div().truncate().child(zip_algorithm.label()))
            .child(icon(IconName::ChevronDown, palette.muted, 14.0));
        let algorithm_control = self.track_dropdown(
            DropdownKind::NewZipAlgorithm,
            algorithm_control.into_any_element(),
            cx,
        );

        let advanced_panel = advanced_open.then(|| {
            let mut card = design::card(&palette)
                .flex()
                .flex_col()
                .gap(px(10.0))
                .p(px(16.0))
                .child(
                    div()
                        .text_size(crate::theme::body())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.text)
                        .child(self.localized("高级设置")),
                )
                .child(
                    div()
                        .text_size(crate::theme::caption())
                        .text_color(palette.muted)
                        .child(self.localized("随压缩格式联动显示可用选项")),
                );
            match format {
                format if format.is_zip() => {
                    card = card
                        .child(design::field_label(&palette, self.localized("压缩方法")))
                        .child(algorithm_control)
                        .child(design::field_label(&palette, self.localized("加密算法")))
                        .child(crate::components::chip(
                            self.localized("AES-256（设置密码后启用）"),
                            &palette,
                        ))
                        .child(design::field_label(&palette, self.localized("CPU 线程数")))
                        .child(crate::components::chip(
                            self.localized("自动（引擎默认）"),
                            &palette,
                        ))
                        .child(design::field_label(&palette, self.localized("压缩包注释")))
                        .child(
                            crate::components::control_height(Input::new(&self.new_comment))
                                .text_size(crate::theme::body())
                                .w_full(),
                        )
                        .child(
                            div()
                                .text_size(crate::theme::caption())
                                .text_color(palette.muted)
                                .child(if comment_value.is_empty() {
                                    self.localized("注释写入 ZIP 中央目录，可留空").to_string()
                                } else {
                                    crate::i18n::tr_fmt(
                                        self.language,
                                        "当前 {} 个字符",
                                        &[&comment_value.chars().count()],
                                    )
                                }),
                        );
                }
                ArchiveFormat::SevenZ => {
                    card = card
                        .child(design::field_label(&palette, self.localized("压缩方法")))
                        .child(crate::components::chip(
                            self.localized("LZMA2 · 固实块"),
                            &palette,
                        ))
                        .child(design::field_label(&palette, self.localized("加密算法")))
                        .child(crate::components::chip(
                            self.localized("AES-256 + 文件名加密（设置密码后启用）"),
                            &palette,
                        ))
                        .child(design::field_label(&palette, self.localized("CPU 线程数")))
                        .child(crate::components::chip(
                            self.localized("自动（引擎默认）"),
                            &palette,
                        ))
                        .child(
                            div()
                                .text_size(crate::theme::caption())
                                .text_color(palette.muted)
                                .child(
                                    self.localized("7Z 使用 LZMA2 固实压缩；头加密随密码启用。"),
                                ),
                        );
                }
                ArchiveFormat::Tar => {
                    card = card.child(
                        div()
                            .text_size(crate::theme::caption())
                            .text_color(palette.muted)
                            .child(self.localized("TAR 仅打包不压缩；修改时间与权限由引擎保留。")),
                    );
                }
                ArchiveFormat::TarGz => {
                    card = card.child(
                        div()
                            .text_size(crate::theme::caption())
                            .text_color(palette.muted)
                            .child(self.localized("TAR.GZ 使用 gzip 单流压缩；不支持密码与注释。")),
                    );
                }
                ArchiveFormat::Lzh => {
                    card = card.child(
                        div()
                            .text_size(crate::theme::caption())
                            .text_color(palette.muted)
                            .child(self.localized(
                                "LZH 使用 LH5 压缩（级别 0 为仅存储）；不支持密码与注释。",
                            )),
                    );
                }
                ArchiveFormat::Iso => {
                    card = card.child(
                        div()
                            .text_size(crate::theme::caption())
                            .text_color(palette.muted)
                            .child(self.localized(
                                "ISO 使用 Joliet 保留中文/长文件名；仅存储，不支持密码。",
                            )),
                    );
                }
                ArchiveFormat::Zpaq => {
                    card =
                        card.child(
                            div()
                                .text_size(crate::theme::caption())
                                .text_color(palette.muted)
                                .child(self.localized(
                                    "ZPAQ 使用 JIDAC 归档（级别 0–5）；不支持密码与注释。",
                                )),
                        );
                }
                _ => {
                    card = card.child(
                        div()
                            .text_size(crate::theme::caption())
                            .text_color(palette.muted)
                            .child(self.localized("当前格式为单文件流压缩，无附加高级选项。")),
                    );
                }
            }
            div()
                .w(px(280.0))
                .flex_none()
                .flex()
                .flex_col()
                .child(card)
                .into_any_element()
        });

        let advanced_toggle = design::link_action(
            "new-advanced-toggle",
            if advanced_open {
                self.localized("收起选项")
            } else {
                self.localized("更多选项")
            },
            &palette,
            cx.listener(|this, _, window, cx| {
                this.toggle_new_advanced(window, cx);
            }),
        );

        let estimate_bar = div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(16.0))
            .child(advanced_toggle)
            .child(
                div()
                    .flex_none()
                    .text_size(crate::theme::body())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(palette.text)
                    .child(self.localized("预估结果")),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(crate::theme::body())
                    .text_color(if source_bytes > 0 {
                        palette.sub
                    } else {
                        palette.muted
                    })
                    .child(estimate),
            )
            .when(self.feature_window.is_some(), |this| {
                this.child(
                    Button::new("new-cancel")
                        .label(self.localized("取消"))
                        .outline()
                        .h(px(36.0))
                        .px(px(16.0))
                        .text_size(crate::theme::body())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_feature_window(window, cx);
                        })),
                )
            })
            .child(
                div().debug_selector(|| "new-create".to_string()).child(
                    Button::new("new-create")
                        .label(self.localized("开始压缩"))
                        .icon(icon(IconName::Play, palette.accent_text, 15.0))
                        .primary()
                        .h(px(36.0))
                        .px(px(16.0))
                        .text_size(crate::theme::body())
                        .disabled(!can_create)
                        .opacity(if can_create { 1.0 } else { 0.45 })
                        .on_click(cx.listener(|this, _, _, cx| this.start_create(cx))),
                ),
            );

        let mut body = vec![
            src_head.into_any_element(),
            sources_box,
            name_row.into_any_element(),
            level_and_volume.into_any_element(),
            credentials_row.into_any_element(),
            checks.into_any_element(),
            output_row.into_any_element(),
        ];
        if let Some(error) = error {
            body.push(
                div()
                    .text_size(crate::theme::caption())
                    .text_color(palette.bad)
                    .child(error)
                    .into_any_element(),
            );
        }

        let main_column = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(14.0))
            .children(body);
        let layout = if let Some(panel) = advanced_panel {
            div()
                .flex()
                .items_start()
                .gap(px(18.0))
                .child(main_column)
                .child(panel)
                .into_any_element()
        } else {
            main_column.into_any_element()
        };

        design::page_shell_bare(
            &palette,
            vec![layout],
            Some(estimate_bar.into_any_element()),
        )
    }
}

fn compression_level_name(level: u8) -> &'static str {
    match level {
        0 => "仅存储 (0)",
        1..=2 => "最快压缩 (1-2)",
        3..=4 => "快速压缩 (3-4)",
        5..=6 => "标准压缩 (5-6)",
        7..=8 => "较好压缩 (7-8)",
        _ => "极限压缩 (9)",
    }
}

fn estimate_ratio(format: ArchiveFormat) -> f64 {
    match format {
        ArchiveFormat::Zip => 0.62,
        ArchiveFormat::SevenZ => 0.55,
        ArchiveFormat::Tar => 1.0,
        ArchiveFormat::TarGz | ArchiveFormat::Gz => 0.62,
        ArchiveFormat::TarBz2 | ArchiveFormat::Bz2 => 0.58,
        ArchiveFormat::TarXz | ArchiveFormat::Xz => 0.56,
        ArchiveFormat::TarZst | ArchiveFormat::Zst => 0.62,
        _ => 0.6,
    }
}
