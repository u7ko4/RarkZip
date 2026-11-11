use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, Sender};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::slider::{SliderEvent, SliderState, SliderValue};
use gpui_kit::component::{
    v_virtual_list, InteractiveElementExt, Sizable, VirtualListScrollHandle,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::*;

use nimzip_engine::{
    checksum_expectations, format_size, hash_file_many, hash_file_with_progress, open_archive,
    parse_checksum_text, ArchiveFormat, ArchiveInfo, Codepage, CreateOutcome, EngineError,
    ExtractMode, ExtractOptions, HashAlgorithm, OpenOptions, PasswordBook, RecoveryControl,
    RecoveryMode, RecoveryOptions, RecoveryOutcome, Settings, ZipAlgorithm,
};

use crate::components::*;
use crate::design;
use crate::i18n::{tr, Language};
use crate::task::*;
use crate::theme::{self, Palette, Skin, ThemeMode};

mod chrome;
mod context_menu;
mod dropdown;
mod main_view;
mod menu;
mod settings_view;

pub(crate) use dropdown::DropdownKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Main,
    Tasks,
    Settings,
    NewArchive,
    PasswordManager,
    PasswordRecovery,
    Hash,
}

/// 应用主菜单中的二级子菜单。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuSubmenu {
    Tools,
    Recent,
}

/// 以独立系统窗口打开的功能页。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FeatureWindow {
    NewArchive,
    PasswordRecovery,
    Hash,
}

impl FeatureWindow {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::NewArchive => "新建压缩文件",
            Self::PasswordRecovery => "密码恢复",
            Self::Hash => "文件哈希",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::NewArchive => IconName::FileText,
            Self::PasswordRecovery => IconName::Key,
            Self::Hash => IconName::Hash,
        }
    }

    fn page(self) -> Page {
        match self {
            Self::NewArchive => Page::NewArchive,
            Self::PasswordRecovery => Page::PasswordRecovery,
            Self::Hash => Page::Hash,
        }
    }

    pub(crate) fn size(self) -> (f32, f32) {
        match self {
            Self::NewArchive => (980.0, 580.0),
            Self::PasswordRecovery => (720.0, 620.0),
            Self::Hash => (980.0, 480.0),
        }
    }

    /// 尺寸记忆使用的稳定键。
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::NewArchive => "new_archive",
            Self::PasswordRecovery => "recovery",
            Self::Hash => "hash",
        }
    }
}

/// 独立新建压缩包窗口的结果状态。
#[derive(Debug, Clone)]
pub(crate) enum FeatureOutcome {
    Created(PathBuf),
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsPage {
    General,
    Associations,
    ContextMenu,
    Extraction,
    Compression,
    View,
    Security,
    Advanced,
    Language,
    Hash,
}

impl SettingsPage {
    const ALL: [SettingsPage; 10] = [
        SettingsPage::General,
        SettingsPage::Associations,
        SettingsPage::ContextMenu,
        SettingsPage::Extraction,
        SettingsPage::Compression,
        SettingsPage::View,
        SettingsPage::Security,
        SettingsPage::Advanced,
        SettingsPage::Language,
        SettingsPage::Hash,
    ];

