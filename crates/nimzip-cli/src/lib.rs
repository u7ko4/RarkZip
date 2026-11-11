use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use clap::{Args, Parser, Subcommand};
use nimzip_engine::{
    create_archive, create_summary, extract_archive, format_size, format_unix_time,
    hash_file_with_progress, open_archive, save_checksum_file, verify_archive, ArchiveFormat,
    CreateOptions, EngineError, ExtractMode, ExtractOptions, HashAlgorithm, OpenOptions,
    PasswordBook, Progress, ProgressSink, Result, Settings, ZipAlgorithm,
};

static CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(Parser, Debug)]
#[command(
    name = "bx",
    version,
    about = "NimZip 无头命令行归档工具",
    long_about = "内容识别、智能解压、密码本与多格式读写，与 NimZip GUI 共用同一引擎。"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// 列出归档内容
    #[command(alias = "list", alias = "v")]
    L(ListArgs),
    /// 解压归档
    #[command(alias = "extract")]
    X(ExtractArgs),
    /// 解压并摊平目录结构
    #[command(name = "e")]
    E(ExtractFlatArgs),
    /// 创建归档
    #[command(alias = "add")]
    A(AddArgs),
    /// 在源文件旁创建归档（资源管理器右键菜单使用）
    #[command(name = "ch", alias = "compress-here")]
    Ch(CompressHereArgs),
    /// 每个源文件/文件夹各创建一个独立归档（右键多选使用）
    #[command(name = "ce", alias = "compress-each")]
    Ce(CompressEachArgs),
    /// 测试归档完整性
    #[command(alias = "test")]
    T(TestArgs),
    /// 计算文件或目录的哈希
    #[command(alias = "hash")]
    H(HashArgs),
    /// 修复 ZIP 归档（扫描本地文件头重建）
    Repair(RepairArgs),
    /// 恢复加密归档密码
    Recover(RecoverArgs),
    /// 从 ZIP 删除条目
    #[command(alias = "delete")]
    D(DeleteArgs),
    /// 重命名 ZIP 中的条目
    #[command(name = "rn", alias = "rename")]
    Rn(RenameArgs),
    /// 修改 ZIP 注释
    #[command(alias = "comment")]
    C(CommentArgs),
    /// 安装/卸载资源管理器右键菜单
    Shell(ShellArgs),
    /// 使用 Windows AMSI 扫描文件缓冲区
    #[command(alias = "s")]
    Scan(ScanArgs),
}

#[derive(Args, Debug)]
struct ListArgs {
    /// 归档路径
    archive: PathBuf,
    /// 密码（可选，默认先试密码本）
    #[arg(short = 'p', long)]
    password: Option<String>,
    /// 输出 JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct ExtractArgs {
    /// 归档路径
    archive: PathBuf,
    /// 解压到指定目录
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
    /// 解压到压缩包所在目录
    #[arg(long, conflicts_with_all = ["output", "folder"])]
    here: bool,
    /// 解压到同名文件夹
    #[arg(long, conflicts_with_all = ["output", "here"])]
    folder: bool,
    /// 剥掉包内唯一的根文件夹
    #[arg(long)]
    strip_root: bool,
    /// 成功后删除压缩包（回收站）
    #[arg(long)]
    delete: bool,
    /// 不递归解压嵌套压缩包
    #[arg(long)]
    no_nested: bool,
    /// 嵌套解压深度
    #[arg(long, default_value_t = 3)]
    depth: u8,
    /// 密码（可选）
    #[arg(short = 'p', long)]
    password: Option<String>,
    /// 解压后使用 Windows AMSI 扫描产物
    #[arg(long)]
    amsi: bool,
    /// AMSI 单文件扫描缓冲上限（KB）
    #[arg(long, default_value_t = 1024)]
    scan_buffer: u32,
}

#[derive(Args, Debug)]
struct ExtractFlatArgs {
    /// 归档路径
    archive: PathBuf,
    /// 输出目录
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
    /// 密码
    #[arg(short = 'p', long)]
    password: Option<String>,
}

