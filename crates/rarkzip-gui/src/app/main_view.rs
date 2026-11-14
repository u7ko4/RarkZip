use std::path::Path;

use gpui_kit::component::slider::Slider;
use gpui_kit::component::tooltip::Tooltip;

use crate::app::*;

/// 物理左键是否仍按下。用于窗口外松开鼠标后清理拖拽状态：
/// GPUI 的 `on_mouse_up_out` 只对元素内的抬起可靠，窗口外抬起收不到事件。
fn left_mouse_button_down() -> bool {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn GetAsyncKeyState(vkey: i32) -> i16;
        }
        const VK_LBUTTON: i32 = 0x01;
        // 最高位表示当前是否按下；最低位是自上次查询以来是否按下过，需忽略。
        (unsafe { GetAsyncKeyState(VK_LBUTTON) } as u16 & 0x8000) != 0
    }
    #[cfg(not(windows))]
    {
        true
    }
}

impl MainView {
    pub(crate) fn render_tree_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        if !self.settings.view.show_tree {
            return div().into_any_element();
        }
        let Some(archive) = &self.archive else {
            return div().into_any_element();
        };
        let root_name = archive
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        let tree = archive.tree.clone();
        let current = self.current_dir.clone();
        let expanded = self.expanded.clone();

        let mut children =
            vec![self.tree_node("", &root_name, 0, current.is_empty(), true, true, cx)];
        let mut keys: Vec<String> = tree.children.keys().cloned().collect();
        keys.sort();
        for key in keys {
            if let Some(child) = tree.children.get(&key) {
                push_tree_rows(child, &mut children, 1, &current, &expanded, &palette, cx);
            }
        }

