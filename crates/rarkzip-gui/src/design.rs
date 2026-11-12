//! 界面重设计（Vol.1 / Vol.2）共用的视觉组件。
//!
//! 设计稿使用浅色卡片、12px 圆角、靛蓝强调色与 30px 页面留白。
//! 这里的辅助函数只负责外观与交互语义，业务状态仍由 `MainView` 持有。

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Disableable;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::{icon, IconName, CONTROL_HEIGHT};
use crate::theme::Palette;

/// 功能页内容区的最大宽度，与设计稿窗口一致。
pub const PAGE_MAX_WIDTH: f32 = 1100.0;
/// 功能页左右留白。
pub const PAGE_PAD_X: f32 = 30.0;

type ClickCallback = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

fn handler(
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    on_click
}

/// 白色卡片：1px 边框 + 12px 圆角 + 极轻投影。
pub fn card(palette: &Palette) -> Div {
    div()
        .rounded(px(12.0))
        .border_1()
        .border_color(palette.line)
        .bg(palette.card)
        .shadow_sm()
}

/// 带内边距的卡片。
pub fn panel(palette: &Palette) -> Div {
    card(palette).px(px(22.0)).py(px(20.0))
}

pub fn field_label(palette: &Palette, text: impl Into<SharedString>) -> Div {
    div()
        .mb(px(7.0))
        .text_size(crate::theme::font_px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(palette.sub)
        .child(text.into())
}

pub fn done_button(
    palette: &Palette,
    onclick: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    Button::new("page-done")
        .label("完成")
        .icon(icon(IconName::Check, palette.accent_text, 15.0))
        .primary()
        .h(px(CONTROL_HEIGHT))
        .px(px(16.0))
        .text_size(crate::theme::font_px(13.5))
        .on_click(onclick)
        .into_any_element()
}

pub fn secondary_action(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    onclick: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .label(label)
        .outline()
        .h(px(CONTROL_HEIGHT))
        .px(px(16.0))
        .text_size(crate::theme::font_px(13.5))
        .on_click(onclick)
}

pub fn primary_icon_action(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    name: IconName,
    palette: &Palette,
    disabled: bool,
    onclick: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .label(label)
        .icon(icon(name, palette.accent_text, 15.0))
        .primary()
        .h(px(CONTROL_HEIGHT))
        .px(px(16.0))
        .text_size(crate::theme::font_px(13.5))
        .disabled(disabled)
        .opacity(if disabled { 0.45 } else { 1.0 })
        .on_click(onclick)
}

pub fn secondary_icon_action(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    name: IconName,
    color: Hsla,
    onclick: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .label(label)
        .icon(icon(name, color, 15.0))
        .outline()
        .h(px(CONTROL_HEIGHT))
        .px(px(16.0))
        .text_size(crate::theme::font_px(13.5))
        .on_click(onclick)
}

pub fn link_action(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    palette: &Palette,
    onclick: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let label = label.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .h(px(26.0))
        .px(px(8.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .cursor_pointer()
        .text_size(crate::theme::font_px(12.5))
        .text_color(palette.accent)
        .hover(|style| style.bg(palette.accent_soft))
        .on_click(onclick)
        .child(label)
        .into_any_element()
}

/// 28px 无边框图标按钮（表格操作列、任务卡片操作）。
pub fn icon_action(
    palette: &Palette,
    id: impl Into<ElementId>,
    name: IconName,
    tint: Hsla,
    disabled: bool,
    tip: &'static str,
    onclick: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let mut element = div()
        .id(id)
        .role(Role::Button)
        .aria_label(tip)
        .w(px(28.0))
        .h(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.0))
        .when(disabled, |this| this.opacity(0.3))
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(palette.accent_soft))
                .on_click(handler(onclick))
        });
    element = element.child(icon(name, tint, 15.0));
    element.into_any_element()
}

