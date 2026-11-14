use std::time::Duration;

use crate::app::*;

impl MainView {
    pub(crate) fn render_app_menu(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let has_archive = self.archive.is_some();
        let is_zip = self
            .archive
            .as_ref()
            .is_some_and(|archive| archive.info.format == crate::app::ArchiveFormat::Zip);
        let has_selection = self.selected.is_some();
        let toolbar_height = self.settings.view.toolbar_height.clamp(40.0, 64.0);
        let menu_top = toolbar_height + 4.0;
        let toggle_top = ((toolbar_height - crate::components::ICON_BUTTON_HEIGHT) / 2.0).max(0.0);
        let max_height = (window.viewport_size().height.as_f32() - menu_top - 8.0).max(200.0);
        // “最近打开文件”只在首页（未打开归档）出现，最多展示 5 条。
        let show_recent = self.settings.advanced.history && !has_archive;
        let recent_files: Vec<String> = if show_recent {
            self.settings
                .ui
                .recent_files
                .iter()
                .take(5)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        let mut panel = div().flex().flex_col();
        panel = panel.child(self.menu_item(
            "menu-open",
            IconName::FolderOpen,
            "打开压缩包",
            Some("Ctrl+O"),
            true,
            |this, window, cx| this.open_file_dialog(window, cx),
            cx,
        ));
        if show_recent {
            panel = panel.child(self.menu_group(
                "menu-recent-group",
                IconName::Clock,
                "最近打开文件",
                MenuSubmenu::Recent,
                cx,
            ));
        }
        panel = panel
            .child(self.menu_item(
                "menu-new-window",
                IconName::AppWindow,
                "新建程序窗口",
                Some("Ctrl+Shift+N"),
                true,
                |this, window, cx| this.open_new_window(window, cx),
                cx,
            ))
            .child(self.menu_item(
                "menu-new",
                IconName::Plus,
                "新建压缩包",
                Some("Ctrl+N"),
                true,
                |this, window, cx| this.open_new_archive_modal(window, cx),
                cx,
            ))
            .child(self.menu_item(
                "menu-extract",
                IconName::Download,
                "解压到指定文件夹…",
                Some("Ctrl+E"),
                has_archive,
                |this, window, cx| this.pick_extract_target(window, cx),
                cx,
            ))
            .child(self.menu_group(
                "menu-tools-group",
                IconName::Wrench,
                "更多工具",
                MenuSubmenu::Tools,
                cx,
            ))
            .child(menu_separator(&palette))
            .child(self.menu_item(
                "menu-reveal",
                IconName::Folder,
                "打开所在位置",
                None,
                has_archive,
                |this, _, _| {
                    if let Some(path) = this.archive.as_ref().map(|archive| archive.path.clone()) {
                        #[cfg(target_os = "windows")]
                        {
                            let _ = std::process::Command::new("explorer.exe")
                                .arg(format!("/select,{}", path.display()))
                                .spawn();
                        }
                        #[cfg(not(target_os = "windows"))]
                        {
                            if let Some(parent) = path.parent() {
                                let _ = open::that(parent);
                            }
                        }
                    }
                },
                cx,
            ))
            .child(self.menu_item(
                "menu-close",
                IconName::X,
                "关闭压缩包",
                None,
                has_archive,
                |this, window, cx| {
                    if this.archive.take().is_some() {
                        this.current_dir.clear();
                        this.expanded.clear();
                        this.selected = None;
                        this.preview = PreviewState::default();
                        this.search_query.clear();
                        this.search.update(cx, |state, cx| {
                            state.set_value("", window, cx);
                        });
                        this.rebuild_rows();
                        this.page = Page::Main;
                        this.show_toast("已关闭压缩包", ToastKind::Info, cx);
                    }
                },
                cx,
            ))
            .child(menu_separator(&palette))
            .child(self.menu_item(
                "menu-add-files",
                IconName::SquarePlus,
                "添加文件…",
                Some("Ins"),
                is_zip,
                |this, _, cx| this.add_files_to_archive(cx),
                cx,
            ))
            .child(self.menu_item(
                "menu-rename",
                IconName::PenLine,
                "重命名…",
                Some("F2"),
                is_zip && has_selection,
                |this, window, cx| this.open_rename_modal(window, cx),
                cx,
            ))
            .child(self.menu_item(
                "menu-delete-entry",
                IconName::Trash,
                "删除选中条目",
                Some("Del"),
                is_zip && has_selection,
                |this, _, cx| this.delete_selected_entry(cx),
                cx,
            ))
            .child(self.menu_item(
                "menu-comment",
                IconName::MessageSquareText,
                "编辑注释…",
                None,
                is_zip,
                |this, window, cx| this.open_comment_modal(window, cx),
                cx,
            ))
            .child(menu_separator(&palette))
            .child(self.menu_item(
                "menu-settings",
                IconName::Settings,
                "设置",
                Some("F5"),
                true,
                |this, _, cx| {
                    this.page = Page::Settings;
                    cx.notify();
                },
                cx,
            ))
            .child(self.menu_item(
                "menu-passwords",
                IconName::KeyRound,
                "密码管理器",
                None,
                true,
                |this, window, cx| this.open_password_manager(window, cx),
                cx,
            ))
            .child(menu_separator(&palette))
            .child(self.menu_item(
                "menu-about",
                IconName::Info,
                "关于 RarkZip",
                None,
                true,
                |this, _, cx| {
                    this.show_toast(
                        format!(
                            "RarkZip {} · Rust + GPUI · M1 MVP",
                            env!("CARGO_PKG_VERSION")
                        ),
                        ToastKind::Info,
                        cx,
                    );
                },
                cx,
            ));

        let tools_submenu = (self.menu_submenu == Some(MenuSubmenu::Tools)).then(|| {
            self.submenu_container("app-tools-submenu", menu_top, max_height, 252.0)
                .child(self.menu_item(
                    "submenu-repair",
                    IconName::Wrench,
                    "修复 ZIP 压缩包…",
                    None,
                    true,
                    |this, _, cx| this.repair_zip_dialog(cx),
                    cx,
                ))
                .child(self.menu_item(
                    "submenu-verify",
                    IconName::ShieldCheck,
                    "测试完整性",
                    Some("Ctrl+T"),
                    has_archive,
                    |this, window, cx| this.start_verify(window, cx),
                    cx,
                ))
                .child(self.menu_item(
                    "submenu-toolbar",
                    IconName::Settings2,
                    "工具栏按钮…",
                    None,
                    true,
                    |this, _, cx| this.open_toolbar_settings(cx),
                    cx,
                ))
                .child(self.menu_item(
                    "submenu-columns",
                    IconName::Columns3,
                    "列设置…",
                    None,
                    true,
                    |this, _, cx| this.open_columns(cx),
                    cx,
                ))
                .child(self.menu_item(
                    "submenu-recovery",
                    IconName::Key,
                    "密码恢复…",
                    None,
                    true,
                    |this, window, cx| this.open_password_recovery(window, cx),
                    cx,
                ))
                .child(self.menu_item(
                    "submenu-crypt",
                    IconName::LockKeyhole,
                    "文件与文本加密…",
                    None,
                    true,
                    |this, _, cx| this.open_crypt_tool(cx),
                    cx,
                ))
                .child(self.menu_item(
                    "submenu-signature",
                    IconName::Signature,
                    "GnuPG 签名工具…",
                    None,
                    true,
                    |this, _, cx| this.open_signature_tool(cx),
                    cx,
                ))
                .child(self.menu_item(
                    "submenu-scan",
                    IconName::ScanSearch,
                    "扫描恶意软件",
                    None,
                    has_archive,
                    |this, _, cx| this.scan_current_archive(cx),
                    cx,
                ))
                .child(self.menu_item(
                    "submenu-hash",
                    IconName::Hash,
                    "文件哈希…",
                    Some("Ctrl+H"),
                    true,
                    |this, _, cx| this.open_hash_modal(cx),
                    cx,
                ))
                .into_any_element()
        });

        let recent_submenu = (self.menu_submenu == Some(MenuSubmenu::Recent)).then(|| {
            let mut list = div().flex().flex_col();
            if recent_files.is_empty() {
                list = list.child(
                    div()
                        .px(px(10.0))
                        .py(px(10.0))
                        .text_size(crate::theme::caption())
                        .text_color(palette.muted)
                        .child(tr(self.language, "暂无最近打开的文件")),
                );
            } else {
                for (index, path) in recent_files.iter().enumerate() {
                    let target = PathBuf::from(path);
                    list = list.child(self.menu_entry(
                        format!("menu-recent-{index}"),
                        IconName::Archive,
                        path.clone(),
                        None,
                        true,
                        move |this, _, cx| this.open_path(target.clone(), None, cx),
                        cx,
                    ));
                }
            }
            list = list.child(menu_separator(&palette)).child(self.menu_item(
                "submenu-recent-clear",
                IconName::Trash,
                "清除最近记录",
                None,
                !recent_files.is_empty(),
                |this, _, cx| {
                    this.settings.clear_recent();
                    this.save_settings();
                    this.show_toast("已清除最近打开记录", ToastKind::Info, cx);
                },
                cx,
            ));
            self.submenu_container("app-recent-submenu", menu_top, max_height, 360.0)
                .child(list)
                .into_any_element()
        });

        div()
            .id("app-menu-layer")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                div()
                    .id("app-menu-backdrop")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(Hsla::transparent_black())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.close_app_menu();
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .id("app-menu")
                    .absolute()
                    .top(px(menu_top))
                    .left(px(12.0))
                    .w(px(272.0))
                    .max_h(px(max_height))
                    .overflow_y_scroll()
                    .p(px(6.0))
                    .rounded(px(crate::theme::RADIUS_OVERLAY))
                    .bg(palette.bg)
                    .border_1()
                    .border_color(palette.line)
                    .shadow_lg()
                    // 全屏遮罩在 DOM 顺序上先于菜单，但同样会参与命中测试；
                    // 菜单必须阻止其收到点击，否则菜单项会表现为无响应。
                    .occlude()
                    .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
                    .child(panel),
            )
            .when_some(tools_submenu, |this, submenu| this.child(submenu))
            .when_some(recent_submenu, |this, submenu| this.child(submenu))
            .child(
                div()
                    .id("app-menu-toggle-hit")
                    .absolute()
                    .top(px(toggle_top))
                    .left(px(12.0))
                    .w(px(crate::components::ICON_BUTTON_SIZE))
                    .h(px(crate::components::ICON_BUTTON_HEIGHT))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_app_menu();
                        cx.stop_propagation();
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn submenu_container(
        &self,
        id: &'static str,
        menu_top: f32,
        max_height: f32,
        width: f32,
    ) -> Stateful<Div> {
        let palette = self.palette;
        div()
            .id(id)
            .absolute()
            .top(px(menu_top))
            .left(px(292.0))
            .w(px(width))
            .max_h(px(max_height))
            .overflow_y_scroll()
            .p(px(6.0))
            .rounded(px(crate::theme::RADIUS_OVERLAY))
            .bg(palette.bg)
            .border_1()
            .border_color(palette.line)
            .shadow_lg()
            // 菜单与全屏遮罩是兄弟节点；必须 occlude，避免遮罩先收到 mouse_down 把菜单关掉。
            .occlude()
            .on_click(|_, _, cx| cx.stop_propagation())
    }

    fn menu_group(
        &self,
        id: &'static str,
        icon_name: IconName,
        label: &'static str,
        target: MenuSubmenu,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let label = tr(self.language, label);
        let entity = cx.entity();
        let open = self.menu_submenu == Some(target);
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .h(px(crate::components::CONTROL_HEIGHT))
            .px(px(8.0))
            .rounded(px(crate::theme::RADIUS_CONTROL))
            .flex()
            .items_center()
            .gap(px(9.0))
            .text_size(crate::theme::body())
            .cursor_pointer()
            .when(open, |this| {
                this.bg(palette.accent_soft).text_color(palette.accent)
            })
            .when(!open, |this| this.text_color(palette.text))
            .hover(|style| style.bg(palette.hover))
            .on_hover(cx.listener(move |this, hovered, _, cx| {
                if *hovered {
                    // 鼠标停留满 1 秒后才展开，避免划过菜单时意外弹出。
                    this.schedule_submenu(target, cx);
                } else {
                    this.menu_hover_serial = this.menu_hover_serial.wrapping_add(1);
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                // 点击属于明确操作，立即展开。
                this.show_submenu(target);
                cx.notify();
            }))
            .on_a11y_action(AccessibleAction::Click, move |_, _, app| {
                entity.update(app, |this, cx| {
                    this.show_submenu(target);
                    cx.notify();
                });
            })
            .child(icon(icon_name, palette.sub, 16.0))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .child(icon(IconName::ChevronRight, palette.sub, 14.0))
            .into_any_element()
    }

    fn schedule_submenu(&mut self, target: MenuSubmenu, cx: &mut Context<Self>) {
        if self.menu_submenu == Some(target) {
            return;
        }
        self.menu_hover_serial = self.menu_hover_serial.wrapping_add(1);
        let serial = self.menu_hover_serial;
        cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1000))
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.menu_open
                    && this.menu_hover_serial == serial
                    && this.menu_submenu != Some(target)
                {
                    this.menu_submenu = Some(target);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn show_submenu(&mut self, target: MenuSubmenu) {
        self.menu_submenu = Some(target);
        self.menu_hover_serial = self.menu_hover_serial.wrapping_add(1);
    }

    pub(crate) fn close_app_menu(&mut self) {
        self.menu_open = false;
        self.menu_submenu = None;
        self.menu_hover_serial = self.menu_hover_serial.wrapping_add(1);
    }

    #[allow(clippy::too_many_arguments)]
    fn menu_item<F>(
        &self,
        id: &'static str,
        icon_name: IconName,
        label: &'static str,
        shortcut: Option<&'static str>,
        enabled: bool,
        action: F,
        cx: &mut Context<Self>,
    ) -> AnyElement
    where
        F: Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    {
        self.menu_entry(
            id,
            icon_name,
            tr(self.language, label),
            shortcut,
            enabled,
            action,
            cx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn menu_entry<F>(
        &self,
        id: impl Into<ElementId>,
        icon_name: IconName,
        label: impl Into<SharedString>,
        shortcut: Option<&'static str>,
        enabled: bool,
        action: F,
        cx: &mut Context<Self>,
    ) -> AnyElement
    where
        F: Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    {
        let palette = self.palette;
        let label: SharedString = label.into();
        let foreground = if enabled {
            palette.text
        } else {
            palette.sub.opacity(0.55)
        };
        let action = std::rc::Rc::new(action);
        let click_action = action.clone();
        let a11y_action = action;
        let entity = cx.entity();
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label.clone())
            .h(px(crate::components::CONTROL_HEIGHT))
            .px(px(8.0))
            .rounded(px(crate::theme::RADIUS_CONTROL))
            .flex()
            .items_center()
            .gap(px(9.0))
            .text_size(crate::theme::body())
            .when(enabled, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(palette.hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_app_menu();
                        click_action(this, window, cx);
                        cx.notify();
                    }))
                    .on_a11y_action(AccessibleAction::Click, move |_, window, app| {
                        let action = a11y_action.clone();
                        entity.update(app, |this, cx| {
                            this.close_app_menu();
                            action(this, window, cx);
                            cx.notify();
                        });
                    })
            })
            .child(icon(
                icon_name,
                if enabled {
                    palette.sub
                } else {
                    palette.sub.opacity(0.45)
                },
                16.0,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(foreground)
                    .child(label),
            )
            .when_some(shortcut, |this, shortcut| {
                this.child(
                    div()
                        .text_size(crate::theme::caption())
                        .text_color(palette.sub.opacity(0.8))
                        .child(shortcut),
                )
            })
            .into_any_element()
    }
}

fn menu_separator(palette: &Palette) -> AnyElement {
    div()
        .h(px(1.0))
        .my(px(5.0))
        .mx(px(4.0))
        .bg(palette.line)
        .into_any_element()
}
