use nimzip_engine::{ArchiveFormat, Codepage, Settings};

use crate::app::impls::ui::level_label;
use crate::app::*;
use crate::design;
use crate::theme::{Palette, ThemeMode};

impl MainView {
    pub(crate) fn render_settings(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let page = self.settings_page;

        let mut nav = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .w(px(200.0))
            .flex_none()
            .h_full()
            .px(px(12.0))
            .py(px(16.0))
            .bg(palette.panel)
            .border_r_1()
            .border_color(palette.line)
            .child(
                div()
                    .px(px(12.0))
                    .pb(px(10.0))
                    .text_size(crate::theme::font_px(11.5))
                    .text_color(palette.muted)
                    .child(crate::i18n::tr(self.language, "设置")),
            );
        for item in SettingsPage::ALL {
            let selected = item == page;
            nav = nav.child(
                div()
                    .id(item.label(self.language))
                    .role(Role::Button)
                    .aria_label(item.label(self.language))
                    .h(px(36.0))
                    .px(px(14.0))
                    .rounded(px(if palette.mica { 4.0 } else { 9.0 }))
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .cursor_pointer()
                    .text_size(crate::theme::font_px(13.5))
                    .font_weight(if selected && !palette.mica {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .when(selected, |this| {
                        if palette.mica {
                            this.bg(palette.hover).text_color(palette.text)
                        } else {
                            this.bg(palette.accent_soft).text_color(palette.accent)
                        }
                    })
                    .when(selected && palette.mica, |this| {
                        this.child(
                            div()
                                .absolute()
                                .left(px(2.0))
                                .top(px(10.0))
                                .w(px(3.0))
                                .h(px(16.0))
                                .rounded(px(2.0))
                                .bg(palette.accent),
                        )
                    })
                    .when(!selected, |this| {
                        this.text_color(palette.sub)
                            .hover(|style| style.bg(palette.hover).text_color(palette.text))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings_page = item;
                        this.settings.ui.settings_page = SettingsPage::ALL
                            .iter()
                            .position(|candidate| *candidate == item)
                            .unwrap_or(0);
                        this.save_settings();
                        cx.notify();
                    }))
                    .child(icon(
                        settings_page_icon(item),
                        if selected {
                            palette.accent
                        } else {
                            palette.muted
                        },
                        16.0,
                    ))
                    .child(item.label(self.language)),
            );
        }

        let mut footer = None;
        let body = match page {
            SettingsPage::General => self.settings_general(cx),
            SettingsPage::Associations => {
                footer = Some(self.associations_footer(cx));
                self.settings_associations(cx)
            }
            SettingsPage::ContextMenu => self.settings_context_menu(cx),
            SettingsPage::Extraction => self.settings_extraction(cx),
            SettingsPage::Compression => self.settings_compression(cx),
            SettingsPage::View => self.settings_view(cx),
            SettingsPage::Security => self.settings_security(cx),
            SettingsPage::Advanced => self.settings_advanced(cx),
            SettingsPage::Language => self.settings_language(cx),
        };

        let done = design::done_button(
            &palette,
            cx.listener(|this, _, _, cx| {
                this.page = Page::Main;
                cx.notify();
            }),
        );

        let trailing = if page == SettingsPage::Associations {
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .child(design::link_action(
                    "assoc-open-settings",
                    "Windows - 设置默认程序",
                    &palette,
                    cx.listener(|this, _, _, cx| {
                        if let Err(error) = open::that("ms-settings:defaultapps") {
                            this.show_toast(
                                format!("无法打开系统设置：{error}"),
                                ToastKind::Error,
                                cx,
                            );
                        }
                    }),
                ))
                .child(done)
                .into_any_element()
        } else {
            done
        };

        let content = if let Some(footer) = footer {
            design::settings_content_with_footer(
                &palette,
                page.label(self.language),
                settings_page_subtitle(page, self.language),
                trailing,
                body,
                footer,
            )
        } else {
            design::settings_content(
                &palette,
                page.label(self.language),
                settings_page_subtitle(page, self.language),
                trailing,
                body,
            )
        };

        div()
            .flex()
            .flex_1()
            .min_h_0()
            .size_full()
            .child(nav)
            .child(content)
            .into_any_element()
    }

    fn settings_general(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let esc = self.settings.general.esc_closes_window;
        let grid = self.settings.general.show_grid_lines;
        let history = self.settings.advanced.history;
        let single_click = self.settings.general.single_click_open;
        let full_row = self.settings.general.full_row_select;

        let card = design::setting_card(&palette)
            .flex()
            .flex_col()
            .child(design::option_row(
                &palette,
                false,
                self.setting_check_box(
                    "settings-esc-close",
                    "Esc 关闭窗口",
                    esc,
                    |settings, value| settings.general.esc_closes_window = value,
                    cx,
                ),
                "Esc 关闭窗口",
                "按下 Esc 直接关闭当前窗口",
                None,
            ))
            .child(design::option_row(
                &palette,
                false,
                self.setting_check_box(
                    "settings-grid-lines",
                    "文件列表显示网格线",
                    grid,
                    |settings, value| settings.general.show_grid_lines = value,
                    cx,
                ),
                "文件列表显示网格线",
                "在条目之间绘制分隔线",
                None,
            ))
            .child(design::option_row(
                &palette,
                false,
                self.setting_check_box(
                    "settings-history",
                    "记住最近打开文件",
                    history,
                    |settings, value| settings.advanced.history = value,
                    cx,
                ),
                "记住最近打开文件",
                "在开始页下拉菜单中显示最近打开的文件（最多 5 个）",
                None,
            ))
            .child(design::option_row(
                &palette,
                false,
                self.setting_check_box(
                    "settings-single-click",
                    "单击打开文件",
                    single_click,
                    |settings, value| settings.general.single_click_open = value,
                    cx,
                ),
                "单击打开文件",
                "关闭时使用双击打开，避免误触",
                None,
            ))
            .child(design::option_row(
                &palette,
                false,
                self.setting_check_box(
                    "settings-full-row",
                    "整行选中",
                    full_row,
                    |settings, value| settings.general.full_row_select = value,
                    cx,
                ),
                "整行选中",
                "点击行内任意位置选中条目",
                None,
            ))
            .child(design::option_row(
                &palette,
                true,
                div().w(px(18.0)).flex_none().into_any_element(),
                "工具栏按钮",
                "隐藏不常用的按钮，保留核心工作流",
                Some(
                    design::secondary_icon_action(
                        "settings-toolbar-buttons",
                        "工具栏按钮…",
                        IconName::SlidersHorizontal,
                        palette.text,
                        cx.listener(|this, _, _, cx| {
                            this.open_toolbar_settings(cx);
                        }),
                    )
                    .into_any_element(),
                ),
            ));

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(card)
            .into_any_element()
    }

    fn settings_extraction(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let smart = self.settings.extraction.smart;
        let parallel = self.settings.extraction.parallel;
        let nested = self.settings.extraction.nested_enabled;
        let depth = self.settings.extraction.nested_depth.clamp(1, 9);
        let memory = self.settings.extraction.max_memory_mb;
        let codepage = Codepage::from_name(&self.settings.extraction.default_codepage);

        let smart_row = design::option_row(
            &palette,
            false,
            design::check_box(
                &palette,
                "settings-smart",
                smart,
                false,
                "智能解压",
                cx.listener(|this, _, _, cx| {
                    this.settings.extraction.smart = !this.settings.extraction.smart;
                    this.save_settings();
                    cx.notify();
                }),
            ),
            "智能解压",
            "无独立根目录时自动建同名文件夹",
            None,
        );
        let parallel_row = design::option_row(
            &palette,
            false,
            design::check_box(
                &palette,
                "settings-parallel",
                parallel,
                false,
                "并行解压",
                cx.listener(|this, _, _, cx| {
                    this.settings.extraction.parallel = !this.settings.extraction.parallel;
                    this.save_settings();
                    cx.notify();
                }),
            ),
            "并行解压",
            "多核同时处理多个压缩包",
            None,
        );
        let nested_row = design::option_row(
            &palette,
            true,
            design::check_box(
                &palette,
                "settings-nested",
                nested,
                false,
                "递归展开嵌套压缩包",
                cx.listener(|this, _, _, cx| {
                    this.settings.extraction.nested_enabled =
                        !this.settings.extraction.nested_enabled;
                    this.save_settings();
                    cx.notify();
                }),
            ),
            "递归展开嵌套压缩包",
            "解压结果中的压缩包继续自动展开",
            None,
        );
        let toggle_card = design::setting_card(&palette)
            .flex()
            .flex_col()
            .child(smart_row)
            .child(parallel_row)
            .child(nested_row);

        let depth_stepper = design::stepper(
            &palette,
            "settings-depth",
            depth.to_string(),
            cx.listener(move |this, _, _, cx| {
                this.settings.extraction.nested_enabled = true;
                this.settings.extraction.nested_depth = depth.saturating_sub(1).max(1);
                this.save_settings();
                cx.notify();
            }),
            cx.listener(move |this, _, _, cx| {
                this.settings.extraction.nested_enabled = true;
                this.settings.extraction.nested_depth = (depth + 1).min(9);
                this.save_settings();
                cx.notify();
            }),
        );

        let memory_control = self.setting_select(
            "settings-memory",
            memory_label(memory),
            130.0,
            DropdownKind::SettingsMemory,
            cx,
        );
        let codepage_control = self.setting_select(
            "settings-codepage",
            codepage.label().to_string(),
            170.0,
            DropdownKind::SettingsCodepage,
            cx,
        );

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(toggle_card)
            .child(
                design::setting_card(&palette)
                    .flex()
                    .flex_col()
                    .child(design::field_row(
                        &palette,
                        false,
                        "嵌套展开深度",
                        depth_stepper,
                    ))
                    .child(design::field_row(
                        &palette,
                        false,
                        "解压内存上限",
                        memory_control,
                    ))
                    .child(design::field_row(
                        &palette,
                        true,
                        "文件名代码页",
                        codepage_control,
                    )),
            )
            .into_any_element()
    }

    fn settings_compression(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let include_root = self.settings.compression.include_root;
        let test_after = self.settings.compression.test_after;
        let level = self.settings.compression.default_level.min(9);
        let format = ArchiveFormat::from_name(&self.settings.compression.default_format)
            .filter(|format| format.can_create())
            .unwrap_or(ArchiveFormat::Zip);
        let exclude = self.settings.compression.exclude.clone();

        let level_control = self.setting_select(
            "settings-default-level",
            format!("{level} · {}", level_label(level)),
            150.0,
            DropdownKind::SettingsLevel,
            cx,
        );
        let format_control = self.setting_select(
            "settings-default-format",
            format.label().to_string(),
            150.0,
            DropdownKind::SettingsFormat,
            cx,
        );

        let toggles = design::setting_card(&palette)
            .flex()
            .flex_col()
            .child(design::option_row(
                &palette,
                false,
                self.setting_check_box(
                    "settings-include-root",
                    "包含顶层文件夹",
                    include_root,
                    |settings, value| settings.compression.include_root = value,
                    cx,
                ),
                "包含顶层文件夹",
                "压缩时保留所选目录本身",
                None,
            ))
            .child(design::option_row(
                &palette,
                true,
                self.setting_check_box(
                    "settings-test-after",
                    "压缩完成后自动测试",
                    test_after,
                    |settings, value| settings.compression.test_after = value,
                    cx,
                ),
                "压缩完成后自动测试",
                "创建后逐条校验 CRC",
                None,
            ));

        let fields = design::setting_card(&palette)
            .flex()
            .flex_col()
            .child(design::field_row(
                &palette,
                false,
                "默认压缩级别",
                level_control,
            ))
            .child(design::field_row(
                &palette,
                false,
                "默认格式",
                format_control,
            ))
            .child(
                div()
                    .py(px(14.0))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(design::field_label(&palette, "默认排除规则"))
                    .child(
                        div()
                            .w_full()
                            .min_h(px(38.0))
                            .px(px(12.0))
                            .py(px(9.0))
                            .rounded(px(9.0))
                            .border_1()
                            .border_color(palette.line)
                            .bg(palette.panel)
                            .font_family("Cascadia Mono")
                            .text_size(crate::theme::font_px(12.0))
                            .text_color(palette.sub)
                            .child(exclude),
                    )
                    .child(
                        div()
                            .text_size(crate::theme::font_px(12.0))
                            .text_color(palette.muted)
                            .child("在新建压缩对话中可逐次覆盖；支持 ; , 换行分隔。"),
                    ),
            );

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(toggles)
            .child(fields)
            .into_any_element()
    }

    fn settings_view(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let preview = self.settings.view.preview_enabled;
        let tree = self.settings.view.show_tree;
        let comment = self.settings.view.show_comment;
        let status = self.settings.view.show_status;
        let mica = self.settings.view.mica;
        let theme_mode = ThemeMode::from_setting(&self.settings.view.theme);
        let icon_size = self.settings.view.toolbar_icon_size.clamp(14.0, 24.0);
        let bar_height = self.settings.view.toolbar_height.clamp(40.0, 64.0);
        let font_size = self.settings.view.font_size.clamp(11.0, 17.0);
        let skin = self.skin;

        let theme_control = self.setting_select(
            "settings-view-theme",
            crate::i18n::tr(self.language, theme_mode.label()).to_string(),
            150.0,
            DropdownKind::SettingsTheme,
            cx,
        );
        let skin_control = self.setting_select(
            "settings-skin",
            skin.label(self.language).to_string(),
            150.0,
            DropdownKind::SettingsSkin,
            cx,
        );

        let icon_minus = self.setting_step(
            "settings-icon-minus",
            "−",
            cx.listener(move |this, _, _, cx| {
                this.settings.view.toolbar_icon_size = (icon_size - 2.0).max(14.0);
                this.save_settings();
                cx.notify();
            }),
        );
        let icon_plus = self.setting_step(
            "settings-icon-plus",
            "＋",
            cx.listener(move |this, _, _, cx| {
                this.settings.view.toolbar_icon_size = (icon_size + 2.0).min(24.0);
                this.save_settings();
                cx.notify();
            }),
        );
        let icon_control = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(icon_minus)
            .child(
                div()
                    .w(px(56.0))
                    .text_center()
                    .font_family("Cascadia Mono")
                    .text_size(crate::theme::font_px(12.5))
                    .text_color(palette.sub)
                    .child(format!("{icon_size:.0} px")),
            )
            .child(icon_plus)
            .child(
                div()
                    .ml(px(6.0))
                    .child(icon(IconName::Folder, palette.sub, icon_size)),
            )
            .into_any_element();
        let bar_minus = self.setting_step(
            "settings-toolbar-height-minus",
            "−",
            cx.listener(move |this, _, _, cx| {
                this.settings.view.toolbar_height = (bar_height - 4.0).max(40.0);
                this.save_settings();
                cx.notify();
            }),
        );
        let bar_plus = self.setting_step(
            "settings-toolbar-height-plus",
            "＋",
            cx.listener(move |this, _, _, cx| {
                this.settings.view.toolbar_height = (bar_height + 4.0).min(64.0);
                this.save_settings();
                cx.notify();
            }),
        );
        let bar_control = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(bar_minus)
            .child(
                div()
                    .w(px(56.0))
                    .text_center()
                    .font_family("Cascadia Mono")
                    .text_size(crate::theme::font_px(12.5))
                    .text_color(palette.sub)
                    .child(format!("{bar_height:.0} px")),
            )
            .child(bar_plus)
            .into_any_element();

        let font_minus = self.setting_step(
            "settings-font-size-minus",
            "−",
            cx.listener(move |this, _, window, cx| {
                this.apply_ui_font_size((font_size - 1.0).max(11.0), window, cx);
            }),
        );
        let font_plus = self.setting_step(
            "settings-font-size-plus",
            "＋",
            cx.listener(move |this, _, window, cx| {
                this.apply_ui_font_size((font_size + 1.0).min(17.0), window, cx);
            }),
        );
        let font_control = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(font_minus)
            .child(
                div()
                    .w(px(56.0))
                    .text_center()
                    .font_family("Cascadia Mono")
                    .text_size(crate::theme::font_px(12.5))
                    .text_color(palette.sub)
                    .child(format!("{font_size:.0} px")),
            )
            .child(font_plus)
            .child(
                div()
                    .ml(px(6.0))
                    .text_size(px(font_size))
                    .text_color(palette.text)
                    .child("Aa 示例"),
            )
            .into_any_element();

        let custom_titlebar = self.settings.view.use_custom_titlebar;
        let mica_entity = cx.entity();
        let toggles = design::setting_card(&palette)
            .flex()
            .flex_col()
            .child(design::option_row(
                &palette,
                false,
                design::check_box(
                    &palette,
                    "settings-mica",
                    mica,
                    false,
                    crate::i18n::tr(self.language, "Mica 主题"),
                    move |_, _, app| {
                        let next = !mica;
                        if next {
                            crate::mica::refresh_wallpaper_tint();
                        }
                        mica_entity.update(app, |this, cx| {
                            this.settings.view.mica = next;
                            this.reapply_theme(cx);
                        });
                    },
                ),
                crate::i18n::tr(self.language, "Mica 主题"),
                "WinUI Gallery 风格 Mica 材质：壁纸色调底色、WinUI 圆角/强调色；浅色/深色跟随当前主题",
                None,
            ))
            .child(design::option_row(
                &palette,
                false,
                self.setting_check_box(
                    "settings-custom-titlebar",
                    "使用自定义标题栏",
                    custom_titlebar,
                    |settings, value| settings.view.use_custom_titlebar = value,
                    cx,
                ),
                "使用自定义标题栏",
                "关闭后使用系统标题栏；新窗口立即生效，当前窗口重启后生效",
                None,
            ))
            .child(design::option_row(
                &palette,
                false,
                self.setting_check_box(
                    "settings-preview",
                    "显示图像预览面板",
                    preview,
                    |settings, value| settings.view.preview_enabled = value,
                    cx,
                ),
                "显示图像预览面板",
                "在右侧显示选中图像的预览",
                None,
            ))
            .child(design::option_row(
                &palette,
                false,
                self.setting_check_box(
                    "settings-tree",
                    "显示树形导航",
                    tree,
                    |settings, value| settings.view.show_tree = value,
                    cx,
                ),
                "显示树形导航",
                "显示左侧目录树导航",
                None,
            ))
            .child(design::option_row(
                &palette,
                false,
                self.setting_check_box(
                    "settings-comment",
                    "显示注释面板",
                    comment,
                    |settings, value| settings.view.show_comment = value,
                    cx,
                ),
                "显示注释面板",
                "显示归档注释面板",
                None,
            ))
            .child(design::option_row(
                &palette,
                true,
                self.setting_check_box(
                    "settings-status",
                    "显示状态栏",
                    status,
                    |settings, value| settings.view.show_status = value,
                    cx,
                ),
                "显示状态栏",
                "显示底部状态栏",
                None,
            ));

        let fields = design::setting_card(&palette)
            .flex()
            .flex_col()
            .child(design::field_row(
                &palette,
                false,
                crate::i18n::tr(self.language, "外观"),
                theme_control,
            ))
            .child(design::field_row(
                &palette,
                false,
                crate::i18n::tr(self.language, "主题皮肤"),
                skin_control,
            ))
            .child(design::field_row(
                &palette,
                false,
                crate::i18n::tr(self.language, "工具栏图标大小"),
                icon_control,
            ))
            .child(design::field_row(
                &palette,
                false,
                crate::i18n::tr(self.language, "工具栏高度"),
                bar_control,
            ))
            .child(design::field_row(
                &palette,
                true,
                crate::i18n::tr(self.language, "界面字号"),
                font_control,
            ));

        let columns = design::setting_card(&palette)
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(crate::theme::font_px(13.5))
                            .text_color(palette.text)
                            .child(crate::i18n::tr(self.language, "列设置")),
                    )
                    .child(
                        div()
                            .text_size(crate::theme::font_px(12.5))
                            .text_color(palette.muted)
                            .child("选择显示列、调整列宽与顺序"),
                    ),
            )
            .child(design::secondary_icon_action(
                "settings-columns",
                crate::i18n::tr(self.language, "列设置…"),
                IconName::Columns3,
                palette.text,
                cx.listener(|this, _, _, cx| this.open_columns(cx)),
            ));

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(toggles)
            .child(fields)
            .child(columns)
            .into_any_element()
    }

    fn settings_security(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let motw = self.settings.security.propagate_motw;
        let scan = self.settings.security.amsi_scan;
        let buffer = self.settings.security.scan_buffer_kb;

        let card = design::setting_card(&palette)
            .flex()
            .flex_col()
            .child(design::option_row(
                &palette,
                false,
                self.setting_check_box(
                    "settings-motw",
                    "解压时传播 Mark-of-the-Web（网络来源标记）",
                    motw,
                    |settings, value| settings.security.propagate_motw = value,
                    cx,
                ),
                "传播 Mark-of-the-Web",
                "将网络来源标记写入解压后的文件",
                None,
            ))
            .child(design::option_row(
                &palette,
                false,
                self.setting_check_box(
                    "settings-amsi",
                    "使用 AMSI 扫描解压内容",
                    scan,
                    |settings, value| settings.security.amsi_scan = value,
                    cx,
                ),
                "使用 AMSI 扫描解压内容",
                "解压后调用 Windows AMSI 检测恶意内容",
                None,
            ))
            .child(design::field_row(
                &palette,
                true,
                "扫描缓冲区",
                chip(format!("{buffer} KB"), &palette).into_any_element(),
            ));

        let note = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(14.0))
            .py(px(12.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(palette.line)
            .bg(palette.panel)
            .text_size(crate::theme::font_px(12.5))
            .text_color(palette.muted)
            .child(icon(IconName::Lock, palette.accent, 15.0))
            .child("密码本使用 Windows DPAPI 加密保存，仅当前用户可解密。");

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(card)
            .child(note)
            .into_any_element()
    }

    fn settings_advanced(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let recycle = self.settings.advanced.recycle_bin;
        let history = self.settings.advanced.history;
        let long_paths = self.settings.advanced.long_paths;
        let preallocate = self.settings.advanced.preallocate;
        let temp_dir = self.settings.advanced.temp_dir.clone();
        let data_dir = nimzip_engine::settings::data_dir();
        let temp_label = if temp_dir.trim().is_empty() {
            "%TEMP%\\NimZip".to_string()
        } else {
            temp_dir
        };

        let toggles = design::setting_card(&palette)
            .flex()
            .flex_col()
            .child(design::option_row(
                &palette,
                false,
                self.setting_check_box(
                    "settings-recycle",
                    "删除文件时移入回收站",
                    recycle,
                    |settings, value| settings.advanced.recycle_bin = value,
                    cx,
                ),
                "删除文件时移入回收站",
                "避免误删后无法恢复",
                None,
            ))
            .child(design::option_row(
                &palette,
                false,
                self.setting_check_box(
                    "settings-history-advanced",
                    "记住最近打开文件",
                    history,
                    |settings, value| settings.advanced.history = value,
                    cx,
                ),
                "记住最近打开文件",
                "在开始页下拉菜单中显示最近打开的文件（最多 5 个）",
                None,
            ))
            .child(design::option_row(
                &palette,
                false,
                self.setting_check_box(
                    "settings-long-paths",
                    "启用 Windows 长路径",
                    long_paths,
                    |settings, value| settings.advanced.long_paths = value,
                    cx,
                ),
                "启用 Windows 长路径",
                "支持超过 260 字符的深层目录",
                Some(design::tag(
                    &palette,
                    "已默认开启",
                    design::TagTone::Neutral,
                )),
            ))
            .child(design::option_row(
                &palette,
                true,
                self.setting_check_box(
                    "settings-preallocate",
                    "解压前预分配磁盘空间",
                    preallocate,
                    |settings, value| settings.advanced.preallocate = value,
                    cx,
                ),
                "解压前预分配磁盘空间",
                "减少磁盘碎片",
                None,
            ));

        let paths = design::setting_card(&palette)
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(14.0))
                    .py(px(14.0))
                    .border_b_1()
                    .border_color(palette.line)
                    .child(
                        div()
                            .text_size(crate::theme::font_px(13.5))
                            .text_color(palette.sub)
                            .child("临时文件目录"),
                    )
                    .child(
                        div()
                            .max_w(px(420.0))
                            .truncate()
                            .font_family("Cascadia Mono")
                            .text_size(crate::theme::font_px(12.0))
                            .text_color(palette.muted)
                            .child(temp_label),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(14.0))
                    .py(px(14.0))
                    .child(
                        div()
                            .text_size(crate::theme::font_px(13.5))
                            .text_color(palette.sub)
                            .child("配置与密码本目录"),
                    )
                    .child(
                        div()
                            .max_w(px(420.0))
                            .truncate()
                            .font_family("Cascadia Mono")
                            .text_size(crate::theme::font_px(12.0))
                            .text_color(palette.muted)
                            .child(data_dir.display().to_string()),
                    ),
            );

        let actions = design::setting_card(&palette)
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(crate::theme::font_px(13.5))
                            .text_color(palette.text)
                            .child("设置文件"),
                    )
                    .child(
                        div()
                            .text_size(crate::theme::font_px(12.5))
                            .text_color(palette.muted)
                            .child("导出备份或从文件恢复全部设置"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(design::secondary_icon_action(
                        "settings-export",
                        "导出设置",
                        IconName::Upload,
                        palette.text,
                        cx.listener(|this, _, window, cx| {
                            this.export_settings(window, cx);
                        }),
                    ))
                    .child(design::secondary_icon_action(
                        "settings-import",
                        "导入设置",
                        IconName::Download,
                        palette.text,
                        cx.listener(|this, _, window, cx| {
                            this.import_settings(window, cx);
                        }),
                    )),
            );

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(toggles)
            .child(paths)
            .child(actions)
            .into_any_element()
    }

    fn settings_language(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let current = self.language;
        let tab = |label: &'static str, value: Language, id: &'static str| {
            let selected = current == value;
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .h(px(34.0))
                .px(px(14.0))
                .rounded(px(9.0))
                .border_1()
                .border_color(if selected {
                    palette.accent_soft
                } else {
                    palette.line
                })
                .flex()
                .items_center()
                .cursor_pointer()
                .text_size(crate::theme::font_px(13.0))
                .when(selected, |this| {
                    this.bg(palette.accent_soft)
                        .text_color(palette.accent)
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(!selected, |this| {
                    this.bg(palette.bg)
                        .text_color(palette.text)
                        .hover(|style| style.border_color(palette.accent))
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_language(value, window, cx);
                }))
                .child(label)
        };

        let language_card = design::setting_card(&palette)
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(design::panel_title(
                &palette,
                IconName::Globe,
                crate::i18n::tr(current, "界面语言"),
            ))
            .child(
                div().flex().flex_wrap().gap(px(6.0)).children(
                    Language::ALL
                        .into_iter()
                        .map(|language| tab(language.label(), language, language.as_setting())),
                ),
            )
            .child(
                div()
                    .text_size(crate::theme::font_px(12.5))
                    .text_color(palette.muted)
                    .child(crate::i18n::tr(
                        current,
                        "切换后立即生效并保存；已内置 20 种语言界面。",
                    )),
            );

        let pack_card = design::setting_card(&palette)
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(design::panel_title(
                &palette,
                IconName::BookOpen,
                crate::i18n::tr(current, "语言包"),
            ))
            .child(
                div()
                    .text_size(crate::theme::font_px(12.5))
                    .line_height(relative(1.6))
                    .text_color(palette.muted)
                    .child(crate::i18n::tr(
                        current,
                        "20 种语言包已内置；切换后菜单、设置导航与列表表头同步更新。",
                    )),
            );

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(language_card)
            .child(pack_card)
            .into_any_element()
    }

    fn shell_options(&self) -> nimzip_shell::ShellOptions {
        nimzip_shell::ShellOptions {
            exe: std::env::current_exe().unwrap_or_default(),
            scope: nimzip_shell::ShellScope::CurrentUser,
            archive_extensions: self.settings.integration.associations.clone(),
            extract_verbs: self.settings.integration.extract_verbs.clone(),
            compress_verbs: self.settings.integration.compress_verbs.clone(),
            empty_verbs: self.settings.integration.empty_verbs.clone(),
            double_click: self.settings.integration.double_click.clone(),
            double_click_folder: self.settings.integration.double_click_folder.clone(),
            cascade: self.settings.integration.cascade,
            hide_icon: self.settings.integration.hide_icon,
        }
    }

    /// 菜单设置变更后立即重写注册表（仅在已安装时）。
    fn apply_shell_menu(&mut self, cx: &mut Context<Self>) {
        if self.settings.integration.disable_menu {
            return;
        }
        if !nimzip_shell::is_installed(nimzip_shell::ShellScope::CurrentUser) {
            return;
        }
        if let Err(error) = nimzip_shell::install(&self.shell_options()) {
            self.show_toast(format!("更新右键菜单失败：{error}"), ToastKind::Error, cx);
        }
    }

    /// 双击动作变更后，更新已经注册的 ProgID 默认命令。
    fn apply_double_click_action(&mut self, cx: &mut Context<Self>) {
        let exe = std::env::current_exe().unwrap_or_default();
        let action = self.settings.integration.double_click.clone();
        let folder = self.settings.integration.double_click_folder.clone();
        let mut failed = 0usize;
        for (_, extensions) in ASSOCIATION_GROUPS.iter() {
            for extension in extensions.iter() {
                if !nimzip_shell::is_association_registered(
                    nimzip_shell::ShellScope::CurrentUser,
                    extension,
                ) {
                    continue;
                }
                if nimzip_shell::register_association_with_action(
                    nimzip_shell::ShellScope::CurrentUser,
                    extension,
                    &exe,
                    &action,
                    &folder,
                )
                .is_err()
                {
                    failed += 1;
                }
            }
        }
        if failed > 0 {
            self.show_toast(
                format!("有 {failed} 个文件关联的双击动作更新失败"),
                ToastKind::Error,
                cx,
            );
        }
    }

    fn settings_context_menu(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let installed = nimzip_shell::is_installed(nimzip_shell::ShellScope::CurrentUser);
        let preview_installed = nimzip_shell::preview_handler_installed();
        let disabled = self.settings.integration.disable_menu;
        let double_click = self.settings.integration.double_click.clone();
        let custom_folder = self.settings.integration.double_click_folder.clone();
        let extract_keys = self.settings.integration.extract_verbs.clone();
        let compress_keys = self.settings.integration.compress_verbs.clone();
        let empty_keys = self.settings.integration.empty_verbs.clone();
        let cascade = self.settings.integration.cascade;
        let hide_icon = self.settings.integration.hide_icon;

        let status_card = design::card(&palette)
            .flex()
            .items_center()
            .gap(px(14.0))
            .px(px(20.0))
            .py(px(16.0))
            .child(
                div()
                    .w(px(44.0))
                    .h(px(44.0))
                    .flex_none()
                    .rounded(px(11.0))
                    .bg(if installed {
                        palette.ok_soft
                    } else {
                        palette.panel
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(
                        IconName::Menu,
                        if installed { palette.ok } else { palette.muted },
                        21.0,
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(crate::theme::font_px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(palette.text)
                            .child("经典资源管理器菜单"),
                    )
                    .child(
                        div()
                            .text_size(crate::theme::font_px(12.5))
                            .text_color(palette.muted)
                            .child(if disabled {
                                "右键菜单已禁用；文件关联仍然保留"
                            } else {
                                "写入 HKCU，无需管理员权限；更改即时生效"
                            }),
                    ),
            )
            .child(design::tag(
                &palette,
                if installed { "已注册" } else { "未注册" },
                if installed {
                    design::TagTone::Ok
                } else {
                    design::TagTone::Neutral
                },
            ))
            .child(if installed {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(design::link_action(
                        "cm-refresh",
                        "更新菜单",
                        &palette,
                        cx.listener(|this, _, _, cx| {
                            this.settings.integration.disable_menu = false;
                            this.save_settings();
                            match nimzip_shell::install(&this.shell_options()) {
                                Ok(()) => this.show_toast(
                                    "右键菜单已更新为当前版本",
                                    ToastKind::Success,
                                    cx,
                                ),
                                Err(error) => this.show_toast(
                                    format!("更新失败：{error}"),
                                    ToastKind::Error,
                                    cx,
                                ),
                            }
                            cx.notify();
                        }),
                    ))
                    .child(design::secondary_action(
                        "cm-uninstall",
                        "卸载",
                        cx.listener(|this, _, _, cx| {
                            this.settings.integration.disable_menu = true;
                            this.save_settings();
                            match nimzip_shell::uninstall(nimzip_shell::ShellScope::CurrentUser) {
                                Ok(()) => this.show_toast(
                                    "已移除 NimZip 右键菜单",
                                    ToastKind::Success,
                                    cx,
                                ),
                                Err(error) => this.show_toast(
                                    format!("卸载失败：{error}"),
                                    ToastKind::Error,
                                    cx,
                                ),
                            }
                            cx.notify();
                        }),
                    ))
                    .into_any_element()
            } else {
                design::primary_icon_action(
                    "cm-install",
                    "注册右键菜单",
                    IconName::ShieldCheck,
                    &palette,
                    false,
                    cx.listener(|this, _, _, cx| {
                        this.settings.integration.disable_menu = false;
                        this.save_settings();
                        match nimzip_shell::install(&this.shell_options()) {
                            Ok(()) => this.show_toast(
                                "已注册 NimZip 经典右键菜单",
                                ToastKind::Success,
                                cx,
                            ),
                            Err(error) => {
                                this.show_toast(format!("注册失败：{error}"), ToastKind::Error, cx)
                            }
                        }
                        cx.notify();
                    }),
                )
                .into_any_element()
            });

        let mut double_rows = div().flex().flex_col();
        for (key, title, description) in DOUBLE_CLICK_OPTIONS.iter() {
            let key_owned = key.to_string();
            double_rows = double_rows.child(radio_row(
                &palette,
                format!("cm-double-{key}"),
                double_click == *key,
                title,
                description,
                cx.listener(move |this, _, _, cx| {
                    this.settings.integration.double_click = key_owned.clone();
                    this.save_settings();
                    this.apply_double_click_action(cx);
                    cx.notify();
                }),
            ));
        }
        let custom_line = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .mx(px(20.0))
            .mb(px(12.0))
            .ml(px(48.0))
            .px(px(14.0))
            .py(px(10.0))
            .rounded(px(9.0))
            .bg(palette.panel)
            .when(double_click != "custom", |this| this.opacity(0.45))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family("Cascadia Mono")
                    .text_size(crate::theme::font_px(12.5))
                    .text_color(if custom_folder.trim().is_empty() {
                        palette.muted
                    } else {
                        palette.text
                    })
                    .child(if custom_folder.trim().is_empty() {
                        "尚未选择目录".to_string()
                    } else {
                        custom_folder.clone()
                    }),
            )
            .child(design::secondary_action(
                "cm-folder",
                "浏览…",
                cx.listener(|this, _, _, cx| {
                    let Some(folder) =
                        crate::components::file_dialog(|| rfd::FileDialog::new().pick_folder())
                    else {
                        return;
                    };
                    this.settings.integration.double_click_folder = folder.display().to_string();
                    this.settings.integration.double_click = "custom".to_string();
                    this.save_settings();
                    this.apply_double_click_action(cx);
                    cx.notify();
                }),
            ));
        let double_card = design::card(&palette)
            .flex()
            .flex_col()
            .child(double_rows)
            .child(custom_line);

        let extract_all: Vec<String> = EXTRACT_MENU_OPTIONS
            .iter()
            .map(|(key, _)| key.to_string())
            .collect();
        let extract_head = menu_group_head(
            &palette,
            "解压菜单",
            "勾选后在压缩包的右键菜单中显示",
            vec![
                design::link_action(
                    "cm-extract-all",
                    "全选",
                    &palette,
                    cx.listener(move |this, _, _, cx| {
                        this.settings.integration.extract_verbs = extract_all.clone();
                        this.save_settings();
                        this.apply_shell_menu(cx);
                        cx.notify();
                    }),
                ),
                design::link_action(
                    "cm-extract-none",
                    "清空",
                    &palette,
                    cx.listener(|this, _, _, cx| {
                        this.settings.integration.extract_verbs.clear();
                        this.save_settings();
                        this.apply_shell_menu(cx);
                        cx.notify();
                    }),
                ),
            ],
        );
        let mut extract_rows = Vec::new();
        for (key, title) in EXTRACT_MENU_OPTIONS.iter() {
            let key_owned = key.to_string();
            let checked = extract_keys.iter().any(|item| item.as_str() == *key);
            extract_rows.push(menu_check_row(
                &palette,
                format!("cm-extract-{key}"),
                checked,
                title,
                cx.listener(move |this, _, _, cx| {
                    let position = this
                        .settings
                        .integration
                        .extract_verbs
                        .iter()
                        .position(|item| item == &key_owned);
                    if let Some(position) = position {
                        this.settings.integration.extract_verbs.remove(position);
                    } else {
                        this.settings
                            .integration
                            .extract_verbs
                            .push(key_owned.clone());
                    }
                    this.save_settings();
                    this.apply_shell_menu(cx);
                    cx.notify();
                }),
            ));
        }
        extract_rows.push(menu_check_row(
            &palette,
            "cm-extract-preview",
            preview_installed,
            "预览压缩文件",
            cx.listener(|this, _, _, cx| {
                if nimzip_shell::preview_handler_installed() {
                    match nimzip_shell::unregister_preview_handler() {
                        Ok(()) => this.show_toast("已卸载预览处理器", ToastKind::Success, cx),
                        Err(error) => {
                            this.show_toast(format!("卸载失败：{error}"), ToastKind::Error, cx)
                        }
                    }
                } else {
                    let dll = std::env::current_exe()
                        .ok()
                        .and_then(|exe| exe.parent().map(|parent| parent.join("nimzip_shell.dll")));
                    let Some(dll) = dll else {
                        this.show_toast("无法定位程序目录", ToastKind::Error, cx);
                        return;
                    };
                    if !dll.is_file() {
                        this.show_toast(
                            format!("找不到预览处理器 DLL：{}", dll.display()),
                            ToastKind::Error,
                            cx,
                        );
                        return;
                    }
                    match nimzip_shell::register_preview_handler(&dll) {
                        Ok(()) => {
                            this.show_toast("已注册 NimZip 预览处理器", ToastKind::Success, cx)
                        }
                        Err(error) => this.show_toast(
                            format!("注册预览处理器失败：{error}"),
                            ToastKind::Error,
                            cx,
                        ),
                    }
                }
                cx.notify();
            }),
        ));
        let extract_card = design::card(&palette).child(menu_grid(extract_rows, 2));

        let compress_all: Vec<String> = COMPRESS_MENU_OPTIONS
            .iter()
            .map(|(key, _)| key.to_string())
            .collect();
        let compress_head = menu_group_head(
            &palette,
            "压缩菜单",
            "勾选后在文件 / 文件夹的右键菜单中显示",
            vec![
                design::link_action(
                    "cm-compress-all",
                    "全选",
                    &palette,
                    cx.listener(move |this, _, _, cx| {
                        this.settings.integration.compress_verbs = compress_all.clone();
                        this.save_settings();
                        this.apply_shell_menu(cx);
                        cx.notify();
                    }),
                ),
                design::link_action(
                    "cm-compress-none",
                    "清空",
                    &palette,
                    cx.listener(|this, _, _, cx| {
                        this.settings.integration.compress_verbs.clear();
                        this.save_settings();
                        this.apply_shell_menu(cx);
                        cx.notify();
                    }),
                ),
            ],
        );
        let mut compress_rows = Vec::new();
        for (key, title) in COMPRESS_MENU_OPTIONS.iter() {
            let key_owned = key.to_string();
            let checked = compress_keys.iter().any(|item| item.as_str() == *key);
            compress_rows.push(menu_check_row(
                &palette,
                format!("cm-compress-{key}"),
                checked,
                title,
                cx.listener(move |this, _, _, cx| {
                    let position = this
                        .settings
                        .integration
                        .compress_verbs
                        .iter()
                        .position(|item| item == &key_owned);
                    if let Some(position) = position {
                        this.settings.integration.compress_verbs.remove(position);
                    } else {
                        this.settings
                            .integration
                            .compress_verbs
                            .push(key_owned.clone());
                    }
                    this.save_settings();
                    this.apply_shell_menu(cx);
                    cx.notify();
                }),
            ));
        }
        let compress_card = design::card(&palette).child(menu_grid(compress_rows, 2));

        let empty_head = menu_group_head(
            &palette,
            "空白处右键菜单",
            "在资源管理器空白处单击右键时显示",
            Vec::new(),
        );
        let mut empty_rows = Vec::new();
        for (key, title) in EMPTY_MENU_OPTIONS.iter() {
            let key_owned = key.to_string();
            let checked = empty_keys.iter().any(|item| item.as_str() == *key);
            empty_rows.push(menu_check_row(
                &palette,
                format!("cm-empty-{key}"),
                checked,
                title,
                cx.listener(move |this, _, _, cx| {
                    let position = this
                        .settings
                        .integration
                        .empty_verbs
                        .iter()
                        .position(|item| item == &key_owned);
                    if let Some(position) = position {
                        this.settings.integration.empty_verbs.remove(position);
                    } else {
                        this.settings
                            .integration
                            .empty_verbs
                            .push(key_owned.clone());
                    }
                    this.save_settings();
                    this.apply_shell_menu(cx);
                    cx.notify();
                }),
            ));
        }
        let empty_card = design::card(&palette).child(menu_grid(empty_rows, 1));

        let other_head = menu_group_head(&palette, "其它", "菜单的整体行为", Vec::new());
        let other_rows = vec![
            menu_check_row(
                &palette,
                "cm-disable",
                disabled,
                "禁用右键菜单",
                cx.listener(|this, _, _, cx| {
                    let next = !this.settings.integration.disable_menu;
                    this.settings.integration.disable_menu = next;
                    let result = if next {
                        nimzip_shell::uninstall(nimzip_shell::ShellScope::CurrentUser)
                    } else {
                        nimzip_shell::install(&this.shell_options())
                    };
                    this.save_settings();
                    match result {
                        Ok(()) => this.show_toast(
                            if next {
                                "已禁用 NimZip 右键菜单"
                            } else {
                                "已重新注册 NimZip 右键菜单"
                            },
                            ToastKind::Success,
                            cx,
                        ),
                        Err(error) => {
                            this.show_toast(format!("操作失败：{error}"), ToastKind::Error, cx)
                        }
                    }
                    cx.notify();
                }),
            ),
            menu_check_row(
                &palette,
                "cm-cascade",
                cascade,
                "层叠右键菜单",
                cx.listener(|this, _, _, cx| {
                    this.settings.integration.cascade = !this.settings.integration.cascade;
                    this.save_settings();
                    this.apply_shell_menu(cx);
                    cx.notify();
                }),
            ),
            menu_check_row(
                &palette,
                "cm-hide-icon",
                hide_icon,
                "隐藏 NimZip 图标",
                cx.listener(|this, _, _, cx| {
                    this.settings.integration.hide_icon = !this.settings.integration.hide_icon;
                    this.save_settings();
                    this.apply_shell_menu(cx);
                    cx.notify();
                }),
            ),
        ];
        let other_card = design::card(&palette).child(menu_grid(other_rows, 2));

        let preview_note = design::info_note(
            &palette,
            "预览处理器需要 nimzip_shell.dll 与 nimzip.exe 放在同一目录。",
        );

        div()
            .flex()
            .flex_col()
            .child(status_card)
            .child(menu_group_head(
                &palette,
                "双击动作",
                "在资源管理器中双击压缩包时执行的操作",
                Vec::new(),
            ))
            .child(double_card)
            .child(extract_head)
            .child(extract_card)
            .child(compress_head)
            .child(compress_card)
            .child(empty_head)
            .child(empty_card)
            .child(other_head)
            .child(other_card)
            .child(preview_note)
            .into_any_element()
    }

    fn settings_associations(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let selected = self.settings.integration.associations.clone();

        let mut groups = div().flex().flex_col().gap(px(14.0));
        for (index, (name, extensions)) in ASSOCIATION_GROUPS.iter().enumerate() {
            let group_count = extensions.len();
            let group_extensions: Vec<String> =
                extensions.iter().map(|ext| ext.to_string()).collect();
            let select_extensions = group_extensions.clone();
            let clear_extensions = group_extensions.clone();

            let mut grid = div()
                .grid()
                .grid_cols(4)
                .gap(px(9.0))
                .px(px(18.0))
                .pb(px(16.0))
                .pt(px(2.0));
            for extension in extensions.iter() {
                let is_selected = selected.iter().any(|item| item.as_str() == *extension);
                let taken = matches!(
                    nimzip_shell::association_status(
                        nimzip_shell::ShellScope::CurrentUser,
                        extension
                    ),
                    nimzip_shell::AssociationStatus::Other(_)
                );
                let key = extension.to_string();
                let chip_key = key.clone();
                grid = grid.child(association_chip(
                    &palette,
                    &chip_key,
                    is_selected,
                    taken,
                    cx.listener(move |this, _, _, cx| {
                        let position = this
                            .settings
                            .integration
                            .associations
                            .iter()
                            .position(|item| item == &key);
                        if let Some(position) = position {
                            this.settings.integration.associations.remove(position);
                        } else {
                            this.settings.integration.associations.push(key.clone());
                        }
                        this.save_settings();
                        cx.notify();
                    }),
                ));
            }

            groups = groups.child(
                design::card(&palette)
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .px(px(18.0))
                            .pt(px(13.0))
                            .pb(px(11.0))
                            .child(
                                div()
                                    .text_size(crate::theme::font_px(13.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(palette.text)
                                    .child(*name),
                            )
                            .child(
                                div()
                                    .text_size(crate::theme::font_px(12.0))
                                    .text_color(palette.muted)
                                    .child(format!("{group_count} 种")),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(2.0))
                                    .child(design::link_action(
                                        format!("assoc-all-{index}"),
                                        "全选",
                                        &palette,
                                        cx.listener(move |this, _, _, cx| {
                                            for extension in &select_extensions {
                                                if !this
                                                    .settings
                                                    .integration
                                                    .associations
                                                    .iter()
                                                    .any(|item| item == extension)
                                                {
                                                    this.settings
                                                        .integration
                                                        .associations
                                                        .push(extension.clone());
                                                }
                                            }
                                            this.save_settings();
                                            cx.notify();
                                        }),
                                    ))
                                    .child(design::link_action(
                                        format!("assoc-none-{index}"),
                                        "清空",
                                        &palette,
                                        cx.listener(move |this, _, _, cx| {
                                            this.settings.integration.associations.retain(|item| {
                                                !clear_extensions.iter().any(|ext| ext == item)
                                            });
                                            this.save_settings();
                                            cx.notify();
                                        }),
                                    )),
                            ),
                    )
                    .child(grid),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(groups)
            .into_any_element()
    }

    fn associations_footer(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let selected = self.settings.integration.associations.clone();
        let selected_count = ASSOCIATION_GROUPS
            .iter()
            .flat_map(|(_, extensions)| extensions.iter())
            .filter(|extension| selected.iter().any(|item| item.as_str() == **extension))
            .count();
        let total_count: usize = ASSOCIATION_GROUPS
            .iter()
            .map(|(_, extensions)| extensions.len())
            .sum();
        let other_count = ASSOCIATION_GROUPS
            .iter()
            .flat_map(|(_, extensions)| extensions.iter())
            .filter(|extension| {
                matches!(
                    nimzip_shell::association_status(
                        nimzip_shell::ShellScope::CurrentUser,
                        extension
                    ),
                    nimzip_shell::AssociationStatus::Other(_)
                )
            })
            .count();

        let footer_note = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_size(crate::theme::font_px(12.5))
            .text_color(palette.muted)
            .child(icon(IconName::Info, palette.muted, 14.0))
            .child("注册的是「打开方式」候选，不强制覆盖 Windows 用户默认选择；被接管的格式关联后会自动切换为 NimZip。");

        let count = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .text_size(crate::theme::font_px(12.5))
            .text_color(palette.sub)
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(palette.accent)
                    .child(selected_count.to_string()),
            )
            .child(format!("/ {total_count} 种格式已选"))
            .when(other_count > 0, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .child(
                            div()
                                .w(px(6.0))
                                .h(px(6.0))
                                .rounded(px(99.0))
                                .bg(Hsla::from(rgb(0xE8A13C))),
                        )
                        .child(format!("{other_count} 种由其它程序接管")),
                )
            });

        let apply = design::primary_icon_action(
            "assoc-apply",
            "立即应用",
            IconName::Check,
            &palette,
            false,
            cx.listener(move |this, _, _, cx| {
                let selected = this.settings.integration.associations.clone();
                let action = this.settings.integration.double_click.clone();
                let folder = this.settings.integration.double_click_folder.clone();
                let exe = std::env::current_exe().unwrap_or_default();
                let mut applied = 0usize;
                let mut errors = Vec::new();
                for (_, extensions) in ASSOCIATION_GROUPS.iter() {
                    for extension in extensions.iter() {
                        let result = if selected.iter().any(|item| item.as_str() == *extension) {
                            nimzip_shell::register_association_with_action(
                                nimzip_shell::ShellScope::CurrentUser,
                                extension,
                                &exe,
                                &action,
                                &folder,
                            )
                        } else {
                            nimzip_shell::unregister_association(
                                nimzip_shell::ShellScope::CurrentUser,
                                extension,
                            )
                        };
                        match result {
                            Ok(()) => applied += 1,
                            Err(error) => errors.push(format!("{extension}: {error}")),
                        }
                    }
                }
                if errors.is_empty() {
                    this.show_toast(
                        format!("已应用 {applied} 种格式的文件关联"),
                        ToastKind::Success,
                        cx,
                    );
                } else {
                    this.show_toast(
                        format!(
                            "部分关联未应用（{} 项）：{}",
                            errors.len(),
                            errors.first().cloned().unwrap_or_default()
                        ),
                        ToastKind::Error,
                        cx,
                    );
                }
                cx.notify();
            }),
        );

        let footer = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(14.0))
            .flex_wrap()
            .pt(px(16.0))
            .child(footer_note)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(count)
                    .child(design::secondary_action(
                        "assoc-basic",
                        "基础类型",
                        cx.listener(|this, _, _, cx| {
                            this.settings.integration.associations = ASSOCIATION_GROUPS
                                .iter()
                                .take(2)
                                .flat_map(|(_, extensions)| extensions.iter())
                                .map(|extension| extension.to_string())
                                .collect();
                            this.save_settings();
                            cx.notify();
                        }),
                    ))
                    .child(design::secondary_action(
                        "assoc-all",
                        "全选",
                        cx.listener(|this, _, _, cx| {
                            this.settings.integration.associations = all_association_extensions();
                            this.save_settings();
                            cx.notify();
                        }),
                    ))
                    .child(design::secondary_action(
                        "assoc-none",
                        "取消全选",
                        cx.listener(|this, _, _, cx| {
                            this.settings.integration.associations.clear();
                            this.save_settings();
                            cx.notify();
                        }),
                    ))
                    .child(apply),
            );

        footer.into_any_element()
    }

    fn setting_check_box<F>(
        &self,
        id: &'static str,
        label: &'static str,
        checked: bool,
        setter: F,
        cx: &mut Context<Self>,
    ) -> AnyElement
    where
        F: Fn(&mut Settings, bool) + 'static,
    {
        let palette = self.palette;
        let entity = cx.entity();
        design::check_box(
            &palette,
            id,
            checked,
            false,
            crate::i18n::tr(self.language, label),
            move |_, _, app| {
                entity.update(app, |this, cx| {
                    setter(&mut this.settings, !checked);
                    this.save_settings();
                    cx.notify();
                });
            },
        )
    }

    fn setting_select(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        width: f32,
        kind: DropdownKind,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let label = label.into();
        let entity = cx.entity();
        let control = div()
            .id(id)
            .role(Role::Button)
            .aria_label(label.clone())
            .h(px(crate::components::CONTROL_HEIGHT))
            .w(px(width))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .px(px(13.0))
            .rounded(px(9.0))
            .border_1()
            .border_color(palette.line_strong)
            .bg(palette.bg)
            .cursor_pointer()
            .text_size(crate::theme::font_px(13.5))
            .text_color(palette.text)
            .hover(|style| style.border_color(palette.accent))
            .on_click(cx.listener(move |this, _, _, cx| this.open_dropdown(kind, cx)))
            .on_a11y_action(AccessibleAction::Click, move |_, _, app| {
                entity.update(app, |this, cx| this.open_dropdown(kind, cx));
            })
            .child(div().truncate().child(label))
            .child(icon(IconName::ChevronDown, palette.muted, 14.0))
            .into_any_element();
        self.track_dropdown(kind, control, cx)
    }

    fn setting_step(
        &self,
        id: &'static str,
        label: &'static str,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        let palette = self.palette;
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .w(px(crate::components::CONTROL_HEIGHT))
            .h(px(crate::components::CONTROL_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .border_1()
            .border_color(palette.line)
            .bg(palette.panel)
            .text_size(crate::theme::font_px(14.0))
            .cursor_pointer()
            .hover(|style| style.bg(palette.hover))
            .on_click(on_click)
            .child(label)
            .into_any_element()
    }

    fn apply_ui_font_size(&mut self, size: f32, _window: &mut Window, cx: &mut Context<Self>) {
        let size = size.clamp(11.0, 17.0);
        self.settings.view.font_size = size;
        crate::theme::set_ui_font_size(size);
        crate::theme::apply(cx, self.palette.dark, self.skin, self.settings.view.mica);
        self.save_settings();
        cx.notify();
    }
}

const DOUBLE_CLICK_OPTIONS: &[(&str, &str, &str)] = &[
    ("open", "用 NimZip 打开", "在 NimZip 主窗口中浏览压缩包内容"),
    (
        "smart",
        "智能解压到此",
        "有独立根目录时直接解压，否则自动建同名文件夹",
    ),
    (
        "folder",
        "解压到“压缩文件名”文件夹",
        "在压缩包所在位置创建同名文件夹并解压",
    ),
    ("custom", "智能解压到指定文件夹", "总是解压到下面指定的位置"),
];

// 顺序与 Bandizip 右键菜单一致：解压到此处 → 删除 → 智能解压 → 删除 →
// 独立文件夹 → 删除 → 其他文件夹 → 管理员 → 测试 → 打开 → 预览。
const EXTRACT_MENU_OPTIONS: &[(&str, &str)] = &[
    ("here", "解压到此处(&E)"),
    ("here_delete", "解压到此处并删除"),
    ("smart", "智能解压到此处(&S)"),
    ("smart_delete", "智能解压到此处并删除"),
    ("folder", "解压到独立文件夹"),
    ("folder_delete", "解压到独立文件夹并删除"),
    ("other", "解压到其他文件夹(&B)..."),
    ("admin", "以管理员身份解压"),
    ("test", "测试压缩文件"),
    ("open", "用 NimZip 打开"),
    ("preview", "压缩文件预览..."),
    ("hash", "计算 SHA-256 哈希"),
];

// 顺序与 Bandizip 一致：ZIP → 删除 → TAR/TGZ → 7Z → GZ/XZ/ZST →
// 逐项压缩 → 新建压缩包 → 管理员压缩。
const COMPRESS_MENU_OPTIONS: &[(&str, &str)] = &[
    ("zip", "压缩为“文件名.zip”"),
    ("zip_delete", "压缩为“文件名.zip”并删除"),
    ("tar", "压缩为“文件名.tar”"),
    ("tgz", "压缩为“文件名.tgz”"),
    ("sevenz", "压缩为“文件名.7z”"),
    ("gz", "压缩为“文件名.gz”"),
    ("xz", "压缩为“文件名.xz”"),
    ("zst", "压缩为“文件名.zst”"),
    ("each_zip", "压缩为独立的“文件名/文件夹名.zip”"),
    ("each_7z", "压缩为独立的“文件名/文件夹名.7z”"),
    ("gui", "新建压缩包…"),
    ("admin", "以管理员身份压缩"),
];

const EMPTY_MENU_OPTIONS: &[(&str, &str)] =
    &[("open", "在此处打开 NimZip"), ("new", "在此处新建压缩包")];

fn menu_group_head(
    palette: &Palette,
    title: &'static str,
    hint: &'static str,
    actions: Vec<AnyElement>,
) -> AnyElement {
    div()
        .flex()
        .items_baseline()
        .gap(px(10.0))
        .mt(px(20.0))
        .mb(px(9.0))
        .child(
            div()
                .text_size(crate::theme::font_px(14.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette.text)
                .child(title),
        )
        .child(
            div()
                .text_size(crate::theme::font_px(12.5))
                .text_color(palette.muted)
                .child(hint),
        )
        .child(div().flex_1())
        .children(actions)
        .into_any_element()
}

fn radio_row(
    palette: &Palette,
    id: impl Into<ElementId>,
    selected: bool,
    title: &'static str,
    description: &'static str,
    onclick: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .role(Role::RadioButton)
        .aria_label(title)
        .aria_toggled(if selected {
            Toggled::True
        } else {
            Toggled::False
        })
        .flex()
        .items_start()
        .gap(px(11.0))
        .px(px(20.0))
        .py(px(11.0))
        .cursor_pointer()
        .hover(|style| style.bg(palette.accent_softer))
        .on_click(onclick)
        .child(
            div()
                .w(px(18.0))
                .h(px(18.0))
                .flex_none()
                .mt(px(1.0))
                .rounded(px(99.0))
                .border(px(if selected { 5.5 } else { 1.5 }))
                .border_color(if selected {
                    palette.accent
                } else {
                    palette.line_strong
                })
                .bg(palette.bg),
        )
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
                        .child(title),
                )
                .child(
                    div()
                        .text_size(crate::theme::font_px(12.5))
                        .text_color(palette.muted)
                        .child(description),
                ),
        )
        .into_any_element()
}

