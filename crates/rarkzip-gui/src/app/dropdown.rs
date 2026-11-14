use crate::app::impls::ui::level_label;
use crate::app::*;

/// 所有自定义 select 控件共用的下拉类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DropdownKind {
    SettingsMemory,
    SettingsCodepage,
    TopCodepage,
    SettingsLevel,
    SettingsFormat,
    SettingsTheme,
    SettingsSkin,
    NewFormat,
    NewZipAlgorithm,
    NewVolumeSize,
}

#[derive(Debug, Clone)]
struct DropdownOption {
    label: String,
    selected: bool,
    enabled: bool,
    separator_before: bool,
}

impl DropdownOption {
    fn new(label: impl Into<String>, selected: bool) -> Self {
        Self {
            label: label.into(),
            selected,
            enabled: true,
            separator_before: false,
        }
    }

    fn separator_before(mut self) -> Self {
        self.separator_before = true;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewFormatChoice {
    Format(ArchiveFormat),
    Sfx,
}

/// 新建压缩格式列表；顺序参考设计稿，末尾补充引擎可创建的其他格式。
const NEW_FORMAT_CHOICES: &[(&str, NewFormatChoice)] = &[
    ("ZIP", NewFormatChoice::Format(ArchiveFormat::Zip)),
    ("ZIPX", NewFormatChoice::Format(ArchiveFormat::Zipx)),
    ("EXE", NewFormatChoice::Sfx),
    ("TAR", NewFormatChoice::Format(ArchiveFormat::Tar)),
    ("TGZ", NewFormatChoice::Format(ArchiveFormat::TarGz)),
    ("LZH", NewFormatChoice::Format(ArchiveFormat::Lzh)),
    ("ISO", NewFormatChoice::Format(ArchiveFormat::Iso)),
    ("7Z", NewFormatChoice::Format(ArchiveFormat::SevenZ)),
    ("GZ", NewFormatChoice::Format(ArchiveFormat::Gz)),
    ("BZ2", NewFormatChoice::Format(ArchiveFormat::Bz2)),
    ("XZ", NewFormatChoice::Format(ArchiveFormat::Xz)),
    ("ZPAQ", NewFormatChoice::Format(ArchiveFormat::Zpaq)),
    ("TAR.BZ2", NewFormatChoice::Format(ArchiveFormat::TarBz2)),
    ("TAR.XZ", NewFormatChoice::Format(ArchiveFormat::TarXz)),
    ("TAR.ZST", NewFormatChoice::Format(ArchiveFormat::TarZst)),
    ("ZST", NewFormatChoice::Format(ArchiveFormat::Zst)),
];

/// 引擎实际可创建的格式集合（用于设置页默认格式）。
const CREATE_FORMATS: &[ArchiveFormat] = &[
    ArchiveFormat::Zip,
    ArchiveFormat::Zipx,
    ArchiveFormat::SevenZ,
    ArchiveFormat::Tar,
    ArchiveFormat::TarGz,
    ArchiveFormat::TarBz2,
    ArchiveFormat::TarXz,
    ArchiveFormat::TarZst,
    ArchiveFormat::Gz,
    ArchiveFormat::Bz2,
    ArchiveFormat::Xz,
    ArchiveFormat::Zst,
    ArchiveFormat::Lzh,
    ArchiveFormat::Iso,
    ArchiveFormat::Zpaq,
];
const ZIP_ALGORITHMS: [ZipAlgorithm; 4] = [
    ZipAlgorithm::Deflate,
    ZipAlgorithm::Bzip2,
    ZipAlgorithm::Lzma,
    ZipAlgorithm::Zstd,
];
const MEMORY_OPTIONS: [u32; 4] = [1024, 2048, 4096, 8192];
const VOLUME_SIZES: [u32; 10] = [0, 10, 50, 100, 200, 500, 700, 1024, 2048, 4096];
const CODEPAGES: &[Codepage] = &Codepage::ALL;
const THEME_MODES: [ThemeMode; 3] = [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark];

impl MainView {
    pub(crate) fn open_dropdown(&mut self, kind: DropdownKind, cx: &mut Context<Self>) {
        self.close_app_menu();
        self.archive_menu = None;
        self.dropdown = if self.dropdown == Some(kind) {
            None
        } else {
            Some(kind)
        };
        cx.notify();
    }

    /// 记录触发控件的屏幕边界，供下拉面板精确锚定。
    pub(crate) fn track_dropdown(
        &self,
        kind: DropdownKind,
        child: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        div()
            .on_children_prepainted(move |bounds, _, app| {
                if let Some(bounds) = bounds.first().copied() {
                    entity.update(app, |this, _| {
                        this.dropdown_bounds.insert(kind, bounds);
                    });
                }
            })
            .child(child)
            .into_any_element()
    }

    fn dropdown_options(&self, kind: DropdownKind) -> Vec<DropdownOption> {
        match kind {
            DropdownKind::SettingsMemory => {
                let current = self.settings.extraction.max_memory_mb;
                MEMORY_OPTIONS
                    .iter()
                    .map(|mb| DropdownOption::new(memory_label(*mb), *mb == current))
                    .collect()
            }
            DropdownKind::SettingsCodepage | DropdownKind::TopCodepage => {
                let current = Codepage::from_name(&self.settings.extraction.default_codepage);
                CODEPAGES
                    .iter()
                    .map(|codepage| {
                        let option =
                            DropdownOption::new(codepage.label().to_string(), *codepage == current);
                        if *codepage == Codepage::Auto {
                            option.separator_before()
                        } else {
                            option
                        }
                    })
                    .collect()
            }
            DropdownKind::SettingsLevel => {
                let current = self.settings.compression.default_level.min(9);
                (0..=9)
                    .map(|level| {
                        DropdownOption::new(
                            format!("{level} · {}", level_label(level)),
                            level == current,
                        )
                    })
                    .collect()
            }
            DropdownKind::SettingsFormat => {
                let current = ArchiveFormat::from_name(&self.settings.compression.default_format)
                    .filter(|format| format.can_create())
                    .unwrap_or(ArchiveFormat::Zip);
                CREATE_FORMATS
                    .iter()
                    .map(|format| {
                        DropdownOption::new(format.label().to_string(), *format == current)
                    })
                    .collect()
            }
            DropdownKind::SettingsTheme => {
                let current = ThemeMode::from_setting(&self.settings.view.theme);
                THEME_MODES
                    .iter()
                    .map(|mode| {
                        DropdownOption::new(
                            crate::i18n::tr(self.language, mode.label()).to_string(),
                            *mode == current,
                        )
                    })
                    .collect()
            }
            DropdownKind::SettingsSkin => Skin::ALL
                .iter()
                .map(|skin| {
                    DropdownOption::new(skin.label(self.language).to_string(), *skin == self.skin)
                })
                .collect(),
            DropdownKind::NewFormat => {
                let (current_format, current_sfx) = self
                    .new_draft
                    .as_ref()
                    .map(|draft| (draft.format, draft.sfx))
                    .unwrap_or((ArchiveFormat::Zip, false));
                NEW_FORMAT_CHOICES
                    .iter()
                    .map(|(label, choice)| match choice {
                        NewFormatChoice::Format(format) => {
                            DropdownOption::new(*label, !current_sfx && *format == current_format)
                        }
                        NewFormatChoice::Sfx => {
                            DropdownOption::new(format!("{label}（自解压）"), current_sfx)
                        }
                    })
                    .collect()
            }
            DropdownKind::NewZipAlgorithm => {
                let current = self
                    .new_draft
                    .as_ref()
                    .map(|draft| draft.zip_algorithm)
                    .unwrap_or_default();
                ZIP_ALGORITHMS
                    .iter()
                    .map(|algorithm| {
                        DropdownOption::new(algorithm.label().to_string(), *algorithm == current)
                    })
                    .collect()
            }
            DropdownKind::NewVolumeSize => {
                let current = self
                    .new_draft
                    .as_ref()
                    .map(|draft| draft.volume_size_mb)
                    .unwrap_or(0);
                VOLUME_SIZES
                    .iter()
                    .map(|mb| DropdownOption::new(volume_size_label(*mb), *mb == current))
                    .collect()
            }
        }
    }

    fn apply_dropdown(
        &mut self,
        kind: DropdownKind,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match kind {
            DropdownKind::SettingsMemory => {
                if let Some(mb) = MEMORY_OPTIONS.get(index) {
                    self.settings.extraction.max_memory_mb = *mb;
                    self.save_settings();
                }
            }
            DropdownKind::SettingsCodepage => {
                if let Some(codepage) = CODEPAGES.get(index) {
                    self.settings.extraction.default_codepage = codepage.name().to_string();
                    self.save_settings();
                }
            }
            DropdownKind::TopCodepage => {
                if let Some(codepage) = CODEPAGES.get(index).copied() {
                    self.settings.extraction.default_codepage = codepage.name().to_string();
                    self.save_settings();
                    let has_archive = self.archive.is_some();
                    if has_archive {
                        self.reload_archive(cx);
                    }
                    self.show_toast(
                        format!("文本编码：{}", codepage.label()),
                        ToastKind::Info,
                        cx,
                    );
                }
            }
            DropdownKind::SettingsLevel => {
                self.settings.compression.default_level = index.min(9) as u8;
                self.save_settings();
            }
            DropdownKind::SettingsFormat => {
                if let Some(format) = CREATE_FORMATS.get(index) {
                    self.settings.compression.default_format = format.extension().to_string();
                    self.save_settings();
                }
            }
            DropdownKind::SettingsTheme => {
                if let Some(mode) = THEME_MODES.get(index).copied() {
                    self.set_theme_mode(mode, window, cx);
                }
            }
            DropdownKind::SettingsSkin => {
                if let Some(skin) = Skin::ALL.get(index).copied() {
                    self.set_skin(skin, window, cx);
                }
            }
            DropdownKind::NewFormat => {
                if let Some((_, choice)) = NEW_FORMAT_CHOICES.get(index) {
                    let name = self.new_name.read(cx).value().to_string();
                    let updated = match choice {
                        NewFormatChoice::Format(format) => {
                            if let Some(draft) = &mut self.new_draft {
                                draft.format = *format;
                                draft.sfx = false;
                                draft.error = None;
                            }
                            replace_archive_extension(&name, *format)
                        }
                        NewFormatChoice::Sfx => {
                            if let Some(draft) = &mut self.new_draft {
                                draft.format = ArchiveFormat::Zip;
                                draft.sfx = true;
                                draft.volume_size_mb = 0;
                                draft.error = None;
                            }
                            crate::app::impls::ensure_sfx_name(&name)
                        }
                    };
                    self.new_name.update(cx, |state, cx| {
                        state.set_value(updated, window, cx);
                    });
                }
            }
            DropdownKind::NewZipAlgorithm => {
                if let Some(algorithm) = ZIP_ALGORITHMS.get(index).copied() {
                    if let Some(draft) = &mut self.new_draft {
                        draft.zip_algorithm = algorithm;
                    }
                }
            }
            DropdownKind::NewVolumeSize => {
                if let Some(mb) = VOLUME_SIZES.get(index).copied() {
                    if let Some(draft) = &mut self.new_draft {
                        draft.volume_size_mb = mb;
                        draft.error = None;
                    }
                }
            }
        }
        self.dropdown = None;
        cx.notify();
    }

    pub(crate) fn render_dropdown_overlay(
        &mut self,
        kind: DropdownKind,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let options = self.dropdown_options(kind);
        let viewport = window.viewport_size();
        let fallback = Bounds {
            origin: point(px(220.0), px(220.0)),
            size: size(px(180.0), px(36.0)),
        };
        let bounds = self.dropdown_bounds.get(&kind).copied().unwrap_or(fallback);
        let width = bounds
            .size
            .width
            .as_f32()
            .max(184.0)
            .min((viewport.width.as_f32() - 16.0).max(184.0));
        let row_height = 32.0;
        let max_height = match kind {
            // 编码列表较长，尽量一次性展开（原型中为长菜单）。
            DropdownKind::SettingsCodepage | DropdownKind::TopCodepage => 540.0,
            _ => 320.0,
        };
        let separator_count = options
            .iter()
            .filter(|option| option.separator_before)
            .count();
        let height = ((options.len() as f32 * row_height) + separator_count as f32 * 9.0 + 8.0)
            .min(max_height)
            .min(viewport.height.as_f32() - 16.0);
        let left = bounds
            .origin
            .x
            .as_f32()
            .clamp(8.0, (viewport.width.as_f32() - width - 8.0).max(8.0));
        let below = bounds.origin.y.as_f32() + bounds.size.height.as_f32() + 4.0;
        let above = bounds.origin.y.as_f32() - height - 4.0;
        let top = if below + height <= viewport.height.as_f32() - 8.0 {
            below
        } else {
            above.max(8.0)
        };

        let mut panel = div()
            .id("dropdown-panel")
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(width))
            .max_h(px(height))
            .overflow_y_scroll()
            .p(px(4.0))
            .rounded(px(crate::theme::RADIUS_OVERLAY))
            .bg(palette.bg)
            .border_1()
            .border_color(palette.line)
            .shadow_lg()
            // 与菜单相同：阻止全屏遮罩抢占面板内部的 mouse_down。
            .occlude()
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()));

