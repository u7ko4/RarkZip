use gpui_kit::component::{Theme, ThemeMode as ComponentThemeMode};
use gpui_kit::*;
use std::sync::atomic::{AtomicU32, Ordering};

/// 用户设置的界面基准字号（px）。所有文字尺寸按 `size / 13` 等比缩放。
static UI_FONT_SIZE_BITS: AtomicU32 = AtomicU32::new(13.0_f32.to_bits());

pub fn set_ui_font_size(size: f32) {
    let clamped = if size.is_finite() {
        size.clamp(11.0, 17.0)
    } else {
        13.0
    };
    UI_FONT_SIZE_BITS.store(clamped.to_bits(), Ordering::Relaxed);
}

pub fn ui_font_size() -> f32 {
    let size = f32::from_bits(UI_FONT_SIZE_BITS.load(Ordering::Relaxed));
    if size.is_finite() {
        size.clamp(11.0, 17.0)
    } else {
        13.0
    }
}

/// 按设置中的界面字号缩放文字尺寸；非文字控件尺寸保持不变。
pub fn font_px(size: f32) -> Pixels {
    px(size * (ui_font_size() / 13.0))
}

// Baseline compatibility tokens used by files that already referenced the
// shared typography/radius names before the WinUI redesign.
pub const RADIUS_CONTROL: f32 = 6.0;
pub const RADIUS_OVERLAY: f32 = 10.0;

pub fn caption() -> Pixels {
    font_px(12.0)
}

pub fn body() -> Pixels {
    font_px(13.0)
}

pub fn subtitle() -> Pixels {
    font_px(16.0)
}

pub fn title() -> Pixels {
    font_px(19.0)
}

