mod modals;

use rarkzip_engine::Settings;

use crate::app::*;
use crate::theme::{self, Palette, ThemeMode};

impl MainView {
    pub(crate) fn export_settings(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let settings_filter = self.localized("RarkZip 设置");
        let Some(path) = crate::components::file_dialog(move || {
            rfd::FileDialog::new()
                .add_filter(settings_filter, &["json"])
                .set_file_name("rarkzip-settings.json")
                .save_file()
        }) else {
            return;
        };
        let result = serde_json::to_vec_pretty(&self.settings)
            .map_err(|error| error.to_string())
            .and_then(|bytes| std::fs::write(&path, bytes).map_err(|error| error.to_string()));
        match result {
            Ok(()) => self.show_toast(
                crate::i18n::tr_fmt(self.language, "设置已导出：{}", &[&path.display()]),
                ToastKind::Success,
                cx,
            ),
            Err(error) => self.show_toast(
                crate::i18n::tr_fmt(self.language, "导出设置失败：{}", &[&error]),
                ToastKind::Error,
                cx,
            ),
        }
    }

    pub(crate) fn import_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings_filter = self.localized("RarkZip 设置");
        let Some(path) = crate::components::file_dialog(move || {
            rfd::FileDialog::new()
                .add_filter(settings_filter, &["json"])
                .pick_file()
        }) else {
            return;
        };
        let imported = std::fs::read(&path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                serde_json::from_slice::<Settings>(&bytes).map_err(|error| error.to_string())
            });
        match imported {
            Ok(settings) => {
                self.settings = settings;
                self.save_settings();
                self.theme_mode = ThemeMode::from_setting(&self.settings.view.theme);
                let dark = self.theme_mode.is_dark(window.appearance());
                self.skin = Skin::from_setting(&self.settings.view.theme_skin);
                theme::apply(cx, dark, self.skin);
                self.palette = Palette::for_skin(dark, self.skin);
                self.settings_page = SettingsPage::ALL
                    .get(self.settings.ui.settings_page)
                    .copied()
                    .unwrap_or(SettingsPage::General);
                self.show_toast(self.localized("设置已导入"), ToastKind::Success, cx);
                cx.notify();
            }
            Err(error) => self.show_toast(
                crate::i18n::tr_fmt(self.language, "导入设置失败：{}", &[&error]),
                ToastKind::Error,
                cx,
            ),
        }
    }
}

pub(crate) fn level_label(level: u8) -> &'static str {
    match level {
        0 => "仅存储",
        1 => "极快",
        2 => "很快",
        3 => "快",
        4 => "较快",
        5 => "标准",
        6 => "较高",
        7 => "高",
        8 => "很高",
        _ => "极限",
    }
}
