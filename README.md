# NimZip

现代、快速、安全的 Windows 压缩与归档管理工具。核心引擎为纯 Rust，界面使用 GPUI Kit（GPUI 0.3 系列），同一引擎同时驱动 GUI 与无头 CLI。

> 本仓库覆盖 M1–M3 的主要内容：真实归档读写、智能解压、密码体系、伪装识别、嵌套解压、哈希、RAR/分卷/SFX、密码恢复、资源管理器 Shell 集成与可用的桌面界面。

## 已实现能力

### 内容识别与归档读写

- 按签名识别 **ZIP / 7Z / TAR / TAR.GZ / TAR.BZ2 / TAR.XZ / TAR.ZST / GZ / BZ2 / XZ / ZST**，扩展名只作为提示。
- ZIP 支持任意长度前缀（SFX、直接改后缀的伪装）：从文件尾回扫 EOCD，并从中央目录偏移计算真实载荷位置；ZIP64 兼容。
- **伪装格式还原**：识别支持的伪装格式后，把还原副本写入临时目录；**原文件始终只读**。
- ZIP 读取：Store / Deflate / BZip2 / LZMA / Zstd / PPMd 等，`ZipCrypto` 与 `AES-256` 解密。
- ZIP 创建：Deflate / BZip2 / LZMA / Zstd，AES-256 加密；自动 ZIP64。
- 7z 读取：LZMA/LZMA2、固实块、头加密（`-hp`）、AES-256。
- 7z 创建：LZMA2（默认固实参数）；支持 AES-256 内容加密与头加密（等价 `-hp`），GUI/CLI 传密码即启用。
- RAR 读取：libunrar（RAR4/RAR5、固实、文件加密、头加密、`.part1.rar` / `.r00` 分卷），RAR 压缩受许可限制不提供。
- 分卷：`.zip.001` / `.7z.001` / `.tar.gz.001` 等按顺序合并到临时目录，原分卷只读；RAR 分卷由 unrar 原生处理。
- 自解压：可创建 SFX EXE（stub 为 NimZip 本体，内嵌 ZIP 载荷），双击后解压到 EXE 所在目录；可识别并读取带 EXE 前缀的 ZIP / 7z SFX 归档。
- tar 系与单流 gzip/bzip2/xz/zstd 读写；单流格式只允许一个文件源。
- ZIP 非 UTF-8 文件名解码：自动 / UTF-8 / GBK / Big5 / Shift-JIS / Windows-1252 / CP437。

### 智能解压与安全