#[derive(Args, Debug)]
struct AddArgs {
    /// 目标归档路径
    archive: PathBuf,
    /// 要压缩的文件或文件夹
    #[arg(required = true)]
    files: Vec<PathBuf>,
    /// 格式：zip/zipx/7z/tar/tgz/tbz/txz/tzst/gz/bz2/xz/zst/lzh/iso/zpaq
    #[arg(short = 't', long = "type")]
    format: Option<String>,
    /// 压缩级别 0-9
    #[arg(short = 'm', long = "mx", default_value_t = 5)]
    level: u8,
    /// 密码（ZIP 使用 AES-256）
    #[arg(short = 'p', long)]
    password: Option<String>,
    /// 排除规则，可重复，例如 -x "*.tmp"
    #[arg(short = 'x', long = "exclude")]
    exclude: Vec<String>,
    /// 不把选中的文件夹本身作为根目录
    #[arg(long)]
    no_root: bool,
    /// 创建后自动测试
    #[arg(long)]
    test: bool,
    /// 测试通过后删除源文件（回收站）
    #[arg(long)]
    delete_sources: bool,
    /// ZIP 压缩算法：deflate/bzip2/lzma/zstd
    #[arg(long, default_value = "deflate")]
    zip_algo: String,
    /// 归档注释
    #[arg(short = 'c', long)]
    comment: Option<String>,
    /// 覆盖已存在的归档
    #[arg(short = 'y', long)]
    yes: bool,
}

#[derive(Args, Debug)]
struct CompressHereArgs {
    /// 源文件或文件夹
    #[arg(required = true)]
    sources: Vec<PathBuf>,
    /// 目标格式：zip/zipx/7z/tar/tgz/gz/xz/zst/lzh/iso/zpaq 等
    #[arg(short = 'f', long = "format", default_value = "zip")]
    format: String,
    /// 成功后删除源文件（回收站）
    #[arg(long)]
    delete_sources: bool,
    /// 密码（可选）
    #[arg(short = 'p', long)]
    password: Option<String>,
}

#[derive(Args, Debug)]
struct CompressEachArgs {
    /// 源文件或文件夹（每个条目各生成一个归档）
    #[arg(required = true)]
    sources: Vec<PathBuf>,
    /// 目标格式：zip/7z/zipx/tar 等
    #[arg(short = 'f', long = "format", default_value = "zip")]
    format: String,
    /// 密码（可选）
    #[arg(short = 'p', long)]
    password: Option<String>,
}

#[derive(Args, Debug)]
struct RepairArgs {
    /// 要修复的 ZIP 文件
    archive: PathBuf,
    /// 输出路径；默认同目录 <名称>.repaired.zip
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct RecoverArgs {
    /// 加密归档路径
    archive: PathBuf,
    /// 掩码模式，例如 "?l?l?d?d"
    #[arg(long)]
    mask: Option<String>,
    /// 暴力模式字符集
    #[arg(long)]
    charset: Option<String>,
    /// 暴力模式最短长度
    #[arg(long, default_value_t = 1)]
    min: usize,
    /// 暴力模式最长长度
    #[arg(long, default_value_t = 4)]
    max: usize,
    /// 字典模式：每行一个候选密码
    #[arg(long)]
    dictionary: Option<PathBuf>,
    /// 线程数
    #[arg(long, default_value_t = 4)]
    threads: usize,
}

#[derive(Args, Debug)]
struct DeleteArgs {
    /// ZIP 归档
    archive: PathBuf,
    /// 要删除的条目路径（可重复）
    #[arg(required = true)]
    entries: Vec<String>,
    /// 密码（加密 ZIP）
    #[arg(short = 'p', long)]
    password: Option<String>,
}

#[derive(Args, Debug)]
struct RenameArgs {
    /// ZIP 归档
    archive: PathBuf,
    /// 原条目路径
    old: String,
    /// 新条目路径
    new: String,
    /// 密码（加密 ZIP）
    #[arg(short = 'p', long)]
    password: Option<String>,
}

#[derive(Args, Debug)]
struct CommentArgs {
    /// ZIP 归档
    archive: PathBuf,
    /// 新注释；省略则清空
    comment: Option<String>,
    /// 密码（加密 ZIP）
    #[arg(short = 'p', long)]
    password: Option<String>,
}

#[derive(Args, Debug)]
struct ScanArgs {
    /// 要扫描的文件
    path: PathBuf,
    /// 扫描缓冲上限（KB）
    #[arg(long, default_value_t = 4096)]
    buffer: u32,
}

#[derive(Args, Debug)]
struct ShellArgs {
    /// install / uninstall / status
    #[arg(value_parser = ["install", "uninstall", "status"])]
    action: String,
}

#[derive(Args, Debug)]
struct TestArgs {
    /// 归档路径
    archive: PathBuf,
    /// 密码（可选）
    #[arg(short = 'p', long)]
    password: Option<String>,
}

#[derive(Args, Debug)]
struct HashArgs {
    /// 文件或目录（目录递归）
    #[arg(required = true)]
    files: Vec<PathBuf>,
    /// 算法：md5/sha1/sha256/sha512/sha3/blake3/xxhash64/crc32
    #[arg(short = 'a', long = "algorithm", default_value = "sha256")]
    algorithm: String,
    /// 同时输出全部算法
    #[arg(long)]
    all: bool,
    /// 保存为校验文件（单算法）
    #[arg(long)]
    save: Option<PathBuf>,
}

use std::sync::Once;

static INSTALL_CTRLC: Once = Once::new();

/// 独立 bx 二进制入口。
pub fn main_entry() -> std::process::ExitCode {
    run_with_args(std::env::args().collect())
}

/// 供 nimzip.exe 在 CLI 模式下复用同一套命令与退出码。
pub fn run_with_args(args: Vec<String>) -> std::process::ExitCode {
    install_ctrlc();
    let cli = Cli::parse_from(args);
    match run(cli) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("bx: {error}");
            if error.is_password_related() {
                std::process::ExitCode::from(2)
            } else {
                std::process::ExitCode::from(1)
            }
        }
    }
}

