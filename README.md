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

### Linux

The archive engine, the `bx` CLI, and the GPUI desktop UI also build and run on Linux (X11 or Wayland). Explorer integration, DPAPI password protection, AMSI scanning, and Mark-of-the-Web propagation are Windows-only; on other platforms they are compiled out or degrade gracefully.

Build dependencies on Debian/Ubuntu:

```bash
sudo apt install build-essential pkg-config libxkbcommon-dev libwayland-dev \
  libx11-xcb-dev libfontconfig1-dev libvulkan-dev libegl1-mesa-dev
```

Then run the same `cargo build --release --workspace` command. The outputs are `target/release/rarkzip` and `target/release/bx` (without `.exe`); `rarkzip_shell` has no effect outside Windows.

Platform notes:

- The password book falls back to a `0600` JSON file when DPAPI is unavailable. It is effectively plaintext at rest, so protect it with filesystem permissions.
- WIM, CAB, ARJ, and ASAR extraction needs `7z`, `7zz`, or `7za` on `PATH`, or set `RARKZIP_7Z` to the executable.
- SFX ZIP creation is Windows-oriented; use regular ZIP, 7z, or TAR archives on other platforms.
- This checkout's `.cargo/config.toml` selects the rsproxy.cn sparse registry mirror. Adjust or remove it if crates.io is reachable from your network.

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
