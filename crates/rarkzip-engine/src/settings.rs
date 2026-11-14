use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::paths::to_extended;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralSettings {
    pub esc_closes_window: bool,
    pub test_after_create: bool,
    pub snap_windows: bool,
    pub show_parent_item: bool,
    pub show_grid_lines: bool,
    pub full_row_select: bool,
    pub single_click_open: bool,
    pub show_start_page: bool,
    pub full_path_title: bool,
    /// 界面语言（BCP-47，如 zh-CN / en-US）。
    pub language: String,
    /// 工具栏按钮隐藏列表（key）。
    pub hidden_toolbar_buttons: Vec<String>,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            esc_closes_window: true,
            test_after_create: false,
            snap_windows: true,
            show_parent_item: true,
            show_grid_lines: false,
            full_row_select: true,
            single_click_open: false,
            show_start_page: true,
            full_path_title: false,
            language: "zh-CN".to_string(),
            hidden_toolbar_buttons: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtractionSettings {
    pub smart: bool,
    pub parallel: bool,
    pub open_folder_after: bool,
    pub tgz_single_pass: bool,
    pub nested_enabled: bool,
    pub nested_depth: u8,
    pub max_memory_mb: u32,
    pub default_codepage: String,
    pub keep_dialog_open: bool,
    pub mac_nfc: bool,
    pub unix_utf8: bool,
}

impl Default for ExtractionSettings {
    fn default() -> Self {
        Self {
            smart: true,
            parallel: true,
            open_folder_after: false,
            tgz_single_pass: true,
            nested_enabled: true,
            nested_depth: 3,
            max_memory_mb: 4096,
            default_codepage: "auto".to_string(),
            keep_dialog_open: false,
            mac_nfc: true,
            unix_utf8: true,
        }
    }
}

/// 默认排除规则；设置页与新建压缩对话框共用。
pub const DEFAULT_EXCLUDE: &str = "*.bak; *.tmp; Thumbs.db";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompressionSettings {
    pub include_root: bool,
    pub default_level: u8,
    pub default_format: String,
    pub test_after: bool,
    pub use_utf8: bool,
    pub zip64: bool,
    pub fast_mode: bool,
    pub exclude: String,
    pub default_dir: String,
}

impl Default for CompressionSettings {
    fn default() -> Self {
        Self {
            include_root: true,
            default_level: 5,
            default_format: "zip".to_string(),
            test_after: false,
            use_utf8: true,
            zip64: true,
            fast_mode: false,
            exclude: DEFAULT_EXCLUDE.to_string(),
            default_dir: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewSettings {
    pub preview_enabled: bool,
    pub show_tree: bool,
    pub show_comment: bool,
    /// 是否已应用“导航默认开、预览/注释默认关”的初始面板状态。
    #[serde(default)]
    pub panels_defaulted: bool,
    pub show_status: bool,
    pub show_toolbar: bool,
    /// 使用应用自绘标题栏（关闭后使用系统标题栏，新窗口/重启后生效）。
    pub use_custom_titlebar: bool,
    pub theme: String,
    /// 内置主题皮肤 key。
    pub theme_skin: String,
    pub font_size: f32,
    /// 工具栏图标的绘制尺寸（px），默认 18。
    pub toolbar_icon_size: f32,
    /// 顶部工具栏高度（px），默认 48。
    pub toolbar_height: f32,
    pub thumbnail_size: f32,
    /// 主文件区视图：list（详细列表）/ icons（图标）。
    pub file_list_view: String,
    /// 文件列表列顺序（规范 key）。
    pub column_order: Vec<String>,
    /// 隐藏的列 key。
    pub hidden_columns: Vec<String>,
    /// 列宽覆盖（逻辑像素）；缺省使用内置默认宽度。
    pub column_widths: HashMap<String, f32>,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            preview_enabled: false,
            show_tree: true,
            show_comment: false,
            panels_defaulted: true,
            show_status: true,
            show_toolbar: true,
            use_custom_titlebar: true,
            theme: "system".to_string(),
            theme_skin: "blue".to_string(),
            font_size: 13.0,
            toolbar_icon_size: 18.0,
            toolbar_height: 48.0,
            thumbnail_size: 104.0,
            file_list_view: "list".to_string(),
            column_order: vec![
                "name".to_string(),
                "original".to_string(),
                "compressed".to_string(),
                "kind".to_string(),
                "modified".to_string(),
                "method".to_string(),
                "encryption".to_string(),
                "crc".to_string(),
                "attributes".to_string(),
                "comment".to_string(),
                "extra".to_string(),
            ],
            hidden_columns: Vec::new(),
            column_widths: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecuritySettings {
    pub propagate_motw: bool,
    pub amsi_scan: bool,
    pub scan_buffer_kb: u32,
}

impl Default for SecuritySettings {
    fn default() -> Self {
        Self {
            propagate_motw: true,
            amsi_scan: false,
            scan_buffer_kb: 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AdvancedSettings {
    pub temp_dir: String,
    pub recycle_bin: bool,
    pub history: bool,
    /// 是否已把“记住最近打开文件”迁移为默认关闭。
    #[serde(default)]
    pub history_defaulted: bool,
    pub long_paths: bool,
    pub preallocate: bool,
    pub compress_locked: bool,
    pub clean_on_cancel: bool,
}

impl Default for AdvancedSettings {
    fn default() -> Self {
        Self {
            temp_dir: String::new(),
            recycle_bin: true,
            history: false,
            history_defaulted: false,
            long_paths: true,
            preallocate: true,
            compress_locked: false,
            clean_on_cancel: false,
        }
    }
}

/// 文件哈希窗口与资源管理器哈希入口的默认行为。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HashSettings {
    /// 默认算法 key（sha256 / sha1 / md5 / sha512 / sha3-256 / blake3 / xxhash64 / crc32）。
    pub algorithms: Vec<String>,
    /// 打开文件哈希窗口后立即开始计算。
    pub auto_start: bool,
    /// 哈希值以大写字母显示和复制。
    pub uppercase_display: bool,
    /// 使用等宽字体显示哈希值。
    pub monospace: bool,
    /// 导出校验和文件时哈希值使用大写。
    pub export_uppercase: bool,
    /// 导出时使用 Unix 换行符（LF）；关闭时使用 CRLF。
    pub export_lf: bool,
    /// 导出时使用空格加星号（` *`）分隔；关闭时使用双空格。
    pub export_star: bool,
}

impl Default for HashSettings {
    fn default() -> Self {
        Self {
            algorithms: ["sha256", "sha1", "md5", "crc32"]
                .into_iter()
                .map(ToString::to_string)
                .collect(),
            auto_start: false,
            uppercase_display: false,
            monospace: true,
            export_uppercase: false,
            export_lf: true,
            export_star: false,
        }
    }
}

/// 文件与文本加密工具的默认选项。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CryptSettings {
    /// 算法代码：0=AES-256，1=RC6，2=Serpent，4=Twofish，5=GOST，
    /// 6=Blowfish-448，7=Threefish-1024，8=SHACAL-2，9=C4-2048。
    pub algorithm: u8,
    /// 文本输出编码：0=Base64，1=Base32，2=纯数字。
    pub text_encoding: u8,
    /// 文本加密时使用 LZMA 压缩。
    pub text_compression: bool,
    /// 每 N 个字符插入一个空格；0 表示关闭。
    pub add_space_every: u8,
    /// 文件压缩：0=自动，1=始终，2=不压缩。
    pub file_compression: u8,
    /// 自定义输出目录；为空时输出到源文件旁边。
    pub output_dir: Option<String>,
    /// 隐写 JPEG 质量（1-100）。
    pub stego_quality: u8,
    /// 隐写载体长边上限；0 表示不缩放。
    pub stego_max_side: u32,
    /// 上次选择文件的位置。
    pub last_file_dir: Option<String>,
    /// 上次选择隐写载体的位置。
    pub last_stego_dir: Option<String>,
}

impl Default for CryptSettings {
    fn default() -> Self {
        Self {
            algorithm: 0,
            text_encoding: 0,
            text_compression: true,
            add_space_every: 0,
            file_compression: 0,
            output_dir: None,
            stego_quality: 90,
            stego_max_side: 1000,
            last_file_dir: None,
            last_stego_dir: None,
        }
    }
}

/// Windows 资源管理器集成：文件关联与右键菜单。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IntegrationSettings {
    /// 勾选的文件关联扩展名（不含前导点）。
    pub associations: Vec<String>,
    /// 双击压缩包的动作：open / smart / folder / custom。
    pub double_click: String,
    /// 双击动作选择 custom 时的目标目录。
    pub double_click_folder: String,
    /// 文件右键菜单中启用的解压/打开动作 key。
    pub extract_verbs: Vec<String>,
    /// 在任意文件类型（含无扩展名）上也显示解压菜单。
    pub extract_any_file: bool,
    /// 在文件右键菜单中显示“计算文件哈希”，支持单文件和多文件选择。
    pub hash_menu: bool,
    /// 文件/文件夹右键菜单中启用的压缩动作 key。
    pub compress_verbs: Vec<String>,
    /// 是否已把“压缩为独立的 ZIP/7Z”加入旧版压缩动作列表。
    #[serde(default)]
    pub each_compress_defaulted: bool,
    /// 是否已把右键菜单默认项收敛为常用集合。
    #[serde(default)]
    pub menu_aligned: bool,
    /// 目录空白处右键菜单动作 key。
    pub empty_verbs: Vec<String>,
    /// 将 RarkZip 动作收进级联子菜单。
    pub cascade: bool,
    /// 右键菜单不显示 RarkZip 图标。
    pub hide_icon: bool,
    /// 禁用 RarkZip 经典右键菜单（文件关联保留）。
    pub disable_menu: bool,
}

impl Default for IntegrationSettings {
    fn default() -> Self {
        Self {
            associations: [
                "zip", "zipx", "rar", "7z", "alz", "egg", "cab", "bh", "001", "arj", "lha", "lzh",
                "pma", "ace", "arc", "aes", "zpaq", "zstd", "br", "pea", "tar", "gz", "tgz", "bz",
                "bz2", "tbz", "tbz2", "xz", "txz", "lzma", "tlz", "lz", "uu", "uue", "xxe", "z",
                "lz4", "zst", "tzst",
            ]
            .into_iter()
            .map(ToString::to_string)
            .collect(),
            double_click: "open".to_string(),
            double_click_folder: String::new(),
            extract_verbs: [
                "here",
                "here_delete",
                "smart",
                "smart_delete",
                "folder",
                "folder_delete",
                "other",
            ]
            .into_iter()
            .map(ToString::to_string)
            .collect(),
            extract_any_file: false,
            hash_menu: true,
            compress_verbs: ["zip", "sevenz", "each_zip", "gui"]
                .into_iter()
                .map(ToString::to_string)
                .collect(),
            each_compress_defaulted: true,
            menu_aligned: false,
            empty_verbs: Vec::new(),
            cascade: true,
            hide_icon: false,
            disable_menu: false,
        }
    }
}

/// 窗口上次的非最大化尺寸。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowSize {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    pub window_width: f32,
    pub window_height: f32,
    /// 主窗口上次关闭时的屏幕位置；从未记录时为 None。
    #[serde(default)]
    pub window_x: Option<f32>,
    #[serde(default)]
    pub window_y: Option<f32>,
    /// 独立功能窗口与快速弹窗的上次尺寸（key 见 GUI 中的窗口标识）。
    pub window_sizes: HashMap<String, WindowSize>,
    pub recent_files: Vec<String>,
    pub last_extract_dir: String,
    /// 快速解压：解压到与压缩包同名的文件夹。
    pub last_extract_same_folder: bool,
    /// 快速解压：解压后删除压缩文件。
    pub last_extract_delete_archive: bool,
    /// 是否已重置旧版快速弹窗尺寸（一次性迁移到贴合内容的默认尺寸）。
    #[serde(default)]
    pub quick_sizes_defaulted: bool,
    pub last_create_dir: String,
    pub settings_page: usize,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            window_width: 1360.0,
            window_height: 820.0,
            window_x: None,
            window_y: None,
            window_sizes: HashMap::new(),
            recent_files: Vec::new(),
            last_extract_dir: String::new(),
            last_extract_same_folder: true,
            last_extract_delete_archive: false,
            quick_sizes_defaulted: false,
            last_create_dir: String::new(),
            settings_page: 0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub general: GeneralSettings,
    pub extraction: ExtractionSettings,
    pub compression: CompressionSettings,
    pub view: ViewSettings,
    pub security: SecuritySettings,
    pub advanced: AdvancedSettings,
    pub integration: IntegrationSettings,
    pub hash: HashSettings,
    pub crypt: CryptSettings,
    pub ui: UiSettings,
}

impl Settings {
    pub fn load() -> Self {
        let path = settings_path();
        let mut settings: Self = std::fs::read(to_extended(&path))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        settings.migrate();
        // 旧版本默认记录最近打开（但从未展示）；新设置默认关闭。
        // 一次性迁移，之后用户手动开启会被保留。
        settings.apply_history_default();
        if !settings.view.panels_defaulted {
            // 旧版本三个面板默认都打开；按新设计迁移为仅导航打开。
            settings.view.preview_enabled = false;
            settings.view.show_comment = false;
            settings.view.show_tree = true;
            settings.view.panels_defaulted = true;
        }
        settings
    }

    /// 把旧版默认右键菜单动词集升级为包含“解压到其他文件夹 / 管理员 / 预览”的新菜单。
    ///
    /// 只在动词集与旧默认值完全一致时迁移，避免覆盖用户自定义选择。
    fn migrate(&mut self) {
        const OLD_EXTRACT: [&str; 5] = ["open", "here", "smart", "folder", "test"];
        const OLD_COMPRESS: [&str; 3] = ["zip", "sevenz", "gui"];
        const OLD_ASSOCIATIONS: [&str; 17] = [
            "zip", "zipx", "rar", "7z", "cab", "lzh", "arj", "tar", "gz", "tgz", "bz2", "tbz",
            "tbz2", "xz", "txz", "zst", "tzst",
        ];
        let is_old_associations = self.integration.associations.len() == OLD_ASSOCIATIONS.len()
            && OLD_ASSOCIATIONS
                .iter()
                .all(|ext| self.integration.associations.iter().any(|item| item == ext));
        if is_old_associations {
            self.integration.associations = IntegrationSettings::default().associations;
        }
        let is_old_extract = self.integration.extract_verbs.len() == OLD_EXTRACT.len()
            && OLD_EXTRACT.iter().all(|key| {
                self.integration
                    .extract_verbs
                    .iter()
                    .any(|item| item == key)
            });
        if is_old_extract {
            self.integration.extract_verbs = IntegrationSettings::default().extract_verbs;
        }
        let is_old_compress = self.integration.compress_verbs.len() == OLD_COMPRESS.len()
            && OLD_COMPRESS.iter().all(|key| {
                self.integration
                    .compress_verbs
                    .iter()
                    .any(|item| item == key)
            });
        if is_old_compress {
            self.integration.compress_verbs = IntegrationSettings::default().compress_verbs;
        }
        if !self.integration.each_compress_defaulted {
            // 旧版压缩动词列表没有“每个条目独立压缩”动作；一次性加入，
            // 之后用户可在设置中自行取消。
            for key in ["each_zip", "each_7z"] {
                if !self
                    .integration
                    .compress_verbs
                    .iter()
                    .any(|item| item == key)
                {
                    self.integration.compress_verbs.push(key.to_string());
                }
            }
            self.integration.each_compress_defaulted = true;
        }
        if !self.ui.quick_sizes_defaulted {
            // 旧版快速弹窗统一使用 600×640，内容较少时留白明显；一次性丢弃旧尺寸，
            // 让新的按内容计算的默认尺寸生效。之后用户手动调整仍会正常记忆。
            self.ui.window_sizes.remove("quick_extract");
            self.ui.window_sizes.remove("quick_extract_progress");
            self.ui.window_sizes.remove("quick_compress");
            self.ui.quick_sizes_defaulted = true;
        }
        // 旧版把哈希动作混在解压菜单里；新版本有独立的哈希设置与菜单开关。
        if self
            .integration
            .extract_verbs
            .iter()
            .any(|key| key == "hash")
        {
            self.integration.hash_menu = true;
            self.integration.extract_verbs.retain(|key| key != "hash");
        }
        if !self.integration.menu_aligned {
            // 把旧版默认集合一次性收敛为常用动作；用户自定义过的
            // 列表保持不变。
            const PREV_EXTRACT: [&str; 8] = [
                "open",
                "here",
                "smart",
                "other",
                "smart_delete",
                "folder",
                "folder_delete",
                "here_delete",
            ];
            const PREV_EXTRACT_FULL: [&str; 11] = [
                "open",
                "smart",
                "folder",
                "here",
                "other",
                "here_delete",
                "smart_delete",
                "folder_delete",
                "admin",
                "test",
                "preview",
            ];
            const PREV_COMPRESS: [&str; 5] = ["zip", "sevenz", "gui", "each_zip", "each_7z"];
            const PREV_COMPRESS_FULL: [&str; 6] =
                ["zip", "sevenz", "gui", "admin", "each_zip", "each_7z"];
            let matches = |keys: &[String], expected: &[&str]| {
                keys.len() == expected.len()
                    && expected
                        .iter()
                        .all(|key| keys.iter().any(|item| item == key))
            };
            if matches(&self.integration.extract_verbs, &PREV_EXTRACT)
                || matches(&self.integration.extract_verbs, &PREV_EXTRACT_FULL)
            {
                self.integration.extract_verbs = IntegrationSettings::default().extract_verbs;
            }
            if matches(&self.integration.compress_verbs, &PREV_COMPRESS)
                || matches(&self.integration.compress_verbs, &PREV_COMPRESS_FULL)
            {
                self.integration.compress_verbs = IntegrationSettings::default().compress_verbs;
            }
            if matches(&self.integration.empty_verbs, &["open", "new"]) {
                self.integration.empty_verbs.clear();
            }
            self.integration.menu_aligned = true;
        }
    }

    /// 一次性把旧配置的“记录最近打开”迁移为默认关闭；迁移后用户手动开启会保留。
    fn apply_history_default(&mut self) {
        if !self.advanced.history_defaulted {
            self.advanced.history = false;
            self.advanced.history_defaulted = true;
        }
    }

    /// 读取某个窗口上次保存的尺寸；不存在时使用默认值。
    pub fn window_size(&self, key: &str, default: (f32, f32)) -> (f32, f32) {
        self.ui
            .window_sizes
            .get(key)
            .map(|size| {
                (
                    size.width.clamp(520.0, 3840.0),
                    size.height.clamp(380.0, 2160.0),
                )
            })
            .unwrap_or(default)
    }

    /// 记住窗口尺寸。仅应在窗口未最大化/全屏时调用。
    pub fn remember_window_size(&mut self, key: &str, width: f32, height: f32) {
        self.ui.window_sizes.insert(
            key.to_string(),
            WindowSize {
                width: width.clamp(520.0, 3840.0),
                height: height.clamp(380.0, 2160.0),
            },
        );
    }

    /// 只把某个窗口的尺寸写回磁盘。
    ///
    /// 多窗口各自持有 Settings 副本；若用内存副本整体 `save()`，
    /// 会覆盖其他窗口刚保存的尺寸。这里先读取最新文件再合并。
    pub fn persist_window_size(key: &str, width: f32, height: f32) -> Result<()> {
        let mut settings = Self::load();
        settings.remember_window_size(key, width, height);
        settings.save()
    }

    /// 只把主窗口尺寸写回磁盘，保留其他窗口刚保存的尺寸与设置。
    pub fn persist_main_window_size(width: f32, height: f32) -> Result<()> {
        let mut settings = Self::load();
        settings.ui.window_width = width;
        settings.ui.window_height = height;
        settings.save()
    }

    /// 只把主窗口位置与尺寸写回磁盘。
    pub fn persist_main_window_bounds(x: f32, y: f32, width: f32, height: f32) -> Result<()> {
        let mut settings = Self::load();
        settings.ui.window_x = Some(x);
        settings.ui.window_y = Some(y);
        settings.ui.window_width = width;
        settings.ui.window_height = height;
        settings.save()
    }

    pub fn save(&self) -> Result<()> {
        let path = settings_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(to_extended(parent))?;
        }
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|err| crate::error::EngineError::Other(err.to_string()))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(to_extended(&tmp), bytes)?;
        std::fs::rename(to_extended(&tmp), to_extended(&path))?;
        Ok(())
    }

    pub fn add_recent(&mut self, path: &std::path::Path) {
        let value = path.to_string_lossy().to_string();
        self.ui.recent_files.retain(|item| item != &value);
        self.ui.recent_files.insert(0, value);
        self.ui.recent_files.truncate(10);
    }

    pub fn clear_recent(&mut self) {
        self.ui.recent_files.clear();
    }
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(".rarkzip"));
    base.join("RarkZip")
}

pub fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

pub fn passwords_path() -> PathBuf {
    data_dir().join("passwords.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_old_default_context_menu_verbs() {
        let mut settings = Settings::default();
        settings.integration.extract_verbs = ["open", "here", "smart", "folder", "test"]
            .into_iter()
            .map(ToString::to_string)
            .collect();
        settings.integration.compress_verbs = ["zip", "sevenz", "gui"]
            .into_iter()
            .map(ToString::to_string)
            .collect();
        settings.migrate();
        assert!(settings
            .integration
            .extract_verbs
            .iter()
            .any(|key| key == "other"));
        assert!(settings
            .integration
            .extract_verbs
            .iter()
            .any(|key| key == "here_delete"));
        assert!(!settings
            .integration
            .extract_verbs
            .iter()
            .any(|key| key == "admin"));
        assert!(settings
            .integration
            .compress_verbs
            .iter()
            .any(|key| key == "each_zip"));
        assert!(!settings
            .integration
            .compress_verbs
            .iter()
            .any(|key| key == "admin"));
    }

    #[test]
    fn moves_legacy_hash_verb_to_dedicated_menu_setting() {
        let mut settings = Settings::default();
        settings.integration.hash_menu = false;
        settings.integration.extract_verbs.push("hash".to_string());
        settings.migrate();
        assert!(settings.integration.hash_menu);
        assert!(!settings
            .integration
            .extract_verbs
            .iter()
            .any(|key| key == "hash"));
    }

    #[test]
    fn hash_settings_default_to_common_algorithms() {
        let settings = HashSettings::default();
        assert_eq!(
            settings.algorithms,
            vec!["sha256", "sha1", "md5", "crc32"]
                .into_iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        );
        assert!(!settings.auto_start);
    }

    #[test]
    fn migrates_previous_eight_verb_default_to_common_set() {
        let mut settings = Settings::default();
        settings.integration.extract_verbs = [
            "open",
            "here",
            "smart",
            "other",
            "smart_delete",
            "folder",
            "folder_delete",
            "here_delete",
        ]
        .into_iter()
        .map(ToString::to_string)
        .collect();
        settings.integration.compress_verbs = ["zip", "sevenz", "gui", "each_zip", "each_7z"]
            .into_iter()
            .map(ToString::to_string)
            .collect();
        settings.integration.empty_verbs = ["open", "new"]
            .into_iter()
            .map(ToString::to_string)
            .collect();
        settings.migrate();
        assert!(!settings
            .integration
            .extract_verbs
            .iter()
            .any(|key| key == "open"));
        assert!(!settings
            .integration
            .compress_verbs
            .iter()
            .any(|key| key == "each_7z"));
        assert!(settings.integration.empty_verbs.is_empty());
        assert!(settings.integration.menu_aligned);
    }
    #[test]
    fn keeps_custom_context_menu_verbs() {
        let mut settings = Settings::default();
        settings.integration.extract_verbs = vec!["smart".to_string()];
        settings.migrate();
        assert_eq!(
            settings.integration.extract_verbs,
            vec!["smart".to_string()]
        );
    }

    #[test]
    fn custom_titlebar_defaults_on() {
        assert!(ViewSettings::default().use_custom_titlebar);
    }

    #[test]
    fn recent_files_default_to_off() {
        let settings = Settings::default();
        assert!(!settings.advanced.history);
        assert!(settings.ui.window_x.is_none());
        assert!(settings.ui.window_y.is_none());
    }

    #[test]
    fn legacy_recent_files_setting_is_migrated_once() {
        let mut settings = Settings::default();
        settings.advanced.history = true;
        settings.advanced.history_defaulted = false;
        settings.apply_history_default();
        assert!(!settings.advanced.history);
        assert!(settings.advanced.history_defaulted);

        // 用户迁移后手动开启，后续加载不再重置。
        settings.advanced.history = true;
        settings.apply_history_default();
        assert!(settings.advanced.history);
    }

    #[test]
    fn panels_default_to_nav_only() {
        let view = ViewSettings::default();
        assert!(view.show_tree);
        assert!(!view.preview_enabled);
        assert!(!view.show_comment);
        assert!(view.panels_defaulted);
        // 旧 settings.json 缺少新字段时按 false 读取，触发一次性迁移。
        let legacy: ViewSettings =
            serde_json::from_str(r#"{"preview_enabled":true,"show_comment":true}"#).unwrap();
        assert!(!legacy.panels_defaulted);
    }

    #[test]
    fn migrates_old_association_defaults() {
        let mut settings = Settings::default();
        settings.integration.associations = [
            "zip", "zipx", "rar", "7z", "cab", "lzh", "arj", "tar", "gz", "tgz", "bz2", "tbz",
            "tbz2", "xz", "txz", "zst", "tzst",
        ]
        .into_iter()
        .map(ToString::to_string)
        .collect();
        settings.migrate();
        assert!(settings
            .integration
            .associations
            .iter()
            .any(|ext| ext == "alz"));
        assert!(settings
            .integration
            .associations
            .iter()
            .any(|ext| ext == "lz4"));
    }
}