fn install_ctrlc() {
    INSTALL_CTRLC.call_once(|| {
        let _ = ctrlc::set_handler(|| {
            CANCELLED.store(true, Ordering::Relaxed);
        });
    });
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::L(args) => list_command(args),
        Command::X(args) => extract_command(args),
        Command::E(args) => extract_flat_command(args),
        Command::A(args) => add_command(args),
        Command::Ch(args) => compress_here_command(args),
        Command::Ce(args) => compress_each_command(args),
        Command::T(args) => test_command(args),
        Command::H(args) => hash_command(args),
        Command::Repair(args) => repair_command(args),
        Command::Recover(args) => recover_command(args),
        Command::D(args) => delete_command(args),
        Command::Rn(args) => rename_command(args),
        Command::C(args) => comment_command(args),
        Command::Shell(args) => shell_command(args),
        Command::Scan(args) => scan_command(args),
    }
}

fn list_command(args: ListArgs) -> Result<()> {
    let info = open_for_list(&args.archive, args.password)?;
    if args.json {
        let entries: Vec<_> = info
            .entries
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "path": entry.path,
                    "directory": entry.is_dir,
                    "size": entry.size,
                    "compressed_size": entry.compressed_size,
                    "modified": entry.modified,
                    "method": entry.method,
                    "encrypted": entry.encrypted,
                    "crc32": entry.crc32,
                })
            })
            .collect();
        let value = serde_json::json!({
            "path": args.archive.to_string_lossy(),
            "format": info.format.extension(),
            "encrypted": info.encrypted,
            "disguised": info.disguised,
            "protected_container": info.protected_container,
            "comment": info.comment,
            "entries": entries,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value)
                .map_err(|error| EngineError::Other(error.to_string()))?
        );
        return Ok(());
    }

    println!("{}  {}", info.format.label(), args.archive.display());
    if info.disguised {
        println!("已去除 apate 伪装（使用临时还原副本）");
    }
    if let Some(comment) = &info.comment {
        println!("注释：{comment}");
    }
    println!(
        "{:<10} {:>12} {:>12}  {:<16} {:<8} {:<5} 路径",
        "类型", "原始", "压缩后", "修改时间", "方法", "加密"
    );
    for entry in &info.entries {
        let kind = if entry.is_dir { "<DIR>" } else { "" };
        println!(
            "{:<10} {:>12} {:>12}  {:<16} {:<8} {:<5} {}",
            kind,
            if entry.is_dir {
                "-".to_string()
            } else {
                format_size(entry.size)
            },
            if entry.is_dir {
                "-".to_string()
            } else {
                format_size(entry.compressed_size)
            },
            format_unix_time(entry.modified),
            entry.method,
            if entry.encrypted { "是" } else { "-" },
            entry.path
        );
    }
    println!(
        "{} 项 · 原始 {} · 压缩后 {} · 压缩比 {}",
        info.file_count(),
        format_size(info.total_size()),
        format_size(info.total_compressed()),
        nimzip_engine::format_ratio(info.total_compressed(), info.total_size())
    );
    Ok(())
}

fn open_for_list(path: &Path, password: Option<String>) -> Result<nimzip_engine::ArchiveInfo> {
    let first = open_archive(
        path,
        &OpenOptions {
            password: password.clone(),
            ..Default::default()
        },
    );
    match first {
        Ok(info) => Ok(info),
        Err(EngineError::PasswordRequired) => {
            let resolved = resolve_password(path, password)?;
            open_archive(
                path,
                &OpenOptions {
                    password: resolved,
                    ..Default::default()
                },
            )
        }
        Err(error) => Err(error),
    }
}