- 智能目标：单文件落当前目录、单个根文件夹原样解压到当前目录、其他结构解压到同名文件夹。
- 解压路径安全检查：拒绝绝对路径、盘符、`..`、NUL、NTFS ADS 与 Windows 保留名；符号链接/硬链接条目跳过并警告。
- 解压前预分配目标空间；Windows 长路径 `\\?\` 归一化；保留修改时间。
- **MOTW 传播**：读取压缩包上的 `Zone.Identifier` 并写到解压产物。
- 删除压缩包/源文件走回收站（失败不静默永久删除）。
- 流式哈希：MD5 / SHA-1 / SHA-256 / SHA-512 / SHA3-256 / BLAKE3 / xxHash64 / CRC32；1 MiB 分块，不整载内存；支持标准 `.sha256`/`.md5` 校验文件。GUI 批量哈希支持多文件/文件夹递归、算法切换、结果复制、校验文件导出与**载入校验文件逐行比对**（匹配/不匹配/未校验）；CLI 支持目录递归与全部算法。
- ZIP 修复：扫描本地文件头重建中央目录，恢复 Store/Deflate/BZip2 条目，跳过损坏/加密/描述符条目并报告。
- 密码恢复：字典 / 掩码 / 暴力三种模式，多线程、暂停/取消；命中后自动记入密码本。
- AMSI 安全扫描：解压产物写盘后按配置缓冲区扫描，命中即删除并警告；工具栏/菜单可扫描当前文件。
- 进度、阶段、取消回调；GUI 使用非模态任务中心。

### 密码体系

- 密码本：顺序即尝试顺序，支持增删、上移/下移；首次运行会写入内置兼容密码并导入程序目录/当前目录的 `passwords.txt`。
- 存储：Windows **DPAPI** 保护每个密码；`settings.json` / `passwords.json` 固定在 `%LOCALAPPDATA%\NimZip`，不再写入程序目录。
- 快速试密：只解压**最小的加密条目**，不整包试跑；7z 头加密在打开阶段校验。
- 自动顺序：**已验证密码 → 空密码 → 密码本顺序 → 手动输入**。
  - 打开头加密归档失败时会自动按密码本重试，成功后再进入主窗口。
  - 解压时由引擎自动执行上述顺序，全部失败才弹出手动密码框。
  - 手动输入的密码校验通过后自动记入密码本末尾。
- 嵌套包优先复用刚验证通过的密码，失败后继续尝试密码本。

### 嵌套解压

- 解压产物中继续识别压缩包（含无扩展名、伪装文件），默认递归 3 层（可设 1–9）。
- 对每个产物做 64 KiB 快速内容探测与伪装格式识别；单次扫描上限 4096 个产物并明确警告。
- 嵌套包使用父包密码 + 密码本自动试密；失败只跳过该嵌套包并写入任务警告。
- 通过 visited 规范化路径防止递归循环。

### GPUI 桌面界面（按产品原型 v5 调整）

- 主窗口：48px 图标工具栏（左侧主菜单）+ 48px 地址栏（面包屑 / 自适应搜索）、目录树、虚拟化文件列表（可排序多列）、图片预览/元数据、ZIP 注释面板、状态栏。
- 控件高度统一为 36px（按钮 / 搜索框 / 单行输入框 / 下拉框 / 菜单项），工具栏图标默认 18px（设置 → 外观可调 14–24px）、顶部工具栏高度默认 48px（设置 → 外观可调 40–64px）、点击区 36×36；表格行 32px，图标与文字层级同步调整。
- **工具栏编码切换**：设置按钮右侧的“帮助”按钮替换为地球图标“文本编码”；下拉包含 默认 / 日语 / 简体中文 / 繁体中文 / 朝鲜语 / 西欧语言 / 中欧语言 / 西里尔语 / 土耳其语 / 希腊语 / 阿拉伯语 / 希伯来语 / 波罗的语 / 泰语 / UTF-8 / 自动检测。切换后保存设置并重新读取当前压缩包，按新编码解码文件名。工具栏按钮设置中的“帮助”项同步改为“编码”。
- **主菜单**：左侧单一菜单入口，已合并原最右侧“更多”菜单并去除重复项；不常用的测试完整性、ZIP 修复、工具栏按钮、列设置、批量哈希、AMSI 扫描与计算哈希收进“更多工具”悬停子菜单；不再显示“文件 / 编辑 / 窗口”及子菜单标题。再次点击主菜单按钮即可关闭；菜单高度按窗口自适应并内部滚动，底部不会被窗口截断。
- **开始页**：仅保留 Logo、NimZip 标题与“打开压缩包 / 新建压缩包”两个主操作按钮，去除标签、特性胶囊与最近文件列表。
- **归档编辑**：ZIP 添加文件/文件夹（拖放或 Ins）、删除条目（Del，目录递归）、重命名（F2）、编辑注释；操作前确认，加密 ZIP 使用已解析密码重写。
- **11 列文件列表**：名称、大小、压缩后、类型、修改时间、方法、加密、CRC、属性、注释、扩展；表头可排序，支持拖拽调序、列显隐、列宽调整与持久化记忆。列表底部提供独立横向滚动条（左右箭头、点击轨道跳转），列宽超出窗口时可左右滚动。
- **归档页工具栏**：文件区顶部提供“导航 / 预览 / 注释”按钮，以及右侧“列表 | 图标”分段切换；导航、右侧预览、底部注释均可独立开关并持久化。默认仅导航打开，预览与注释关闭；窗口较窄时工具栏自动收缩为图标模式。
- **列表 / 图标视图**：切换状态持久化到 `view.file_list_view`；图标视图以网格显示文件夹/文件图标、名称与大小/类型，支持双击进入/打开、Ctrl 多选和右键菜单。图标视图右侧提供 64–200 px 尺寸滑杆，并支持 Ctrl+滚轮缩放；默认 104 px。
- **包内右键菜单**：在文件列表或图标视图右键文件/文件夹弹出菜单，空白处弹出目录菜单；支持解压全部（选择目录）、解压选定、解压全部到此处、打开、使用 NimZip/关联程序打开、添加文件、新建文件夹、复制到系统剪贴板、从剪贴板粘贴文件（ZIP）、删除（多选/目录递归）、重命名、全部选择、转到上一级、复制完整路径。Ctrl+单击多选，“全部选择”选中当前目录；支持 Ctrl+C / Ctrl+V。
- **主窗口分栏**：目录树、列表、图片预览、注释面板和状态栏；注释面板提供 ZIP 注释编辑入口。
- **窗口状态记忆**：主窗口、新建压缩包 / 文件哈希 / 密码恢复独立窗口、快速解压 / 快速压缩弹窗分别记住上次非最大化尺寸，下次打开自动恢复；最大化或全屏关闭不会覆盖已记住的尺寸。
- **多窗口**：文件菜单 / `Ctrl+Shift+N` 新建独立程序窗口，`Ctrl+Shift+W` 关闭当前窗口；每个窗口独立打开归档、任务和设置视图，共享设置与密码本。
- **资源管理器预览处理器**：`nimzip_shell.dll` 实现 COM `IPreviewHandler`，在资源管理器预览窗格显示归档格式、加密状态和前 5000 个条目；设置 → 右键菜单可一键注册/卸载（HKCU，无需管理员）。
- **Win11 新式右键菜单（源码可选，本轮不打包）**：`nimzip_shell.dll` 实现 `IExplorerCommand`，`packaging/win11/AppxManifest.xml` 保留 `windows.fileExplorerContextMenus` 与 COM SurrogateServer 声明。按用户要求，本轮已取消 MSIX 打包、证书安装与 Explorer 实测；相关脚本仅作为可选后续材料保留。
- **压缩/解压性能**：ZIP 创建使用 libdeflate 并行压缩（按文件分组并行、随机/已压缩数据抽样后直接 Store），ZIP 解压对多文件按块并行；7z 使用 16 线程 LZMA2 分块压缩。208 MB / 102 文件基准：ZIP 创建 2.08 s → 1.01 s，7z 创建 43.4 s → 10.5 s，ZIP 解压 0.64 s → 0.41 s。加密 ZIP、Zipx、SFX 与超 3 GB 归档保留原写入路径。
- **Shell 集成**：设置 → 文件关联覆盖 53 种扩展名——压缩包文件（zip/zipx/rar/7z/alz/egg/cab/bh/001/arj/lha/lzh/pma/ace/arc/aes/zpaq/zstd/br/pea）、UNIX 与流压缩（tar/gz/tgz/bz/bz2/tbz/tbz2/xz/txz/lzma/tlz/lz/uu/uue/xxe/z/lz4/zst/tzst）、软件包（jar/war/apk/ipa/xpi/deb/asar）、光盘镜像（iso/img/isz/udf/wim/bin/i00）；支持分组全选/清空、实际关联状态提示与批量应用；全部写入 HKCU，不覆盖系统 UserChoice。右键菜单由 `nimzip_shell.dll` 的 `IContextMenu` 对单选/多选统一动态生成：开启级联时显示 `NimZip (&N)` 子菜单，关闭时平铺在顶层（无多余分隔线）；压缩包显示解压/删除/智能解压/独立文件夹/其他文件夹，多选压缩包时追加压缩组；普通文件/文件夹显示压缩组，动态名称使用文件或文件夹名，目标已存在时自动追加 ` (2)`，多选合并压缩使用所在文件夹名。单选压缩包会动态显示“解压到 a\”等当前归档名称；多选时改用“智能解压到独立文件夹/解压到独立文件夹”。右键菜单提供常用默认项，仍可在设置中增删。
- **自定义标题栏**：设置 → 外观新增“使用自定义标题栏”，默认开启；主窗口、独立功能窗口、快速弹窗与新建程序窗口统一遵循该设置，关闭后改用系统标题栏（新窗口立即生效，当前主窗口重启后生效）。
- **独立功能窗口**：密码恢复、文件哈希、新建压缩包改为独立系统窗口，各自带自绘标题栏（可最小化/最大化/关闭），完成后关闭窗口即可；新建压缩包完成后可在结果页直接打开压缩包。
- **右键菜单与快捷弹窗**：菜单顺序：解压到此处 → 解压到此处并删除 → 智能解压 → 智能解压并删除 → 解压到当前归档名文件夹 → 并删除 → 解压到其他文件夹 → 管理员解压 → 测试 → 打开 → 预览；压缩组为 ZIP → ZIP 并删除 → TAR → TGZ → 7Z → GZ/XZ/ZST → 逐项压缩 → 新建压缩包 → 管理员压缩。级联菜单标题为“NimZip (&N)”，级联父项和动作项均显示 NimZip 图标；关闭级联后同样按该顺序平铺，不插入分组分隔线。快速弹窗选择路径页为 620×640，直接解压/压缩进度页按内容收紧为 640×405；完成后自动关闭弹窗，失败时保留错误页。
- **新建压缩**：格式下拉覆盖 ZIP / ZIPX / EXE(自解压) / TAR / TGZ / LZH / ISO / 7Z / GZ / BZ2 / XZ / ZPAQ，并补充 TAR.BZ2 / TAR.XZ / TAR.ZST / ZST。ZIPX 使用 Zstd/LZMA/BZip2 等 ZIP 高级方法；LZH 使用 LH5（级别 0 为仅存储）；ISO 使用 Joliet 保留中文/长文件名；ZPAQ 使用 JIDAC 归档。支持 0–9 级别、密码（ZIP/ZIPX AES-256；7Z AES-256 + 文件名加密）、包含根文件夹、压缩后测试、删除源文件、排除过滤器、输出路径、SFX。
- **高级设置**：新建压缩包底部“更多选项”展开右侧高级面板并加宽窗口；按格式联动显示可用的压缩方法（ZIP：Deflate / BZip2 / LZMA / Zstd）、加密算法、CPU 线程策略、ZIP 注释等；TAR/TAR.GZ/GZ 显示对应说明。所有可用项均直接写入创建参数。
- **任务中心**：v5 卡片式运行/完成/失败/取消状态、进度条、试密中状态、取消/输入密码/重试/打开文件夹、全部清除、关闭并返回主界面。
- **设置中心**：9 页导航（常规 / 文件关联 / 右键菜单 / 解压 / 压缩 / 外观 / 安全 / 高级 / 语言）；全部页面可实际修改并持久化，包含列显隐/排序/宽度、工具栏按钮、主题皮肤与语言。
- 密码弹窗、哈希结果卡（复制）、密码管理器（DPAPI 加密、导入导出、顺序调整）、密码恢复向导、浅色/暗色/跟随系统主题。
- **i18n 20 种语言包**：设置 → 语言提供简体中文、English、日本語、한국어、Deutsch、Français、Español、Русский、Português、Italiano、Nederlands、Polski、Türkçe、العربية、हिन्दी、ไทย、Tiếng Việt、Bahasa Indonesia、Українська、Čeština；菜单、设置导航、表头与核心界面即时切换并持久化。中文/英文为人工完整文案，其余语言各覆盖 87 个常用 UI 键（核心导航人工翻译，其余机器翻译），未覆盖动态文案回退英文。
- **主题皮肤**：内置默认蓝 / 翡翠绿 / 紫罗兰 / 琥珀 / 玫瑰 / 石板灰 6 套强调色，浅色与暗色各自适配；设置 → 外观可切换主题皮肤、明暗模式、界面字号与标题栏样式，切换即时生效并持久化。
- `nimzip.exe <archive>` 直接打开归档。

### CLI（`bx.exe`，或 `nimzip.exe --cli`）

`nimzip.exe` 是 Windows GUI 子系统程序，双击启动不会创建控制台窗口。
`dist/bx.exe` 是同引擎的无头 CLI，脚本、重定向和批处理请优先使用它。

为兼容旧用法，`nimzip.exe` 传入 CLI 参数（`--cli`、`bx` 别名或 `nimzip <子命令> ...`）
时会附加到父控制台执行；没有父控制台（例如资源管理器右键命令）时只创建隐藏控制台，
不会显示黑色窗口。但 cmd/PowerShell 不会等待 GUI 子系统进程结束，
因此需要同步退出码或捕获输出时请使用 `bx.exe`。

```text
bx.exe l <archive> [--json] [-p 密码]           列出条目
bx.exe x <archive> [-o 目录] [-p 密码] [--here] [--folder] [--strip-root]
                                                     解压（默认智能；自动试密）
