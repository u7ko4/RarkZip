//! 多语言 UI 文案。中文/英文为手工维护的完整文案；其余 18 种语言由
//! `i18n_packs.rs` 提供核心导航的手工翻译与常用界面的机器翻译，未覆盖的键回退英文。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Zh,
    En,
    Ja,
    Ko,
    De,
    Fr,
    Es,
    Ru,
    Pt,
    It,
    Nl,
    Pl,
    Tr,
    Ar,
    Hi,
    Th,
    Vi,
    Id,
    Uk,
    Cs,
}

include!("i18n_packs.rs");

impl Language {
    pub const ALL: [Language; 20] = [
        Language::Zh,
        Language::En,
        Language::Ja,
        Language::Ko,
        Language::De,
        Language::Fr,
        Language::Es,
        Language::Ru,
        Language::Pt,
        Language::It,
        Language::Nl,
        Language::Pl,
        Language::Tr,
        Language::Ar,
        Language::Hi,
        Language::Th,
        Language::Vi,
        Language::Id,
        Language::Uk,
        Language::Cs,
    ];

    pub fn from_setting(value: &str) -> Self {
        let value = value.to_ascii_lowercase();
        if value.starts_with("zh") {
            return Self::Zh;
        }
        if value.starts_with("en") {
            return Self::En;
        }
        for language in Self::ALL.into_iter().skip(2) {
            if value == language.as_setting().to_ascii_lowercase() {
                return language;
            }
        }
        Self::Zh
    }

    pub fn as_setting(self) -> &'static str {
        match self {
            Self::Zh => "zh-CN",
            Self::En => "en-US",
            Self::Ja => "ja-JP",
            Self::Ko => "ko-KR",
            Self::De => "de-DE",
            Self::Fr => "fr-FR",
            Self::Es => "es-ES",
            Self::Ru => "ru-RU",
            Self::Pt => "pt-BR",
            Self::It => "it-IT",
            Self::Nl => "nl-NL",
            Self::Pl => "pl-PL",
            Self::Tr => "tr-TR",
            Self::Ar => "ar-SA",
            Self::Hi => "hi-IN",
            Self::Th => "th-TH",
            Self::Vi => "vi-VN",
            Self::Id => "id-ID",
            Self::Uk => "uk-UA",
            Self::Cs => "cs-CZ",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Zh => "简体中文",
            Self::En => "English",
            Self::Ja => "日本語",
            Self::Ko => "한국어",
            Self::De => "Deutsch",
            Self::Fr => "Français",
            Self::Es => "Español",
            Self::Ru => "Русский",
            Self::Pt => "Português",
            Self::It => "Italiano",
            Self::Nl => "Nederlands",
            Self::Pl => "Polski",
            Self::Tr => "Türkçe",
            Self::Ar => "العربية",
            Self::Hi => "हिन्दी",
            Self::Th => "ไทย",
            Self::Vi => "Tiếng Việt",
            Self::Id => "Bahasa Indonesia",
            Self::Uk => "Українська",
            Self::Cs => "Čeština",
        }
    }
}