/// 危险操作图标按钮：悬停时使用浅红底。
pub fn danger_icon_action(
    palette: &Palette,
    id: impl Into<ElementId>,
    name: IconName,
    disabled: bool,
    tip: &'static str,
    onclick: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let mut element = div()
        .id(id)
        .role(Role::Button)
        .aria_label(tip)
        .w(px(28.0))
        .h(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.0))
        .when(disabled, |this| this.opacity(0.3))
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(palette.bad_soft))
                .on_click(handler(onclick))
        });
    element = element.child(icon(name, palette.sub, 15.0));
    element.into_any_element()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagTone {
    Neutral,
    Accent,
    Manual,
    Ok,
    Danger,
}

pub fn tag(palette: &Palette, text: impl Into<SharedString>, tone: TagTone) -> AnyElement {
    let (fg, bg) = match tone {
        TagTone::Neutral => (palette.sub, palette.panel),
        TagTone::Accent => (palette.accent, palette.accent_soft),
        TagTone::Manual => (palette.warn, palette.warn_soft),
        TagTone::Ok => (palette.ok, palette.ok_soft),
        TagTone::Danger => (palette.bad, palette.bad_soft),
    };
    div()
        .h(px(22.0))
        .px(px(10.0))
        .rounded(px(99.0))
        .flex()
        .items_center()
        .border_1()
        .border_color(bg)
        .bg(bg)
        .text_size(crate::theme::font_px(12.0))
        .text_color(fg)
        .child(text.into())
        .into_any_element()
}

