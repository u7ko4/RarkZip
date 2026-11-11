//! NimZip 图形界面入口。
//!
//! release 使用 Windows GUI 子系统，双击启动不会创建控制台窗口。
//! CLI 参数会附加到父控制台执行；脚本请优先使用独立的 `bx.exe`。

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod clipboard;
mod components;
mod design;
mod i18n;
mod mica;
mod quick;
mod task;
mod theme;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use gpui_kit::*;
use nimzip_engine::Settings;

fn main() -> std::process::ExitCode {
    let mut args: Vec<String> = std::env::args().collect();
    let exe_stem = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        })
        .or_else(|| {
            args.first().map(|arg| {
                PathBuf::from(arg)
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default()
            })
        })
        .unwrap_or_default();

    if args.len() == 1 {
        if let Ok(Some((info, exe))) = nimzip_engine::sfx::detect_current_exe() {
            // 静默自解压可能由脚本调用并依赖 stdout；仅附加父控制台，
            // 不主动新建窗口。
            if std::env::var_os("NIMZIP_SFX_SILENT").is_some() {
                let _ = attach_parent_console();
            }
            return run_sfx(info, exe);
        }
    }

    let invoked_as_bx = exe_stem == "bx";
    let forced_cli = args.iter().skip(1).any(|arg| arg == "--cli" || arg == "-c");
    let mut forced_gui = args.iter().skip(1).any(|arg| arg == "--gui");
    if forced_cli {
        args.retain(|arg| arg != "--cli" && arg != "-c");
    }
    if forced_gui {
        args.retain(|arg| arg != "--gui");
    }

    // 资源管理器“以管理员身份解压/压缩”：去掉标记后通过 UAC 重新启动自身。
    let elevated = args.iter().skip(1).any(|arg| arg == "--elevated");
    if elevated {
        args.retain(|arg| arg != "--elevated");
        if relaunch_elevated(&args[1..]) {
            return std::process::ExitCode::SUCCESS;
        }
        eprintln!("NimZip: 未获得管理员权限，操作已取消");
        return std::process::ExitCode::SUCCESS;
    }

    // 资源管理器右键菜单入口：--new <文件/文件夹> 打开新建压缩包页；
    // --new-here <目录> 以该目录作为输出位置打开新建压缩包页。
    let new_sources = take_flag_values(&mut args, "--new");
    let new_output = take_flag_value(&mut args, "--new-here");
    if !new_sources.is_empty() || new_output.is_some() {
        forced_gui = true;
    }

    // 快速解压 / 快速压缩弹出窗口入口。
    let quick_extract = take_flag_value(&mut args, "--quick-extract");
    let quick_compress = take_flag_value(&mut args, "--quick-compress");
    // 资源管理器“解压到此处 / 智能解压 / 独立文件夹”：直接以 GUI 进度窗口执行，
    // 不再走命令行，避免弹出控制台窗口。
    let extract_here = take_flag_value(&mut args, "--extract-here");
    let extract_smart = take_flag_value(&mut args, "--extract-smart");
    let extract_folder = take_flag_value(&mut args, "--extract-folder");
    let delete_source = args.iter().any(|arg| arg == "--delete");
    if delete_source {
        args.retain(|arg| arg != "--delete");
    }
    let quick_format = if quick_compress.is_some() {
        take_flag_value(&mut args, "--format")
            .map(|path| path.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_else(|| "zip".to_string())
    } else {
        "zip".to_string()
    };
    let quick_delete_sources = args.iter().any(|arg| arg == "--delete-sources");
    if quick_delete_sources {
        args.retain(|arg| arg != "--delete-sources");
    }
    let preset_request = extract_here
        .map(|archive| (archive, quick::ExtractPreset::Here))
        .or_else(|| extract_smart.map(|archive| (archive, quick::ExtractPreset::Smart)))
        .or_else(|| extract_folder.map(|archive| (archive, quick::ExtractPreset::Folder)));
    let compress_sources = if let Some(source) = quick_compress.as_ref() {
        match compress_batch_sources(source, &quick_format, quick_delete_sources) {
            CompressBatch::Leader(sources) => Some(sources),
            CompressBatch::Follower => return std::process::ExitCode::SUCCESS,
        }
    } else {
        None
    };

    let quick_action = if let Some(archive) = quick_extract {
        forced_gui = true;
        Some(quick::QuickAction::ExtractTo { archive })
    } else if let Some((archive, preset)) = preset_request {
        forced_gui = true;
        // 多选时资源管理器会为每一项启动一个进程。同一批任务只做静默解压，
        // 避免一次弹出多个进度窗口；单选时显示设计的进度弹窗。
        if let Some(code) = run_batch_preset_if_needed(&archive, preset, delete_source) {
            return code;
        }
        Some(quick::QuickAction::ExtractPreset {
            archive,
            preset,
            delete: delete_source,
        })
    } else if let Some(sources) = compress_sources {
        forced_gui = true;
        let format = nimzip_engine::ArchiveFormat::from_name(&quick_format)
            .filter(|format| format.can_create())
            .unwrap_or(nimzip_engine::ArchiveFormat::Zip);
        Some(quick::QuickAction::Compress {
            sources,
            format,
            delete_sources: quick_delete_sources,
        })
    } else {
        None
    };

    let first = args.get(1).map(String::as_str);
    let known_command = matches!(
        first,
        Some(
            "l" | "list"
                | "x"
                | "extract"
                | "a"
                | "add"
                | "t"
                | "test"
                | "h"
                | "hash"
                | "repair"
                | "recover"
                | "d"
                | "delete"
                | "rn"
                | "rename"
                | "c"
                | "comment"
                | "shell"
        )
    );
    let help_or_version = matches!(first, Some("--help" | "-h" | "--version" | "-V"));
    let cli_mode = !forced_gui
        && (invoked_as_bx || forced_cli || help_or_version || (known_command && args.len() >= 3));

    // 同一份 nimzip.exe：以 bx 命名、传入 --cli，或使用 `nimzip l archive.zip`
    // 形式时走无头 CLI；否则启动 GPUI 主窗口。GUI 子系统不会创建控制台，
    // CLI 模式主动附加到父控制台（没有父控制台时新建一个）。
    if cli_mode {
        ensure_console();
        return nimzip_cli::run_with_args(args);
    }

    let initial_path = if new_sources.is_empty() && new_output.is_none() {
        args.get(1).map(PathBuf::from)
    } else {
        None
    };
    let settings = Settings::load();
    let feature_window =
        (!new_sources.is_empty() || new_output.is_some()).then_some(app::FeatureWindow::NewArchive);
    let (width, height) = if let Some(action) = &quick_action {
        let (width, remembered_height) = settings.window_size(action.key(), action.default_size());
        // 进度页按实际内容收紧窗口，避免沿用旧版本记住的 560 高而底部留白。
        let height = action
            .progress_height(settings.view.font_size)
            .unwrap_or(remembered_height);
        (width, height)
    } else if let Some(feature) = feature_window {
        settings.window_size(feature.key(), feature.size())
    } else {
        (
            settings.ui.window_width.clamp(520.0, 3840.0),
            settings.ui.window_height.clamp(420.0, 2160.0),
        )
    };
    // Mica 主题下，独立功能窗口/快速弹窗使用系统标题栏；主窗口保留用户设置。
    let custom_titlebar = settings.view.use_custom_titlebar
        && !(settings.view.mica && (quick_action.is_some() || feature_window.is_some()));
    let window_background = WindowBackgroundAppearance::Opaque;
    let window_title = if quick_action.is_some() {
        "NimZip"
    } else {
        feature_window
            .map(app::FeatureWindow::title)
            .unwrap_or("NimZip")
    };
    let window_size = size(px(width), px(height));
    // 恢复上次关闭时的窗口位置；首次启动仍使用左上角附近的默认位置。
    let main_origin = match (settings.ui.window_x, settings.ui.window_y) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => point(
            px(x.clamp(-20000.0, 20000.0)),
            px(y.clamp(-20000.0, 20000.0)),
        ),
        _ => point(px(60.0), px(60.0)),
    };

    crate::mica::refresh_wallpaper_tint();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let window_bounds = if quick_action.is_some() || feature_window.is_some() {
                WindowBounds::centered(window_size, cx)
            } else {
                WindowBounds::Windowed(Bounds {
                    origin: main_origin,
                    size: window_size,
                })
            };
            let options = WindowOptions {
                window_bounds: Some(window_bounds),
                window_background,
                titlebar: Some(TitlebarOptions {
                    title: Some(window_title.into()),
                    appears_transparent: custom_titlebar,
                    traffic_light_position: None,
                }),
                // 快速解压/压缩弹窗按内容收紧高度，允许比主窗口更矮。
                window_min_size: Some(if quick_action.is_some() {
                    size(px(480.0), px(350.0))
                } else {
                    size(px(520.0), px(420.0))
                }),
                ..Default::default()
            };
            let path = initial_path.clone();
            let new_sources = new_sources.clone();
            let new_output = new_output.clone();
            let feature = feature_window;
            if let Some(action) = quick_action.clone() {
                gpui_kit::open_window(options, cx, move |window, cx| {
                    cx.new(|cx| quick::QuickView::new(window, cx, action.clone()))
                })
                .expect("failed to open NimZip quick window");
            } else {
                gpui_kit::open_window(options, cx, move |window, cx| {
                    cx.new(|cx| match feature {
                        Some(feature) => app::MainView::new_feature(
                            window,
                            cx,
                            feature,
                            new_sources.clone(),
                            new_output.clone(),
                        ),
                        None => app::MainView::new(window, cx, path.clone(), None, None),
                    })
                })
                .expect("failed to open NimZip window");
            }
            cx.activate(true);
        });

    std::process::ExitCode::SUCCESS
}

