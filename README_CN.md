# RarkZip

现代、快速的 Windows 压缩与归档管理工具，核心引擎使用纯 Rust，桌面界面基于 GPUI。

RarkZip 提供归档浏览与编辑、智能解压、嵌套解压、密码管理、文件哈希和 Windows 资源管理器集成。

[English](README.md)

## 主要功能

- **归档浏览与编辑** — 打开、浏览、解压、测试和编辑 ZIP；支持列表/图标视图、目录树、图片预览和注释。
- **智能解压** — 自动选择目标目录，处理同名文件冲突，支持选择性解压。
- **嵌套解压** — 自动识别解压产物中的压缩包并递归展开，优先复用上层已验证的密码。
- **密码体系** — 密码本使用 Windows DPAPI 保护，支持快速试密、自动重试和密码恢复。
- **文件哈希** — 支持 MD5、SHA-1、SHA-256、SHA-512、SHA3-256、BLAKE3、xxHash64、CRC32；支持批量计算、校验、复制和校验和导出。
- **安全防护** — 阻止路径穿越，过滤不安全链接，传播 Mark-of-the-Web，删除走回收站，可选 AMSI 扫描。
- **资源管理器集成** — 文件关联、单文件/多文件右键菜单、可选预览处理器。
- **原生桌面界面** — 浅色/深色主题、强调色、多窗口、20 种界面语言和高 DPI 支持。
- **无头 CLI** — 通过 `bx.exe` 使用同一引擎，便于脚本和自动化。

## 支持格式

| 操作 | 格式 |
| --- | --- |
| 浏览 / 解压 | ZIP、ZIPX、7z、TAR、TAR.GZ、TAR.BZ2、TAR.XZ、TAR.ZST、GZ、BZ2、XZ、ZST、RAR（只读）、ISO、LZH、ZPAQ、WIM、CAB、ARJ、ASAR |
| 创建 | ZIP、7z、TAR、TAR.GZ、TAR.BZ2、TAR.XZ、TAR.ZST、GZ、BZ2、XZ、ZST、SFX ZIP |
| 分卷 | `.zip.001`、`.7z.001`、`.part1.rar`、`.r00` 等 |
| 加密 | ZIP AES-256 与兼容的 ZipCrypto，7z AES-256（可选头加密） |

> WIM、CAB、ARJ、ASAR 格式需要系统安装 7-Zip 或 NanaZip。

## 环境要求

- Windows 10 或 Windows 11（x64）
- Rust stable（MSVC 工具链）
- Visual Studio Build Tools（MSVC C++ 生成工具与 Windows SDK）
- 首次构建需要联网下载依赖

## 构建

```powershell
git clone <repository-url>
cd RarkZip
cargo build --release --workspace
```

构建产物：

- `target\release\rarkzip.exe` — 桌面主程序
- `target\release\bx.exe` — 无头命令行工具
- `target\release\rarkzip_shell.dll` — 可选的资源管理器预览/右键菜单扩展

使用资源管理器集成时，请将 `rarkzip_shell.dll` 与 `rarkzip.exe` 放在同一目录。

## 使用

启动桌面程序：

```powershell
target\release\rarkzip.exe
```

常用 CLI 命令：

```powershell
bx.exe l archive.zip
bx.exe x archive.zip -o output
bx.exe a archive.zip file1 folder2
bx.exe t archive.zip
bx.exe h file1 file2
bx.exe repair archive.zip
bx.exe recover archive.zip --mask ?l?l?d?d
```

可在 **设置 → 文件关联 / 右键菜单** 中启用或更新资源管理器集成，也可以运行：

```powershell
rarkzip.exe --cli shell install
```

## 工程结构

```text
crates/
  rarkzip-engine/   归档引擎、解压、加密、哈希与密码恢复
  rarkzip-gui/      GPUI 桌面界面
  rarkzip-cli/      bx 命令行工具
  rarkzip-shell/    Windows 资源管理器右键菜单与预览处理器
```

## 测试

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## 许可证

MIT，详见 [LICENSE](LICENSE)。