        div()
            .w(px(200.0))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .font_family(crate::theme::ui_font_family())
            .bg(palette.page)
            .border_r_1()
            .border_color(palette.line)
            .child(
                div()
                    .id("tree-scroll")
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .p(px(8.0))
                    .flex_1()
                    .overflow_y_scroll()
                    .children(children),
            )
            .when(self.archive.is_some(), |this| {
                this.child(
                    div()
                        .mx(px(10.0))
                        .mb(px(10.0))
                        .p(px(8.0))
                        .rounded(px(7.0))
                        .border_1()
                        .border_dashed()
                        .border_color(palette.line)
                        .text_size(crate::theme::font_px(12.0))
                        .text_color(palette.sub)
                        .flex()
                        .justify_center()
                        .child(self.localized("拖放文件到此处添加")),
                )
            })
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn tree_node(
        &mut self,
        path: &str,
        label: &str,
        depth: usize,
        selected: bool,
        expanded: bool,
        has_children: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let path_owned = path.to_string();
        div()
            .id(format!("tree-{path}"))
            .flex()
            .items_center()
            .gap(px(7.0))
            .h(px(32.0))
            .pl(px(8.0 + depth as f32 * 15.0))
            .pr(px(8.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .text_size(crate::theme::font_px(13.0))
            .when(selected, |this| {
                this.bg(palette.accent_soft).text_color(palette.accent)
            })
            .when(!selected, |this| {
                this.text_color(palette.text)
                    .hover(|style| style.bg(palette.hover))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.navigate_to(path_owned.clone(), cx);
                if this.expanded.contains(&path_owned) && !path_owned.is_empty() {
                    this.expanded.remove(&path_owned);
                } else {
                    this.expanded.insert(path_owned.clone());
                }
                cx.notify();
            }))
            .child(
                div()
                    .w(px(12.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(has_children, |this| {
                        this.child(icon(
                            if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            },
                            palette.sub,
                            13.0,
                        ))
                    }),
            )
            .child(icon(
                IconName::Folder,
                if selected {
                    palette.accent
                } else {
                    palette.sub
                },
                14.0,
            ))
            .child(div().flex_1().min_w_0().truncate().child(label.to_string()))
            .into_any_element()
    }

    pub(crate) fn render_file_toolbar(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let panel_width = usize::from(self.settings.view.show_tree) as f32 * 200.0
            + usize::from(self.settings.view.preview_enabled) as f32 * 200.0;
        let available = window.viewport_size().width.as_f32() - panel_width - 20.0;
        let compact = available < 620.0;

        let nav_active = self.settings.view.show_tree;
        let preview_active = self.settings.view.preview_enabled;
        let comment_active = self.settings.view.show_comment;
        let nav = self.file_toolbar_toggle(
            "file-toolbar-nav",
            self.localized("导航"),
            IconName::ListTree,
            nav_active,
            |this, cx| this.toggle_tree_panel(cx),
            cx,
        );
        let preview = self.file_toolbar_toggle(
            "file-toolbar-preview",
            self.localized("预览"),
            IconName::Image,
            preview_active,
            |this, cx| this.toggle_preview_panel(cx),
            cx,
        );
        let comment = self.file_toolbar_toggle(
            "file-toolbar-comment",
            self.localized("注释"),
            IconName::MessageSquareText,
            comment_active,
            |this, cx| this.toggle_comment_panel(cx),
            cx,
        );

        let close_archive = self.file_toolbar_toggle(
            "file-toolbar-close",
            self.localized("关闭压缩包"),
            IconName::X,
            false,
            |this, cx| this.close_current_archive(cx),
            cx,
        );

        let list_segment = self.file_view_segment(
            "file-view-list",
            self.localized("列表"),
            IconName::List,
            FileViewMode::List,
            true,
            false,
            cx,
        );
        let icons_segment = self.file_view_segment(
            "file-view-icons",
            self.localized("图标"),
            IconName::LayoutGrid,
            FileViewMode::Icons,
            false,
            true,
            cx,
        );
        let segments = div()
            .flex()
            .items_center()
            .h(px(32.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(palette.line_strong)
            .overflow_hidden()
            .child(list_segment)
            .child(div().w(px(1.0)).h(px(18.0)).bg(palette.line))
            .child(icons_segment);

        let icon_size = self.settings.view.thumbnail_size.clamp(64.0, 200.0);
        let zoom = (self.file_view == FileViewMode::Icons).then(|| {
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(icon(IconName::LayoutGrid, palette.muted, 13.0))
                .child(
                    div().w(px(if compact { 66.0 } else { 100.0 })).child(
                        Slider::new(&self.icon_size_slider)
                            .w_full()
                            .h(px(20.0))
                            .accessibility_label(self.localized("图标大小")),
                    ),
                )
                .when(!compact, |this| {
                    this.child(
                        div()
                            .w(px(44.0))
                            .text_size(crate::theme::font_px(11.5))
                            .text_color(palette.sub)
                            .child(format!("{icon_size:.0} px")),
                    )
                })
                .into_any_element()
        });

        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(48.0))
            .px(px(10.0))
            .flex_none()
            .bg(palette.page)
            .border_b_1()
            .border_color(palette.line)
            .child(nav)
            .child(preview)
            .child(comment)
            .child(div().w(px(1.0)).h(px(18.0)).mx(px(2.0)).bg(palette.line))
            .child(close_archive)
            .child(div().flex_1())
            .when_some(zoom, |this, zoom| this.child(zoom))
            .child(segments)
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn file_toolbar_toggle(
        &self,
        id: &'static str,
        label: &'static str,
        icon_name: IconName,
        active: bool,
        action: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .w(px(32.0))
            .h(px(32.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(if active {
                palette.accent_soft
            } else {
                palette.line
            })
            .bg(if active {
                palette.accent_soft
            } else {
                palette.bg
            })
            .flex()
            .items_center()
            .justify_center()
            .text_size(crate::theme::font_px(12.5))
            .text_color(if active { palette.accent } else { palette.sub })
            .cursor_pointer()
            .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
            .when(!active, |this| {
                this.hover(|style| style.bg(palette.hover).border_color(palette.line_strong))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                action(this, cx);
            }))
            .child(icon(
                icon_name,
                if active { palette.accent } else { palette.sub },
                15.0,
            ))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn file_view_segment(
        &self,
        id: &'static str,
        label: &'static str,
        icon_name: IconName,
        mode: FileViewMode,
        round_left: bool,
        round_right: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let active = self.file_view == mode;
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .w(px(34.0))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .when(round_left, |this| {
                this.rounded_tl(px(8.0)).rounded_bl(px(8.0))
            })
            .when(round_right, |this| {
                this.rounded_tr(px(8.0)).rounded_br(px(8.0))
            })
            .text_size(crate::theme::font_px(12.5))
            .cursor_pointer()
            .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
            .bg(if active {
                palette.accent_soft
            } else {
                palette.bg
            })
            .text_color(if active { palette.accent } else { palette.sub })
            .when(!active, |this| this.hover(|style| style.bg(palette.hover)))
            .on_click(cx.listener(move |this, _, _, cx| this.set_file_view(mode, cx)))
            .child(icon(
                icon_name,
                if active { palette.accent } else { palette.sub },
                15.0,
            ))
            .into_any_element()
    }

    pub(crate) fn render_list_header(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let sort_column = self.sort_column;
        let sort_direction = self.sort_direction;
        let mut header = div()
            .flex()
            .items_center()
            .w_full()
            .h(px(36.0))
            .px(px(14.0))
            .bg(palette.panel)
            .border_b_1()
            .border_color(palette.line)
            .font_family(crate::theme::ui_font_family())
            .text_size(crate::theme::font_px(12.0))
            .text_color(palette.muted);
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
        for key in order {
            let Some(column) = SortColumn::from_key(&key) else {
                continue;
            };
            if column != SortColumn::Name && hidden.iter().any(|hidden| hidden == &key) {
                continue;
            }
            let width = self
                .settings
                .view
                .column_widths
                .get(&key)
                .copied()
                .unwrap_or_else(|| default_column_width(&key));
            header = header.child(self.sort_header(
                column,
                tr(self.language, column.label()),
                px(width),
                column == SortColumn::Name,
                sort_column,
                sort_direction,
                cx,
            ));
        }
        header.into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn sort_header(
        &mut self,
        column: SortColumn,
        label: &str,
        width: Pixels,
        flex: bool,
        active: SortColumn,
        direction: SortDirection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let mut element = div()
            .id(format!("sort-{label}"))
            .flex()
            .items_center()
            .gap(px(3.0))
            .h_full()
            .cursor_pointer()
            .when(flex, |this| this.flex_1().min_w_0())
            .when(!flex, |this| this.w(width).flex_shrink_0())
            .hover(|style| style.text_color(palette.text))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_sort(column);
                cx.notify();
            }))
            .on_drag(column, |_, _, _, cx| cx.new(|_| gpui_kit::Empty))
            .on_drop(cx.listener(move |this, source: &SortColumn, _, cx| {
                this.move_column(*source, column, cx);
            }))
            .child(label.to_string());
        if active == column {
            element = element.child(icon(
                if direction == SortDirection::Asc {
                    IconName::ChevronUp
                } else {
                    IconName::ChevronDown
                },
                palette.accent,
                11.0,
            ));
        }
        element.into_any_element()
    }

    pub(crate) fn render_entry_row(
        &mut self,
        row_index: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let Some(row) = self.rows.get(row_index).cloned() else {
            return div().into_any_element();
        };
        match row {
            Row::Entry(index) => {
                let Some(archive) = &self.archive else {
                    return div().into_any_element();
                };
                let Some(entry) = archive.info.find(index) else {
                    return div().into_any_element();
                };
                let entry = entry.clone();
                let selected = self.multi_selected.contains(&index)
                    || (self.multi_selected.is_empty() && self.selected == Some(index));
                let is_dir = entry.is_dir;
                let is_image = is_image_name(&entry.path);
                let name = if is_dir {
                    format!("{}/", entry.name())
                } else {
                    entry.name().to_string()
                };
                let display_method = if is_dir || entry.method == "Store" {
                    self.localized("存储").to_string()
                } else {
                    entry.method.clone()
                };
                let kind_label = crate::app::entry_kind_label(&entry, self.language);

                let entry_path = entry.path.clone();
                let size_label = if is_dir {
                    "—".to_string()
                } else {
                    format_size(entry.size)
                };
                let compressed_label = if is_dir || entry.compressed_size == 0 {
                    "—".to_string()
                } else {
                    format_size(entry.compressed_size)
                };
                let modified_label = format_short_time(entry.modified);
                let encryption_label = if entry.encrypted {
                    self.localized("是")
                } else {
                    "—"
                };
                let crc_label = entry
                    .crc32
                    .map(|crc| format!("{crc:08X}"))
                    .unwrap_or_else(|| "—".to_string());
                let attributes_label = if entry.is_symlink {
                    self.localized("链接")
                } else if is_dir {
                    self.localized("目录")
                } else {
                    self.localized("文件")
                };
                let comment_label = entry.comment.clone().unwrap_or_else(|| "—".to_string());
                let extra_label = if entry.is_symlink {
                    self.localized("符号链接")
                } else if entry.encrypted {
                    self.localized("AES / 加密")
                } else {
                    "—"
                };
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
                let cell = |text: String, color: Hsla, size: f32| {
                    div()
                        .font_family(crate::theme::ui_font_family())
                        .truncate()
                        .text_size(crate::theme::font_px(size))
                        .text_color(color)
                        .child(text)
                };
                let mut row_element = div()
                    .id(format!("row-{index}"))
                    .font_family(crate::theme::ui_font_family())
                    .w_full()
                    .flex()
                    .items_center()
                    .h(px(32.0))
                    .px(px(14.0))
                    .when(selected, |this| this.bg(palette.accent_soft))
                    .when(!selected, |this| {
                        this.hover(|style| style.bg(palette.accent_softer))
                    })
                    .when(self.settings.general.show_grid_lines, |this| {
                        this.border_b_1().border_color(palette.line)
                    })
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.select_entry_with_modifiers(index, &event.modifiers, cx);
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.open_archive_context_menu(
                                Some(index),
                                event.position.x.as_f32(),
                                event.position.y.as_f32(),
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    )
                    .on_double_click(cx.listener(move |this, _, window, cx| {
                        if is_dir {
                            this.navigate_to(entry_path.clone(), cx);
                        } else {
                            this.open_entry(index, window, cx);
                        }
                    }));
                for key in order {
                    let Some(column) = SortColumn::from_key(&key) else {
                        continue;
                    };
                    if column != SortColumn::Name && hidden.iter().any(|hidden| hidden == &key) {
                        continue;
                    }
                    let width = widths
                        .get(&key)
                        .copied()
                        .unwrap_or_else(|| default_column_width(&key));
                    match column {
                        SortColumn::Name => {
                            row_element = row_element.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .flex_1()
                                    .min_w_0()
                                    .child(icon(
                                        if is_dir {
                                            IconName::Folder
                                        } else if is_image {
                                            IconName::Image
                                        } else {
                                            IconName::FileText
                                        },
                                        if is_dir { palette.accent } else { palette.sub },
                                        14.0,
                                    ))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(crate::theme::font_px(13.0))
                                            .when(selected, |this| this.text_color(palette.accent))
                                            .child(name.clone()),
                                    )
                                    .when(entry.encrypted, |this| {
                                        this.child(icon(IconName::Lock, palette.warn, 12.0))
                                    }),
                            );
                        }
                        SortColumn::Original => {
                            row_element =
                                row_element.child(div().w(px(width)).flex_shrink_0().child(cell(
                                    size_label.clone(),
                                    palette.sub,
                                    12.0,
                                )));
                        }
                        SortColumn::Compressed => {
                            row_element =
                                row_element.child(div().w(px(width)).flex_shrink_0().child(cell(
                                    compressed_label.clone(),
                                    palette.sub,
                                    12.0,
                                )));
                        }
                        SortColumn::Type => {
                            row_element =
                                row_element.child(div().w(px(width)).flex_shrink_0().child(cell(
                                    kind_label.clone(),
                                    palette.sub,
                                    12.0,
                                )));
                        }
                        SortColumn::Modified => {
                            row_element =
                                row_element.child(div().w(px(width)).flex_shrink_0().child(cell(
                                    modified_label.clone(),
                                    palette.sub,
                                    12.0,
                                )));
                        }
                        SortColumn::Method => {
                            row_element =
                                row_element.child(div().w(px(width)).flex_shrink_0().child(cell(
                                    display_method.clone(),
                                    palette.sub,
                                    12.0,
                                )));
                        }
                        SortColumn::Encryption => {
                            row_element =
                                row_element.child(div().w(px(width)).flex_shrink_0().child(cell(
                                    encryption_label.to_string(),
                                    if entry.encrypted {
                                        palette.warn
                                    } else {
                                        palette.sub
                                    },
                                    12.0,
                                )));
                        }
                        SortColumn::Crc => {
                            row_element =
                                row_element.child(div().w(px(width)).flex_shrink_0().child(cell(
                                    crc_label.clone(),
                                    palette.sub,
                                    11.5,
                                )));
                        }
                        SortColumn::Attributes => {
                            row_element =
                                row_element.child(div().w(px(width)).flex_shrink_0().child(cell(
                                    attributes_label.to_string(),
                                    palette.sub,
                                    12.0,
                                )));
                        }
                        SortColumn::Comment => {
                            row_element =
                                row_element.child(div().w(px(width)).flex_shrink_0().child(cell(
                                    comment_label.clone(),
                                    palette.sub,
                                    11.5,
                                )));
                        }
                        SortColumn::Extra => {
                            row_element =
                                row_element.child(div().w(px(width)).flex_shrink_0().child(cell(
                                    extra_label.to_string(),
                                    palette.sub,
                                    11.5,
                                )));
                        }
                    }
                }
                row_element.into_any_element()
            }
        }
    }

    /// 横向滚动几何：返回（可见视口宽度、最大横向偏移、轨道宽度）。
    ///
    /// GPUI 的 `ScrollHandle` 在首帧尚未完成布局时 `bounds` / `max_offset` 都是 0，
    /// 直接使用会把滚动条画成禁用态。这里在视口缺失时用轨道宽度加左右箭头等
    /// 固定占位回退，最大偏移则直接由表格内容宽度推出，保证首帧即可显示正确滑块。
    pub(crate) fn h_scroll_geometry(&self) -> (f32, f32, f32) {
        /// 左右内边距 4×2 + 两个 16px 箭头 + 两个 4px 间距。
        const BAR_CHROME_WIDTH: f32 = 48.0;
        let live_viewport = self.table_scroll.bounds().size.width.as_f32();
        let track_width = self
            .table_scroll_bounds
            .map(|bounds| bounds.size.width.as_f32())
            .filter(|width| *width > 1.0)
            .unwrap_or(live_viewport);
        let viewport = if live_viewport > 1.0 {
            live_viewport
        } else if track_width > 1.0 {
            track_width + BAR_CHROME_WIDTH
        } else {
            0.0
        };
        let max = (self.table_width() - viewport).max(0.0);
        (viewport, max, track_width)
    }

    pub(crate) fn render_horizontal_scrollbar(
        &mut self,
        palette: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (viewport, max, track_width) = self.h_scroll_geometry();
        // GPUI 的滚动偏移向右为负（[-max, 0]），这里统一换算成“已向右滚动的距离”。
        let scroll = (-self.table_scroll.offset().x.as_f32()).clamp(0.0, max);
        let overflow = max > 1.0 && viewport > 1.0;
        let travel_width = (track_width - 4.0).max(1.0);
        let thumb_width = if overflow {
            (travel_width * (viewport / (viewport + max))).clamp(0.0, travel_width)
        } else {
            travel_width
        }
        .max(24.0_f32.min(travel_width));
        let travel = (travel_width - thumb_width).max(0.0);
        let thumb_left = if overflow && max > 0.0 {
            (scroll / max) * travel
        } else {
            0.0
        };
        let entity = cx.entity();
        let max_for_click = max;
        let thumb_width_for_click = thumb_width;
        let travel_for_click = travel.max(1.0);

        let scroll_by = |this: &mut Self, delta: f32, cx: &mut Context<Self>| {
            let (_, max, _) = this.h_scroll_geometry();
            let current = (-this.table_scroll.offset().x.as_f32()).clamp(0.0, max);
            let target = (current + delta).clamp(0.0, max);
            let y = this.table_scroll.offset().y;
            this.table_scroll.set_offset(point(px(-target), y));
            cx.notify();
        };

        let thumb = div()
            .id("h-scroll-thumb")
            .absolute()
            .left(px(thumb_left))
            .top(px(2.0))
            .h(px(8.0))
            .w(px(thumb_width))
            .rounded(px(99.0))
            .bg(if overflow {
                palette.line_strong
            } else {
                palette.line
            })
            .cursor_pointer()
            .when(overflow, |this| {
                this.hover(|style| style.bg(palette.muted))
                    // 记录按下时指针相对滑块左侧的偏移；移动事件由窗口根节点统一处理。
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            let Some(bounds) = this.table_scroll_bounds else {
                                return;
                            };
                            let thumb_left = bounds.origin.x.as_f32()
                                + if max_for_click > 0.0 {
                                    ((-this.table_scroll.offset().x.as_f32())
                                        .clamp(0.0, max_for_click)
                                        / max_for_click)
                                        * travel_for_click
                                } else {
                                    0.0
                                };
                            this.h_scroll_drag = Some(event.position.x.as_f32() - thumb_left);
                            cx.stop_propagation();
                        }),
                    )
            });