    fn label(self, language: Language) -> &'static str {
        tr(
            language,
            match self {
                Self::General => "常规",
                Self::Associations => "文件关联",
                Self::ContextMenu => "右键菜单",
                Self::Extraction => "解压",
                Self::Compression => "压缩",
                Self::View => "外观",
                Self::Security => "安全",
                Self::Advanced => "高级",
                Self::Language => "语言",
                Self::Hash => "哈希",
            },
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortColumn {
    Name,
    Compressed,
    Original,
    Type,
    Modified,
    Method,
    Encryption,
    Crc,
    Attributes,
    Comment,
    Extra,
}

impl SortColumn {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Original => "original",
            Self::Compressed => "compressed",
            Self::Type => "kind",
            Self::Modified => "modified",
            Self::Method => "method",
            Self::Encryption => "encryption",
            Self::Crc => "crc",
            Self::Attributes => "attributes",
            Self::Comment => "comment",
            Self::Extra => "extra",
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "name" => Self::Name,
            "original" => Self::Original,
            "compressed" => Self::Compressed,
            "kind" | "type" => Self::Type,
            "modified" => Self::Modified,
            "method" => Self::Method,
            "encryption" => Self::Encryption,
            "crc" => Self::Crc,
            "attributes" => Self::Attributes,
            "comment" => Self::Comment,
            "extra" => Self::Extra,
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Self::Name => "名称",
            Self::Compressed => "压缩后",
            Self::Original => "大小",
            Self::Type => "类型",
            Self::Modified => "修改时间",
            Self::Method => "方法",
            Self::Encryption => "加密",
            Self::Crc => "CRC",
            Self::Attributes => "属性",
            Self::Comment => "注释",
            Self::Extra => "扩展",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileViewMode {
    List,
    Icons,
}

impl FileViewMode {
    fn from_setting(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "icons" | "grid" | "thumbnails" => Self::Icons,
            _ => Self::List,
        }
    }

    fn as_setting(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Icons => "icons",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContextTarget {
    Row(usize),
    Blank,
}

#[derive(Debug, Clone, Copy)]
struct ArchiveContextMenu {
    x: f32,
    y: f32,
    target: ContextTarget,
}

#[derive(Debug, Clone)]
enum Row {
    Entry(usize),
}

#[derive(Default, Clone)]
struct DirNode {
    path: String,
    children: BTreeMap<String, DirNode>,
    direct_files: usize,
    total_files: usize,
}

impl DirNode {
    fn count_files(&mut self) -> usize {
        self.total_files = self.direct_files
            + self
                .children
                .values_mut()
                .map(|child| child.count_files())
                .sum::<usize>();
        self.total_files
    }
}

struct ArchiveState {
    path: PathBuf,
    info: ArchiveInfo,
    password: Option<String>,
    tree: DirNode,
}

#[derive(Default)]
struct PreviewState {
    entry: Option<usize>,
    image_path: Option<PathBuf>,
    dimensions: Option<(u32, u32)>,
    error: Option<String>,
    cache: HashMap<usize, PathBuf>,
    generation: u64,
}

#[derive(Debug, Clone)]
enum PendingAction {
    Open {
        path: PathBuf,
    },
    Extract {
        path: PathBuf,
        options: ExtractOptions,
    },
    Verify {
        path: PathBuf,
    },
    OpenEntry {
        archive: PathBuf,
        index: usize,
        entry: String,
    },
}

#[derive(Debug, Clone)]
enum Modal {
    Password {
        archive: PathBuf,
        pending: PendingAction,
        error: Option<String>,
    },
    Rename {
        index: usize,
    },
    NewFolder {
        parent: String,
    },
    Comment,
    Columns,
    Toolbar,
    ConfirmDeletePassword {
        index: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryUiMode {
    Dictionary,
    Mask,
    Brute,
}

#[derive(Debug, Clone)]
struct HashBatchItem {
    path: PathBuf,
    hashes: Vec<(HashAlgorithm, String)>,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HashExportFormat {
    Sha256Sum,
    Md5Sum,
    Sfv,
}

impl HashExportFormat {
    const ALL: [Self; 3] = [Self::Sha256Sum, Self::Md5Sum, Self::Sfv];

    fn label(self) -> &'static str {
        match self {
            Self::Sha256Sum => "SHA256SUM",
            Self::Md5Sum => "MD5SUM",
            Self::Sfv => "SFV",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Sha256Sum => "sha256",
            Self::Md5Sum => "md5",
            Self::Sfv => "sfv",
        }
    }
}

#[derive(Debug, Clone)]
struct HashContextMenu {
    x: f32,
    y: f32,
    file: PathBuf,
    algorithm: Option<HashAlgorithm>,
    hash: Option<String>,
}

fn hash_algorithms_from_settings(settings: &Settings) -> Vec<HashAlgorithm> {
    let mut algorithms: Vec<HashAlgorithm> = settings
        .hash
        .algorithms
        .iter()
        .filter_map(|name| HashAlgorithm::from_name(name))
        .collect();
    algorithms.sort_by_key(|algorithm| {
        HashAlgorithm::ALL
            .iter()
            .position(|candidate| candidate == algorithm)
            .unwrap_or(usize::MAX)
    });
    algorithms.dedup();
    algorithms.truncate(6);
    if algorithms.is_empty() {
        vec![
            HashAlgorithm::Sha256,
            HashAlgorithm::Sha1,
            HashAlgorithm::Md5,
            HashAlgorithm::Crc32,
        ]
    } else {
        algorithms
    }
}

fn hash_expectations_from_text(text: &str) -> HashMap<String, String> {
    let entries = parse_checksum_text(text);
    if !entries.is_empty() {
        return checksum_expectations(&entries);
    }
    let compact: String = text
        .trim()
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != ':' && *ch != '-')
        .collect::<String>()
        .to_ascii_lowercase();
    let valid_length = matches!(compact.len(), 8 | 16 | 32 | 40 | 64 | 128);
    if valid_length && compact.chars().all(|ch| ch.is_ascii_hexdigit()) {
        HashMap::from([("*".to_string(), compact)])
    } else {
        HashMap::new()
    }
}

#[derive(Debug)]
enum HashBatchEvent {
    Progress {
        run_id: u64,
        done: usize,
        total: usize,
    },
    Done {
        run_id: u64,
        results: Vec<HashBatchItem>,
    },
}

#[derive(Debug)]
enum RecoveryEvent {
    Progress {
        run_id: u64,
        attempts: u64,
        total: u64,
    },
    Done {
        run_id: u64,
        result: std::result::Result<RecoveryOutcome, String>,
    },
}

struct NewArchiveDraft {
    sources: Vec<PathBuf>,
    source_bytes: u64,
    format: ArchiveFormat,
    level: u8,
    include_root: bool,
    test_after: bool,
    delete_sources: bool,
    output_dir: Option<PathBuf>,
    zip_algorithm: ZipAlgorithm,
    sfx: bool,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToastKind {
    Info,
    Success,
    Error,
}

struct Toast {
    message: String,
    kind: ToastKind,
    serial: u64,
}

pub struct MainView {
    focus_handle: FocusHandle,
    palette: Palette,
    theme_mode: ThemeMode,
    skin: Skin,
    settings: Settings,
    language: Language,
    page: Page,
    menu_open: bool,
    menu_submenu: Option<MenuSubmenu>,
    /// 悬停计时令牌：离开或切换菜单项时递增，使旧的 1 秒延时失效。
    menu_hover_serial: u64,
    dropdown: Option<DropdownKind>,
    dropdown_bounds: HashMap<DropdownKind, Bounds<Pixels>>,
    settings_page: SettingsPage,
    archive: Option<ArchiveState>,
    selected: Option<usize>,
    multi_selected: HashSet<usize>,
    archive_menu: Option<ArchiveContextMenu>,
    file_view: FileViewMode,
    current_dir: String,
    expanded: HashSet<String>,
    sort_column: SortColumn,
    sort_direction: SortDirection,
    search: Entity<InputState>,
    search_query: String,
    rows: Vec<Row>,
    row_sizes: Rc<Vec<Size<Pixels>>>,
    list_scroll: VirtualListScrollHandle,
    table_scroll: ScrollHandle,
    hash_table_scroll: ScrollHandle,
    table_scroll_bounds: Option<Bounds<Pixels>>,
    h_scroll_drag: Option<f32>,
    tasks: Vec<TaskItem>,
    task_sender: Sender<TaskEvent>,
    task_receiver: Receiver<TaskEvent>,
    next_task_id: u64,
    toast: Option<Toast>,
    toast_serial: u64,
    modal: Option<Modal>,
    new_draft: Option<NewArchiveDraft>,
    new_password: Entity<InputState>,
    new_name: Entity<InputState>,
    new_comment: Entity<InputState>,
    new_advanced_open: bool,
    new_advanced_base_width: Option<f32>,
    level_slider: Entity<SliderState>,
    icon_size_slider: Entity<SliderState>,
    password_input: Entity<InputState>,
    exclude_input: Entity<InputState>,
    settings_exclude_input: Entity<InputState>,
    password_book: PasswordBook,
    password_new: Entity<InputState>,
    password_note: Entity<InputState>,
    password_reveal: bool,
    recovery_path: Option<PathBuf>,
    recovery_mode: RecoveryUiMode,
    recovery_dictionary: Vec<String>,
    recovery_dictionary_path: Option<PathBuf>,
    recovery_mask: Entity<InputState>,
    recovery_charset: Entity<InputState>,
    recovery_min: u8,
    recovery_max: u8,
    recovery_threads: usize,
    recovery_running: bool,
    recovery_paused: bool,
    recovery_attempts: u64,
    recovery_total: u64,
    recovery_result: Option<String>,
    recovery_error: Option<String>,
    recovery_control: RecoveryControl,
    recovery_run_id: u64,
    recovery_sender: Sender<RecoveryEvent>,
    recovery_receiver: Receiver<RecoveryEvent>,
    rename_input: Entity<InputState>,
    comment_input: Entity<InputState>,
    hash_batch_paths: Vec<PathBuf>,
    hash_batch_algorithms: Vec<HashAlgorithm>,
    hash_batch_results: Vec<HashBatchItem>,
    hash_batch_running: bool,
    hash_batch_progress: (usize, usize),
    hash_batch_error: Option<String>,
    hash_batch_expected: HashMap<String, String>,
    hash_verify_input: Entity<InputState>,
    hash_export_format: HashExportFormat,
    hash_selected_row: Option<usize>,
    hash_context_menu: Option<HashContextMenu>,
    hash_batch_run_id: u64,
    hash_batch_sender: Sender<HashBatchEvent>,
    hash_batch_receiver: Receiver<HashBatchEvent>,
    preview: PreviewState,
    preview_sender: Sender<PreviewEvent>,
    preview_receiver: Receiver<PreviewEvent>,
    open_sender: Sender<OpenEntryEvent>,
    open_receiver: Receiver<OpenEntryEvent>,
    preview_ready: bool,
    last_title: String,
    /// 构造窗口时使用的标题栏模式（GPUI 的透明标题栏只能在创建时指定）。
    custom_titlebar: bool,
    /// 独立功能窗口标记；None 表示主窗口。
    feature_window: Option<FeatureWindow>,
    /// 独立窗口内“开始压缩”后的结果状态。
    feature_result: Option<FeatureOutcome>,
}

#[derive(Debug)]
struct PreviewEvent {
    generation: u64,
    entry: usize,
    result: Result<(PathBuf, Option<(u32, u32)>), String>,
}

#[derive(Debug)]
struct OpenEntryEvent {
    name: String,
    result: Result<(), String>,
}

impl MainView {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        initial_path: Option<PathBuf>,
        new_source: Option<PathBuf>,
        new_output: Option<PathBuf>,
    ) -> Self {
        Self::build(window, cx, initial_path, new_source, new_output, None)
    }

    /// 创建一个只包含单个功能页的独立窗口。
    pub fn new_feature(
        window: &mut Window,
        cx: &mut Context<Self>,
        feature: FeatureWindow,
        new_sources: Vec<PathBuf>,
        new_output: Option<PathBuf>,
    ) -> Self {
        let mut view = Self::build(window, cx, None, None, None, Some(feature));
        match feature {
            FeatureWindow::NewArchive => {
                view.init_new_archive_draft(new_sources, new_output, window, cx)
            }
            FeatureWindow::PasswordRecovery => {
                view.init_password_recovery(cx);
                if let Some(path) = new_sources.into_iter().next() {
                    view.recovery_path = Some(path);
                }
            }
            FeatureWindow::Hash => {
                view.init_hash(cx);
                for path in new_sources {
                    if !view.hash_batch_paths.contains(&path) {
                        view.hash_batch_paths.push(path);
                    }
                }
                if view.settings.hash.auto_start && !view.hash_batch_paths.is_empty() {
                    view.start_hash_batch(cx);
                }
            }
        }
        view
    }

    fn build(
        window: &mut Window,
        cx: &mut Context<Self>,
        initial_path: Option<PathBuf>,
        new_source: Option<PathBuf>,
        new_output: Option<PathBuf>,
        feature_window: Option<FeatureWindow>,
    ) -> Self {
        let settings = Settings::load();
        let theme_mode = ThemeMode::from_setting(&settings.view.theme);
        let skin = Skin::from_setting(&settings.view.theme_skin);
        let dark = theme_mode.is_dark(window.appearance());
        theme::set_ui_font_size(settings.view.font_size);
        theme::apply(cx, dark, skin);
        let palette = Palette::for_skin(dark, skin);
        let language = Language::from_setting(&settings.general.language);

        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(if language == Language::En {
                "Search in archive…  Ctrl+F"
            } else {
                "在压缩包内查找…  Ctrl+F"
            })
        });
        let new_password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("可选密码")
        });
        let new_name = cx.new(|cx| InputState::new(window, cx).placeholder("归档名称"));
        let new_comment =
            cx.new(|cx| InputState::new(window, cx).placeholder("压缩包注释（可选）"));
        let level_slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(9.0)
                .step(1.0)
                .default_value(5.0)
        });
        let icon_size = settings.view.thumbnail_size.clamp(64.0, 200.0);
        let icon_size_slider = cx.new(|_| {
            SliderState::new()
                .min(64.0)
                .max(200.0)
                .step(4.0)
                .default_value(icon_size)
        });
        let password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("输入密码")
        });
        let exclude_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(settings.compression.exclude.clone())
        });
        let settings_exclude_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(settings.compression.exclude.clone())
        });
        let password_new = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("输入新密码")
        });
        let password_note = cx.new(|cx| InputState::new(window, cx).placeholder("备注（可选）"));
        let password_book = PasswordBook::load_default();
        let recovery_mask = cx.new(|cx| InputState::new(window, cx).default_value("?l?l?d?d"));
        let recovery_charset = cx.new(|cx| {
            InputState::new(window, cx).default_value("abcdefghijklmnopqrstuvwxyz0123456789")
        });
        let rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("新条目路径"));
        let comment_input = cx.new(|cx| InputState::new(window, cx).placeholder("压缩包注释"));
        let hash_verify_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("粘贴期望的哈希值或校验和文件内容，自动匹配下方结果")
        });
        let hash_algorithms = hash_algorithms_from_settings(&settings);

        let (task_sender, task_receiver) = unbounded();
        let (recovery_sender, recovery_receiver) = unbounded();
        let (hash_batch_sender, hash_batch_receiver) = unbounded();
        let (preview_sender, preview_receiver) = unbounded();
        let (open_sender, open_receiver) = unbounded();
        let focus_handle = cx.focus_handle();
        let custom_titlebar = settings.view.use_custom_titlebar;
        let file_view = FileViewMode::from_setting(&settings.view.file_list_view);
        let settings_page = SettingsPage::ALL
            .get(settings.ui.settings_page)
            .copied()
            .unwrap_or(SettingsPage::General);
        let initial_page = feature_window
            .map(FeatureWindow::page)
            .unwrap_or(Page::Main);

        let mut view = Self {
            focus_handle,
            palette,
            theme_mode,
            skin,
            settings,
            language,
            page: initial_page,
            menu_open: false,
            menu_submenu: None,
            menu_hover_serial: 0,
            dropdown: None,
            dropdown_bounds: HashMap::new(),
            settings_page,
            archive: None,
            selected: None,
            multi_selected: HashSet::new(),
            archive_menu: None,
            file_view,
            current_dir: String::new(),
            expanded: HashSet::new(),
            sort_column: SortColumn::Name,
            sort_direction: SortDirection::Asc,
            search,
            search_query: String::new(),
            rows: Vec::new(),
            row_sizes: Rc::new(Vec::new()),
            list_scroll: VirtualListScrollHandle::new(),
            table_scroll: ScrollHandle::new(),
            hash_table_scroll: ScrollHandle::new(),
            table_scroll_bounds: None,
            h_scroll_drag: None,
            tasks: Vec::new(),
            task_sender,
            task_receiver,
            next_task_id: 1,
            toast: None,
            toast_serial: 0,
            modal: None,
            new_draft: None,
            new_password,
            new_name,
            new_comment,
            new_advanced_open: false,
            new_advanced_base_width: None,
            level_slider,
            icon_size_slider,
            password_input,
            exclude_input,
            settings_exclude_input,
            password_book,
            password_new,
            password_note,
            password_reveal: false,
            recovery_path: None,
            recovery_mode: RecoveryUiMode::Dictionary,
            recovery_dictionary: Vec::new(),
            recovery_dictionary_path: None,
            recovery_mask,
            recovery_charset,
            recovery_min: 1,
            recovery_max: 6,
            recovery_threads: std::thread::available_parallelism()
                .map(|value| value.get())
                .unwrap_or(4)
                .min(16),
            recovery_running: false,
            recovery_paused: false,
            recovery_attempts: 0,
            recovery_total: 0,
            recovery_result: None,
            recovery_error: None,
            recovery_control: RecoveryControl::new(),
            recovery_run_id: 0,
            recovery_sender,
            recovery_receiver,
            rename_input,
            comment_input,
            hash_batch_paths: Vec::new(),
            hash_batch_algorithms: hash_algorithms,
            hash_batch_results: Vec::new(),
            hash_batch_running: false,
            hash_batch_progress: (0, 0),
            hash_batch_error: None,
            hash_batch_expected: HashMap::new(),
            hash_verify_input,
            hash_export_format: HashExportFormat::Sha256Sum,
            hash_selected_row: None,
            hash_context_menu: None,
            hash_batch_run_id: 0,
            hash_batch_sender,
            hash_batch_receiver,
            preview: PreviewState::default(),
            preview_sender,
            preview_receiver,
            open_sender,
            open_receiver,
            preview_ready: true,
            last_title: feature_window
                .map(|feature| feature.title().to_string())
                .unwrap_or_else(|| "NimZip".to_string()),
            custom_titlebar,
            feature_window,
            feature_result: None,
        };

        cx.subscribe(&view.search, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.search_query = this.search.read(cx).value().to_string();
                this.rebuild_rows();
                this.selected = None;
                this.multi_selected.clear();
                this.archive_menu = None;
                this.refresh_preview(cx);
                cx.notify();
            }
        })
        .detach();

        cx.subscribe(&view.password_input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_password(cx);
            }
        })
        .detach();

        cx.subscribe(
            &view.settings_exclude_input,
            |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.settings.compression.exclude =
                        this.settings_exclude_input.read(cx).value().to_string();
                    this.save_settings();
                }
            },
        )
        .detach();

        cx.subscribe(
            &view.hash_verify_input,
            |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = this.hash_verify_input.read(cx).value().to_string();
                    this.hash_batch_expected = hash_expectations_from_text(&text);
                    cx.notify();
                }
            },
        )
        .detach();

        window.focus(&view.focus_handle, cx);
        let weak_for_close = cx.entity().downgrade();
        let is_feature_window = view.feature_window.is_some();
        window.on_window_should_close(&*cx, move |window, cx| {
            let _ = weak_for_close.update(cx, |view, _| {
                if let Some(feature) = view.feature_window {
                    view.save_feature_window_state(window, feature);
                    if view.recovery_running {
                        view.recovery_control.cancel();
                    }
                } else {
                    view.save_window_state(window);
                }
            });
            true
        });
        eprintln!(
            "NimZip start: page=Main settings_page=General archive={}",
            initial_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "-".to_string())
        );
        cx.subscribe(&view.level_slider, |this, _, event: &SliderEvent, cx| {
            let value = match event {
                SliderEvent::Change(value) | SliderEvent::Release(value) => *value,
            };
            let level = match value {
                SliderValue::Single(value) => value.round() as u8,
                SliderValue::Range(_, value) => value.round() as u8,
            };
            if let Some(draft) = &mut this.new_draft {
                draft.level = level.min(9);
            }
            cx.notify();
        })
        .detach();

        cx.subscribe(
            &view.icon_size_slider,
            |this, _, event: &SliderEvent, cx| {
                let value = match event {
                    SliderEvent::Change(value) | SliderEvent::Release(value) => *value,
                };
                let size = match value {
                    SliderValue::Single(value) => value,
                    SliderValue::Range(_, value) => value,
                }
                .clamp(64.0, 200.0)
                .round();
                this.settings.view.thumbnail_size = size;
                this.save_settings();
                cx.notify();
            },
        )
        .detach();

        view.start_poll_loop(cx);
        view.refresh_preview(cx);

        if is_feature_window {
            return view;
        }

        if let Ok(page) = std::env::var("NIMZIP_START_PAGE") {
            match page.as_str() {
                "settings" => view.page = Page::Settings,
                "tasks" => view.page = Page::Tasks,
                _ => {}
            }
        }
        if let Ok(modal) = std::env::var("NIMZIP_START_MODAL") {
            match modal.as_str() {
                "new" => view.open_new_archive_modal(window, cx),
                "password-manager" => view.open_password_manager(window, cx),
                "recovery" => view.open_password_recovery(window, cx),
                "hash-batch" => view.open_hash_batch(cx),
                "columns" => view.open_columns(cx),
                "toolbar" => view.open_toolbar_settings(cx),
                _ => {}
            }
        }

        if let Some(path) = initial_path {
            view.open_path(path, None, cx);
        }
        if new_source.is_some() || new_output.is_some() {
            view.open_new_archive_with(window, cx, new_source.into_iter().collect(), new_output);
        }
        view
    }

    fn start_poll_loop(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |weak, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(90))
                .await;
            let alive = weak
                .update(cx, |view, cx| {
                    view.drain_events(cx);
                    true
                })
                .unwrap_or(false);
            if !alive {
                break;
            }
        })
        .detach();
    }

    fn drain_events(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        while let Ok(event) = self.task_receiver.try_recv() {
            self.handle_task_event(event, cx);
            changed = true;
        }
        while let Ok(event) = self.recovery_receiver.try_recv() {
            self.handle_recovery_event(event, cx);
            changed = true;
        }
        while let Ok(event) = self.hash_batch_receiver.try_recv() {
            self.handle_hash_batch_event(event, cx);
            changed = true;
        }
        while let Ok(event) = self.open_receiver.try_recv() {
            match event.result {
                Ok(()) => self.show_toast(
                    format!("已用系统程序打开：{}", event.name),
                    ToastKind::Info,
                    cx,
                ),
                Err(error) => {
                    self.show_toast(format!("打开文件失败：{error}"), ToastKind::Error, cx)
                }
            }
            changed = true;
        }
        while let Ok(event) = self.preview_receiver.try_recv() {
            if event.generation == self.preview.generation
                && self.preview.entry == Some(event.entry)
            {
                match event.result {
                    Ok((path, dimensions)) => {
                        self.preview.image_path = Some(path);
                        self.preview.dimensions = dimensions;
                        self.preview.error = None;
                    }
                    Err(error) => {
                        self.preview.image_path = None;
                        self.preview.error = Some(error);
                    }
                }
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }

    fn handle_task_event(&mut self, event: TaskEvent, cx: &mut Context<Self>) {
        match event {
            TaskEvent::Progress {
                id,
                phase,
                percent,
                detail,
            } => {
                if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
                    task.phase = phase.to_string();
                    task.progress = percent;
                    if !detail.is_empty() {
                        task.detail = detail;
                    }
                }
            }
            TaskEvent::Warn { id, message } => {
                if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
                    task.detail = message;
                }
            }
            TaskEvent::Finished { id, result } => {
                self.finish_task(id, result, cx);
            }
        }
        cx.notify();
    }

    fn finish_task(&mut self, id: u64, result: JobResult, cx: &mut Context<Self>) {
        let cancel_state = self
            .tasks
            .iter()
            .find(|task| task.id == id)
            .map(|task| task.cancel.load(Ordering::Relaxed));
        if cancel_state != Some(false) {
            if cancel_state == Some(true) {
                self.set_task_cancelled(id);
            }
            cx.notify();
            return;
        }
        match result {
            JobResult::Open {
                path,
                password,
                result,
            } => match result {
                Ok(info) => {
                    self.set_task_completed(
                        id,
                        format!(
                            "{} 项 · 原始 {}",
                            info.file_count(),
                            format_size(info.total_size())
                        ),
                    );
                    self.install_archive(path, info, password, cx);
                    self.page = Page::Main;
                }
                Err(error) if error.is_password_related() => {
                    self.set_task_failed(id, "需要密码");
                    self.show_password_modal(path.clone(), PendingAction::Open { path }, None, cx);
                }
                Err(error) => {
                    self.set_task_failed(id, error.to_string());
                    self.show_toast(format!("打开失败：{error}"), ToastKind::Error, cx);
                }
            },
            JobResult::Extract { path, result } => match result {
                Ok(outcome) => {
                    let detail = format!(
                        "{} 个文件 · {} · 嵌套 {} 个",
                        outcome.files,
                        format_size(outcome.bytes),
                        outcome.nested_archives
                    );
                    self.set_task_completed(id, detail);
                    if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
                        task.dest = Some(outcome.dest.clone());
                    }
                    if self.settings.extraction.open_folder_after {
                        let _ = open::that(&outcome.dest);
                    }
                    self.show_toast(
                        format!("解压完成：{} → {}", path.display(), outcome.dest.display()),
                        ToastKind::Success,
                        cx,
                    );
                }
                Err(EngineError::Cancelled) => {
                    self.set_task_cancelled(id);
                }
                Err(error) if error.is_password_related() => {
                    self.set_task_failed(id, "需要密码");
                    if let Some(task) = self.tasks.iter().find(|task| task.id == id) {
                        if let Some(RetrySpec::Extract {
                            path: archive,
                            options,
                        }) = &task.retry
                        {
                            let mut options = options.clone();
                            options.password = None;
                            self.show_password_modal(
                                archive.clone(),
                                PendingAction::Extract {
                                    path: archive.clone(),
                                    options,
                                },
                                None,
                                cx,
                            );
                        }
                    }
                }
                Err(error) => {
                    self.set_task_failed(id, error.to_string());
                    self.show_toast(format!("解压失败：{error}"), ToastKind::Error, cx);
                }
            },
            JobResult::Create {
                dest,
                password,
                result,
            } => {
                let standalone = self.feature_window == Some(FeatureWindow::NewArchive);
                match result {
                    Ok(outcome) => {
                        self.set_task_completed(id, create_detail(&outcome));
                        self.show_toast(
                            format!("已创建 {}", dest.display()),
                            ToastKind::Success,
                            cx,
                        );
                        if standalone {
                            self.feature_result = Some(FeatureOutcome::Created(dest));
                        } else {
                            self.open_path_with_password(dest, password, cx);
                        }
                    }
                    Err(EngineError::Cancelled) => {
                        self.set_task_cancelled(id);
                        if standalone {
                            self.feature_result = Some(FeatureOutcome::Cancelled);
                        }
                    }
                    Err(error) => {
                        self.set_task_failed(id, error.to_string());
                        if standalone {
                            self.feature_result = Some(FeatureOutcome::Failed(error.to_string()));
                        }
                        self.show_toast(format!("压缩失败：{error}"), ToastKind::Error, cx);
                    }
                }
            }
            JobResult::Verify { path, result } => match result {
                Ok(report) => {
                    self.set_task_completed(
                        id,
                        format!(
                            "{} · {} 项 · {}",
                            path.file_name()
                                .map(|name| name.to_string_lossy().to_string())
                                .unwrap_or_default(),
                            report.entries,
                            format_size(report.bytes)
                        ),
                    );
                    self.show_toast(
                        format!("校验通过 · {} 项", report.entries),
                        ToastKind::Success,
                        cx,
                    );
                }
                Err(error) if error.is_password_related() => {
                    self.set_task_failed(id, "需要密码");
                    if let Some(task) = self.tasks.iter().find(|task| task.id == id) {
                        if let Some(RetrySpec::Verify { path, .. }) = &task.retry {
                            self.show_password_modal(
                                path.clone(),
                                PendingAction::Verify { path: path.clone() },
                                None,
                                cx,
                            );
                        }
                    }
                }
                Err(EngineError::Cancelled) => {
                    self.set_task_cancelled(id);
                }
                Err(error) => {
                    self.set_task_failed(id, error.to_string());
                    self.show_toast(format!("校验失败：{error}"), ToastKind::Error, cx);
                }
            },
            JobResult::Hash {
                path,
                algorithm,
                result,
            } => match result {
                Ok(hash) => {
                    self.set_task_completed(
                        id,
                        format!("{} · {}", algorithm.label(), short_hash(&hash)),
                    );
                    self.show_toast(
                        format!("哈希完成：{}", path.display()),
                        ToastKind::Success,
                        cx,
                    );
                }
                Err(EngineError::Cancelled) => {
                    self.set_task_cancelled(id);
                }
                Err(error) => {
                    self.set_task_failed(id, error.to_string());
                    self.show_toast(format!("哈希失败：{error}"), ToastKind::Error, cx);
                }
            },
        }
    }

    fn set_task_completed(&mut self, id: u64, detail: String) {
        if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
            task.status = TaskStatus::Completed;
            task.progress = 100.0;
            task.phase = "完成".to_string();
            task.detail = detail;
        }
    }

    fn set_task_cancelled(&mut self, id: u64) {
        if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
            task.status = TaskStatus::Cancelled;
            task.phase = "已取消".to_string();
            task.detail = "已取消".to_string();
        }
    }

    fn set_task_failed(&mut self, id: u64, message: impl Into<String>) {
        if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
            let message = message.into();
            task.status = TaskStatus::Failed;
            task.phase = "失败".to_string();
            task.detail = message;
        }
    }

    fn install_archive(
        &mut self,
        path: PathBuf,
        info: ArchiveInfo,
        password: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let tree = build_tree(&info);
        let root = single_root(&info);
        self.current_dir = root.clone().unwrap_or_default();
        self.expanded.clear();
        self.expanded.insert(String::new());
        if let Some(root) = &root {
            self.expanded.insert(root.clone());
        }
        self.selected = None;
        self.multi_selected.clear();
        self.archive_menu = None;
        self.preview = PreviewState::default();
        self.archive = Some(ArchiveState {
            path: path.clone(),
            info,
            password,
            tree,
        });
        if self.settings.advanced.history {
            self.settings.add_recent(&path);
        }
        self.save_settings();
        self.rebuild_rows();
        self.refresh_preview(cx);
        cx.notify();
    }

    // Placeholder implementations are added in the following stages.
    fn open_path(&mut self, path: PathBuf, password: Option<String>, cx: &mut Context<Self>) {
        self.open_path_with_password(path, password, cx);
    }

    fn open_path_with_password(
        &mut self,
        path: PathBuf,
        password: Option<String>,
        _cx: &mut Context<Self>,
    ) {
        let id = self.next_task_id;
        self.next_task_id += 1;
        let mut task = TaskItem::new(
            id,
            format!(
                "打开 {}",
                path.file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default()
            ),
            Some(RetrySpec::Open {
                path: path.clone(),
                password: password.clone(),
            }),
        );
        task.detail = path.display().to_string();
        self.tasks.insert(0, task);
        let sender = self.task_sender.clone();
        std::thread::spawn(move || {
            let mut resolved = password;
            let mut result = open_archive(
                &path,
                &OpenOptions {
                    password: resolved.clone(),
                    codepage: Codepage::Auto,
                },
            );

            // 头加密的 7z / 损坏前缀等：先试空密码，再按密码本顺序自动试密。
            if matches!(&result, Err(error) if error.is_password_related()) && resolved.is_some() {
                if let Ok(info) = open_archive(
                    &path,
                    &OpenOptions {
                        password: None,
                        codepage: Codepage::Auto,
                    },
                ) {
                    resolved = None;
                    result = Ok(info);
                }
            }
            if matches!(&result, Err(error) if error.is_password_related())
                && nimzip_engine::supports_quick_password_test(&path)
            {
                let _ = sender.send(TaskEvent::Progress {
                    id,
                    phase: "尝试密码",
                    percent: 0.0,
                    detail: "正在按密码本顺序自动尝试…".to_string(),
                });
                let book = PasswordBook::load_default();
                if let Ok(Some(found)) = book.try_all(&path) {
                    resolved = Some(found.clone());
                    result = open_archive(
                        &path,
                        &OpenOptions {
                            password: Some(found),
                            codepage: Codepage::Auto,
                        },
                    );
                }
            }

            let _ = sender.send(TaskEvent::Finished {
                id,
                result: JobResult::Open {
                    path,
                    password: resolved,
                    result,
                },
            });
        });
    }

    fn show_password_modal(
        &mut self,
        archive: PathBuf,
        pending: PendingAction,
        error: Option<String>,
        _cx: &mut Context<Self>,
    ) {
        self.modal = Some(Modal::Password {
            archive,
            pending,
            error,
        });
    }

    fn show_toast(&mut self, message: impl Into<String>, kind: ToastKind, cx: &mut Context<Self>) {
        self.toast_serial += 1;
        let serial = self.toast_serial;
        self.toast = Some(Toast {
            message: message.into(),
            kind,
            serial,
        });
        cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(4200))
                .await;
            let _ = weak.update(cx, |view, cx| {
                if view
                    .toast
                    .as_ref()
                    .is_some_and(|toast| toast.serial == serial)
                {
                    view.toast = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn rebuild_rows(&mut self) {
        let Some(archive) = &self.archive else {
            self.rows.clear();
            self.row_sizes = Rc::new(Vec::new());
            return;
        };
        let mut rows = Vec::new();
        let searching = !self.search_query.trim().is_empty();
        if searching {
            let query = self.search_query.to_lowercase();
            let mut entries: Vec<usize> = archive
                .info
                .entries
                .iter()
                .filter(|entry| entry.path.to_lowercase().contains(&query))
                .map(|entry| entry.index)
                .collect();
            sort_entries(
                &archive.info,
                &mut entries,
                self.sort_column,
                self.sort_direction,
            );
            rows.extend(entries.into_iter().map(Row::Entry));
        } else {
            let mut entries: Vec<usize> = archive
                .info
                .entries
                .iter()
                .filter(|entry| parent_of(&entry.path) == self.current_dir)
                .map(|entry| entry.index)
                .collect();
            sort_entries(
                &archive.info,
                &mut entries,
                self.sort_column,
                self.sort_direction,
            );
            rows.extend(entries.into_iter().map(Row::Entry));
        }
        self.rows = rows;
        self.row_sizes =
            Rc::new(std::iter::repeat_n(size(px(1.0), px(29.0)), self.rows.len()).collect());
    }

    fn set_sort(&mut self, column: SortColumn) {
        if self.sort_column == column {
            self.sort_direction = match self.sort_direction {
                SortDirection::Asc => SortDirection::Desc,
                SortDirection::Desc => SortDirection::Asc,
            };
        } else {
            self.sort_column = column;
            self.sort_direction = SortDirection::Asc;
        }
        self.rebuild_rows();
    }

    pub(crate) fn table_width(&self) -> f32 {
        let hidden = &self.settings.view.hidden_columns;
        let mut total = 240.0f32;
        for key in [
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
            if hidden.iter().any(|hidden| hidden == key.key()) {
                continue;
            }
            total += self
                .settings
                .view
                .column_widths
                .get(key.key())
                .copied()
                .unwrap_or_else(|| default_column_width(key.key()));
        }
        total
    }

    pub(crate) fn move_column(
        &mut self,
        source: SortColumn,
        target: SortColumn,
        cx: &mut Context<Self>,
    ) {
        if source == target {
            return;
        }
        let source_key = source.key();
        let target_key = target.key();
        let order = &mut self.settings.view.column_order;
        let Some(index) = order.iter().position(|key| key == source_key) else {
            return;
        };
        let moved = order.remove(index);
        let target_index = order
            .iter()
            .position(|key| key == target_key)
            .unwrap_or(order.len());
        order.insert(target_index, moved);
        self.save_settings();
        cx.notify();
    }

    fn refresh_preview(&mut self, _cx: &mut Context<Self>) {
        self.preview.generation += 1;
        self.preview.image_path = None;
        self.preview.dimensions = None;
        self.preview.error = None;
        let Some(archive) = &self.archive else {
            self.preview.entry = None;
            return;
        };
        let Some(index) = self.selected else {
            self.preview.entry = None;
            return;
        };
        self.preview.entry = Some(index);
        let Some(entry) = archive.info.find(index) else {
            return;
        };
        if entry.is_dir {
            return;
        }
        if let Some(path) = self.preview.cache.get(&index).filter(|path| path.exists()) {
            self.preview.image_path = Some(path.clone());
            self.preview.dimensions = image::image_dimensions(path).ok();
            return;
        }
        if !is_image_name(&entry.path) {
            return;
        }
        if archive.info.encrypted && archive.password.is_none() {
            self.preview.error = Some("加密条目：解压时输入密码后可预览".to_string());
            return;
        }
        if !self.preview_ready {
            self.preview.error = Some("预览功能初始化中…".to_string());
            return;
        }
        let generation = self.preview.generation;
        let sender = self.preview_sender.clone();
        let archive_path = archive.path.clone();
        let password = archive.password.clone();
        let entry_path = entry.path.clone();
        let cache_path = preview_cache_path(&archive_path, index, &entry_path);
        std::thread::spawn(move || {
            let result = extract_preview(
                &archive_path,
                password.as_deref(),
                index,
                &entry_path,
                &cache_path,
            );
            let _ = sender.send(PreviewEvent {
                generation,
                entry: index,
                result,
            });
        });
    }

    /// 列表/图标共用的点击选择；Ctrl 保留并切换多选。
    fn select_entry_with_modifiers(
        &mut self,
        index: usize,
        modifiers: &Modifiers,
        cx: &mut Context<Self>,
    ) {
        self.archive_menu = None;
        if modifiers.control {
            if self.multi_selected.is_empty() {
                if let Some(current) = self.selected {
                    self.multi_selected.insert(current);
                }
            }
            if !self.multi_selected.remove(&index) {
                self.multi_selected.insert(index);
                self.selected = Some(index);
            } else if self.selected == Some(index) {
                self.selected = self.multi_selected.iter().next().copied();
            }
        } else {
            self.multi_selected.clear();
            self.selected = Some(index);
        }
        self.refresh_preview(cx);
        cx.notify();
    }

    /// 当前应参与批量操作的全部条目索引。
    fn selected_indices(&self) -> Vec<usize> {
        if self.multi_selected.is_empty() {
            self.selected.into_iter().collect()
        } else {
            let mut indices: Vec<usize> = self.multi_selected.iter().copied().collect();
            indices.sort_unstable();
            indices
        }
    }

    fn visible_entry_indices(&self) -> Vec<usize> {
        self.rows.iter().map(|Row::Entry(index)| *index).collect()
    }

    fn select_all_visible(&mut self, cx: &mut Context<Self>) {
        self.multi_selected = self.visible_entry_indices().into_iter().collect();
        self.selected = self.multi_selected.iter().next().copied();
        self.refresh_preview(cx);
        cx.notify();
    }

    fn open_archive_context_menu(
        &mut self,
        index: Option<usize>,
        x: f32,
        y: f32,
        cx: &mut Context<Self>,
    ) {
        match index {
            Some(index) => {
                if !self.multi_selected.contains(&index) {
                    self.multi_selected.clear();
                    self.selected = Some(index);
                }
            }
            None => {
                self.multi_selected.clear();
                self.selected = None;
            }
        }
        self.archive_menu = Some(ArchiveContextMenu {
            x,
            y,
            target: index
                .map(ContextTarget::Row)
                .unwrap_or(ContextTarget::Blank),
        });
        self.refresh_preview(cx);
        cx.notify();
    }

    fn toggle_tree_panel(&mut self, cx: &mut Context<Self>) {
        self.settings.view.show_tree = !self.settings.view.show_tree;
        self.save_settings();
        cx.notify();
    }

    fn toggle_preview_panel(&mut self, cx: &mut Context<Self>) {
        self.settings.view.preview_enabled = !self.settings.view.preview_enabled;
        self.save_settings();
        if self.settings.view.preview_enabled {
            self.refresh_preview(cx);
        }
        cx.notify();
    }

    fn toggle_comment_panel(&mut self, cx: &mut Context<Self>) {
        self.settings.view.show_comment = !self.settings.view.show_comment;
        self.save_settings();
        cx.notify();
    }

    fn set_icon_size(&mut self, size: f32, window: &mut Window, cx: &mut Context<Self>) {
        let size = size.clamp(64.0, 200.0).round();
        self.settings.view.thumbnail_size = size;
        self.icon_size_slider
            .update(cx, |state, cx| state.set_value(size, window, cx));
        self.save_settings();
        cx.notify();
    }

    fn set_file_view(&mut self, view: FileViewMode, cx: &mut Context<Self>) {
        if self.file_view == view {
            return;
        }
        self.file_view = view;
        self.settings.view.file_list_view = view.as_setting().to_string();
        self.save_settings();
        self.archive_menu = None;
        cx.notify();
    }

    fn navigate_to(&mut self, dir: String, cx: &mut Context<Self>) {
        self.current_dir = dir.clone();
        if !dir.is_empty() {
            self.expanded.insert(dir);
        }
        self.selected = None;
        self.multi_selected.clear();
        self.archive_menu = None;
        self.refresh_preview(cx);
        self.rebuild_rows();
        cx.notify();
    }

    fn go_up(&mut self, cx: &mut Context<Self>) {
        if !self.current_dir.is_empty() {
            let parent = parent_of(&self.current_dir);
            self.navigate_to(parent, cx);
        }
    }

    fn close_modal(&mut self, cx: &mut Context<Self>) {
        self.modal = None;
        self.new_draft = None;
        cx.notify();
    }

    fn close_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(feature) = self.feature_window {
            self.save_feature_window_state(window, feature);
            window.remove_window();
            return;
        }
        self.page = Page::Main;
        self.modal = None;
        self.new_draft = None;
        cx.notify();
    }

    fn open_file_dialog(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let picked = crate::components::file_dialog(|| {
            rfd::FileDialog::new()
                .add_filter(
                    "压缩包",
                    &[
                        "zip", "7z", "rar", "tar", "gz", "bz2", "xz", "zst", "tgz", "tbz", "txz",
                        "tzst", "zipx",
                    ],
                )
                .pick_file()
        });
        if let Some(path) = picked {
            self.open_path(path, None, cx);
        }
    }

    pub(crate) fn close_current_archive(&mut self, cx: &mut Context<Self>) {
        if self.archive.take().is_none() {
            return;
        }
        self.current_dir.clear();
        self.expanded.clear();
        self.selected = None;
        self.multi_selected.clear();
        self.archive_menu = None;
        self.preview = PreviewState::default();
        self.search_query.clear();
        self.rebuild_rows();
        self.page = Page::Main;
        self.show_toast("已关闭压缩包", ToastKind::Info, cx);
    }

    pub(crate) fn open_new_window(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let persisted = Settings::load();
        let width = persisted.ui.window_width.clamp(520.0, 3840.0);
        let height = persisted.ui.window_height.clamp(420.0, 2160.0);
        let custom_titlebar = self.settings.view.use_custom_titlebar;
        let window_bounds = WindowBounds::Windowed(Bounds {
            origin: point(px(80.0), px(80.0)),
            size: size(px(width), px(height)),
        });
        let options = WindowOptions {
            window_bounds: Some(window_bounds),
            window_background: WindowBackgroundAppearance::Opaque,
            titlebar: Some(TitlebarOptions {
                title: Some("NimZip".into()),
                appears_transparent: custom_titlebar,
                traffic_light_position: None,
            }),
            ..Default::default()
        };
        match cx.open_window(options, |window, cx| {
            cx.new(|cx| MainView::new(window, cx, None, None, None))
        }) {
            Ok(_) => {}
            Err(error) => self.show_toast(format!("新建窗口失败：{error}"), ToastKind::Error, cx),
        }
    }

    /// 以独立系统窗口打开密码恢复 / 文件哈希 / 新建压缩包。
    pub(crate) fn open_feature_window(
        &mut self,
        feature: FeatureWindow,
        sources: Vec<PathBuf>,
        output: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        // 从磁盘读取最新尺寸：主窗口内存中的 Settings 可能早于其他窗口刚保存的尺寸。
        let persisted = Settings::load();
        self.settings.ui.window_sizes = persisted.ui.window_sizes.clone();
        let (width, height) = persisted.window_size(feature.key(), feature.size());
        let custom_titlebar = self.settings.view.use_custom_titlebar;
        let bounds = WindowBounds::centered(size(px(width), px(height)), cx);
        let options = WindowOptions {
            window_bounds: Some(bounds),
            window_background: WindowBackgroundAppearance::Opaque,
            titlebar: Some(TitlebarOptions {
                title: Some(feature.title().into()),
                appears_transparent: custom_titlebar,
                traffic_light_position: None,
            }),
            window_min_size: Some(size(px(620.0), px(460.0))),
            ..Default::default()
        };
        match cx.open_window(options, move |window, cx| {
            cx.new(|cx| MainView::new_feature(window, cx, feature, sources.clone(), output.clone()))
        }) {
            Ok(handle) => {
                // 独立窗口是模态工作流的入口，聚焦到新窗口。
                let _ = handle.update(cx, |_, window, _| window.activate_window());
            }
            Err(error) => {
                self.show_toast(format!("无法打开窗口：{error}"), ToastKind::Error, cx);
            }
        }
    }

    /// 关闭独立功能窗口；主窗口内调用时退回主页面。
    pub(crate) fn close_feature_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(feature) = self.feature_window {
            self.save_feature_window_state(window, feature);
            window.remove_window();
        } else {
            self.page = Page::Main;
            cx.notify();
        }
    }

    /// 保存设置前合并磁盘上的窗口尺寸，避免主窗口旧副本覆盖其他窗口刚保存的尺寸。
    pub(crate) fn save_settings(&mut self) {
        let latest = Settings::load();
        self.settings.ui.window_sizes = latest.ui.window_sizes;
        // 位置只由关闭窗口时写入；合并磁盘值，避免其他设置保存把最新位置覆盖掉。
        self.settings.ui.window_x = latest.ui.window_x;
        self.settings.ui.window_y = latest.ui.window_y;
        let _ = self.settings.save();
    }

    fn save_window_state(&mut self, window: &Window) {
        // 只记住普通窗口的位置与尺寸；最大化/全屏不覆盖上次的调整结果。
        if let WindowBounds::Windowed(bounds) = window.window_bounds() {
            let x = bounds.origin.x.as_f32();
            let y = bounds.origin.y.as_f32();
            let width = bounds.size.width.as_f32();
            let height = bounds.size.height.as_f32();
            self.settings.ui.window_x = Some(x);
            self.settings.ui.window_y = Some(y);
            self.settings.ui.window_width = width;
            self.settings.ui.window_height = height;
            // 只合并位置与尺寸，避免用旧的内存副本覆盖其他窗口刚保存的尺寸。
            let _ = Settings::persist_main_window_bounds(x, y, width, height);
        }
    }

    fn save_feature_window_state(&mut self, window: &Window, feature: FeatureWindow) {
        if let WindowBounds::Windowed(bounds) = window.window_bounds() {
            let width = bounds.size.width.as_f32();
            let height = bounds.size.height.as_f32();
            self.settings
                .remember_window_size(feature.key(), width, height);
            let _ = Settings::persist_window_size(feature.key(), width, height);
        }
    }

    fn set_language(&mut self, language: Language, window: &mut Window, cx: &mut Context<Self>) {
        self.language = language;
        self.settings.general.language = language.as_setting().to_string();
        let placeholder = if language == Language::Zh {
            "在压缩包内查找…  Ctrl+F"
        } else {
            "Search in archive…  Ctrl+F"
        };
        self.search.update(cx, |state, cx| {
            state.set_placeholder(placeholder, window, cx);
        });
        self.save_settings();
        cx.notify();
    }

    fn set_skin(&mut self, skin: Skin, window: &mut Window, cx: &mut Context<Self>) {
        self.skin = skin;
        self.settings.view.theme_skin = skin.as_setting().to_string();
        let dark = self.theme_mode.is_dark(window.appearance());
        theme::apply(cx, dark, skin);
        self.palette = Palette::for_skin(dark, skin);
        self.save_settings();
        cx.notify();
    }

    fn toggle_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dark = self.palette.dark;
        self.theme_mode = if dark {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        self.settings.view.theme = self.theme_mode.as_setting().to_string();
        theme::apply(cx, !dark, self.skin);
        self.palette = Palette::for_skin(!dark, self.skin);
        self.save_settings();
        window.set_window_title(&self.window_title());
        cx.notify();
    }

    fn set_theme_mode(&mut self, mode: ThemeMode, window: &mut Window, cx: &mut Context<Self>) {
        self.theme_mode = mode;
        self.settings.view.theme = mode.as_setting().to_string();
        let dark = mode.is_dark(window.appearance());
        theme::apply(cx, dark, self.skin);
        self.palette = Palette::for_skin(dark, self.skin);
        self.save_settings();
        cx.notify();
    }

    fn window_title(&self) -> String {
        if let Some(feature) = self.feature_window {
            return feature.title().to_string();
        }
        match &self.archive {
            Some(archive) => format!(
                "NimZip — {}",
                archive
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default()
            ),
            None => "NimZip".to_string(),
        }
    }

    fn running_tasks(&self) -> usize {
        self.tasks.iter().filter(|task| task.is_running()).count()
    }

    fn active_progress(&self) -> Option<(String, f32)> {
        self.tasks
            .iter()
            .find(|task| task.is_running())
            .map(|task| (task.phase.clone(), task.progress))
    }
}

