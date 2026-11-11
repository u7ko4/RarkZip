//! 压缩包内文件/文件夹右键菜单。

use std::rc::Rc;

use crate::app::*;

impl MainView {
    pub(crate) fn render_archive_context_menu(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(menu) = self.archive_menu else {
            return div().into_any_element();
        };
        let palette = self.palette;
        let viewport = window.viewport_size();
        let indices = self.selected_indices();
        let has_selection = !indices.is_empty();
        let single = indices.len() == 1;
        let is_zip = self
            .archive
            .as_ref()
            .is_some_and(|archive| archive.info.format.is_zip());
        let at_root = self.current_dir.is_empty();
        let open_index = indices.first().copied();
        let is_dir = open_index
            .and_then(|index| {
                self.archive
                    .as_ref()
                    .and_then(|archive| archive.info.find(index))
            })
            .is_some_and(|entry| entry.is_dir);
        let selected_for_extract = (!indices.is_empty()).then_some(indices.clone());

        let mut panel = div().flex().flex_col();
        let mut rows = 0usize;
        let mut separators = 0usize;

        match menu.target {
            ContextTarget::Row(_) => {
                rows += 3;
                panel = panel
                    .child(self.context_menu_item(
                        "ctx-extract-all",
                        "解压全部文件…",
                        Some("Ctrl+E"),
                        true,
                        move |this, window, cx| {
                            this.context_extract_to(None, window, cx);
                        },
                        cx,
                    ))
                    .child(self.context_menu_item(
                        "ctx-extract-selected",
                        "解压选定的文件",
                        None,
                        has_selection,
                        move |this, window, cx| {
                            this.context_extract_to(selected_for_extract.clone(), window, cx);
                        },
                        cx,
                    ))
                    .child(self.context_menu_item(
                        "ctx-extract-here",
                        "解压全部文件到此处",
                        None,
                        true,
                        |this, window, cx| this.context_extract_here(window, cx),
                        cx,
                    ));
                panel = panel.child(self.context_separator(&palette));
                separators += 1;
                rows += 3;
                panel = panel
                    .child(self.context_menu_item(
                        "ctx-open",
                        "打开",
                        Some("Enter"),
                        single,
                        move |this, window, cx| {
                            if let Some(index) = open_index {
                                if is_dir {
                                    if let Some(entry) = this
                                        .archive
                                        .as_ref()
                                        .and_then(|archive| archive.info.find(index))
                                    {
                                        let path = entry.path.clone();
                                        this.navigate_to(path, cx);
                                    }
                                } else {
                                    this.open_entry(index, window, cx);
                                }
                            }
                        },
                        cx,
                    ))
                    .child(self.context_menu_item(
                        "ctx-open-nimzip",
                        "使用 NimZip 打开",
                        None,
                        single,
                        |this, window, cx| this.context_open_nimzip(window, cx),
                        cx,
                    ))
                    .child(self.context_menu_item(
                        "ctx-open-system",
                        "使用关联的程序打开",
                        None,
                        single,
                        move |this, window, cx| {
                            if let Some(index) = open_index {
                                this.open_entry(index, window, cx);
                            }
                        },
                        cx,
                    ));
                panel = panel.child(self.context_separator(&palette));
                separators += 1;
                rows += 6;
                panel = panel
                    .child(self.context_menu_item(
                        "ctx-add",
                        "添加文件",
                        Some("Ins"),
                        is_zip,
                        |this, _, cx| this.add_files_to_archive(cx),
                        cx,
                    ))
                    .child(self.context_menu_item(
                        "ctx-delete",
                        "删除文件",
                        Some("Del"),
                        is_zip && has_selection,
                        |this, _, cx| this.delete_selected_entry(cx),
                        cx,
                    ))
                    .child(self.context_menu_item(
                        "ctx-rename",
                        "重命名文件",
                        None,
                        is_zip && single,
                        |this, window, cx| this.open_rename_modal(window, cx),
                        cx,
                    ))
                    .child(self.context_menu_item(
                        "ctx-select-all",
                        "全部选择",
                        Some("Ctrl+A"),
                        true,
                        |this, _, cx| this.select_all_visible(cx),
                        cx,
                    ))
                    .child(self.context_menu_item(
                        "ctx-up",
                        "转到上一级文件夹",
                        Some("Backspace"),
                        !at_root,
                        |this, _, cx| this.go_up(cx),
                        cx,
                    ))
                    .child(self.context_menu_item(
                        "ctx-new-folder",
                        "新建文件夹",
                        None,
                        is_zip,
                        |this, window, cx| this.open_new_folder_modal(window, cx),
                        cx,
                    ));
                panel = panel.child(self.context_separator(&palette));
                separators += 1;
                rows += 3;
                panel = panel
                    .child(self.context_menu_item(
                        "ctx-copy",
                        "复制到剪贴板",
                        Some("Ctrl+C"),
                        has_selection,
                        |this, _, cx| this.copy_selected_entries(cx),
                        cx,
                    ))
                    .child(self.context_menu_item(
                        "ctx-paste",
                        "从剪贴板粘贴文件",
                        Some("Ctrl+V"),
                        is_zip,
                        |this, _, cx| this.paste_clipboard_entries(cx),
                        cx,
                    ))
                    .child(self.context_menu_item(
                        "ctx-copy-paths",
                        "将完整路径复制到剪贴板中",
                        None,
                        has_selection,
                        |this, _, cx| this.copy_selected_entry_paths(cx),
                        cx,
                    ));
            }
            ContextTarget::Blank => {
                rows += 5;
                panel = panel
                    .child(self.context_menu_item(
                        "ctx-blank-extract-all",
                        "解压全部文件…",
                        Some("Ctrl+E"),
                        true,
                        |this, window, cx| this.context_extract_to(None, window, cx),
                        cx,
                    ))
                    .child(self.context_menu_item(
                        "ctx-blank-extract-here",
                        "解压全部文件到此处",
                        None,
                        true,
                        |this, window, cx| this.context_extract_here(window, cx),
                        cx,
                    ));
                panel = panel.child(self.context_separator(&palette));
                separators += 1;
                rows += 3;
                panel = panel
                    .child(self.context_menu_item(
                        "ctx-blank-add",
                        "添加文件",
                        Some("Ins"),
                        is_zip,
                        |this, _, cx| this.add_files_to_archive(cx),
                        cx,
                    ))
                    .child(self.context_menu_item(
                        "ctx-blank-select-all",
                        "全部选择",
                        Some("Ctrl+A"),
                        true,
                        |this, _, cx| this.select_all_visible(cx),
                        cx,
                    ))
                    .child(self.context_menu_item(
                        "ctx-blank-up",
                        "转到上一级文件夹",
                        Some("Backspace"),
                        !at_root,
                        |this, _, cx| this.go_up(cx),
                        cx,
                    ));
                panel = panel.child(self.context_separator(&palette));
                separators += 1;
                rows += 2;
                panel = panel
                    .child(self.context_menu_item(
                        "ctx-blank-new-folder",
                        "新建文件夹",
                        None,
                        is_zip,
                        |this, window, cx| this.open_new_folder_modal(window, cx),
                        cx,
                    ))
                    .child(self.context_menu_item(
                        "ctx-blank-paste",
                        "从剪贴板粘贴文件",
                        Some("Ctrl+V"),
                        is_zip,
                        |this, _, cx| this.paste_clipboard_entries(cx),
                        cx,
                    ));
            }
        }

        let width = 286.0;
        let estimated_height = rows as f32 * 30.0 + separators as f32 * 9.0 + 12.0;
        let left = menu
            .x
            .clamp(8.0, (viewport.width.as_f32() - width - 8.0).max(8.0));
        let top = menu.y.clamp(
            8.0,
            (viewport.height.as_f32() - estimated_height - 8.0).max(8.0),
        );

        div()
            .id("archive-context-layer")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                div()
                    .id("archive-context-backdrop")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(Hsla::transparent_black())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.archive_menu = None;
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, _, _, cx| {
                            this.archive_menu = None;
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .id("archive-context-menu")
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(width))
                    .max_h(px((viewport.height.as_f32() - 16.0).max(120.0)))
                    .overflow_y_scroll()
                    .p(px(5.0))
                    .rounded(px(crate::theme::RADIUS_OVERLAY))
                    .bg(palette.bg)
                    .border_1()
                    .border_color(palette.line_strong)
                    .shadow_lg()
                    .occlude()
                    .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
                    .child(panel),
            )
            .into_any_element()
    }

    fn context_separator(&self, palette: &Palette) -> AnyElement {
        div()
            .h(px(1.0))
            .mx(px(8.0))
            .my(px(4.0))
            .bg(palette.line)
            .into_any_element()
    }

    fn context_menu_item<F>(
        &self,
        id: &'static str,
        label: &'static str,
        shortcut: Option<&'static str>,
        enabled: bool,
        action: F,
        cx: &mut Context<Self>,
    ) -> AnyElement
    where
        F: Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    {
        let palette = self.palette;
        let action = Rc::new(action);
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .h(px(30.0))
            .px(px(9.0))
            .rounded(px(crate::theme::RADIUS_CONTROL))
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_size(crate::theme::body())
            .when(enabled, |this| {
                this.cursor_pointer()
                    .text_color(palette.text)
                    .hover(|style| style.bg(palette.accent_soft).text_color(palette.accent))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.archive_menu = None;
                        action(this, window, cx);
                        cx.notify();
                    }))
            })
            .when(!enabled, |this| this.text_color(palette.muted).opacity(0.6))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .when_some(shortcut, |this, shortcut| {
                this.child(
                    div()
                        .text_size(crate::theme::caption())
                        .text_color(palette.muted)
                        .child(shortcut),
                )
            })
            .into_any_element()
    }
}