        for (index, option) in options.into_iter().enumerate() {
            let DropdownOption {
                label,
                selected,
                enabled,
                separator_before,
            } = option;
            if separator_before {
                panel = panel.child(div().h(px(1.0)).mx(px(6.0)).my(px(4.0)).bg(palette.line));
            }
            let entity = cx.entity();
            let mut row = div()
                .id(format!("dropdown-{}-{index}", kind.key()))
                .debug_selector(|| format!("dropdown-{}-{index}", kind.key()))
                .role(Role::Button)
                .aria_label(label.clone())
                .h(px(30.0))
                .px(px(8.0))
                .rounded(px(crate::theme::RADIUS_CONTROL))
                .flex()
                .items_center()
                .gap(px(8.0))
                .text_size(crate::theme::body())
                .when(!enabled, |this| this.opacity(0.45));
            if enabled {
                row = row
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.apply_dropdown(kind, index, window, cx);
                    }))
                    .on_a11y_action(AccessibleAction::Click, move |_, window, app| {
                        entity.update(app, |this, cx| {
                            this.apply_dropdown(kind, index, window, cx);
                        });
                    });
            }
            row = row
                .child(
                    div()
                        .w(px(14.0))
                        .text_size(crate::theme::caption())
                        .text_color(palette.accent)
                        .child(if selected { "✓" } else { "" }),
                )
                .child(div().flex_1().min_w_0().truncate().child(label));
            row = if selected {
                row.bg(palette.accent_soft).text_color(palette.accent)
            } else if enabled {
                row.text_color(palette.text)
                    .hover(|style| style.bg(palette.hover))
            } else {
                row.text_color(palette.muted)
            };
            panel = panel.child(row);
        }

        div()
            .id("dropdown-layer")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                div()
                    .id("dropdown-backdrop")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(Hsla::transparent_black())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.dropdown = None;
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
            .child(panel)
            .into_any_element()
    }
}