fn parent_of(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_default()
}

fn single_root(info: &ArchiveInfo) -> Option<String> {
    let mut root: Option<String> = None;
    let mut has_nested = false;
    for entry in &info.entries {
        let first = entry.path.split('/').next()?.to_string();
        if entry.path.contains('/') {
            has_nested = true;
        } else if !entry.is_dir {
            return None;
        }
        match &root {
            Some(existing) if existing != &first => return None,
            None => root = Some(first),
            _ => {}
        }
    }
    has_nested.then_some(root?).filter(|root| !root.is_empty())
}

fn build_tree(info: &ArchiveInfo) -> DirNode {
    let mut root = DirNode::default();
    for entry in &info.entries {
        let components: Vec<String> = entry
            .path
            .split('/')
            .filter(|part| !part.is_empty())
            .map(ToString::to_string)
            .collect();
        if components.is_empty() {
            continue;
        }
        let parent_components = if entry.is_dir {
            components.as_slice()
        } else {
            &components[..components.len() - 1]
        };
        let mut node = &mut root;
        for component in parent_components {
            node = node
                .children
                .entry(component.clone())
                .or_insert_with(|| DirNode {
                    path: String::new(),
                    children: BTreeMap::new(),
                    direct_files: 0,
                    total_files: 0,
                });
        }
        if !entry.is_dir {
            node.direct_files += 1;
        }
    }
    assign_paths(&mut root, String::new());
    root.count_files();
    root
}

