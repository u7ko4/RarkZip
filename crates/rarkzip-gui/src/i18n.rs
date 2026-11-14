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
        "文件与文本加密" => "File & Text Encryption",
        "文件与文本加密…" => "File & Text Encryption…",
        "密码" => "Password",
        "确认密码" => "Confirm password",
        "算法" => "Algorithm",
        "显示密码" => "Show password",
        "本地加密，不联网" => "Local encryption, no network",
        "文本" => "Text",
        "隐写" => "Steganography",
        "输入或粘贴要加密的文本" => "Enter or paste text to encrypt",
        "输入或粘贴要解密的文本" => "Enter or paste text to decrypt",
        "就绪" => "Ready",
        "准备" => "Preparing",
        "正在压缩" => "Compressing",
        "校验" => "Verify",
        "正在解压" => "Extracting",
        "处理中" => "Processing",
        "处理中…" => "Processing…",
        "处理完成" => "Completed",
        "请输入密码" => "Enter a password",
        "两次输入的密码不一致" => "Passwords do not match",
        "开始处理文件" => "Starting files",
        "没有要加密的文本" => "No text to encrypt",
        "没有要解密的文本" => "No text to decrypt",
        "还没有待处理的文件" => "No files queued",
        "等待" => "Waiting",
        "失败" => "Failed",
        "移除" => "Remove",
        "自动" => "Auto",
        "始终" => "Always",
        "不压缩" => "Store",
        "与源文件相同" => "Same as source",
        "拖放文件或文件夹到这里" => "Drop files or folders here",
        "普通文件将加密为 .enc；兼容的 .enc 文件将解密" => {
            "Regular files are encrypted to .enc; compatible .enc files are decrypted"
        }
        "选择文件" => "Choose Files",
        "选择文件夹" => "Choose Folder",
        "待处理文件" => "Files",
        "清空" => "Clear",
        "压缩模式" => "Compression",
        "输出目录" => "Output folder",
        "选择目录…" => "Choose Folder…",
        "恢复默认" => "Use default",
        "开始处理" => "Start",
        "停止" => "Stop",
        "输出编码" => "Output encoding",
        "LZMA 压缩" => "LZMA compression",
        "每 N 个字符加空格" => "Space every N characters",
        "关闭" => "Off",
        "解密时自动识别编码、压缩与算法" => {
            "Encoding, compression and algorithm are detected automatically when decrypting"
        }
        "明文" => "Plaintext",
        "密文" => "Ciphertext",
        "复制" => "Copy",
        "已复制" => "Copied",
        "纯数字" => "Numbers",
        "选择或拖入载体图片" => "Choose or drop a carrier image",
        "支持 JPG / PNG / BMP；输出为 JPEG 隐写图" => {
            "Supports JPG / PNG / BMP; the output is a JPEG steganogram"
        }
        "载体图片" => "Carrier image",
        "选择图片…" => "Choose Image…",
        "隐写参数" => "Steganography settings",
        "JPEG 质量" => "JPEG quality",
        "最大边长" => "Max side",
        "原图" => "Original",
        "输出文件" => "Output file",
        "选择载体后自动生成" => "Generated after choosing a carrier",
        "要隐藏的文本" => "Text to hide",
        "与文本页共享" => "Shared with the Text tab",
        "制作隐写图" => "Create Steganogram",
        "从隐写图读取" => "Read from Steganogram",
        "隐写图已生成" => "Steganogram created",
        "已从隐写图读取文本" => "Text extracted from steganogram",
        "GnuPG 签名工具" => "GnuPG Signature Tool",
        "GnuPG 签名工具…" => "GnuPG Signature Tool…",
        "公钥" => "Public keys",
        "公钥已吊销" => "Key revoked",
        "公钥已过期" => "Key expired",
        "公钥无效" => "Invalid key",
        "公钥未通过自签名校验。" => "The public key failed its self-signature check.",
        "分离签名" => "Detached signature",
        "分离签名需要原始文件；明文签名可留空" => {
            "Detached signatures need the original file; clearsigned messages do not"
        }
        "分离签名需要选择被签名的原始文件" => {
            "Choose the signed file for this detached signature"
        }
        "密钥 ID" => "Key ID",
        "密钥创建" => "Key created",
        "密钥过期" => "Key expires",
        "导入公钥…" => "Import Key…",
        "导入签名者的公钥（.asc / .gpg）后即可离线校验。" => {
            "Import the signer's public key (.asc / .gpg) to verify offline."
        }
        "尚未导入公钥" => "No public keys imported",
        "尚未校验" => "Not verified yet",
        "已导入公钥" => "Public key imported",
        "已移除公钥" => "Public key removed",
        "打开目录" => "Open Folder",
        "指纹" => "Fingerprint",
        "支持 .sig / .asc / .gpg 与明文签名" => {
            "Supports .sig / .asc / .gpg and clearsigned messages"
        }
        "无法完成校验。" => "Verification could not be completed.",
        "明文签名" => "Clearsigned",
        "更换…" => "Change…",
        "未知签名者" => "Unknown signer",
        "本地公钥环" => "Local keyring",
        "本地公钥环中没有签发此签名的公钥，请先导入。" => {
            "The signing key is not in the local keyring. Import it first."
        }
        "校验失败" => "Verification failed",
        "校验完成" => "Verification complete",
        "校验签名" => "Verify Signature",
        "校验结果" => "Verification Result",
        "正在校验…" => "Verifying…",
        "清除" => "Clear",
        "移除公钥" => "Remove key",
        "签名与文件内容不匹配，文件可能已被修改。" => {
            "The signature does not match the file; the file may have been modified."
        }
        "签名与文件内容匹配，且公钥位于本地公钥环中。" => {
            "The signature matches the file, and the key is in the local keyring."
        }
        "签名与文本在同一文件中，无需原始文件" => {
            "The signature and text are in one file; no original file is needed"
        }
        "签名密钥已被吊销，不应信任。" => {
            "The signing key has been revoked and should not be trusted."
        }
        "签名已超过有效期。" => "The signature has expired.",
        "签名已过期" => "Signature expired",
        "签名文件" => "Signature file",
        "签名无效" => "Invalid signature",
        "签名时间" => "Signed at",
        "签名有效" => "Valid signature",
        "签名本身有效，但签名密钥已过期。" => {
            "The signature is valid, but the signing key has expired."
        }
        "签名者" => "Signer",
        "缺少公钥" => "Public key missing",
        "至" => "until",
        "被签名文件" => "Signed file",
        "详情" => "Details",
        "请选择签名文件" => "Choose a signature file",
        "请选择签名文件和原始文件" => "Choose both the signature and the original file",
        "选择或拖入原始文件" => "Choose or drop the original file",
        "选择或拖入签名文件" => "Choose or drop a signature file",
        "选择文件…" => "Choose File…",
        "选择签名…" => "Choose Signature…",
        "选择签名文件和被签名文件后开始校验" => {
            "Choose a signature and a signed file to start"
        }
        "签名" => "Sign",
        "私钥" => "Private keys",
        "签名私钥" => "Signing key",
        "导入私钥…" => "Import Key…",
        "尚未导入私钥" => "No private keys imported",
        "仅支持带口令的 OpenPGP 私钥。" => "Only passphrase-protected OpenPGP private keys are supported.",
        "选择或拖入要签名的文件" => "Choose or drop a file to sign",
        "支持任意文件类型" => "Supports any file type",
        "要签名的文件" => "File to sign",
        "签名选项" => "Signing options",
        "未选择私钥" => "No signing key selected",
        "选择文件后自动生成" => "Generated after choosing a file",
        "另存为…" => "Save As…",
        "ASCII armor (.asc)" => "ASCII armor (.asc)",
        "私钥口令" => "Private key passphrase",
        "显示口令" => "Show passphrase",
        "正在签名…" => "Signing…",
        "签名完成" => "Signing complete",
        "签名失败" => "Signing failed",
        "生成签名" => "Create Signature",
        "请选择文件、私钥和输出位置" => "Choose a file, signing key and output location",
        "请输入私钥口令" => "Enter the private key passphrase",
        "已吊销" => "Revoked",
        "移除私钥" => "Remove key",
        "签名结果" => "Signature Result",
        "ASCII armor" => "ASCII armor",
        "二进制签名" => "Binary signature",
        "尚未生成签名" => "No signature created yet",
        "选择文件、私钥并输入口令后生成签名" => {
            "Choose a file and signing key, then enter the passphrase to create a signature"
        }
        "签名已生成" => "Signature created",
        "签名文件已写入磁盘。" => "The signature file has been written to disk.",
        "格式" => "Format",
        "请选择要签名的文件" => "Choose a file to sign",
        "请选择签名输出文件" => "Choose where to save the signature",
        "请选择签名私钥" => "Choose a signing key",
        "已导入私钥" => "Private key imported",
        "已移除私钥" => "Private key removed",
        "原文件不会被修改，签名会写入单独的 .sig / .asc 文件。" => {
            "The original file is not modified; the signature is written to a separate .sig / .asc file."
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