fn menu_check_row(
    palette: &Palette,
    id: impl Into<ElementId>,
    checked: bool,
    label: &'static str,
    onclick: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .role(Role::CheckBox)
        .aria_label(label)
        .aria_toggled(if checked {
            Toggled::True
        } else {
            Toggled::False
        })
        .flex()
        .items_center()
        .gap(px(10.0))
        .py(px(9.0))
        .border_b_1()
        .border_color(palette.panel)
        .cursor_pointer()
        .text_size(crate::theme::font_px(13.5))
        .text_color(palette.sub)
        .hover(|style| style.text_color(palette.accent))
        .on_click(onclick)
        .child(design::check_box_visual(palette, checked, false))
        .child(div().min_w_0().truncate().child(label))
        .into_any_element()
}

fn menu_grid(rows: Vec<AnyElement>, columns: usize) -> AnyElement {
    if columns <= 1 {
        return div()
            .flex()
            .flex_col()
            .px(px(20.0))
            .py(px(6.0))
            .children(rows)
            .into_any_element();
    }
    let mut buckets: Vec<Vec<AnyElement>> = (0..columns).map(|_| Vec::new()).collect();
    for (index, row) in rows.into_iter().enumerate() {
        buckets[index % columns].push(row);
    }
    div()
        .flex()
        .gap(px(30.0))
        .px(px(20.0))
        .py(px(6.0))
        .children(
            buckets
                .into_iter()
                .map(|bucket| div().flex().flex_col().flex_1().min_w_0().children(bucket)),
        )
        .into_any_element()
}