bx.exe a <archive> <files...> [-t zip|7z|tar|tgz|tbz|txz|tzst|gz|bz2|xz|zst|sfx]
            [-p 密码] [-x "*.tmp"] [--no-root] [--test] [--delete-sources]
                                                     创建归档
bx.exe ch <files...> [-f zip|7z|tar|tgz|gz|xz|zst] [--delete-sources]
                                                     在源文件旁创建归档（右键菜单使用）
bx.exe t <archive> [-p 密码]                     校验完整性
bx.exe h <files...> [-a sha256] [--all] [--save file]  计算哈希
bx.exe repair <archive.zip> [-o output]          修复 ZIP
bx.exe recover <archive> [--mask ?l?l?d?d] [--dictionary file] [--charset ...] [--min/--max] [--threads N]
bx.exe d <archive.zip> <paths...>                删除 ZIP 条目
bx.exe rn <archive.zip> <old> <new>              重命名 ZIP 条目
bx.exe c <archive.zip> [comment]                 修改 ZIP 注释
bx.exe shell install|uninstall|status            资源管理器右键菜单

# 显式 CLI / GUI 模式
nimzip.exe --cli l archive.zip
nimzip.exe --gui photos.zip

# GUI 独立窗口 / 快速弹窗入口（资源管理器菜单使用）
nimzip.exe --new <文件或文件夹>              新建压缩包独立窗口
nimzip.exe --new-here <目录>                 以目录作为输出位置新建压缩包
nimzip.exe --quick-extract <archive>         选择解压路径 → 密码 → 冲突 → 进度
nimzip.exe --quick-compress <path> [--format zip|7z]   快速压缩进度窗口
```

`dist/bx.exe` 由 `cargo build --release -p nimzip-cli` 生成，与 `nimzip.exe` 共用同一引擎。

退出码：`0` 成功；`2` 密码错误/缺失；`1` 其他错误。

## 工程结构

```text
Cargo.toml                     workspace
crates/
  nimzip-engine/               纯 Rust 引擎（无 UI 依赖）
    src/archive.rs             清单模型、打开/校验/试密门面
    src/disguise.rs            伪装格式识别/还原与临时副本缓存
    src/extract.rs             智能目标、密码自动解析、嵌套递归
    src/create.rs              归档创建（zip/7z/tar/单流）
    src/zip_backend.rs         ZIP 读写/解密/路径安全
    src/sevenz_backend.rs      7z 读写/加密读取
    src/tar_backend.rs         tar 系与单流
    src/password.rs            密码本 + DPAPI + passwords.txt 导入
    src/hash.rs                流式哈希
    src/sniff.rs               签名/尾部 EOCD 识别
  nimzip-cli/                  bx 命令行
  nimzip-gui/                  nimzip 图形界面（GPUI Kit）
  nimzip-shell/                Windows 资源管理器右键菜单与文件关联