        let track = div()
            .id("h-scroll-track")
            .relative()
            .flex_1()
            .h_full()
            .rounded(px(99.0))
            .bg(palette.panel)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.h_scroll_drag = None;
                    let Some(bounds) = this.table_scroll_bounds else {
                        return;
                    };
                    let left = event.position.x.as_f32() - bounds.origin.x.as_f32();
                    if max_for_click <= 1.0 || travel_for_click <= 1.0 {
                        return;
                    }
                    let ratio =
                        ((left - thumb_width_for_click / 2.0) / travel_for_click).clamp(0.0, 1.0);
                    let y = this.table_scroll.offset().y;
                    this.table_scroll
                        .set_offset(point(px(-ratio * max_for_click), y));
                    cx.notify();
                    cx.stop_propagation();
                }),
            )
            .on_scroll(cx.listener(move |this, event: &ScrollEvent, _, cx| {
                // 滚轮向下 / Shift+滚轮向下 → 向右滚动，与原生滚动偏移方向相反。
                let delta = match event.delta {
                    ScrollDelta::Pixels(delta) => -(delta.x.as_f32() + delta.y.as_f32()),
                    ScrollDelta::Lines(delta) => -(delta.x + delta.y) * 32.0,
                };
                if delta.abs() > 0.0 {
                    scroll_by(this, delta, cx);
                }
                cx.stop_propagation();
            }))
            .child(thumb);

        div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .h(px(14.0))
            .flex_none()
            .px(px(4.0))
            .bg(palette.bar)
            .border_t_1()
            .border_color(palette.line)
            .on_children_prepainted(move |bounds, window, app| {
                if let Some(bounds) = bounds.get(1).copied() {
                    let changed = entity.update(app, |this, _| {
                        // 首帧渲染时滚动容器还没有 bounds，滚动条会先画成禁用态；
                        // 拿到轨道尺寸后再安排一帧，显示真实滑块。
                        if this.table_scroll_bounds != Some(bounds) {
                            this.table_scroll_bounds = Some(bounds);
                            true
                        } else {
                            false
                        }
                    });
                    // `cx.notify()` 在绘制阶段会被忽略，必须通过窗口安排下一帧。
                    if changed {
                        window.request_animation_frame();
                    }
                }
            })
            .child(
                div()
                    .id("h-scroll-left")
                    .role(Role::Button)
                    .aria_label(self.localized("向左滚动"))
                    .w(px(16.0))
                    .h(px(16.0))
                    .rounded(px(4.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|style| style.bg(palette.hover))
                    .on_click(cx.listener(move |this, _, _, cx| scroll_by(this, -140.0, cx)))
                    .child(icon(IconName::ChevronLeft, palette.sub, 12.0)),
            )
            .child(track)
            .child(
                div()
                    .id("h-scroll-right")
                    .role(Role::Button)
                    .aria_label(self.localized("向右滚动"))
                    .w(px(16.0))
                    .h(px(16.0))
                    .rounded(px(4.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|style| style.bg(palette.hover))
                    .on_click(cx.listener(move |this, _, _, cx| scroll_by(this, 140.0, cx)))
                    .child(icon(IconName::ChevronRight, palette.sub, 12.0)),
            )
            .into_any_element()
    }

    pub(crate) fn update_h_scroll_drag(&mut self, pointer_x: f32, cx: &mut Context<Self>) {
        let Some(drag_offset) = self.h_scroll_drag else {
            return;
        };
        // 如果在窗口外松开鼠标，GPUI 收不到 up 事件；下次移动时按物理按键状态
        // 结束拖拽，避免鼠标移动继续带动滑块。
        if !left_mouse_button_down() {
            self.h_scroll_drag = None;
            cx.notify();
            return;
        }
        let Some(bounds) = self.table_scroll_bounds else {
            return;
        };
        let (viewport, max, track_width) = self.h_scroll_geometry();
        if max <= 1.0 || viewport <= 1.0 || track_width <= 1.0 {
            return;
        }
        let travel_width = (track_width - 4.0).max(1.0);
        let thumb_width = (travel_width * (viewport / (viewport + max)))
            .clamp(0.0, travel_width)
            .max(24.0_f32.min(travel_width));
        let travel = (travel_width - thumb_width).max(1.0);
        let left = pointer_x - bounds.origin.x.as_f32() - drag_offset;
        let ratio = (left / travel).clamp(0.0, 1.0);
        let y = self.table_scroll.offset().y;
        self.table_scroll.set_offset(point(px(-ratio * max), y));
        cx.notify();
    }

    /// 根节点统一处理滚轮：位于横向滚动条上时直接横向滚动；其他位置仅刷新
    /// 滚动条显示，使原生 Shift+滚轮 / 触摸板横向滚动后滑块同步。
    pub(crate) fn handle_root_scroll(&mut self, event: &ScrollEvent, cx: &mut Context<Self>) {
        let over_bar = self
            .table_scroll_bounds
            .is_some_and(|bounds| bounds.contains(&event.position));
        if over_bar {
            let delta = match event.delta {
                ScrollDelta::Pixels(delta) => -(delta.x.as_f32() + delta.y.as_f32()),
                ScrollDelta::Lines(delta) => -(delta.x + delta.y) * 32.0,
            };
            if delta.abs() > 0.0 {
                let (_, max, _) = self.h_scroll_geometry();
                let current = (-self.table_scroll.offset().x.as_f32()).clamp(0.0, max);
                let target = (current + delta).clamp(0.0, max);
                let y = self.table_scroll.offset().y;
                self.table_scroll.set_offset(point(px(-target), y));
            }
        }
        cx.notify();
    }

    pub(crate) fn render_icon_grid(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let Some(archive) = &self.archive else {
            return div().into_any_element();
        };
        let indices = self.visible_entry_indices();
        if indices.is_empty() {
            return div()
                .id("icon-empty")
                .flex_1()
                .min_h_0()
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
                .child(empty_panel(
                    if self.search_query.trim().is_empty() {
                        self.localized("此文件夹为空")
                    } else {
                        self.localized("没有匹配的条目")
                    },
                    &palette,
                ))
                .into_any_element();
        }
        let multi = self.multi_selected.clone();
        let selected_single = self.selected;
        let icon_size = self.settings.view.thumbnail_size.clamp(64.0, 200.0);
        let tile_width = icon_size + 24.0;
        let tile_height = icon_size + 58.0;
        let draw_icon = (icon_size * 0.55).clamp(28.0, 72.0);
        let mut grid = div().flex().flex_wrap().gap(px(6.0)).p(px(12.0));
        for index in indices {
            let Some(entry) = archive.info.find(index) else {
                continue;
            };
            let entry = entry.clone();
            let selected =
                multi.contains(&index) || (multi.is_empty() && selected_single == Some(index));
            let is_dir = entry.is_dir;
            let is_image = is_image_name(&entry.path);
            let name = if is_dir {
                format!("{}/", entry.name())
            } else {
                entry.name().to_string()
            };
            let size = if is_dir {
                self.localized("文件夹").to_string()
            } else {
                format_size(entry.size)
            };
            let icon_name = if is_dir {
                IconName::Folder
            } else if is_image {
                IconName::Image
            } else {
                IconName::FileText
            };
            let entry_path = entry.path.clone();
            let tile = div()
                .id(format!("icon-{index}"))
                .role(Role::Button)
                .aria_label(name.clone())
                .w(px(tile_width))
                .h(px(tile_height))
                .flex_none()
                .rounded(px(9.0))
                .border_1()
                .border_color(if selected {
                    palette.accent
                } else {
                    palette.line
                })
                .bg(if selected {
                    palette.accent_soft
                } else {
                    palette.bg
                })
                .when(!selected, |this| {
                    this.hover(|style| style.bg(palette.accent_softer))
                })
                .cursor_pointer()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(5.0))
                .px(px(6.0))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.select_entry_with_modifiers(index, &event.modifiers, cx);
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.open_archive_context_menu(
                            Some(index),
                            event.position.x.as_f32(),
                            event.position.y.as_f32(),
                            cx,
                        );
                        cx.stop_propagation();
                    }),
                )
                .on_double_click(cx.listener(move |this, _, window, cx| {
                    if is_dir {
                        this.navigate_to(entry_path.clone(), cx);
                    } else {
                        this.open_entry(index, window, cx);
                    }
                }))
                .child(
                    div()
                        .text_size(crate::theme::font_px(12.5))
                        .when(selected, |this| this.text_color(palette.accent))
                        .child(icon(
                            icon_name,
                            if is_dir || selected {
                                palette.accent
                            } else {
                                palette.sub
                            },
                            draw_icon,
                        )),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(30.0))
                        .text_center()
                        .line_clamp(2)
                        .text_size(crate::theme::font_px(12.0))
                        .text_color(if selected {
                            palette.accent
                        } else {
                            palette.text
                        })
                        .child(name),
                )
                .child(
                    div()
                        .text_size(crate::theme::font_px(10.5))
                        .text_color(palette.muted)
                        .child(if is_dir {
                            self.localized("文件夹").to_string()
                        } else {
                            format!(
                                "{} · {}",
                                size,
                                crate::app::entry_kind_label(&entry, self.language)
                            )
                        }),
                );
            grid = grid.child(tile);
        }
        div()
            .id("icon-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .on_scroll(cx.listener(|this, event: &ScrollEvent, window, cx| {
                if !event.modifiers.control {
                    return;
                }
                let delta = match event.delta {
                    ScrollDelta::Pixels(delta) => delta.y.as_f32(),
                    ScrollDelta::Lines(delta) => delta.y * 20.0,
                };
                let step = if delta > 0.0 { 8.0 } else { -8.0 };
                this.set_icon_size(this.settings.view.thumbnail_size + step, window, cx);
                cx.stop_propagation();
            }))
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
            .child(grid)
            .into_any_element()
    }

    pub(crate) fn render_preview_panel(&mut self, _cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        if !self.settings.view.preview_enabled {
            return div().into_any_element();
        }
        let Some(archive) = &self.archive else {
            return div().into_any_element();
        };
        let selected = self
            .selected
            .and_then(|index| archive.info.find(index).cloned());
        let image_path = self.preview.image_path.clone();
        let dimensions = self.preview.dimensions;
        let preview_error = self.preview.error.clone();

        let mut panel = div()
            .w(px(200.0))
            .flex_shrink_0()
            .font_family(crate::theme::ui_font_family())
            .border_l_1()
            .border_color(palette.line)
            .bg(palette.page)
            .p(px(12.0))
            .flex()
            .flex_col()
            .gap(px(10.0));
        panel = panel.child(
            div()
                .text_size(crate::theme::font_px(12.0))
                .text_color(palette.sub)
                .child(tr(self.language, "图像预览")),
        );
        panel = panel.child(
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .rounded(px(8.0))
                .border_1()
                .border_color(palette.line)
                .bg(palette.panel)
                .overflow_hidden()
                .flex()
                .items_center()
                .justify_center()
                .when_some(image_path, |this, path| {
                    this.child(img(path).w_full().h_full().object_fit(ObjectFit::Contain))
                })
                .when(self.preview.image_path.is_none(), |this| {
                    this.child(icon(
                        selected
                            .as_ref()
                            .map(|entry| {
                                if entry.is_dir {
                                    IconName::Folder
                                } else if is_image_name(&entry.path) {
                                    IconName::Image
                                } else {
                                    IconName::FileText
                                }
                            })
                            .unwrap_or(IconName::Image),
                        palette.sub,
                        40.0,
                    ))
                }),
        );

        if let Some(entry) = &selected {
            panel = panel.child(
                div()
                    .text_size(crate::theme::font_px(13.0))
                    .text_color(palette.text)
                    .truncate()
                    .child(entry.name().to_string()),
            );
            let kind = if entry.is_dir {
                self.localized("文件夹").to_string()
            } else {
                let label = extension_label(&entry.path);
                if label == "文件" {
                    self.localized("文件").to_string()
                } else {
                    label
                }
            };
            let mut info = div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .text_size(crate::theme::font_px(12.0))
                .text_color(palette.sub);
            if let Some((width, height)) = dimensions {
                info = info.child(format!("{width} × {height} · {kind}"));
            } else {
                info = info.child(kind);
            }
            if !entry.is_dir {
                info = info.child(format!(
                    "{} → {}",
                    format_size(entry.size),
                    format_size(entry.compressed_size)
                ));
            }
            if let Some(error) = preview_error {
                info = info.child(div().text_color(palette.warn).child(error));
            }
            panel = panel.child(info);
        } else {
            panel = panel.child(
                div()
                    .text_size(crate::theme::font_px(12.0))
                    .text_color(palette.sub)
                    .child(tr(self.language, "选择文件以预览")),
            );
        }
        panel.into_any_element()
    }

    pub(crate) fn render_status_bar(&mut self, _cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let active = self.active_progress();
        let mut bar = div()
            .flex()
            .items_center()
            .font_family(crate::theme::ui_font_family())
            .gap(px(20.0))
            .h(px(34.0))
            .px(px(14.0))
            .bg(palette.bar)
            .border_t_1()
            .border_color(palette.line)
            .text_size(crate::theme::font_px(12.5))
            .text_color(palette.sub);

        if let Some(archive) = &self.archive {
            let indices = self.selected_indices();
            let selected_count = indices.len();
            let (selected_size, selected_compressed) = indices
                .iter()
                .filter_map(|index| archive.info.find(*index))
                .filter(|entry| !entry.is_dir)
                .fold((0u64, 0u64), |(size, compressed), entry| {
                    (size + entry.size, compressed + entry.compressed_size)
                });
            let selection_label = if selected_count > 0 {
                crate::i18n::tr_fmt(
                    self.language,
                    "已选择 {} 项 · 原始 {}（压缩后 {}）",
                    &[
                        &selected_count,
                        &format_size(selected_size),
                        &format_size(selected_compressed),
                    ],
                )
            } else {
                crate::i18n::tr_fmt(self.language, "{} 个对象", &[&archive.info.file_count()])
            };
            bar = bar.child(selection_label).child(crate::i18n::tr_fmt(
                self.language,
                "原始 {} → 压缩后 {}（压缩率 {}）",
                &[
                    &format_size(archive.info.total_size()),
                    &format_size(archive.info.total_compressed()),
                    &saved_ratio(archive.info.total_compressed(), archive.info.total_size()),
                ],
            ));
        } else {
            bar = bar.child(tr(self.language, "打开压缩包或新建压缩包"));
        }

        if let Some((phase, percent)) = active {
            bar = bar.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .w(px(72.0))
                            .child(progress_bar(percent, &palette, palette.accent)),
                    )
                    .child(format!("{phase} {:.0}%", percent)),
            );
        }

        bar.child(div().flex_1())
            .child(
                div()
                    .text_size(crate::theme::font_px(12.0))
                    .text_color(palette.muted)
                    .child(if self.file_view == FileViewMode::Icons {
                        tr(
                            self.language,
                            "双击进入 · F2 重命名 · Ctrl+E 解压 · 图标视图下 Ctrl+滚轮 缩放",
                        )
                    } else {
                        tr(self.language, "双击进入 · F2 重命名 · Ctrl+E 解压")
                    }),
            )
            .into_any_element()
    }
}