pub fn ui_font_family() -> &'static str {
    "Microsoft YaHei UI"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub fn from_setting(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }

    pub fn as_setting(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "跟随系统",
            Self::Light => "浅色",
            Self::Dark => "暗色",
        }
    }

    pub fn is_dark(self, appearance: WindowAppearance) -> bool {
        match self {
            Self::System => matches!(
                appearance,
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
            Self::Light => false,
            Self::Dark => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skin {
    Blue,
    Emerald,
    Violet,
    Amber,
    Rose,
    Slate,
}

impl Skin {
    pub const ALL: [Skin; 6] = [
        Skin::Blue,
        Skin::Emerald,
        Skin::Violet,
        Skin::Amber,
        Skin::Rose,
        Skin::Slate,
    ];

    pub fn from_setting(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "emerald" | "green" => Self::Emerald,
            "violet" | "purple" => Self::Violet,
            "amber" | "orange" => Self::Amber,
            "rose" | "pink" => Self::Rose,
            "slate" | "gray" => Self::Slate,
            _ => Self::Blue,
        }
    }

    pub fn as_setting(self) -> &'static str {
        match self {
            Self::Blue => "blue",
            Self::Emerald => "emerald",
            Self::Violet => "violet",
            Self::Amber => "amber",
            Self::Rose => "rose",
            Self::Slate => "slate",
        }
    }

    pub fn label(self, language: crate::i18n::Language) -> &'static str {
        let (zh, en) = match self {
            Self::Blue => ("默认蓝", "Blue"),
            Self::Emerald => ("翡翠绿", "Emerald"),
            Self::Violet => ("紫罗兰", "Violet"),
            Self::Amber => ("琥珀", "Amber"),
            Self::Rose => ("玫瑰", "Rose"),
            Self::Slate => ("石板灰", "Slate"),
        };
        if language == crate::i18n::Language::En {
            en
        } else {
            zh
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Hsla,
    /// 页面/大面板背景。
    pub page: Hsla,
    /// 卡片背景。
    pub card: Hsla,
    pub panel: Hsla,
    pub hover: Hsla,
    pub line: Hsla,
    /// 输入框、可选卡片等需要更明显边界时使用。
    pub line_strong: Hsla,
    pub text: Hsla,
    pub sub: Hsla,
    /// 辅助说明文字（比 `sub` 更浅）。
    pub muted: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub accent_active: Hsla,
    pub accent_text: Hsla,
    pub accent_soft: Hsla,
    /// 选中卡片、行悬停使用的极浅强调色。
    pub accent_softer: Hsla,
    pub ok: Hsla,
    pub ok_soft: Hsla,
    pub warn: Hsla,
    pub warn_soft: Hsla,
    pub bad: Hsla,
    pub bad_soft: Hsla,
    pub bar: Hsla,
    pub overlay: Hsla,
    pub white: Hsla,
    pub dark: bool,
}

fn color(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn alpha(hex: u32, a: u8) -> Hsla {
    Hsla::from(Rgba {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a: a as f32 / 255.0,
    })
}

impl Palette {
    pub fn light() -> Self {
        Self {
            bg: color(0xFFFFFF),
            page: color(0xFFFFFF),
            card: color(0xFFFFFF),
            panel: color(0xF7F9FC),
            hover: color(0xF0F3F8),
            line: color(0xE4E8F0),
            line_strong: color(0xD4DAE6),
            text: color(0x1B2333),
            sub: color(0x5A6579),
            muted: color(0x8A94A8),
            accent: color(0x3B5BDB),
            accent_hover: color(0x3350C4),
            accent_active: color(0x2B45A8),
            accent_text: color(0xFFFFFF),
            accent_soft: color(0xE8EDFD),
            accent_softer: color(0xF3F6FE),
            ok: color(0x1F9D6B),
            ok_soft: color(0xE4F5ED),
            warn: color(0x9A6B1B),
            warn_soft: color(0xFFF7E8),
            bad: color(0xD6455D),
            bad_soft: color(0xFBEDEF),
            bar: color(0xFAFBFD),
            overlay: alpha(0x1B2333, 96),
            white: color(0xFFFFFF),
            dark: false,
        }
    }

    pub fn dark() -> Self {
        Self {
            bg: color(0x1B1C21),
            page: color(0x1B1C21),
            card: color(0x1B1C21),
            panel: color(0x23252C),
            hover: color(0x2C2F38),
            line: color(0x353944),
            line_strong: color(0x454A58),
            text: color(0xE8EAF0),
            sub: color(0xA3ABB8),
            muted: color(0x7B8496),
            accent: color(0x8FA3F5),
            accent_hover: color(0xA8B8F8),
            accent_active: color(0x768BE8),
            accent_text: color(0x0B1430),
            accent_soft: color(0x2A3150),
            accent_softer: color(0x232840),
            ok: color(0x5DCAA5),
            ok_soft: color(0x123D34),
            warn: color(0xEBB35F),
            warn_soft: color(0x453312),
            bad: color(0xF0959F),
            bad_soft: color(0x4A1A22),
            bar: color(0x1F2025),
            overlay: alpha(0x000000, 120),
            white: color(0xFFFFFF),
            dark: true,
        }
    }

    pub fn from_dark(dark: bool) -> Self {
        if dark {
            Self::dark()
        } else {
            Self::light()
        }
    }

    pub fn for_skin(dark: bool, skin: Skin) -> Self {
        let mut palette = Self::from_dark(dark);
        let (accent, hover, active, soft, softer, text) = match (skin, dark) {
            (Skin::Blue, false) => (0x3B5BDB, 0x3350C4, 0x2B45A8, 0xE8EDFD, 0xF3F6FE, 0xFFFFFF),
            (Skin::Blue, true) => (0x8FA3F5, 0xA8B8F8, 0x768BE8, 0x2A3150, 0x232840, 0x0B1430),
            (Skin::Emerald, false) => (0x0F6E56, 0x0B5A45, 0x094A39, 0xE1F5EE, 0xECF8F2, 0xFFFFFF),
            (Skin::Emerald, true) => (0x5DCAA5, 0x7BD9B8, 0x45B08C, 0x0B3B32, 0x0E2F27, 0x052E25),
            (Skin::Violet, false) => (0x6D4AA6, 0x5A3C8C, 0x4A3075, 0xF0EAF9, 0xF5F1FC, 0xFFFFFF),
            (Skin::Violet, true) => (0xC4A7F0, 0xD5BFF5, 0xA98BD8, 0x332A45, 0x2A2338, 0x1E1430),
            (Skin::Amber, false) => (0xA35F00, 0x8A5000, 0x703F00, 0xFFF3E0, 0xFFF8EC, 0xFFFFFF),
            (Skin::Amber, true) => (0xF2B45C, 0xF7C77D, 0xD99A3F, 0x453114, 0x3A2B12, 0x2B1A00),
            (Skin::Rose, false) => (0xA32D5A, 0x8B244C, 0x731C3E, 0xFBEAF1, 0xFDF1F5, 0xFFFFFF),
            (Skin::Rose, true) => (0xF0A0B8, 0xF5B8CB, 0xD987A1, 0x45222D, 0x3A1C26, 0x2D0E18),
            (Skin::Slate, false) => (0x3F4A5A, 0x34404F, 0x2A3440, 0xEDF0F5, 0xF1F4F8, 0xFFFFFF),
            (Skin::Slate, true) => (0xA8B8CC, 0xC0CDDD, 0x8FA2B9, 0x29313D, 0x252B34, 0x0D1420),
        };
        palette.accent = color(accent);
        palette.accent_hover = color(hover);
        palette.accent_active = color(active);
        palette.accent_soft = color(soft);
        palette.accent_softer = color(softer);
        palette.accent_text = color(text);
        palette
    }
}

/// 同步 GPUI Kit 语义主题，使 Input/Button/Switch 等内置控件与自绘界面一致。
pub fn apply(cx: &mut App, dark: bool, skin: Skin) {
    let palette = Palette::for_skin(dark, skin);
    Theme::change(
        if dark {
            ComponentThemeMode::Dark
        } else {
            ComponentThemeMode::Light
        },
        None,
        cx,
    );
    Theme::update(cx, |theme| {
        theme.font_family = ui_font_family().into();
        theme.mono_font_family = "Cascadia Mono".into();
        theme.font_size = px(ui_font_size());
        theme.mono_font_size = px((ui_font_size() - 0.5).max(9.0));
        theme.radius = px(8.0);
        theme.radius_lg = px(12.0);
        theme.focus_ring = true;

        theme.colors.background = palette.bg;
        theme.colors.foreground = palette.text;
        theme.colors.border = palette.line;
        theme.colors.input = palette.line_strong;
        theme.colors.accent = palette.accent_soft;
        theme.colors.accent_foreground = palette.accent;
        theme.colors.primary = palette.accent;
        theme.colors.primary_hover = palette.accent_hover;
        theme.colors.primary_active = palette.accent_active;
        theme.colors.primary_foreground = palette.accent_text;
        theme.colors.secondary = palette.panel;
        theme.colors.secondary_hover = palette.hover;
        theme.colors.secondary_active = palette.hover;
        theme.colors.secondary_foreground = palette.text;
        theme.colors.button = palette.bar;
        theme.colors.button_hover = palette.hover;
        theme.colors.button_active = palette.hover;
        theme.colors.button_foreground = palette.text;
        theme.colors.button_primary = palette.accent;
        theme.colors.button_primary_hover = palette.accent_hover;
        theme.colors.button_primary_active = palette.accent_active;
        theme.colors.button_primary_foreground = palette.accent_text;
        theme.colors.button_secondary = palette.bar;
        theme.colors.button_secondary_hover = palette.hover;
        theme.colors.button_secondary_active = palette.hover;
        theme.colors.button_secondary_foreground = palette.text;
        theme.colors.muted = palette.hover;
        theme.colors.muted_foreground = palette.sub;
        theme.colors.caret = palette.accent;
        theme.colors.selection = palette.accent_soft;
        theme.colors.ring = palette.accent;
        theme.colors.list = palette.bg;
        theme.colors.list_hover = palette.hover;
        theme.colors.list_active = palette.accent_soft;
        theme.colors.list_active_border = palette.accent;
        theme.colors.list_head = palette.panel;
        theme.colors.list_even = palette.bg;
        theme.colors.table = palette.bg;
        theme.colors.table_even = palette.bg;
        theme.colors.table_hover = palette.accent_softer;
        theme.colors.table_active = palette.accent_soft;
        theme.colors.table_active_border = palette.accent;
        theme.colors.table_head = palette.panel;
        theme.colors.table_head_foreground = palette.muted;
        theme.colors.table_row_border = palette.line;
        theme.colors.tab = palette.panel;
        theme.colors.tab_active = palette.bg;
        theme.colors.tab_foreground = palette.sub;
        theme.colors.tab_active_foreground = palette.text;
        theme.colors.tab_bar = palette.bar;
        theme.colors.tab_bar_segmented = palette.panel;
        theme.colors.switch = palette.line_strong;
        theme.colors.switch_thumb = palette.white;
        theme.colors.progress_bar = palette.accent;
        theme.colors.slider_bar = palette.accent;
        theme.colors.slider_thumb = palette.accent;
        theme.colors.scrollbar = palette.bg;
        theme.colors.scrollbar_thumb = palette.line;
        theme.colors.scrollbar_thumb_hover = palette.muted;
        theme.colors.popover = palette.bg;
        theme.colors.popover_foreground = palette.text;
        theme.colors.danger = palette.bad;
        theme.colors.danger_hover = palette.bad;
        theme.colors.danger_active = palette.bad;
        theme.colors.danger_foreground = palette.white;
        theme.colors.warning = palette.warn;
        theme.colors.warning_hover = palette.warn;
        theme.colors.warning_active = palette.warn;
        theme.colors.warning_foreground = palette.white;
        theme.colors.success = palette.ok;
        theme.colors.success_hover = palette.ok;
        theme.colors.success_active = palette.ok;
        theme.colors.success_foreground = palette.white;
        theme.colors.overlay = palette.overlay;
        theme.colors.title_bar = palette.bar;
        theme.colors.title_bar_border = palette.line;
        theme.colors.status_bar = palette.bar;
        theme.colors.status_bar_border = palette.line;
        theme.colors.sidebar = palette.panel;
        theme.colors.sidebar_foreground = palette.text;
        theme.colors.sidebar_accent = palette.accent_soft;
        theme.colors.sidebar_accent_foreground = palette.accent;
        theme.colors.sidebar_border = palette.line;
        theme.colors.link = palette.accent;
        theme.colors.link_hover = palette.accent_hover;
        theme.colors.link_active = palette.accent_active;
    });
}