/// 通过 UAC 以管理员身份重新启动当前可执行文件。
#[cfg(windows)]
fn relaunch_elevated(args: &[String]) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
    fn quote(arg: &str) -> String {
        if arg.is_empty() || arg.chars().any(|ch| ch.is_whitespace() || ch == '"') {
            format!("\"{}\"", arg.replace('"', "\\\""))
        } else {
            arg.to_string()
        }
    }

    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let parameters = args
        .iter()
        .map(|arg| quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let operation = wide("runas");
    let file = wide(&exe.to_string_lossy());
    let params = wide(&parameters);
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(operation.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR(params.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    result.0 as isize > 32
}

#[cfg(not(windows))]
fn relaunch_elevated(_args: &[String]) -> bool {
    false
}

/// FNV-1a：跨进程稳定，保证同一批 Explorer 启动的进程落到同一目录。
fn stable_hash(key: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

fn preset_marker_dir(archive: &Path, preset: quick::ExtractPreset) -> PathBuf {
    let parent = archive
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_string_lossy()
        .to_ascii_lowercase();
    let hash = stable_hash(&format!("{parent}|{preset:?}"));
    std::env::temp_dir()
        .join("NimZip")
        .join("extract-batch")
        .join(format!("{hash:016x}"))
}

fn compress_batch_dir(source: &Path, format: &str, delete_sources: bool) -> PathBuf {
    let parent = source
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_string_lossy()
        .to_ascii_lowercase();
    let hash = stable_hash(&format!("{parent}|{format}|{delete_sources}"));
    std::env::temp_dir()
        .join("NimZip")
        .join("compress-batch")
        .join(format!("{hash:016x}"))
}

enum CompressBatch {
    Leader(Vec<PathBuf>),
    Follower,
}

/// 资源管理器多选时，Player 动词会为每个文件各启动一个进程。这里把同一目录、
/// 同一格式的并发启动合并为一次压缩：首个进程等待约 450ms 收集所有源路径并
/// 显示一个进度窗口，其余进程写完路径后直接退出。
fn compress_batch_sources(source: &Path, format: &str, delete_sources: bool) -> CompressBatch {
    let dir = compress_batch_dir(source, format, delete_sources);
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let stale = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                .is_some_and(|age| age > Duration::from_secs(8));
            if stale {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let lock = dir.join("leader.lock");
    let mut leader = false;
    for _ in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
        {
            Ok(mut file) => {
                use std::io::Write as _;
                let _ = write!(file, "{}", std::process::id());
                leader = true;
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let age = std::fs::metadata(&lock)
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                    .unwrap_or_default();
                if age > Duration::from_secs(8) {
                    let _ = std::fs::remove_file(&lock);
                    continue;
                }
                break;
            }
            Err(_) => {
                leader = true;
                break;
            }
        }
    }
    let marker = dir.join(format!("{}.path", std::process::id()));
    let _ = std::fs::write(&marker, source.to_string_lossy().as_bytes());
    if !leader {
        return CompressBatch::Follower;
    }
    // 给 Explorer 同一批逐项启动的其他进程留出登记时间。
    std::thread::sleep(Duration::from_millis(450));
    let mut sources = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "path")
            {
                if let Ok(bytes) = std::fs::read(&path) {
                    let text = String::from_utf8_lossy(&bytes).to_string();
                    if !text.is_empty() {
                        sources.push(PathBuf::from(text));
                    }
                }
            }
        }
    }
    if sources.is_empty() {
        sources.push(source.to_path_buf());
    }
    sources.sort();
    sources.dedup();
    let _ = std::fs::remove_dir_all(&dir);
    CompressBatch::Leader(sources)
}