/// 文件关联页展示的受支持格式。仅包含 NimZip 能识别的归档类型。
const ASSOCIATION_GROUPS: &[(&str, &[&str])] = &[
    (
        "压缩包文件",
        &[
            "zip", "zipx", "rar", "7z", "alz", "egg", "cab", "bh", "001", "arj", "lha", "lzh",
            "pma", "ace", "arc", "aes", "zpaq", "zstd", "br", "pea",
        ],
    ),
    (
        "UNIX 与流压缩",
        &[
            "tar", "gz", "tgz", "bz", "bz2", "tbz", "tbz2", "xz", "txz", "lzma", "tlz", "lz", "uu",
            "uue", "xxe", "z", "lz4", "zst", "tzst",
        ],
    ),
    (
        "软件包",
        &["jar", "war", "apk", "ipa", "xpi", "deb", "asar"],
    ),
    (
        "CD / DVD 镜像",
        &["iso", "img", "isz", "udf", "wim", "bin", "i00"],
    ),
];

fn all_association_extensions() -> Vec<String> {
    ASSOCIATION_GROUPS
        .iter()
        .flat_map(|(_, extensions)| extensions.iter())
        .map(|extension| extension.to_string())
        .collect()
}

fn association_chip(
    palette: &Palette,
    extension: &str,
    selected: bool,
    taken: bool,
    onclick: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(format!("assoc-chip-{extension}"))
        .role(Role::CheckBox)
        .aria_label(extension.to_string())
        .aria_toggled(if selected {
            Toggled::True
        } else {
            Toggled::False
        })
        .h(px(34.0))
        .px(px(12.0))
        .rounded(px(99.0))
        .border_1()
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
        .flex()
        .items_center()
        .gap(px(8.0))
        .cursor_pointer()
        .font_family("Cascadia Mono")
        .text_size(crate::theme::font_px(12.5))
        .text_color(if selected { palette.text } else { palette.sub })
        .hover(|style| style.border_color(palette.accent).text_color(palette.text))
        .on_click(onclick)
        .child(
            div()
                .w(px(15.0))
                .h(px(15.0))
                .flex_none()
                .rounded(px(4.5))
                .border(px(1.5))
                .border_color(if selected {
                    palette.accent
                } else {
                    palette.line_strong
                })
                .bg(if selected { palette.accent } else { palette.bg })
                .flex()
                .items_center()
                .justify_center()
                .child(if selected {
                    icon(IconName::Check, palette.white, 9.0).into_any_element()
                } else {
                    div().into_any_element()
                }),
        )
        .child(extension.to_string())
        .when(taken, |this| {
            this.child(
                div()
                    .w(px(6.0))
                    .h(px(6.0))
                    .ml(px(2.0))
                    .rounded(px(99.0))
                    .bg(palette.warn),
            )
        })
        .into_any_element()
}