pub fn tr(language: Language, zh: &'static str) -> &'static str {
    if language == Language::Zh {
        return zh;
    }
    if language == Language::En {
        return en(zh);
    }
    if let Some(pack) = pack(language) {
        if let Some((_, value)) = pack.iter().find(|(key, _)| *key == zh) {
            return value;
        }
    }
    en(zh)
}

fn en(zh: &'static str) -> &'static str {
    match zh {
        "文件" => "File",
        "窗口" => "Window",
        "编辑" => "Edit",
        "帮助" => "Help",
        "打开压缩包" => "Open Archive",
        "新建压缩包" | "新建压缩" => "New Archive",
        "选择文件，或直接拖入窗口" => "Choose a file, or drop it here",
        "添加文件并选择压缩格式" => "Add files and choose a compression format",
        "支持 ZIP · 7Z · RAR · TAR 等 50+ 格式" => {
            "Supports ZIP · 7Z · RAR · TAR and 50+ formats"
        }
        "快速打开" => "Quick open",
        "解压到指定文件夹…" => "Extract to Folder…",
        "测试完整性" => "Test Integrity",
        "扫描恶意软件" => "Scan for Malware",
        "计算哈希" => "Calculate Hash",
        "修复 ZIP 压缩包…" => "Repair ZIP…",
        "批量哈希…" => "Batch Hash…",
        "列设置…" => "Columns…",
        "工具栏按钮…" => "Toolbar Buttons…",
        "打开所在位置" => "Show in Folder",
        "关闭压缩包" => "Close Archive",
        "新建程序窗口" => "New Window",
        "关闭当前窗口" => "Close Window",
        "退出 RarkZip" => "Exit RarkZip",
        "任务中心" => "Task Center",
        "设置" => "Settings",
        "密码管理器" => "Password Manager",
        "密码恢复…" => "Password Recovery…",
        "切换浅色 / 暗色主题" => "Toggle Light / Dark Theme",
        "关于 RarkZip" => "About RarkZip",
        "添加文件…" => "Add Files…",
        "重命名…" => "Rename…",
        "删除选中条目" => "Delete Selected",
        "编辑注释…" => "Edit Comment…",
        "隐藏到系统托盘" => "Hide to Tray",
        "导入设置…" => "Import Settings…",
        "导出设置…" => "Export Settings…",
        "打开配置目录" => "Open Data Folder",
        "常规" => "General",
        "文件关联" => "File Associations",
        "右键菜单" => "Context Menu",
        "解压" => "Extraction",
        "压缩" => "Compression",
        "外观" => "Appearance",
        "安全" => "Security",
        "高级" => "Advanced",
        "语言" => "Language",
        "名称" => "Name",
        "大小" => "Size",
        "压缩后" => "Compressed",
        "类型" => "Type",
        "修改时间" => "Modified",
        "方法" => "Method",
        "加密" => "Encrypted",
        "CRC" => "CRC",
        "属性" => "Attributes",
        "注释" => "Comment",
        "扩展" => "Extra",
        "完成" => "Done",
        "现代、快速、安全的压缩与归档管理" => "Modern, fast and secure archive management",
        "智能解压" => "Smart Extract",
        "最近打开" => "Recent Files",
        "最近打开文件" => "Recent Files",
        "记住最近打开文件" => "Remember recently opened files",
        "暂无最近打开的文件" => "No recently opened files",
        "清除最近记录" => "Clear Recent Files",
        "界面语言" => "Interface language",
        "切换后立即生效并保存；当前首批提供简体中文与 English。" => {
            "Changes apply immediately and are saved. 20 language packs are available."
        }
        "切换后立即生效并保存；已内置 20 种语言界面。" => {
            "Changes apply immediately and are saved. 20 language packs are built in."
        }
        "20 种语言包已内置；切换后菜单、设置导航与列表表头同步更新。" => {
            "20 language packs are built in; menus, settings navigation and column headers update with the language."
        }
        "语言包" => "Language packs",
        "主题皮肤" => "Theme skin",
        "检查更新" => "Check for Updates",
        "显示图像预览面板" => "Show image preview panel",
        "显示树形导航" => "Show folder tree",
        "显示注释面板" => "Show comment panel",
        "显示状态栏" => "Show status bar",
        "工具栏图标大小" => "Toolbar icon size",
        "工具栏高度" => "Toolbar height",
        "界面字号" => "UI font size",
        "跟随系统" => "System",
        "浅色" => "Light",
        "暗色" => "Dark",
        "最小化 / 关闭窗口时隐藏到系统托盘" => "Minimize / close window to system tray",
        "文件列表显示网格线" => "Show grid lines in file list",
        "整行选中" => "Full-row selection",
        "单击打开文件（否则双击）" => "Open files with a single click",
        "记录最近打开的压缩包" => "Remember recently opened archives",
        "Esc 关闭窗口" => "Esc closes the window",
        "工具栏合并到标题栏" => "Merge toolbar into title bar",
        "使用自定义标题栏" => "Use custom title bar",
        "主菜单" => "Main menu",
        "更多工具" => "More Tools",
        "未打开归档" => "No archive open",
        "图像预览" => "Image preview",
        "选择文件以预览" => "Select a file to preview",
        "打开压缩包或新建压缩" => "Open an archive or create one",
        "双击进入 · F2 重命名 · Ctrl+E 解压" => {
            "Double-click to open · F2 rename · Ctrl+E extract"
        }
        "双击进入 · F2 重命名 · Ctrl+E 解压 · 图标视图下 Ctrl+滚轮 缩放" => {
            "Double-click to open · F2 rename · Ctrl+E extract · Ctrl+wheel zooms icon view"
        }
        "设置项与菜单将随语言切换；第三方语言包可按 JSON 结构扩展。" => {
            "Menus and settings switch with the selected language; third-party language packs can use the JSON structure."
        }
        _ => zh,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twenty_languages_are_available() {
        assert_eq!(Language::ALL.len(), 20);
        for language in Language::ALL {
            assert!(!language.label().is_empty());
            assert_eq!(Language::from_setting(language.as_setting()), language);
        }
    }

    #[test]
    fn every_translated_pack_is_complete() {
        for language in Language::ALL.into_iter().skip(2) {
            let pack = pack(language).expect("language pack");
            assert_eq!(pack.len(), 87, "{language:?}");
            assert!(pack
                .iter()
                .all(|(key, value)| !key.is_empty() && !value.is_empty()));
        }
    }

    #[test]
    fn core_strings_switch_per_language() {
        assert_eq!(tr(Language::Zh, "设置"), "设置");
        assert_eq!(tr(Language::En, "设置"), "Settings");
        assert_eq!(tr(Language::Ja, "设置"), "設定");
        assert_eq!(tr(Language::De, "文件"), "Datei");
        assert_eq!(tr(Language::Ar, "语言"), "اللغة");
        assert_eq!(tr(Language::Ja, "显示状态栏"), "ステータスバーを表示する");
        assert_eq!(
            tr(Language::Ja, "未打开归档"),
            "アーカイブが開かれていません"
        );
        assert_eq!(tr(Language::De, "密码管理器"), "Passwort-Manager");
        assert_eq!(tr(Language::Zh, "未收录"), "未收录");
    }
}