/// 资源管理器多选时会为每个压缩包各启动一个进程。同一父目录、同一解压方式
/// 在短时间内的多次启动视为一批：静默解压，避免弹出 N 个进度窗口；只有单个
/// 压缩包时才返回 `None`，继续打开设计的进度弹窗。
fn run_batch_preset_if_needed(
    archive: &Path,
    preset: quick::ExtractPreset,
    delete: bool,
) -> Option<std::process::ExitCode> {
    let dir = preset_marker_dir(archive, preset);
    let _ = std::fs::create_dir_all(&dir);
    let lock = dir.join("leader.lock");
    let mut leader = false;
    for _ in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
        {
            Ok(mut file) => {
                use std::io::Write as _;
                let _ = write!(file, "{}", std::process::id());
                leader = true;
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                // 崩溃残留的锁超过 8 秒视为过期；否则说明这是同一批的后续进程。
                let age = std::fs::metadata(&lock)
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                    .unwrap_or_default();
                if age > Duration::from_secs(8) {
                    let _ = std::fs::remove_file(&lock);
                    continue;
                }
                break;
            }
            Err(_) => {
                // 无法创建会话锁时优先显示进度窗口。
                leader = true;
                break;
            }
        }
    }
    if leader {
        // 锁只用于合并 Explorer 同一批逐项启动的进程，短暂保留后自动释放。
        let lock_path = lock.clone();
        let dir_path = dir.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(3));
            let _ = std::fs::remove_file(&lock_path);
            let _ = std::fs::remove_dir(&dir_path);
        });
        return None;
    }
    let result = extract_preset_silently(archive, preset, delete);
    let _ = std::fs::remove_dir(&dir);
    match result {
        Ok(()) => Some(std::process::ExitCode::SUCCESS),
        Err(error) if error.is_password_related() => None,
        Err(error) => {
            let _ = rfd::MessageDialog::new()
                .set_title("NimZip 解压失败")
                .set_description(format!("{}\n\n{error}", archive.display()))
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
            Some(std::process::ExitCode::from(1))
        }
    }
}