/// 圆角胶囊徽标（标题旁的计数、状态计数）。
pub fn pill(palette: &Palette, text: impl Into<SharedString>, tone: TagTone) -> AnyElement {
    let (fg, bg) = match tone {
        TagTone::Neutral => (palette.sub, palette.panel),
        TagTone::Accent => (palette.accent, palette.accent_soft),
        TagTone::Manual => (palette.warn, palette.warn_soft),
        TagTone::Ok => (palette.ok, palette.ok_soft),
        TagTone::Danger => (palette.bad, palette.bad_soft),
    };
    div()
        .h(px(24.0))
        .px(px(11.0))
        .rounded(px(99.0))
        .flex()
        .items_center()
        .gap(px(6.0))
        .bg(bg)
        .text_size(crate::theme::font_px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .child(text.into())
        .into_any_element()
}

pub fn page_head(
    palette: &Palette,
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    trailing: AnyElement,
) -> Div {
    div()
        .flex()
        .items_start()
        .justify_between()
        .gap(px(16.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(5.0))
                .min_w_0()
                .child(
                    div()
                        .text_size(crate::theme::font_px(19.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.text)
                        .child(title.into()),
                )
                .child(
                    div()
                        .text_size(crate::theme::font_px(13.0))
                        .text_color(palette.sub)
                        .child(subtitle.into()),
                ),
        )
        .child(div().flex_none().child(trailing))
}

/// 功能页整页框架：标题行 + 可滚动内容 + 固定在底部的操作区。
pub fn page_shell(
    palette: &Palette,
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    trailing: AnyElement,
    body: Vec<AnyElement>,
    footer: Option<AnyElement>,
) -> AnyElement {
    page_shell_inner(
        palette,
        Some(page_head(palette, title, subtitle, trailing)),
        body,
        footer,
    )
}

/// 无页头版本：独立窗口已有标题栏，内容直接从页面主体开始。
pub fn page_shell_bare(
    palette: &Palette,
    body: Vec<AnyElement>,
    footer: Option<AnyElement>,
) -> AnyElement {
    page_shell_inner(palette, None, body, footer)
}

fn page_shell_inner(
    palette: &Palette,
    head: Option<Div>,
    body: Vec<AnyElement>,
    footer: Option<AnyElement>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .size_full()
        .bg(palette.page)
        .child(
            div()
                .id("page-scroll")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(
                    div().w_full().flex().justify_center().child(
                        div()
                            .w_full()
                            .max_w(px(PAGE_MAX_WIDTH))
                            .px(px(PAGE_PAD_X))
                            .pt(px(20.0))
                            .pb(px(20.0))
                            .flex()
                            .flex_col()
                            .gap(px(14.0))
                            .children(head)
                            .children(body),
                    ),
                ),
        )
        .when_some(footer, |this, footer| {
            this.child(
                div()
                    .flex_none()
                    .w_full()
                    .flex()
                    .justify_center()
                    .bg(palette.bar)
                    .border_t_1()
                    .border_color(palette.line)
                    .child(
                        div()
                            .w_full()
                            .max_w(px(PAGE_MAX_WIDTH))
                            .px(px(PAGE_PAD_X))
                            .pt(px(10.0))
                            .pb(px(12.0))
                            .child(footer),
                    ),
            )
        })
        .into_any_element()
}

pub fn info_note(palette: &Palette, text: impl Into<SharedString>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .text_size(crate::theme::font_px(12.5))
        .text_color(palette.muted)
        .child(icon(IconName::Info, palette.muted, 14.0))
        .child(div().flex_1().min_w_0().child(text.into()))
        .into_any_element()
}

/// 26px 圆形步骤点 + 标签。
pub fn step_dot(
    palette: &Palette,
    index: usize,
    label: &'static str,
    state: StepState,
) -> AnyElement {
    let (dot_fg, dot_bg, dot_border, label_color) = match state {
        StepState::Active => (
            palette.accent_text,
            palette.accent,
            palette.accent_soft,
            palette.text,
        ),
        StepState::Done => (
            palette.accent,
            palette.accent_soft,
            palette.accent_soft,
            palette.sub,
        ),
        StepState::Pending => (palette.muted, palette.panel, palette.line, palette.muted),
    };
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(
            div()
                .w(px(26.0))
                .h(px(26.0))
                .rounded(px(99.0))
                .flex()
                .items_center()
                .justify_center()
                .bg(dot_bg)
                .border(px(1.0))
                .border_color(dot_border)
                .text_size(crate::theme::font_px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(dot_fg)
                .child(index.to_string()),
        )
        .child(
            div()
                .text_size(crate::theme::font_px(13.0))
                .when(state == StepState::Active, |this| {
                    this.font_weight(FontWeight::SEMIBOLD)
                })
                .text_color(label_color)
                .child(label),
        )
        .into_any_element()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Active,
    Done,
    Pending,
}

pub fn step_line(palette: &Palette) -> Div {
    div()
        .w(px(44.0))
        .h(px(1.5))
        .flex_none()
        .mx(px(12.0))
        .bg(palette.line_strong)
}

/// 自定义开关；整块可点击，带无障碍 Switch 语义。
pub fn switch(
    palette: &Palette,
    id: impl Into<ElementId>,
    checked: bool,
    onclick: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(
            div()
                .id(id)
                .role(Role::Switch)
                .aria_label("显示密码")
                .aria_toggled(if checked {
                    Toggled::True
                } else {
                    Toggled::False
                })
                .w(px(36.0))
                .h(px(20.0))
                .rounded(px(99.0))
                .relative()
                .cursor_pointer()
                .bg(if checked {
                    palette.accent
                } else {
                    palette.line_strong
                })
                .on_click(handler(onclick))
                .child(
                    div()
                        .absolute()
                        .top(px(2.0))
                        .left(px(if checked { 18.0 } else { 2.0 }))
                        .w(px(16.0))
                        .h(px(16.0))
                        .rounded(px(99.0))
                        .bg(palette.white)
                        .shadow_sm(),
                ),
        )
        .child(
            div()
                .text_size(crate::theme::font_px(13.0))
                .text_color(palette.sub)
                .child("显示密码"),
        )
        .into_any_element()
}

/// 18px 方形勾选框。
pub fn check_box(
    palette: &Palette,
    id: impl Into<ElementId>,
    checked: bool,
    disabled: bool,
    aria: impl Into<SharedString>,
    onclick: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let aria = aria.into();
    let mut element = div()
        .id(id)
        .role(Role::CheckBox)
        .aria_label(aria)
        .aria_toggled(if checked {
            Toggled::True
        } else {
            Toggled::False
        })
        .w(px(18.0))
        .h(px(18.0))
        .flex_none()
        .rounded(px(5.5))
        .border(px(1.5))
        .border_color(if checked {
            palette.accent
        } else {
            palette.line_strong
        })
        .bg(if checked { palette.accent } else { palette.bg })
        .flex()
        .items_center()
        .justify_center()
        .when(disabled, |this| this.opacity(0.55))
        .when(!disabled, |this| {
            this.cursor_pointer().on_click(handler(onclick))
        });
    if checked {
        element = element.child(icon(IconName::Check, palette.white, 11.0));
    }
    element.into_any_element()
}

/// 仅供整行可点击场景使用的复选方框（本身不接收点击）。
pub fn check_box_visual(palette: &Palette, checked: bool, disabled: bool) -> AnyElement {
    let mut element = div()
        .w(px(18.0))
        .h(px(18.0))
        .flex_none()
        .rounded(px(5.5))
        .border(px(1.5))
        .border_color(if checked {
            palette.accent
        } else {
            palette.line_strong
        })
        .bg(if checked { palette.accent } else { palette.bg })
        .flex()
        .items_center()
        .justify_center()
        .when(disabled, |this| this.opacity(0.55));
    if checked {
        element = element.child(icon(IconName::Check, palette.white, 11.0));
    }
    element.into_any_element()
}

/// 整块可点击的复选标签。
pub fn check_chip(
    palette: &Palette,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    disabled: bool,
    onclick: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let label = label.into();
    let mut element = div()
        .id(id)
        .role(Role::CheckBox)
        .aria_label(label.clone())
        .aria_toggled(if checked {
            Toggled::True
        } else {
            Toggled::False
        })
        .h(px(34.0))
        .px(px(14.0))
        .rounded(px(9.0))
        .border_1()
        .border_color(if checked {
            palette.accent
        } else {
            palette.line
        })
        .bg(if checked {
            palette.accent_softer
        } else {
            palette.bg
        })
        .flex()
        .items_center()
        .gap(px(9.0))
        .text_size(crate::theme::font_px(13.0))
        .text_color(if checked { palette.text } else { palette.sub })
        .when(disabled, |this| this.opacity(0.5))
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.border_color(palette.accent).text_color(palette.text))
                .on_click(handler(onclick))
        })
        .child(
            div()
                .w(px(17.0))
                .h(px(17.0))
                .flex_none()
                .rounded(px(5.0))
                .border(px(1.5))
                .border_color(if checked {
                    palette.accent
                } else {
                    palette.line_strong
                })
                .bg(if checked { palette.accent } else { palette.bg })
                .flex()
                .items_center()
                .justify_center()
                .child(if checked {
                    icon(IconName::Check, palette.white, 11.0).into_any_element()
                } else {
                    div().into_any_element()
                }),
        )
        .child(label);
    if disabled {
        element = element.cursor_default();
    }
    element.into_any_element()
}