fn push_tree_rows(
    node: &DirNode,
    out: &mut Vec<AnyElement>,
    depth: usize,
    current: &str,
    expanded: &HashSet<String>,
    palette: &Palette,
    cx: &mut Context<MainView>,
) {
    let label = node
        .path
        .rsplit('/')
        .next()
        .unwrap_or(&node.path)
        .to_string();
    let is_expanded = expanded.contains(&node.path);
    let is_current = current == node.path;
    let path = node.path.clone();
    let path_for_click = path.clone();
    let has_children = !node.children.is_empty();
    out.push(
        div()
            .id(format!("tree-node-{path}"))
            .flex()
            .items_center()
            .gap(px(7.0))
            .h(px(32.0))
            .pl(px(8.0 + depth as f32 * 15.0))
            .pr(px(8.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .text_size(crate::theme::font_px(13.0))
            .when(is_current, |this| {
                this.bg(palette.accent_soft).text_color(palette.accent)
            })
            .when(!is_current, |this| {
                this.text_color(palette.text)
                    .hover(|style| style.bg(palette.hover))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.navigate_to(path_for_click.clone(), cx);
                if this.expanded.contains(&path_for_click) {
                    this.expanded.remove(&path_for_click);
                } else {
                    this.expanded.insert(path_for_click.clone());
                }
                cx.notify();
            }))
            .child(
                div()
                    .w(px(12.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(has_children, |this| {
                        this.child(icon(
                            if is_expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            },
                            palette.sub,
                            13.0,
                        ))
                    }),
            )
            .child(icon(
                IconName::Folder,
                if is_current {
                    palette.accent
                } else {
                    palette.sub
                },
                13.0,
            ))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .into_any_element(),
    );
    if is_expanded {
        let mut keys: Vec<String> = node.children.keys().cloned().collect();
        keys.sort();
        for key in keys {
            if let Some(child) = node.children.get(&key) {
                push_tree_rows(child, out, depth + 1, current, expanded, palette, cx);
            }
        }
    }
}

pub(crate) fn format_short_time(timestamp: Option<i64>) -> String {
    use chrono::{Datelike, Local, TimeZone};
    match timestamp {
        Some(ts) if ts > 0 => match Local.timestamp_opt(ts, 0).single() {
            Some(date) => {
                if date.year() == Local::now().year() {
                    date.format("%m-%d %H:%M").to_string()
                } else {
                    date.format("%Y-%m-%d").to_string()
                }
            }
            None => "—".to_string(),
        },
        _ => "—".to_string(),
    }
}

pub(crate) fn saved_ratio(compressed: u64, original: u64) -> String {
    if original == 0 {
        return "—".to_string();
    }
    let saved = (1.0 - compressed as f64 / original as f64) * 100.0;
    format!("{:.0}%", saved.clamp(0.0, 100.0))
}

pub(crate) fn extension_label(path: &str) -> String {
    Path::new(path)
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_uppercase())
        .filter(|extension| !extension.is_empty())
        .unwrap_or_else(|| "文件".to_string())
}