fn extract_preset_silently(
    archive: &Path,
    preset: quick::ExtractPreset,
    delete: bool,
) -> std::result::Result<(), nimzip_engine::EngineError> {
    let settings = Settings::load();
    // 混合选择时 Explorer 也会对普通文件调用解压命令；静默忽略，避免弹错误框。
    let is_archive = archive
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            settings
                .integration
                .associations
                .iter()
                .any(|item| item.trim_start_matches('.').eq_ignore_ascii_case(extension))
        });
    if !is_archive {
        return Ok(());
    }
    let mode = match preset {
        quick::ExtractPreset::Here => nimzip_engine::ExtractMode::Here,
        quick::ExtractPreset::Smart => nimzip_engine::ExtractMode::Smart,
        quick::ExtractPreset::Folder => nimzip_engine::ExtractMode::Folder,
    };
    let options = nimzip_engine::ExtractOptions {
        mode,
        passwords: nimzip_engine::PasswordBook::load_default()
            .entries()
            .iter()
            .map(|entry| entry.password.clone())
            .collect(),
        codepage: nimzip_engine::Codepage::from_name(&settings.extraction.default_codepage),
        overwrite: true,
        propagate_motw: settings.security.propagate_motw,
        nested_depth: if settings.extraction.nested_enabled {
            settings.extraction.nested_depth
        } else {
            0
        },
        delete_archive: delete,
        amsi_scan: settings.security.amsi_scan,
        amsi_buffer_kb: settings.security.scan_buffer_kb,
        ..Default::default()
    };
    nimzip_engine::extract_archive(archive, &options, &nimzip_engine::NoopSink).map(|_| ())
}

