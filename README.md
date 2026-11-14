# RarkZip

A fast, modern archive manager for Windows, powered by a native Rust engine and a GPUI desktop interface.

RarkZip combines everyday archive browsing and editing with smart extraction, nested archive handling, password management, hashing, and Windows Explorer integration.

[![Windows Build](https://github.com/u7ko4/RarkZip/actions/workflows/windows-build.yml/badge.svg)](https://github.com/u7ko4/RarkZip/actions/workflows/windows-build.yml)

[中文文档](README_CN.md)

## Features

- **Archive browsing and editing** — open, browse, extract, test, and edit ZIP archives with list and icon views, folder tree, image preview, and comments.
- **Smart extraction** — automatically chooses the right destination, handles name collisions, and supports selective extraction.
- **Nested archives** — detects archives inside extracted files and expands them recursively, reusing an already verified password across layers.
- **Password support** — password book protected with Windows DPAPI, quick password verification, automatic retry, and password recovery.
- **Hash workspace** — MD5, SHA-1, SHA-256, SHA-512, SHA3-256, BLAKE3, xxHash64, and CRC32; batch hashing, verification, clipboard actions, and checksum export.
- **Encryption workspace** — an independent window for file/folder encryption, text encryption, and JPEG steganography, with nine cipher choices, Argon2id key derivation, and compatible legacy containers.
- **Signature workspace** — GnuPG-compatible signature creation and verification with local public and private keyrings; supports detached and clearsigned files.
- **Safety first** — path traversal protection, unsafe link filtering, Mark-of-the-Web propagation, recycle-bin deletion, and optional AMSI scanning.
- **Explorer integration** — file associations, single/multi-selection context menus, and an optional preview handler.
- **Native desktop UI** — light/dark themes, accent colors, multiple windows, 20 interface languages, and high-DPI support.
- **Headless CLI** — the same engine is available through `bx.exe` for scripts and automation.

## Supported Formats

| Operation | Formats |
| --- | --- |
| Browse / extract | ZIP, ZIPX, 7z, TAR, TAR.GZ, TAR.BZ2, TAR.XZ, TAR.ZST, GZ, BZ2, XZ, ZST, RAR (read-only), ISO, LZH, ZPAQ, WIM, CAB, ARJ, ASAR |
| Create | ZIP, 7z, TAR, TAR.GZ, TAR.BZ2, TAR.XZ, TAR.ZST, GZ, BZ2, XZ, ZST, SFX ZIP |
| Split / multi-volume | `.zip.001`, `.7z.001`, `.part1.rar`, `.r00`, and related formats |
| Encryption | ZIP AES-256 and compatible ZipCrypto, 7z AES-256 with optional header encryption |
| File/text encryption | `.enc` v4 (reads v2/v3), text v3 (reads v2), JPEG F5 steganography |

> WIM, CAB, ARJ, and ASAR support requires 7-Zip or NanaZip to be installed.

## Requirements

- Windows 10 or Windows 11 (x64)
- Rust stable with the MSVC toolchain
- Visual Studio Build Tools (MSVC C++ tools and Windows SDK)
- Internet access for the first dependency download

## Build

```powershell
git clone <repository-url>
cd RarkZip
cargo build --release --workspace
```

Build outputs:

- `target\release\rarkzip.exe` — desktop application
- `target\release\bx.exe` — headless command-line interface
- `target\release\rarkzip_shell.dll` — optional Explorer preview/context-menu extension

To use Explorer integration, keep `rarkzip_shell.dll` in the same directory as `rarkzip.exe`.

## Usage

Launch the desktop application:

```powershell
target\release\rarkzip.exe
```

Common CLI commands:

```powershell
bx.exe l archive.zip
bx.exe x archive.zip -o output
bx.exe a archive.zip file1 folder2
bx.exe t archive.zip
bx.exe h file1 file2
bx.exe repair archive.zip
bx.exe recover archive.zip --mask ?l?l?d?d
```

Enable or update Explorer integration from **Settings → File Associations / Context Menu**, or run:

```powershell
rarkzip.exe --cli shell install
```

## Project Structure

```text
crates/
  rarkzip-engine/   Archive engine, extraction, encryption, hashing, recovery
  rarkzip-gui/      GPUI desktop application
  rarkzip-cli/      bx command-line interface
  rarkzip-shell/    Windows Explorer context menu and preview handler
```

## Testing

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

MIT. See [LICENSE](LICENSE).
