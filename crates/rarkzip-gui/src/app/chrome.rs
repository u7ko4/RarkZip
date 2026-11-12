use crate::app::*;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::Icon;
use gpui_kit::component::{Disableable, Sizable};

impl MainView {
    pub(crate) fn render_top_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let has_archive = self.archive.is_some();
        let running = self.running_tasks();
        let hidden_buttons = self.settings.general.hidden_toolbar_buttons.clone();
        let show_button = |key: &str| !hidden_buttons.iter().any(|hidden| hidden == key);
        let toolbar_icon_size = self.settings.view.toolbar_icon_size.clamp(14.0, 24.0);
        let toolbar_height = self.settings.view.toolbar_height.clamp(40.0, 64.0);

        let icon_button = |id: &'static str, name: IconName, tip: &'static str| {
            Button::new(id)
                .child(icon(name, palette.text, toolbar_icon_size))
                .ghost()
                .small()
                .w(px(crate::components::ICON_BUTTON_SIZE))
                .h(px(crate::components::ICON_BUTTON_HEIGHT))
                .accessibility_label(tip)
                .tooltip(tip)
        };
        let separator = || div().w(px(1.0)).h(px(22.0)).bg(palette.line).mx(px(8.0));

        let extract = Button::new("top-extract")
            .child(icon(IconName::Download, palette.accent, toolbar_icon_size))
            .small()
            .w(px(crate::components::ICON_BUTTON_SIZE))
            .h(px(crate::components::ICON_BUTTON_HEIGHT))
            .custom(
                ButtonCustomVariant::new(cx)
                    .color(palette.accent_soft)
                    .foreground(palette.accent)
                    .hover(palette.accent_soft)
                    .active(palette.accent_soft),
            )
            .tooltip("解压 (Ctrl+E)")
            .accessibility_label("解压 (Ctrl+E)");
        let extract = if has_archive {
            extract
                .on_click(cx.listener(|this, _, window, cx| this.start_extract_smart(window, cx)))
        } else {
            extract.disabled(true).opacity(0.5)
        };