/// 数字步进器：−  value  ＋。
pub fn stepper(
    palette: &Palette,
    id: &'static str,
    value: impl Into<SharedString>,
    on_minus: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_plus: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let step_button = |button_id: String, label: &'static str, onclick: ClickCallback| {
        div()
            .id(button_id)
            .role(Role::Button)
            .aria_label(label)
            .w(px(32.0))
            .h(px(34.0))
            .flex()
            .items_center()
            .justify_center()
            .bg(palette.panel)
            .cursor_pointer()
            .text_size(crate::theme::font_px(16.0))
            .text_color(palette.sub)
            .hover(|style| style.bg(palette.accent_soft).text_color(palette.accent))
            .on_click(move |event, window, cx| onclick(event, window, cx))
            .child(label)
    };
    div()
        .flex()
        .items_center()
        .rounded(px(9.0))
        .border_1()
        .border_color(palette.line_strong)
        .bg(palette.card)
        .overflow_hidden()
        .child(step_button(format!("{id}-minus"), "−", Box::new(on_minus)))
        .child(
            div()
                .w(px(52.0))
                .h(px(34.0))
                .flex()
                .items_center()
                .justify_center()
                .text_size(crate::theme::font_px(14.0))
                .font_weight(FontWeight::NORMAL)
                .text_color(palette.text)
                .child(value.into()),
        )
        .child(step_button(format!("{id}-plus"), "＋", Box::new(on_plus)))
        .into_any_element()
}