fn assign_paths(node: &mut DirNode, prefix: String) {
    let keys: Vec<String> = node.children.keys().cloned().collect();
    for key in keys {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}/{key}")
        };
        if let Some(child) = node.children.get_mut(&key) {
            child.path = path.clone();
            assign_paths(child, path);
        }
    }
}

fn sort_entries(
    info: &ArchiveInfo,
    entries: &mut [usize],
    column: SortColumn,
    direction: SortDirection,
) {
    entries.sort_by(|left, right| {
        let a = info.find(*left);
        let b = info.find(*right);
        let ordering = match (a, b) {
            (Some(a), Some(b)) => {
                if column == SortColumn::Name {
                    b.is_dir
                        .cmp(&a.is_dir)
                        .then_with(|| a.path.to_lowercase().cmp(&b.path.to_lowercase()))
                } else {
                    b.is_dir.cmp(&a.is_dir).then_with(|| match column {
                        SortColumn::Compressed => a.compressed_size.cmp(&b.compressed_size),
                        SortColumn::Original => a.size.cmp(&b.size),
                        SortColumn::Type => entry_kind_label(a).cmp(&entry_kind_label(b)),
                        SortColumn::Modified => a.modified.cmp(&b.modified),
                        SortColumn::Method => a.method.cmp(&b.method),
                        SortColumn::Encryption => a.encrypted.cmp(&b.encrypted),
                        SortColumn::Crc => a.crc32.cmp(&b.crc32),
                        SortColumn::Attributes => a.is_symlink.cmp(&b.is_symlink),
                        SortColumn::Comment => a.comment.cmp(&b.comment),
                        SortColumn::Extra => a.encrypted.cmp(&b.encrypted),
                        SortColumn::Name => std::cmp::Ordering::Equal,
                    })
                }
            }
            _ => std::cmp::Ordering::Equal,
        };
        if direction == SortDirection::Asc {
            ordering
        } else {
            ordering.reverse()
        }
    });
}