        let task_icon = div()
            .relative()
            .child(
                icon_button("top-tasks", IconName::ClipboardList, "任务中心").on_click(
                    cx.listener(|this, _, _, cx| {
                        this.page = Page::Tasks;
                        cx.notify();
                    }),
                ),
            )
            .when(running > 0, |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(1.0))
                        .right(px(1.0))
                        .w(px(14.0))
                        .h(px(14.0))
                        .rounded(px(99.0))
                        .bg(palette.accent)
                        .text_color(palette.accent_text)
                        .text_size(crate::theme::font_px(9.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(running.to_string()),
                )
            });

        let left = div()
            .flex()
            .items_center()
            .gap(px(3.0))
            .h_full()
            .child(
                icon_button("top-menu", IconName::Menu, tr(self.language, "主菜单")).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.archive_menu = None;
                        if this.menu_open {
                            this.close_app_menu();
                        } else {
                            this.menu_open = true;
                            this.menu_submenu = None;
                            this.menu_hover_serial = this.menu_hover_serial.wrapping_add(1);
                        }
                        cx.notify();
                    }),
                ),
            )
            .child(separator())
            .child(
                icon_button("top-new", IconName::Plus, "新建压缩包 (Ctrl+N)").on_click(
                    cx.listener(|this, _, window, cx| this.open_new_archive_modal(window, cx)),
                ),
            )
            .child(
                icon_button("top-open", IconName::FolderOpen, "打开压缩包 (Ctrl+O)")
                    .on_click(cx.listener(|this, _, window, cx| this.open_file_dialog(window, cx))),
            )
            .child(extract)
            .when(show_button("add"), |this| {
                this.child(
                    icon_button("top-add", IconName::SquarePlus, "添加文件到压缩包")
                        .disabled(!has_archive)
                        .opacity(if has_archive { 1.0 } else { 0.5 })
                        .on_click(cx.listener(|this, _, _, cx| this.add_files_to_archive(cx))),
                )
            })
            .child(separator())
            .when(show_button("test"), |this| {
                this.child(
                    icon_button("top-test", IconName::ShieldCheck, "测试完整性 (Ctrl+T)")
                        .disabled(!has_archive)
                        .opacity(if has_archive { 1.0 } else { 0.5 })
                        .on_click(cx.listener(|this, _, window, cx| this.start_verify(window, cx))),
                )
            })
            .when(show_button("scan"), |this| {
                this.child(
                    icon_button("top-scan", IconName::ScanSearch, "AMSI 扫描恶意软件")
                        .disabled(!has_archive)
                        .opacity(if has_archive { 1.0 } else { 0.5 })
                        .on_click(cx.listener(|this, _, _, cx| this.scan_current_archive(cx))),
                )
            })
            .when(show_button("hash"), |this| {
                this.child(
                    icon_button("top-hash", IconName::Hash, "文件哈希 (Ctrl+H)")
                        .on_click(cx.listener(|this, _, _, cx| this.open_hash_modal(cx))),
                )
            })
            .when(show_button("delete"), |this| {
                this.child(
                    icon_button("top-delete", IconName::Trash, "删除选中条目 (Del)")
                        .disabled(!has_archive || self.selected.is_none())
                        .opacity(if has_archive && self.selected.is_some() {
                            1.0
                        } else {
                            0.5
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.delete_selected_entry(cx))),
                )
            })
            .child(separator())
            .child(
                icon_button("top-settings", IconName::Settings, "设置 (F5)").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.page = Page::Settings;
                        cx.notify();
                    },
                )),
            )
            .when(show_button("encoding"), |this| {
                this.child(
                    self.track_dropdown(
                        DropdownKind::TopCodepage,
                        icon_button("top-encoding", IconName::Globe, "文本编码")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_dropdown(DropdownKind::TopCodepage, cx);
                            }))
                            .into_any_element(),
                        cx,
                    ),
                )
            });

        let controls = if self.custom_titlebar {
            let max_icon = if window.is_maximized() {
                IconName::Copy
            } else {
                IconName::Square
            };
            Some(
                div()
                    .flex()
                    .items_center()
                    .h_full()
                    .child(self.window_control(
                        "win-min",
                        IconName::Minus,
                        WindowControlArea::Min,
                        false,
                        toolbar_height,
                    ))
                    .child(self.window_control(
                        "win-max",
                        max_icon,
                        WindowControlArea::Max,
                        false,
                        toolbar_height,
                    ))
                    .child(self.window_control(
                        "win-close",
                        IconName::X,
                        WindowControlArea::Close,
                        true,
                        toolbar_height,
                    ))
                    .into_any_element(),
            )
        } else {
            None
        };

        div()
            .flex()
            .items_center()
            .gap(px(3.0))
            .h(px(toolbar_height))
            .pl(px(12.0))
            .pr(px(if self.custom_titlebar { 0.0 } else { 12.0 }))
            .bg(palette.bar)
            .border_b_1()
            .border_color(palette.line)
            .child(left)
            .child({
                let drag_area = div()
                    .id("top-drag-area")
                    .flex_1()
                    .h_full()
                    .when(self.custom_titlebar, |this| {
                        this.window_control_area(WindowControlArea::Drag)
                    })
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            this.open_context_menu(
                                ContextTarget::Titlebar,
                                None,
                                event.position.x.as_f32(),
                                event.position.y.as_f32(),
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    );
                // X11/Wayland 不处理 WindowControlArea 的命中测试，直接调用
                // 平台的移动/最大化接口。
                #[cfg(not(target_os = "windows"))]
                let drag_area = drag_area
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_move();
                    })
                    .on_double_click(|_, window, _| window.zoom_window());
                drag_area
            })
            .child(task_icon)
            .child(
                icon_button("top-theme", IconName::Contrast, "切换浅色/暗色主题")
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_theme(window, cx))),
            )
            .when_some(controls, |this, controls| this.child(controls))
            .into_any_element()
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

    /// 独立功能窗口的自绘标题栏（仅在“使用自定义标题栏”开启时渲染）。
    pub(crate) fn render_feature_titlebar(
        &mut self,
        feature: FeatureWindow,
        window: &Window,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
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
            .child({
                let drag_area = div()
                    .id("feature-drag-area")
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag)
                    .child(icon(
                        feature.icon(),
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
                            .child(feature.title()),
                    );
                #[cfg(not(target_os = "windows"))]
                let drag_area = drag_area
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_move();
                    })
                    .on_double_click(|_, window, _| window.zoom_window());
                drag_area
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .h_full()
                    .child(self.window_control(
                        "feature-win-min",
                        IconName::Minus,
                        WindowControlArea::Min,
                        false,
                        crate::theme::TITLEBAR_HEIGHT,
                    ))
                    .child(self.window_control(
                        "feature-win-max",
                        max_icon,
                        WindowControlArea::Max,
                        false,
                        crate::theme::TITLEBAR_HEIGHT,
                    ))
                    .child(self.window_control(
                        "feature-win-close",
                        IconName::X,
                        WindowControlArea::Close,
                        true,
                        crate::theme::TITLEBAR_HEIGHT,
                    )),
            )
            .into_any_element()
    }

    pub(crate) fn render_address_bar(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let current = self.current_dir.clone();
        let segments: Vec<String> = current
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(ToString::to_string)
            .collect();

        let mut crumb = div()
            .flex()
            .items_center()
            .gap(px(5.0))
            .flex_1()
            .min_w_0()
            .h(px(crate::components::CONTROL_HEIGHT))
            .px(px(12.0))
            .rounded(px(8.0))
            .bg(palette.panel)
            .border_1()
            .border_color(palette.line)
            .overflow_hidden()
            .child(icon(IconName::Folder, palette.sub, 16.0));

        if let Some(archive) = &self.archive {
            let archive_name = archive
                .path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "归档".to_string());
            crumb = crumb.child(
                div()
                    .id("crumb-root")
                    .text_size(crate::theme::font_px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(palette.accent)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.navigate_to(String::new(), cx);
                    }))
                    .child(archive_name),
            );
            for (index, segment) in segments.iter().enumerate() {
                let target = segments[..=index].join("/");
                crumb = crumb.child(div().text_color(palette.sub).child("▸"));
                crumb = crumb.child(
                    div()
                        .id(format!("crumb-{index}"))
                        .text_size(crate::theme::font_px(13.0))
                        .text_color(palette.text)
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.navigate_to(target.clone(), cx);
                        }))
                        .child(segment.clone()),
                );
            }
            crumb = crumb.child(div().text_color(palette.sub).child("▸"));
        } else {
            crumb = crumb.child(
                div()
                    .text_size(crate::theme::font_px(13.0))
                    .text_color(palette.sub)
                    .child(tr(self.language, "未打开归档")),
            );
        }

        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .h(px(48.0))
            .px(px(10.0))
            .font_family(crate::theme::ui_font_family())
            .bg(palette.bg)
            .border_b_1()
            .border_color(palette.line)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.open_context_menu(
                        ContextTarget::AddressBar,
                        None,
                        event.position.x.as_f32(),
                        event.position.y.as_f32(),
                        cx,
                    );
                    cx.stop_propagation();
                }),
            )
            .child(
                Button::new("addr-up")
                    .child(icon(IconName::ArrowUpToLine, palette.sub, 18.0))
                    .ghost()
                    .small()
                    .w(px(crate::components::ICON_BUTTON_SIZE))
                    .h(px(crate::components::CONTROL_HEIGHT))
                    .tooltip("上级目录 (Backspace)")
                    .disabled(current.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| this.go_up(cx))),
            )
            .child(crumb)
            .child(
                div().w(px(320.0)).flex_shrink_0().child(
                    crate::components::control_height(Input::new(&self.search))
                        .text_size(crate::theme::font_px(13.5))
                        .cleanable(true)
                        .prefix(icon(IconName::Search, palette.sub, 16.0)),
                ),
            )
            .into_any_element()
    }

    pub(crate) fn render_main(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.archive.is_none() {
            return self.render_start_page(cx);
        }
        let palette = self.palette;
        let center = match self.file_view {
            FileViewMode::List => {
                let table_width = self.table_width();
                let list_header = self.render_list_header(cx);
                let entry_list = self.render_entry_list(window, cx);
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .child(self.render_file_toolbar(window, cx))
                    .child(
                        div()
                            .id("table-hscroll")
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .min_w_0()
                            .overflow_x_scroll()
                            // 仅横向容器：垂直滚轮交给内部虚拟列表，避免被本容器
                            // 转换为横向滚动（此前滚轮会左右移动文件列表）。
                            .restrict_scroll_to_axis()
                            .track_scroll(&self.table_scroll)
                            // 滚轮/Shift+滚轮改变偏移后通知底部自定义滚动条同步。
                            .on_scroll(cx.listener(|_, _: &ScrollEvent, _, cx| cx.notify()))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                    this.open_archive_context_menu(
                                        None,
                                        event.position.x.as_f32(),
                                        event.position.y.as_f32(),
                                        cx,
                                    );
                                    cx.stop_propagation();
                                }),
                            )
                            .child(
                                div()
                                    .w(px(table_width))
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_h_0()
                                    .child(list_header)
                                    .child(entry_list),
                            ),
                    )
                    .child(self.render_horizontal_scrollbar(&palette, cx))
                    .child(self.render_comment_panel(cx))
                    .into_any_element()
            }
            FileViewMode::Icons => div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .child(self.render_file_toolbar(window, cx))
                .child(self.render_icon_grid(window, cx))
                .child(self.render_comment_panel(cx))
                .into_any_element(),
        };
        div()
            .flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .min_w_0()
            .child(self.render_tree_panel(cx))
            .child(center)
            .child(self.render_preview_panel(cx))
            .into_any_element()
    }
}