fn extract_command(args: ExtractArgs) -> Result<()> {
    let password = resolve_password(&args.archive, args.password)?;
    let mode = if let Some(output) = args.output {
        ExtractMode::Custom(output)
    } else if args.here {
        ExtractMode::Here
    } else if args.folder {
        ExtractMode::Folder
    } else {
        ExtractMode::Smart
    };
    let options = ExtractOptions {
        mode,
        password,
        passwords: PasswordBook::load_default()
            .entries()
            .iter()
            .map(|entry| entry.password.clone())
            .collect(),
        strip_root: args.strip_root,
        nested_depth: if args.no_nested { 0 } else { args.depth },
        delete_archive: args.delete,
        amsi_scan: args.amsi,
        amsi_buffer_kb: args.scan_buffer,
        ..Default::default()
    };
    let sink = ConsoleSink::default();
    let outcome = extract_archive(&args.archive, &options, &sink)?;
    println!(
        "解压完成：{} · {} 个文件 · {} · 嵌套 {} 个{}",
        outcome.dest.display(),
        outcome.files,
        format_size(outcome.bytes),
        outcome.nested_archives,
        if outcome.skipped.is_empty() {
            String::new()
        } else {
            format!(" · 跳过 {}", outcome.skipped.len())
        }
    );
    for skipped in &outcome.skipped {
        eprintln!("跳过：{skipped}");
    }
    Ok(())
}

fn extract_flat_command(args: ExtractFlatArgs) -> Result<()> {
    let password = resolve_password(&args.archive, args.password)?;
    let output = args.output.unwrap_or_else(|| {
        args.archive
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    });
    std::fs::create_dir_all(nimzip_engine::paths::to_extended(&output))?;
    let temp = std::env::temp_dir().join(format!(
        "nimzip-flat-{}-{}",
        std::process::id(),
        nimzip_engine::util::now_unix()
    ));
    let options = ExtractOptions {
        mode: ExtractMode::Custom(temp.clone()),
        password,
        nested_depth: 0,
        ..Default::default()
    };
    nimzip_engine::extract_archive(&args.archive, &options, &ConsoleSink::default())?;
    let mut count = 0usize;
    for entry in walkdir::WalkDir::new(&temp).follow_links(false) {
        let entry = entry.map_err(|error| EngineError::Other(error.to_string()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str() else {
            continue;
        };
        let mut target = output.join(name);
        let mut suffix = 2usize;
        while target.exists() {
            target = output.join(format!(
                "{}-{suffix}.{}",
                entry
                    .path()
                    .file_stem()
                    .map(|value| value.to_string_lossy())
                    .unwrap_or_default(),
                entry
                    .path()
                    .extension()
                    .map(|value| value.to_string_lossy())
                    .unwrap_or_default()
            ));
            suffix += 1;
        }
        std::fs::copy(entry.path(), &target)?;
        count += 1;
    }
    let _ = std::fs::remove_dir_all(&temp);
    println!("摊平解压完成：{} 个文件 → {}", count, output.display());
    Ok(())
}

fn add_command(args: AddArgs) -> Result<()> {
    let sfx = args
        .format
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("sfx"))
        || args
            .archive
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"));
    if sfx {
        return add_sfx_command(args);
    }
    let format = args
        .format
        .as_deref()
        .and_then(ArchiveFormat::from_name)
        .or_else(|| ArchiveFormat::from_extension(&args.archive))
        .unwrap_or(ArchiveFormat::Zip);
    if !format.can_create() {
        return Err(EngineError::UnsupportedFormat(format!(
            "{} 不支持创建",
            format.label()
        )));
    }
    let zip_algorithm = match args.zip_algo.to_ascii_lowercase().as_str() {
        "deflate" => ZipAlgorithm::Deflate,
        "bzip2" => ZipAlgorithm::Bzip2,
        "lzma" => ZipAlgorithm::Lzma,
        "zstd" => ZipAlgorithm::Zstd,
        other => {
            return Err(EngineError::Other(format!(
                "未知 ZIP 算法：{other}（可用 deflate/bzip2/lzma/zstd）"
            )))
        }
    };
    let test_after = args.test || args.delete_sources;
    let options = CreateOptions {
        format,
        level: args.level.min(9),
        password: args.password,
        zip_algorithm,
        include_root: !args.no_root,
        exclude: args.exclude,
        overwrite: args.yes,
        comment: args.comment,
        test_after,
        delete_sources: args.delete_sources,
        sfx: false,
    };
    let sink = ConsoleSink::default();
    let outcome = create_archive(&args.files, &args.archive, &options, &sink)?;
    println!("创建完成：{}", create_summary(&outcome));
    Ok(())
}

fn compress_here_command(args: CompressHereArgs) -> Result<()> {
    let format = ArchiveFormat::from_name(&args.format)
        .filter(|format| format.can_create())
        .ok_or_else(|| {
            EngineError::UnsupportedFormat(format!("{} 不支持创建", args.format.trim()))
        })?;
    let Some(first) = args.sources.first() else {
        return Err(EngineError::Other("缺少源文件".to_string()));
    };
    let parent = first
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let stem = if first.is_dir() || format.is_single_stream() {
        first.file_name()
    } else {
        first.file_stem()
    }
    .map(|name| name.to_string_lossy().to_string())
    .filter(|name| !name.is_empty())
    .unwrap_or_else(|| "archive".to_string());
    let archive = nimzip_engine::paths::unique_path(&parent, &stem, Some(format.extension()));
    let options = CreateOptions {
        format,
        level: 5,
        password: args.password,
        zip_algorithm: ZipAlgorithm::Deflate,
        include_root: true,
        exclude: Vec::new(),
        overwrite: false,
        comment: None,
        test_after: args.delete_sources,
        delete_sources: args.delete_sources,
        sfx: false,
    };
    let sink = ConsoleSink::default();
    let outcome = create_archive(&args.sources, &archive, &options, &sink)?;
    println!("创建完成：{}", create_summary(&outcome));
    Ok(())
}

fn compress_each_command(args: CompressEachArgs) -> Result<()> {
    let format = ArchiveFormat::from_name(&args.format)
        .filter(|format| format.can_create())
        .ok_or_else(|| {
            EngineError::UnsupportedFormat(format!("{} 不支持创建", args.format.trim()))
        })?;
    let sink = ConsoleSink::default();
    let mut created = 0usize;
    let mut errors = Vec::new();
    for source in &args.sources {
        if !source.exists() {
            errors.push(format!("{}：源文件不存在", source.display()));
            continue;
        }
        let parent = source
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let stem = if source.is_dir() || format.is_single_stream() {
            source.file_name()
        } else {
            source.file_stem()
        }
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "archive".to_string());
        let archive = nimzip_engine::paths::unique_path(&parent, &stem, Some(format.extension()));
        let options = CreateOptions {
            format,
            level: 5,
            password: args.password.clone(),
            zip_algorithm: ZipAlgorithm::Deflate,
            include_root: true,
            exclude: Vec::new(),
            overwrite: false,
            comment: None,
            test_after: false,
            delete_sources: false,
            sfx: false,
        };
        match create_archive(std::slice::from_ref(source), &archive, &options, &sink) {
            Ok(outcome) => {
                println!("创建完成：{}", create_summary(&outcome));
                created += 1;
            }
            Err(error) => errors.push(format!("{}：{error}", source.display())),
        }
    }
    for error in &errors {
        eprintln!("压缩失败：{error}");
    }
    if created == 0 && !errors.is_empty() {
        return Err(EngineError::Other(errors.join("；")));
    }
    Ok(())
}