pub(crate) fn entry_kind_label(entry: &nimzip_engine::ArchiveEntry) -> String {
    if entry.is_dir {
        return "文件夹".to_string();
    }
    std::path::Path::new(&entry.path)
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_uppercase())
        .filter(|ext| !ext.is_empty())
        .unwrap_or_else(|| "文件".to_string())
}

pub(crate) fn default_column_width(key: &str) -> f32 {
    match key {
        "original" | "compressed" => 72.0,
        "kind" => 62.0,
        "modified" => 90.0,
        "method" => 66.0,
        "encryption" => 54.0,
        "crc" => 74.0,
        "attributes" => 58.0,
        "comment" | "extra" => 70.0,
        _ => 80.0,
    }
}

fn is_image_name(name: &str) -> bool {
    matches!(
        Path::new(name)
            .extension()
            .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tiff" | "tif" | "heic")
    )
}

fn preview_cache_path(archive: &Path, index: usize, entry: &str) -> PathBuf {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    archive.hash(&mut hasher);
    let dir = std::env::temp_dir()
        .join("NimZip")
        .join("preview")
        .join(format!("{:016x}", hasher.finish()));
    dir.join(entry.replace('/', std::path::MAIN_SEPARATOR_STR))
        .with_file_name(format!(
            "{}-{}",
            index,
            Path::new(entry)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "preview".to_string())
        ))
}