```

## 构建

前置：

- Windows 10/11 x64
- Rust stable（MSVC 工具链）
- Visual Studio Build Tools（MSVC C++ 生成工具 + Windows SDK；ZPAQ 的 libzpaq 通过 cc 编译）
- 首次构建需要联网拉取 crates

```powershell
cargo build --release -p nimzip-gui
```

产物：

- `target/release/nimzip.exe` —— 主程序（GUI，Windows GUI 子系统，启动无控制台）
- `target/release/bx.exe` —— 无头 CLI（脚本/重定向使用）
- `target/release/nimzip_shell.dll` —— 可选资源管理器预览处理器（需要预览功能时与 exe 同目录）

把 `nimzip.exe`、`bx.exe` 放到任意目录即可免安装运行：图标与前端资源已嵌入二进制；设置、密码本等用户数据按 Windows 规范固定写入 `%LOCALAPPDATA%\NimZip`，不依赖程序目录写权限。

一键生成免安装包到 `dist\nimzip.exe`：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-portable.ps1
```

开发构建与测试：

```powershell
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

当前测试覆盖：26 项引擎集成测试（格式往返、RAR、分卷、加密试密、密码恢复、ZIP 编辑/修复、SFX、伪装还原、自动密码本、嵌套解压、zip-slip、哈希/校验文件比对、7z AES-256 写入）+ 2 项 CLI 端到端 + 8 项 shell 单元/COM + 3 项 GUI i18n 单元测试。

端到端回归脚本：`bash scripts/regression.sh`（ZIP/7Z/TAR.GZ/RAR、分卷、修复、密码恢复、ZIP 编辑、SFX、哈希、右键菜单注册）。

## 设计说明

- **引擎/UI 完全解耦**：GUI、CLI 都只通过 `nimzip-engine` 的公开 API 工作；进度/取消通过 `ProgressSink`。
- **密码交互不在阻塞回调里**：引擎自动试密到达 `PasswordRequired` 后返回；GUI 弹窗后以新密码重试，CLI 用终端输入。
- **伪装不修改源文件**：伪装还原只写 `%TEMP%\NimZip\revealed`，按源路径 + 大小 + 修改时间缓存，避免逐条试密时反复复制大文件。
- **默认安全**：路径逃逸拒绝、符号链接跳过、MOTW 传播、删除走回收站、内存上限与扫描上限。
- **界面主题**：自绘 `Palette` 同步到 GPUI Kit 的语义主题，内置控件（Input/Button/Switch/Checkbox）与自绘界面保持一致。

## 已知限制 / 后续路线

- 7z 写入采用 LZMA2 + 可选 AES-256；未暴露固实块/字典/线程数等细粒度参数。
- ZIP/XZ/GZ/BZ2/ZST/TAR 及 ZIPX、LZH、ISO、ZPAQ 为内置读写实现；ZPAQ 通过 libzpaq（zpaq_rs）JIDAC 引擎，ISO 通过 libmkiso 原生写入/读取，LZH 通过 oxiarc-archive 原生读写。WIM/CAB/ARJ/ASAR 通过外部 7-Zip / NanaZip 后端读取。已实测 CAB 列表/解压。
- 工具栏按钮支持显隐定制；列支持显隐、拖拽调序、宽度调整与持久化记忆。
- 20 种语言包已内置；中文/英文为人工完整文案，其余 18 种各覆盖 87 个常用 UI 键（核心导航人工翻译，其余机器翻译），尚未经母语审校。6 套主题皮肤、多窗口与预览处理器已交付。
- 预览处理器需要 `nimzip_shell.dll` 与 `nimzip.exe` 同目录；核心 GUI/CLI 仍是单个 `nimzip.exe`。
- 按用户要求，本轮已取消 Win11 新式菜单的 MSIX 打包、安装与 Explorer 实测；`packaging/win11` 与相关脚本仅保留为可选后续材料，不随本轮发布构建。
- 若后续恢复 MSIX 分发，自签名证书需替换为企业/受信任代码签名证书，并以管理员权限安装。
- 嵌套包如果密码不在密码本中，会跳过并在任务中提示；不会为每个嵌套包单独弹窗。
- AMSI 依赖系统 Provider；Provider 不可用时只提示，不阻塞解压。

## 环境提示：Smart App Control / WDAC

在启用 **Smart App Control** 或企业 WDAC 且要求签名应用的机器上，新编译的未签名 `nimzip.exe`/测试二进制可能被系统拒绝执行（错误 4551）。这是操作系统应用控制策略，不是程序错误。开发时可以选择：

- 为构建产物配置受信任的代码签名证书；
- 由管理员调整 WDAC/SAC 策略或加入允许列表；
- 在隔离的开发机/虚拟机中关闭 Smart App Control 后再运行。