fn add_sfx_command(args: AddArgs) -> Result<()> {
    if args.password.is_some() {
        return Err(EngineError::Other(
            "SFX 暂不支持加密载荷；请先创建加密 ZIP，再用 GUI 封装".to_string(),
        ));
    }
    if args.archive.exists() && !args.yes {
        return Err(EngineError::AlreadyExists(args.archive.clone()));
    }
    if let Some(parent) = args.archive.parent() {
        std::fs::create_dir_all(nimzip_engine::paths::to_extended(parent))?;
    }
    let temp = std::env::temp_dir().join(format!("nimzip-sfx-{}.zip", std::process::id()));
    let options = CreateOptions {
        format: ArchiveFormat::Zip,
        level: args.level.min(9),
        password: None,
        zip_algorithm: match args.zip_algo.to_ascii_lowercase().as_str() {
            "bzip2" => ZipAlgorithm::Bzip2,
            "lzma" => ZipAlgorithm::Lzma,
            "zstd" => ZipAlgorithm::Zstd,
            _ => ZipAlgorithm::Deflate,
        },
        include_root: !args.no_root,
        exclude: args.exclude,
        overwrite: true,
        comment: args.comment,
        test_after: true,
        delete_sources: false,
        sfx: false,
    };
    let sink = ConsoleSink::default();
    create_archive(&args.files, &temp, &options, &sink)?;
    let stub = sfx_stub()?;
    let config = nimzip_engine::SfxConfig {
        title: "NimZip 自解压".to_string(),
        ..Default::default()
    };
    let result = nimzip_engine::sfx::create_sfx(&temp, &args.archive, &stub, &config);
    let _ = std::fs::remove_file(&temp);
    result?;
    if args.test {
        verify_archive(&args.archive, None, &sink)?;
    }
    println!(
        "SFX 创建完成：{}（自解压到 EXE 所在目录）",
        args.archive.display()
    );
    Ok(())
}