fn extract_preview(
    archive: &Path,
    password: Option<&str>,
    index: usize,
    entry: &str,
    target: &Path,
) -> Result<(PathBuf, Option<(u32, u32)>), String> {
    let dir = target
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let options = ExtractOptions {
        mode: ExtractMode::Custom(dir.clone()),
        password: password.map(ToString::to_string),
        selected: Some(vec![index]),
        nested_depth: 0,
        propagate_motw: false,
        overwrite: true,
        ..Default::default()
    };
    nimzip_engine::extract_archive(archive, &options, &nimzip_engine::NoopSink)
        .map_err(|error| error.to_string())?;
    let extracted = dir.join(entry.replace('/', std::path::MAIN_SEPARATOR_STR));
    if extracted.exists() {
        std::fs::rename(&extracted, target).map_err(|error| error.to_string())?;
    }
    let dimensions = image::image_dimensions(target).ok();
    Ok((target.to_path_buf(), dimensions))
}

fn short_hash(hash: &str) -> String {
    if hash.len() > 16 {
        format!("{}…", &hash[..16])
    } else {
        hash.to_string()
    }
}

fn create_detail(outcome: &CreateOutcome) -> String {
    format!(
        "{} 个文件 · {} → {} ({})",
        outcome.entries,
        format_size(outcome.source_bytes),
        format_size(outcome.compressed_bytes),
        outcome.ratio
    )
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.window_title();
        if title != self.last_title {
            window.set_window_title(&title);
            self.last_title = title;
        }
        if self.feature_window.is_some() {
            return self.render_feature_window(window, cx);
        }
        let palette = self.palette;
        let content = match self.page {
            Page::Main => self.render_main(window, cx),
            Page::Tasks => self.render_tasks(window, cx),
            Page::Settings => self.render_settings(window, cx),
            Page::NewArchive => self.render_new_archive_page(window, cx),
            Page::PasswordManager => self.render_password_manager_page(window, cx),
            Page::PasswordRecovery => self.render_password_recovery_page(cx),
            Page::Hash => self.render_hash_page(window, cx),
        };
        // 任务中心、设置以及各功能页使用整页布局，不再显示归档地址栏与搜索框；
        // 首页（未打开归档）也不再显示地址栏，直接进入卡片式开始页。
        let address_bar = (self.page == Page::Main && self.archive.is_some())
            .then(|| self.render_address_bar(window, cx));
        let archive_context = (self.page == Page::Main && self.archive_menu.is_some())
            .then(|| self.render_archive_context_menu(window, cx));
        let dropdown_overlay = self
            .dropdown
            .map(|kind| self.render_dropdown_overlay(kind, window, cx));
        let modal = self.modal.clone();
        let toast = self.toast.as_ref().map(|toast| ToastSnapshot {
            message: toast.message.clone(),
            kind: toast.kind,
        });
        div()
            .id("nimzip-root")
            .track_focus(&self.focus_handle)
            .on_key_down(
                cx.listener(|this, event, window, cx| this.handle_key_down(event, window, cx)),
            )
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.handle_external_drop(paths.paths(), window, cx);
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.h_scroll_drag.is_some() {
                    this.update_h_scroll_drag(event.position.x.as_f32(), cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.h_scroll_drag = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.h_scroll_drag = None),
            )
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                let over_bar = this
                    .table_scroll_bounds
                    .is_some_and(|bounds| bounds.contains(&event.position));
                this.handle_root_scroll(event, cx);
                if over_bar {
                    cx.stop_propagation();
                }
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(palette.bg)
            .text_color(palette.text)
            .font_family(crate::theme::ui_font_family())
            .relative()
            .child(self.render_top_bar(window, cx))
            .when_some(address_bar, |this, address_bar| this.child(address_bar))
            .child(div().flex().flex_1().min_h_0().child(content))
            .child(self.render_status_bar(cx))
            .when_some(archive_context, |this, menu| this.child(menu))
            .when(self.menu_open, |this| {
                this.child(self.render_app_menu(window, cx))
            })
            .when_some(dropdown_overlay, |this, overlay| this.child(overlay))
            .when_some(toast, |this, toast| {
                this.child(render_toast(
                    &toast,
                    &palette,
                    cx.listener(|this, _, _, cx| {
                        this.toast = None;
                        cx.notify();
                    }),
                ))
            })
            .when_some(modal, |this, modal| {
                this.child(self.render_modal(modal, window, cx))
            })
            .into_any_element()
    }
}