fn settings_page_icon(page: SettingsPage) -> IconName {
    match page {
        SettingsPage::General => IconName::House,
        SettingsPage::Associations => IconName::Cable,
        SettingsPage::ContextMenu => IconName::Menu,
        SettingsPage::Extraction => IconName::ArchiveRestore,
        SettingsPage::Compression => IconName::Package,
        SettingsPage::View => IconName::Palette,
        SettingsPage::Security => IconName::ShieldCheck,
        SettingsPage::Advanced => IconName::SlidersHorizontal,
        SettingsPage::Language => IconName::Globe,
    }
}

fn settings_page_subtitle(page: SettingsPage, language: Language) -> &'static str {
    let english = language == Language::En;
    match page {
        SettingsPage::General => {
            if english {
                "Startup and list behavior; changes apply immediately"
            } else {
                "启动与列表行为，所有更改即时生效"
            }
        }
        SettingsPage::Associations => {
            if english {
                "Choose NimZip as an option for archives; Windows still decides the default app"
            } else {
                "选择 NimZip 作为压缩包的默认打开方式；注册的是「打开方式」候选，不强制覆盖 Windows 用户默认选择"
            }
        }
        SettingsPage::ContextMenu => {
            if english {
                "Manage NimZip entries and double-click behavior in Explorer; changes apply immediately"
            } else {
                "管理 Windows 资源管理器右键菜单中 NimZip 的入口与双击行为，更改即时生效"
            }
        }
        SettingsPage::Extraction => {
            if english {
                "Control extraction behavior and resource usage; changes apply immediately"
            } else {
                "控制解压行为与资源占用，所有更改即时生效"
            }
        }
        SettingsPage::Compression => {
            if english {
                "Default format, compression level and exclude rules"
            } else {
                "默认格式、压缩级别与排除规则"
            }
        }
        SettingsPage::View => {
            if english {
                "Appearance, layout and list display"
            } else {
                "界面外观、布局与列表显示"
            }
        }
        SettingsPage::Security => {
            if english {
                "Extraction safety and malicious content scanning"
            } else {
                "解压安全策略与恶意内容扫描"
            }
        }
        SettingsPage::Advanced => {
            if english {
                "Temporary files, recycle bin and configuration import/export"
            } else {
                "临时目录、回收站与配置导入导出"
            }
        }
        SettingsPage::Language => {
            if english {
                "Interface language and language packs"
            } else {
                "界面语言与语言包"
            }
        }
    }
}

fn memory_label(mb: u32) -> String {
    if mb >= 1024 && mb % 1024 == 0 {
        format!("{} GB", mb / 1024)
    } else {
        format!("{} MB", mb)
    }
}