/// 虚线拖放区。
pub fn dropzone(
    palette: &Palette,
    icon_name: IconName,
    icon_size: f32,
    main: impl Into<SharedString>,
    sub: impl Into<SharedString>,
    actions: Vec<AnyElement>,
    min_height: f32,
) -> AnyElement {
    let sub = sub.into();
    div()
        .w_full()
        .min_h(px(min_height))
        .rounded(px(12.0))
        .border(px(1.5))
        .border_dashed()
        .border_color(palette.line_strong)
        .bg(palette.panel)
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .py(px(18.0))
        .child(icon(icon_name, palette.muted, icon_size))
        .child(
            div()
                .text_size(crate::theme::font_px(14.0))
                .text_color(palette.sub)
                .child(main.into()),
        )
        .when(!sub.is_empty(), |this| {
            this.child(
                div()
                    .text_size(crate::theme::font_px(12.5))
                    .text_color(palette.muted)
                    .child(sub),
            )
        })
        .when(!actions.is_empty(), |this| {
            this.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .mt(px(6.0))
                    .children(actions),
            )
        })
        .into_any_element()
}

pub fn panel_title(palette: &Palette, icon_name: IconName, text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .text_size(crate::theme::font_px(13.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(palette.text)
        .child(icon(icon_name, palette.accent, 16.0))
        .child(text.into())
}

/// 列设置 / 工具栏按钮弹窗共用的行容器。
pub fn row_list(palette: &Palette) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .rounded(px(11.0))
        .border_1()
        .border_color(palette.line)
        .overflow_hidden()
}

pub fn row_list_row(palette: &Palette, last: bool, fixed: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .px(px(16.0))
        .py(px(10.0))
        .min_h(px(44.0))
        .when(fixed, |this| this.bg(palette.panel))
        .when(!last, |this| this.border_b_1().border_color(palette.line))
        .when(!fixed, |this| {
            this.hover(|style| style.bg(palette.accent_softer))
        })
}

pub fn fixed_tag(palette: &Palette, text: impl Into<SharedString>) -> AnyElement {
    div()
        .h(px(20.0))
        .px(px(8.0))
        .rounded(px(99.0))
        .border_1()
        .border_color(palette.line)
        .flex()
        .items_center()
        .text_size(crate::theme::font_px(11.0))
        .text_color(palette.muted)
        .child(text.into())
        .into_any_element()
}

/// 设置项卡片。
pub fn setting_card(palette: &Palette) -> Div {
    card(palette).px(px(20.0)).py(px(6.0))
}

/// 带标题 + 描述的开关行。
pub fn option_row(
    palette: &Palette,
    last: bool,
    leading: AnyElement,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    tail: Option<AnyElement>,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .py(px(13.0))
        .when(!last, |this| this.border_b_1().border_color(palette.line))
        .child(leading)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(crate::theme::font_px(13.5))
                        .text_color(palette.text)
                        .child(title.into()),
                )
                .child(
                    div()
                        .text_size(crate::theme::font_px(12.5))
                        .text_color(palette.muted)
                        .child(description.into()),
                ),
        )
        .when_some(tail, |this, tail| {
            this.child(div().flex_shrink_0().child(tail))
        })
        .into_any_element()
}