struct ToastSnapshot {
    message: String,
    kind: ToastKind,
}

fn start_action_card(
    palette: Palette,
    id: &'static str,
    icon: AnyElement,
    icon_bg: Hsla,
    title: &'static str,
    subtitle: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(title)
        .flex_1()
        .min_w(px(160.0))
        .max_w(px(280.0))
        .h(px(182.0))
        .rounded(px(12.0))
        .border_1()
        .border_color(palette.line)
        .bg(palette.card)
        .shadow_sm()
        .cursor_pointer()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .hover(move |style| style.border_color(palette.accent))
        .on_click(on_click)
        .child(
            div()
                .w(px(52.0))
                .h(px(52.0))
                .rounded(px(13.0))
                .bg(icon_bg)
                .flex()
                .items_center()
                .justify_center()
                .child(icon),
        )
        .child(
            div()
                .mt(px(13.0))
                .text_size(crate::theme::font_px(16.0))
                .font_weight(FontWeight::BOLD)
                .text_color(palette.text)
                .child(title),
        )
        .child(
            div()
                .mt(px(5.0))
                .text_size(crate::theme::font_px(11.5))
                .text_color(palette.sub)
                .text_center()
                .child(subtitle),
        )
        .into_any_element()
}

impl MainView {
    fn render_start_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let open_card = start_action_card(
            palette,
            "start-open",
            icon(IconName::FolderPlus, palette.accent, 22.0).into_any_element(),
            palette.accent_softer,
            tr(self.language, "打开压缩包"),
            tr(self.language, "选择文件，或直接拖入窗口"),
            cx.listener(|this, _, window, cx| {
                this.open_file_dialog(window, cx);
            }),
        );
        let new_card = start_action_card(
            palette,
            "start-new",
            icon(IconName::Plus, palette.sub, 22.0).into_any_element(),
            palette.hover,
            tr(self.language, "新建压缩包"),
            tr(self.language, "添加文件并选择压缩格式"),
            cx.listener(|this, _, window, cx| {
                this.open_new_archive_modal(window, cx);
            }),
        );
        div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(620.0))
                    .px(px(20.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(24.0))
                    .child(
                        div()
                            .flex()
                            .w_full()
                            .gap(px(22.0))
                            .child(open_card)
                            .child(new_card),
                    ),
            )
            .into_any_element()
    }

    fn render_feature_window(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(feature) = self.feature_window else {
            return div().into_any_element();
        };
        let palette = self.palette;
        let titlebar = self
            .custom_titlebar
            .then(|| self.render_feature_titlebar(feature, window, cx));
        let content = match feature {
            FeatureWindow::NewArchive => {
                if self.new_draft.is_some() {
                    self.render_new_archive_page(window, cx)
                } else {
                    self.render_new_archive_progress(window, cx)
                }
            }
            FeatureWindow::PasswordRecovery => self.render_password_recovery_page(cx),
            FeatureWindow::Hash => self.render_hash_page(window, cx),
        };
        // 独立窗口同样需要下拉层、模态框、Toast 与拖放处理，
        // 否则格式/算法下拉等控件会表现为“点了没反应”。
        let dropdown_overlay = self
            .dropdown
            .map(|kind| self.render_dropdown_overlay(kind, window, cx));
        let modal = self.modal.clone();
        let hash_menu = (feature == FeatureWindow::Hash)
            .then(|| self.hash_context_menu.clone())
            .flatten()
            .map(|menu| self.render_hash_context_menu(menu, window, cx));
        let toast = self.toast.as_ref().map(|toast| ToastSnapshot {
            message: toast.message.clone(),
            kind: toast.kind,
        });
        div()
            .id("nimzip-feature-root")
            .track_focus(&self.focus_handle)
            .on_key_down(
                cx.listener(|this, event, window, cx| this.handle_key_down(event, window, cx)),
            )
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.handle_external_drop(paths.paths(), window, cx);
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(palette.bg)
            .text_color(palette.text)
            .font_family(crate::theme::ui_font_family())
            .relative()
            .when_some(titlebar, |this, titlebar| this.child(titlebar))
            .child(div().flex_1().min_h_0().child(content))
            .when_some(dropdown_overlay, |this, overlay| this.child(overlay))
            .when_some(toast, |this, toast| {
                this.child(render_toast(
                    &toast,
                    &palette,
                    cx.listener(|this, _, _, cx| {
                        this.toast = None;
                        cx.notify();
                    }),
                ))
            })
            .when_some(modal, |this, modal| {
                this.child(self.render_modal(modal, window, cx))
            })
            .when_some(hash_menu, |this, menu| this.child(menu))
            .into_any_element()
    }

    /// 独立“新建压缩包”窗口中的进度与结果视图。
    fn render_new_archive_progress(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let outcome = self.feature_result.clone();
        let body = if let Some(outcome) = outcome {
            let (icon_name, color, soft, title, detail, dest) = match &outcome {
                FeatureOutcome::Created(dest) => (
                    IconName::CircleCheck,
                    palette.ok,
                    palette.ok_soft,
                    "压缩包已创建",
                    "压缩完成，可以打开查看内容。",
                    Some(dest.clone()),
                ),
                FeatureOutcome::Failed(error) => (
                    IconName::CircleX,
                    palette.bad,
                    palette.bad_soft,
                    "压缩失败",
                    error.as_str(),
                    None,
                ),
                FeatureOutcome::Cancelled => (
                    IconName::CircleX,
                    palette.muted,
                    palette.panel,
                    "已取消",
                    "任务已停止，未完成的输出文件已清理。",
                    None,
                ),
            };
            let open_button = dest.as_ref().map(|dest| {
                let dest = dest.clone();
                Button::new("feature-open-created")
                    .label("打开压缩包")
                    .primary()
                    .h(px(crate::components::CONTROL_HEIGHT))
                    .px(px(16.0))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(feature) = this.feature_window {
                            this.save_feature_window_state(window, feature);
                        }
                        this.feature_window = None;
                        this.page = Page::Main;
                        this.open_path_with_password(dest.clone(), None, cx);
                        window.set_window_title("NimZip");
                        cx.notify();
                    }))
            });
            let retry = matches!(
                outcome,
                FeatureOutcome::Failed(_) | FeatureOutcome::Cancelled
            )
            .then(|| {
                Button::new("feature-retry")
                    .label("返回")
                    .outline()
                    .h(px(crate::components::CONTROL_HEIGHT))
                    .px(px(16.0))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.init_new_archive_draft(Vec::new(), None, window, cx);
                    }))
            });
            let close = Button::new("feature-close")
                .label("关闭")
                .outline()
                .h(px(crate::components::CONTROL_HEIGHT))
                .px(px(16.0))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.close_feature_window(window, cx);
                }));
            div()
                .flex()
                .flex_col()
                .gap(px(16.0))
                .p(px(30.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(14.0))
                        .child(
                            div()
                                .w(px(48.0))
                                .h(px(48.0))
                                .flex_none()
                                .rounded(px(14.0))
                                .bg(soft)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon(icon_name, color, 24.0)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.0))
                                .child(
                                    div()
                                        .text_size(crate::theme::font_px(17.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(palette.text)
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .text_size(crate::theme::font_px(13.0))
                                        .text_color(palette.sub)
                                        .child(detail.to_string()),
                                ),
                        ),
                )
                .when_some(dest, |this, dest| {
                    this.child(
                        div()
                            .px(px(14.0))
                            .py(px(10.0))
                            .rounded(px(9.0))
                            .border_1()
                            .border_color(palette.line)
                            .bg(palette.panel)
                            .font_family("Cascadia Mono")
                            .text_size(crate::theme::font_px(12.5))
                            .text_color(palette.text)
                            .child(dest.display().to_string()),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(px(10.0))
                        .when_some(retry, |this, retry| this.child(retry))
                        .when_some(open_button, |this, open| this.child(open))
                        .child(close),
                )
                .into_any_element()
        } else {
            let task = self.tasks.first();
            let percent = task.map(|task| task.progress).unwrap_or(0.0);
            let phase = task
                .map(|task| task.phase.clone())
                .unwrap_or_else(|| "准备中".to_string());
            let detail = task.map(|task| task.detail.clone()).unwrap_or_default();
            let stop = task.is_some().then(|| {
                Button::new("feature-stop")
                    .label("停止")
                    .outline()
                    .h(px(crate::components::CONTROL_HEIGHT))
                    .px(px(16.0))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(task) = this.tasks.first() {
                            task.cancel.store(true, Ordering::Relaxed);
                        }
                        cx.notify();
                    }))
            });
            div()
                .flex()
                .flex_col()
                .gap(px(16.0))
                .p(px(30.0))
                .child(
                    div()
                        .text_size(crate::theme::font_px(17.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.text)
                        .child("正在压缩"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_size(crate::theme::font_px(13.0))
                        .text_color(palette.sub)
                        .child(phase)
                        .child(format!("{percent:.0}%")),
                )
                .child(
                    div()
                        .h(px(8.0))
                        .w_full()
                        .rounded(px(999.0))
                        .bg(palette.line)
                        .overflow_hidden()
                        .child(
                            div()
                                .h_full()
                                .w(relative(percent / 100.0))
                                .rounded(px(999.0))
                                .bg(palette.accent),
                        ),
                )
                .child(
                    div()
                        .font_family("Cascadia Mono")
                        .text_size(crate::theme::font_px(12.5))
                        .text_color(palette.muted)
                        .child(detail),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .when_some(stop, |this, stop| this.child(stop)),
                )
                .into_any_element()
        };
        design::page_shell_bare(&palette, vec![body], None)
    }

    fn render_entry_list(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        if self.rows.is_empty() {
            return empty_panel(
                if self.search_query.trim().is_empty() {
                    "此文件夹为空"
                } else {
                    "没有匹配的条目"
                },
                &palette,
            )
            .into_any_element();
        }
        let view = cx.entity();
        v_virtual_list(
            view,
            "entry-list",
            self.row_sizes.clone(),
            |this, range, window, cx| {
                range
                    .map(|index| this.render_entry_row(index, window, cx))
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&self.list_scroll)
        .flex_1()
        .min_h_0()
        .into_any_element()
    }

    fn render_comment_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let Some(archive) = &self.archive else {
            return div().into_any_element();
        };
        if !self.settings.view.show_comment {
            return div().into_any_element();
        }
        let is_zip = archive.info.format.is_zip();
        let comment = archive
            .info
            .comment
            .as_ref()
            .filter(|comment| !comment.trim().is_empty())
            .cloned();
        if !is_zip && comment.is_none() {
            return div().into_any_element();
        }
        let shown = comment.unwrap_or_else(|| "暂无注释".to_string());
        let mut header = div().flex().items_center().justify_between().child(
            div()
                .font_family(crate::theme::ui_font_family())
                .text_size(crate::theme::font_px(11.5))
                .text_color(palette.sub)
                .child("压缩包注释"),
        );
        if is_zip {
            header = header.child(
                Button::new("comment-edit")
                    .label(if shown == "暂无注释" {
                        "添加"
                    } else {
                        "编辑"
                    })
                    .ghost()
                    .small()
                    .h(px(24.0))
                    .px(px(8.0))
                    .text_size(crate::theme::font_px(11.5))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_comment_modal(window, cx);
                    })),
            );
        }
        div()
            .flex_shrink_0()
            .h(px(82.0))
            .border_t_1()
            .border_color(palette.line)
            .bg(palette.bar)
            .p(px(9.0))
            .flex()
            .flex_col()
            .gap(px(4.0))
            .font_family(crate::theme::ui_font_family())
            .child(header)
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .font_family(crate::theme::ui_font_family())
                    .line_height(relative(1.5))
                    .text_size(crate::theme::font_px(12.5))
                    .text_color(palette.text)
                    .child(shown),
            )
            .into_any_element()
    }
}

fn render_toast(
    toast: &ToastSnapshot,
    palette: &Palette,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let (fg, bg, icon_name) = match toast.kind {
        ToastKind::Info => (palette.accent, palette.accent_soft, IconName::Info),
        ToastKind::Success => (palette.ok, palette.ok_soft, IconName::CircleCheck),
        ToastKind::Error => (palette.bad, palette.bad_soft, IconName::CircleX),
    };
    div()
        .absolute()
        .right(px(16.0))
        .bottom(px(44.0))
        .w(px(360.0))
        .rounded(px(11.0))
        .border_1()
        .border_color(palette.line)
        .bg(bg)
        .shadow_lg()
        .p(px(10.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(icon(icon_name, fg, 16.0))
        .child(
            div()
                .flex_1()
                .text_size(crate::theme::font_px(12.0))
                .text_color(palette.text)
                .child(toast.message.clone()),
        )
        .child(
            Button::new("toast-close")
                .icon(icon(IconName::X, palette.sub, 12.0))
                .ghost()
                .small()
                .on_click(on_close),
        )
        .into_any_element()
}

// The remaining stage-specific methods are implemented in app/impls.rs.
mod impls;