fn take_flag_value(args: &mut Vec<String>, flag: &str) -> Option<PathBuf> {
    let position = args.iter().position(|arg| arg == flag)?;
    args.remove(position);
    if position < args.len() {
        Some(PathBuf::from(args.remove(position)))
    } else {
        None
    }
}

fn take_flag_values(args: &mut Vec<String>, flag: &str) -> Vec<PathBuf> {
    let mut values = Vec::new();
    while let Some(position) = args.iter().position(|arg| arg == flag) {
        args.remove(position);
        if position < args.len() {
            values.push(PathBuf::from(args.remove(position)));
        }
    }
    values
}

fn run_sfx(info: nimzip_engine::SfxInfo, exe: PathBuf) -> std::process::ExitCode {
    let silent = std::env::var_os("NIMZIP_SFX_SILENT").is_some();
    if !silent && (!info.config.prompt.trim().is_empty() || !info.config.title.trim().is_empty()) {
        let title = if info.config.title.trim().is_empty() {
            "NimZip 自解压"
        } else {
            info.config.title.as_str()
        };
        let message = if info.config.prompt.trim().is_empty() {
            format!(
                "将解压到：{}",
                exe.parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .display()
            )
        } else {
            info.config.prompt.clone()
        };
        let result = rfd::MessageDialog::new()
            .set_title(title)
            .set_description(message)
            .set_buttons(rfd::MessageButtons::OkCancel)
            .show();
        if result != rfd::MessageDialogResult::Ok {
            return std::process::ExitCode::SUCCESS;
        }
    }
    match nimzip_engine::sfx::extract_sfx(&exe, &exe, &nimzip_engine::NoopSink) {
        Ok(destination) => {
            if !info.config.run_after.trim().is_empty() {
                let target = PathBuf::from(&info.config.run_after);
                let target = if target.is_absolute() {
                    target
                } else {
                    destination.join(target)
                };
                let _ = open::that(target);
            }
            if silent {
                println!("{}", destination.display());
            } else {
                let _ = rfd::MessageDialog::new()
                    .set_title("NimZip 自解压")
                    .set_description(format!("解压完成：{}", destination.display()))
                    .set_buttons(rfd::MessageButtons::Ok)
                    .show();
                let _ = open::that(destination);
            }
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            if silent {
                eprintln!("SFX 解压失败：{error}");
            } else {
                let _ = rfd::MessageDialog::new()
                    .set_title("NimZip 自解压失败")
                    .set_description(error.to_string())
                    .set_buttons(rfd::MessageButtons::Ok)
                    .show();
            }
            std::process::ExitCode::from(1)
        }
    }
}

/// 附加到启动本进程的父控制台。成功返回 `true`，不创建新窗口。
#[cfg(windows)]
fn attach_parent_console() -> bool {
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
}

#[cfg(not(windows))]
fn attach_parent_console() -> bool {
    false
}

/// 为 CLI 模式准备控制台：优先附加父控制台；没有父控制台时创建一个并立即隐藏。
///
/// 资源管理器右键菜单会以无控制台方式启动 `nimzip.exe --cli ...`。此前
/// `AllocConsole` 会弹出一个黑色控制台窗口；这里保留 stdout/stderr 句柄供
/// 命令行输出使用，但把控制台窗口隐藏起来。
#[cfg(windows)]
fn ensure_console() {
    if attach_parent_console() {
        return;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn AllocConsole() -> i32;
        fn GetConsoleWindow() -> *mut std::ffi::c_void;
    }
    #[link(name = "user32")]
    extern "system" {
        fn ShowWindow(hwnd: *mut std::ffi::c_void, cmd: i32) -> i32;
    }
    const SW_HIDE: i32 = 0;
    unsafe {
        if AllocConsole() != 0 {
            let console = GetConsoleWindow();
            if !console.is_null() {
                let _ = ShowWindow(console, SW_HIDE);
            }
        }
    }
}

#[cfg(not(windows))]
fn ensure_console() {}