fn sfx_stub() -> Result<PathBuf> {
    let current = std::env::current_exe()?;
    let stem = current
        .file_stem()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if stem == "nimzip" {
        return Ok(current);
    }
    if let Some(parent) = current.parent() {
        let candidate = parent.join("nimzip.exe");
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(EngineError::Other(
        "找不到 NimZip 主程序作为 SFX stub；请使用 dist/nimzip.exe 创建".to_string(),
    ))
}

fn test_command(args: TestArgs) -> Result<()> {
    let password = resolve_password(&args.archive, args.password)?;
    let sink = ConsoleSink::default();
    let report = verify_archive(&args.archive, password.as_deref(), &sink)?;
    println!(
        "校验通过：{} · {} 项 · {}",
        report.format.label(),
        report.entries,
        format_size(report.bytes)
    );
    Ok(())
}

fn hash_command(args: HashArgs) -> Result<()> {
    let algorithms = if args.all {
        HashAlgorithm::ALL.to_vec()
    } else {
        vec![HashAlgorithm::from_name(&args.algorithm).ok_or_else(|| {
            EngineError::Other(format!(
                "未知哈希算法：{}（可用 sha256/sha1/md5/sha512/sha3/blake3/xxhash64/crc32）",
                args.algorithm
            ))
        })?]
    };
    if args.save.is_some() && algorithms.len() != 1 {
        return Err(EngineError::Other(
            "保存校验文件时只能使用一种算法".to_string(),
        ));
    }

    let files = collect_files(&args.files)?;
    if files.is_empty() {
        return Err(EngineError::Other("没有可计算的文件".to_string()));
    }
    let sink = ConsoleSink::default();
    let mut checksum_entries = Vec::new();
    for file in &files {
        if args.all {
            println!("{}", file.display());
        }
        for algorithm in &algorithms {
            let hash = hash_file_with_progress(*algorithm, file, &sink)?;
            if args.all {
                println!("  {:<9} {hash}", algorithm.label());
            } else {
                println!("{hash}  {}", file.display());
            }
            if args.save.is_some() {
                checksum_entries.push((hash, file.clone()));
            }
        }
    }
    if let Some(path) = args.save {
        save_checksum_file(&path, &checksum_entries)?;
        eprintln!("校验文件已保存：{}", path.display());
    }
    Ok(())
}

fn scan_command(args: ScanArgs) -> Result<()> {
    match nimzip_engine::scan_file(&args.path, args.buffer as u64 * 1024)? {
        nimzip_engine::AmsiVerdict::Clean => {
            println!("未检测到威胁：{}", args.path.display());
            Ok(())
        }
        nimzip_engine::AmsiVerdict::Detected => Err(EngineError::Other(format!(
            "AMSI 检测到恶意内容：{}",
            args.path.display()
        ))),
        nimzip_engine::AmsiVerdict::Unavailable(error) => {
            Err(EngineError::Other(format!("AMSI 不可用：{error}")))
        }
    }
}

fn shell_command(args: ShellArgs) -> Result<()> {
    use nimzip_shell::{ShellOptions, ShellScope};
    match args.action.as_str() {
        "install" => {
            let exe = shell_exe()?;
            let settings = Settings::load();
            nimzip_shell::install(&ShellOptions {
                exe,
                scope: ShellScope::CurrentUser,
                archive_extensions: settings.integration.associations.clone(),
                extract_verbs: settings.integration.extract_verbs.clone(),
                hash_menu: settings.integration.hash_menu,
                compress_verbs: settings.integration.compress_verbs.clone(),
                empty_verbs: settings.integration.empty_verbs.clone(),
                double_click: settings.integration.double_click.clone(),
                double_click_folder: settings.integration.double_click_folder.clone(),
                cascade: settings.integration.cascade,
                hide_icon: settings.integration.hide_icon,
            })
            .map_err(|error| EngineError::Other(error.to_string()))?;
            println!("已安装 NimZip 经典右键菜单");
        }
        "uninstall" => {
            nimzip_shell::uninstall(ShellScope::CurrentUser)
                .map_err(|error| EngineError::Other(error.to_string()))?;
            println!("已卸载 NimZip 经典右键菜单");
        }
        "status" => {
            let installed = nimzip_shell::is_installed(ShellScope::CurrentUser);
            println!(
                "经典右键菜单：{}",
                if installed { "已安装" } else { "未安装" }
            );
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn shell_exe() -> Result<PathBuf> {
    let current = std::env::current_exe()?;
    if current
        .file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case("nimzip"))
    {
        return Ok(current);
    }
    if let Some(parent) = current.parent() {
        let candidate = parent.join("nimzip.exe");
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(EngineError::Other(
        "右键菜单需要指向 nimzip.exe；请在 dist 或安装目录运行".to_string(),
    ))
}

fn repair_command(args: RepairArgs) -> Result<()> {
    let output = args.output.unwrap_or_else(|| {
        let stem = args
            .archive
            .file_stem()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| "archive".to_string());
        args.archive.with_file_name(format!("{stem}.repaired.zip"))
    });
    let report = nimzip_engine::repair_zip(&args.archive, &output, &ConsoleSink::default())?;
    println!(
        "修复完成：恢复 {}/{} 项，跳过 {} 项，输出 {}",
        report.recovered,
        report.entries,
        report.skipped,
        report.output.display()
    );
    Ok(())
}

fn recover_command(args: RecoverArgs) -> Result<()> {
    use nimzip_engine::{RecoveryControl, RecoveryMode, RecoveryOptions};
    let mode = if let Some(path) = args.dictionary {
        let text = std::fs::read_to_string(&path)
            .map_err(|error| EngineError::Other(format!("读取字典失败：{error}")))?;
        RecoveryMode::Dictionary(
            text.lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect(),
        )
    } else if let Some(charset) = args.charset {
        RecoveryMode::Brute {
            charset,
            min_len: args.min,
            max_len: args.max,
        }
    } else {
        RecoveryMode::Mask(args.mask.unwrap_or_else(|| "?l?l?l?l".to_string()))
    };
    let control = RecoveryControl::new();
    eprintln!("开始恢复密码…按 Ctrl+C 取消");
    let last = std::sync::atomic::AtomicU64::new(0);
    let outcome = nimzip_engine::recover_password(
        &args.archive,
        &RecoveryOptions {
            mode,
            threads: args.threads.max(1),
        },
        &control,
        |attempts, total| {
            let step = (attempts.saturating_mul(100))
                .checked_div(total)
                .unwrap_or(0)
                .saturating_div(5);
            if attempts == 0 || attempts == total || last.swap(step, Ordering::Relaxed) != step {
                eprintln!("已尝试 {attempts} / {total}");
            }
        },
    )?;
    if let Some(password) = outcome.password {
        println!("找到密码：{password}");
        Ok(())
    } else if outcome.cancelled {
        Err(EngineError::Cancelled)
    } else {
        Err(EngineError::Other(format!(
            "未找到密码（尝试 {} 次，用时 {:.1} 秒）",
            outcome.attempts,
            outcome.elapsed.as_secs_f64()
        )))
    }
}

fn find_entry_indices(info: &nimzip_engine::ArchiveInfo, names: &[String]) -> Result<Vec<usize>> {
    let mut indices = Vec::new();
    for name in names {
        let name = name.trim_matches('/');
        let Some(entry) = info
            .entries
            .iter()
            .find(|entry| entry.path == name || entry.name() == name)
        else {
            return Err(EngineError::NotFound(PathBuf::from(name)));
        };
        if !indices.contains(&entry.index) {
            indices.push(entry.index);
        }
    }
    Ok(indices)
}

fn delete_command(args: DeleteArgs) -> Result<()> {
    let password = resolve_password(&args.archive, args.password)?;
    let info = open_archive(
        &args.archive,
        &OpenOptions {
            password: password.clone(),
            ..Default::default()
        },
    )?;
    if !info.format.is_zip() {
        return Err(EngineError::UnsupportedFormat(
            "当前仅支持编辑 ZIP 归档".to_string(),
        ));
    }
    let mut delete = find_entry_indices(&info, &args.entries)?;
    let mut expanded = Vec::new();
    for index in &delete {
        if let Some(entry) = info.find(*index) {
            if entry.is_dir {
                let prefix = format!("{}/", entry.path.trim_end_matches('/'));
                expanded.extend(
                    info.entries
                        .iter()
                        .filter(|candidate| candidate.path.starts_with(&prefix))
                        .map(|candidate| candidate.index),
                );
            }
        }
    }
    delete.extend(expanded);
    delete.sort_unstable();
    delete.dedup();
    let report = nimzip_engine::edit_zip(
        &args.archive,
        &args.archive,
        password.as_deref(),
        &nimzip_engine::ZipEditOptions {
            delete,
            ..Default::default()
        },
    )?;
    println!(
        "已删除，重写 {} 个条目：{}",
        report.written,
        args.archive.display()
    );
    Ok(())
}

fn rename_command(args: RenameArgs) -> Result<()> {
    let password = resolve_password(&args.archive, args.password)?;
    let info = open_archive(
        &args.archive,
        &OpenOptions {
            password: password.clone(),
            ..Default::default()
        },
    )?;
    if !info.format.is_zip() {
        return Err(EngineError::UnsupportedFormat(
            "当前仅支持编辑 ZIP 归档".to_string(),
        ));
    }
    let index = find_entry_indices(&info, std::slice::from_ref(&args.old))?[0];
    let mut rename = std::collections::HashMap::new();
    rename.insert(index, args.new.clone());
    let report = nimzip_engine::edit_zip(
        &args.archive,
        &args.archive,
        password.as_deref(),
        &nimzip_engine::ZipEditOptions {
            rename,
            ..Default::default()
        },
    )?;
    println!(
        "已重命名为 {}，重写 {} 个条目：{}",
        args.new,
        report.written,
        args.archive.display()
    );
    Ok(())
}

fn comment_command(args: CommentArgs) -> Result<()> {
    let password = resolve_password(&args.archive, args.password)?;
    let info = open_archive(
        &args.archive,
        &OpenOptions {
            password: password.clone(),
            ..Default::default()
        },
    )?;
    if !info.format.is_zip() {
        return Err(EngineError::UnsupportedFormat(
            "当前仅支持编辑 ZIP 注释".to_string(),
        ));
    }
    let comment = args.comment.unwrap_or_default();
    let report = nimzip_engine::edit_zip(
        &args.archive,
        &args.archive,
        password.as_deref(),
        &nimzip_engine::ZipEditOptions {
            comment: Some(comment.clone()),
            ..Default::default()
        },
    )?;
    println!(
        "注释已更新（{} 个字符），重写 {} 个条目：{}",
        comment.chars().count(),
        report.written,
        report.output.display()
    );
    Ok(())
}

fn collect_files(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            for entry in walkdir::WalkDir::new(path).follow_links(false) {
                let entry = entry.map_err(|error| EngineError::Other(error.to_string()))?;
                if entry.file_type().is_file() {
                    files.push(entry.path().to_path_buf());
                }
            }
        } else if path.is_file() {
            files.push(path.clone());
        } else {
            return Err(EngineError::NotFound(path.clone()));
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn resolve_password(path: &Path, provided: Option<String>) -> Result<Option<String>> {
    if let Some(password) = provided {
        return Ok(Some(password));
    }

    match open_archive(path, &OpenOptions::default()) {
        Ok(info) if !info.encrypted => return Ok(None),
        Ok(_) | Err(EngineError::PasswordRequired) | Err(EngineError::WrongPassword) => {}
        Err(error) => return Err(error),
    }

    let mut book = PasswordBook::load_default();
    if let Some(password) = book.try_all(path)? {
        eprintln!("使用密码本中的密码");
        return Ok(Some(password));
    }

    if !std::io::stdin().is_terminal() {
        return Err(EngineError::PasswordRequired);
    }
    let prompt = format!("请输入 {} 的密码（留空取消）：", path.display());
    let password = rpassword::prompt_password(prompt)
        .map_err(|error| EngineError::Other(format!("读取密码失败：{error}")))?;
    if password.is_empty() {
        return Err(EngineError::PasswordRequired);
    }
    if !nimzip_engine::test_password(path, &password)? {
        return Err(EngineError::WrongPassword);
    }
    if let Err(error) = book.remember(&password) {
        eprintln!("警告：保存密码到密码本失败：{error}");
    }
    Ok(Some(password))
}

#[derive(Default)]
struct ConsoleSink {
    last_percent: AtomicI32,
}

impl ProgressSink for ConsoleSink {
    fn update(&self, progress: &Progress) {
        let percent = progress.percent() as i32;
        let previous = self.last_percent.swap(percent, Ordering::Relaxed);
        if percent == previous {
            return;
        }
        if percent < 100 && percent % 5 != 0 {
            return;
        }
        let current = truncate(&progress.current, 48);
        if std::io::stderr().is_terminal() {
            eprint!(
                "\r{:<8} {:>3}%  {:<48}",
                progress.phase.label(),
                percent,
                current
            );
            if percent >= 100 {
                eprintln!();
            }
            let _ = std::io::stderr().flush();
        } else {
            eprintln!("{:<8} {:>3}%  {}", progress.phase.label(), percent, current);
        }
    }

    fn warn(&self, message: &str) {
        eprintln!("警告：{message}");
    }

    fn is_cancelled(&self) -> bool {
        CANCELLED.load(Ordering::Relaxed)
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut output: String = value.chars().take(max_chars).collect();
    if value.chars().count() > max_chars {
        output.push('…');
    }
    output
}