/// 设置项卡片中的字段行。
pub fn field_row(
    palette: &Palette,
    last: bool,
    label: impl Into<SharedString>,
    control: AnyElement,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(14.0))
        .py(px(14.0))
        .when(!last, |this| this.border_b_1().border_color(palette.line))
        .child(
            div()
                .w(px(120.0))
                .flex_none()
                .text_size(crate::theme::font_px(13.5))
                .text_color(palette.sub)
                .child(label.into()),
        )
        .child(control)
        .into_any_element()
}

/// 弹窗框架：白色卡片、标题区、内容区、底部按钮。
/// 普通表单内容由整个内容区滚动。
#[allow(clippy::too_many_arguments)]
pub fn modal_shell(
    palette: &Palette,
    width: f32,
    max_height: f32,
    title: impl Into<SharedString>,
    subtitle: Option<String>,
    hint: Option<String>,
    body: Vec<AnyElement>,
    footer: Vec<AnyElement>,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    modal_shell_impl(
        palette, width, max_height, title, subtitle, hint, body, footer, false, on_close,
    )
}

/// 列表型弹窗：内容区不滚动，列表自身填满可用高度并滚动，
/// 外框、表头与底部按钮保持不动。
#[allow(clippy::too_many_arguments)]
pub fn modal_shell_fill(
    palette: &Palette,
    width: f32,
    max_height: f32,
    title: impl Into<SharedString>,
    subtitle: Option<String>,
    hint: Option<String>,
    body: Vec<AnyElement>,
    footer: Vec<AnyElement>,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    modal_shell_impl(
        palette, width, max_height, title, subtitle, hint, body, footer, true, on_close,
    )
}

#[allow(clippy::too_many_arguments)]
fn modal_shell_impl(
    palette: &Palette,
    width: f32,
    max_height: f32,
    title: impl Into<SharedString>,
    subtitle: Option<String>,
    hint: Option<String>,
    body: Vec<AnyElement>,
    footer: Vec<AnyElement>,
    // true：内容自身是填满弹窗的滚动列表（只滚列表内部，外框/表头保持不动）；
    // false：普通表单内容，由整个内容区滚动。
    body_fill: bool,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let card = div().w(px(width));
    // 列表型弹窗使用固定可用高度，让列表自身填满并滚动；表单弹窗按内容收缩。
    let card = if body_fill {
        card.h(px(max_height))
    } else {
        card.max_h(px(max_height))
    };
    card.rounded(px(14.0))
        .bg(palette.bg)
        .shadow_lg()
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(
            div()
                .relative()
                .flex()
                .items_start()
                .gap(px(14.0))
                .px(px(26.0))
                .pt(px(22.0))
                .pb(px(2.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(crate::theme::font_px(17.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(palette.text)
                                .child(title.into()),
                        )
                        .when_some(subtitle, |this, subtitle| {
                            this.child(
                                div()
                                    .text_size(crate::theme::font_px(13.0))
                                    .text_color(palette.sub)
                                    .child(subtitle),
                            )
                        })
                        .when_some(hint, |this, hint| {
                            this.child(
                                div()
                                    .text_size(crate::theme::font_px(12.5))
                                    .text_color(palette.muted)
                                    .child(hint),
                            )
                        }),
                )
                .child(
                    div()
                        .id("modal-close")
                        .role(Role::Button)
                        .aria_label("关闭")
                        .w(px(30.0))
                        .h(px(30.0))
                        .flex_none()
                        .rounded(px(8.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|style| style.bg(palette.bad_soft))
                        .on_click(on_close)
                        .child(icon(IconName::X, palette.muted, 16.0)),
                ),
        )
        .child(if body_fill {
            // 列表型弹窗：内容区不滚动，由列表自身滚动，外框与表头保持不动。
            div()
                .id("modal-body")
                .flex()
                .flex_col()
                .gap(px(12.0))
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .px(px(26.0))
                .py(px(18.0))
                .children(body)
        } else {
            // 普通表单：整块内容区滚动；子项保持自身高度，避免列表被压缩后裁掉。
            div()
                .id("modal-body")
                .flex()
                .flex_col()
                .gap(px(12.0))
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(26.0))
                .py(px(18.0))
                .children(
                    body.into_iter()
                        .map(|child| div().w_full().flex_shrink_0().child(child)),
                )
        })
        .child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(10.0))
                .px(px(26.0))
                .py(px(14.0))
                .border_t_1()
                .border_color(palette.line)
                .bg(palette.panel)
                // 卡片使用 overflow_hidden 做矩形裁剪，页脚背景会盖住底部
                // 圆角，因此页脚自身也要按相同半径裁出底部圆角。
                .rounded_b(px(14.0))
                .children(footer),
        )
        .into_any_element()
}