impl DropdownKind {
    fn key(self) -> &'static str {
        match self {
            Self::SettingsMemory => "memory",
            Self::SettingsCodepage => "codepage",
            Self::TopCodepage => "top-codepage",
            Self::SettingsLevel => "level",
            Self::SettingsFormat => "format",
            Self::SettingsTheme => "theme",
            Self::SettingsSkin => "skin",
            Self::NewFormat => "new-format",
            Self::NewZipAlgorithm => "new-zip-algorithm",
            Self::NewVolumeSize => "new-volume-size",
        }
    }
}

pub(crate) fn volume_size_label(mb: u32) -> String {
    if mb == 0 {
        "不分卷".to_string()
    } else if mb >= 1024 && mb % 1024 == 0 {
        format!("{} GB", mb / 1024)
    } else {
        format!("{mb} MB")
    }
}

fn memory_label(mb: u32) -> String {
    if mb >= 1024 && mb % 1024 == 0 {
        format!("{} GB", mb / 1024)
    } else {
        format!("{} MB", mb)
    }
}

fn replace_archive_extension(name: &str, format: ArchiveFormat) -> String {
    let name = name.trim();
    if name.is_empty() {
        return format!("archive.{}", format.extension());
    }
    let lower = name.to_ascii_lowercase();
    let extension = format.extension();
    if lower.ends_with(&format!(".{extension}")) {
        return name.to_string();
    }
    for suffix in [
        "tar.gz", "tar.bz2", "tar.xz", "tar.zst", "tgz", "tbz2", "tbz", "txz", "tzst", "zipx",
        "zip", "7z", "tar", "gz", "bz2", "xz", "zst", "exe", "lzh", "lha", "iso", "zpaq",
    ] {
        if lower.ends_with(&format!(".{suffix}")) {
            let stem = &name[..name.len() - suffix.len() - 1];
            return format!("{stem}.{extension}");
        }
    }
    format!("{name}.{extension}")
}