/// 与设置页一致的可滚动内容区。
pub fn settings_content(
    palette: &Palette,
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    trailing: AnyElement,
    body: AnyElement,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(palette.page)
        .child(
            div()
                .id("settings-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(
                    div()
                        .w_full()
                        .px(px(30.0))
                        .pt(px(24.0))
                        .pb(px(28.0))
                        .flex()
                        .flex_col()
                        .gap(px(14.0))
                        .child(
                            div()
                                .w_full()
                                .max_w(px(900.0))
                                .flex()
                                .flex_col()
                                .gap(px(14.0))
                                .child(page_head(palette, title, subtitle, trailing))
                                .child(body),
                        ),
                ),
        )
        .into_any_element()
}

/// 设置页的“可滚动主体 + 固定底部操作栏”布局。
pub fn settings_content_with_footer(
    palette: &Palette,
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    trailing: AnyElement,
    body: AnyElement,
    footer: AnyElement,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(palette.page)
        .child(
            div()
                .id("settings-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(
                    div()
                        .w_full()
                        .px(px(30.0))
                        .pt(px(24.0))
                        .pb(px(20.0))
                        .flex()
                        .flex_col()
                        .gap(px(14.0))
                        .child(
                            div()
                                .w_full()
                                .max_w(px(900.0))
                                .flex()
                                .flex_col()
                                .gap(px(14.0))
                                .child(page_head(palette, title, subtitle, trailing))
                                .child(body),
                        ),
                ),
        )
        .child(
            div()
                .flex_none()
                .w_full()
                .bg(palette.bar)
                .border_t_1()
                .border_color(palette.line)
                .child(
                    div()
                        .w_full()
                        .max_w(px(900.0))
                        .px(px(30.0))
                        .py(px(12.0))
                        .child(footer),
                ),
        )
        .into_any_element()
}

/// 自绘窗口内容的圆角半径。
pub const WINDOW_CORNER_RADIUS: f32 = 12.0;

/// 客户端装饰窗口的内容外壳。
///
/// `gpui-component` 的 Root 插件已经负责阴影、边框、缩放热区和透明留白
/// （`window_border`），这里只需给内容本身裁出圆角；服务端装饰时原样返回。
pub fn window_frame(decorations: Decorations, inner: AnyElement) -> AnyElement {
    let Decorations::Client { tiling } = decorations else {
        return inner;
    };
    let rounding = px(WINDOW_CORNER_RADIUS);

    div()
        .size_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .when(!(tiling.top || tiling.right), |this| {
            this.rounded_tr(rounding)
        })
        .when(!(tiling.top || tiling.left), |this| {
            this.rounded_tl(rounding)
        })
        .when(!(tiling.bottom || tiling.right), |this| {
            this.rounded_br(rounding)
        })
        .when(!(tiling.bottom || tiling.left), |this| {
            this.rounded_bl(rounding)
        })
        .child(inner)
        .into_any_element()
}
