use std::path::Path;

use serde::{Deserialize, Serialize};

/// 支持的归档格式。`Unknown` 用于无法识别的内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArchiveFormat {
    Zip,
    Zipx,
    SevenZ,
    Rar,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    TarZst,
    Gz,
    Bz2,
    Xz,
    Zst,
    Iso,
    Wim,
    Cab,
    Lzh,
    Arj,
    Asar,
    Zpaq,
    Unknown,
}

impl ArchiveFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Zip => "ZIP",
            Self::Zipx => "ZIPX",
            Self::SevenZ => "7Z",
            Self::Rar => "RAR",
            Self::Tar => "TAR",
            Self::TarGz => "TAR.GZ",
            Self::TarBz2 => "TAR.BZ2",
            Self::TarXz => "TAR.XZ",
            Self::TarZst => "TAR.ZST",
            Self::Gz => "GZ",
            Self::Bz2 => "BZ2",
            Self::Xz => "XZ",
            Self::Zst => "ZST",
            Self::Iso => "ISO",
            Self::Wim => "WIM",
            Self::Cab => "CAB",
            Self::Lzh => "LZH",
            Self::Arj => "ARJ",
            Self::Asar => "ASAR",
            Self::Zpaq => "ZPAQ",
            Self::Unknown => "未知",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Zipx => "zipx",
            Self::SevenZ => "7z",
            Self::Rar => "rar",
            Self::Tar => "tar",
            Self::TarGz => "tar.gz",
            Self::TarBz2 => "tar.bz2",
            Self::TarXz => "tar.xz",
            Self::TarZst => "tar.zst",
            Self::Gz => "gz",
            Self::Bz2 => "bz2",
            Self::Xz => "xz",
            Self::Zst => "zst",
            Self::Iso => "iso",
            Self::Wim => "wim",
            Self::Cab => "cab",
            Self::Lzh => "lzh",
            Self::Arj => "arj",
            Self::Asar => "asar",
            Self::Zpaq => "zpaq",
            Self::Unknown => "",
        }
    }

    pub fn is_zip(self) -> bool {
        matches!(self, Self::Zip | Self::Zipx)
    }

    pub fn is_seven_z(self) -> bool {
        self == Self::SevenZ
    }

    pub fn is_tar_family(self) -> bool {
        matches!(
            self,
            Self::Tar | Self::TarGz | Self::TarBz2 | Self::TarXz | Self::TarZst
        )
    }

    pub fn is_single_stream(self) -> bool {
        matches!(self, Self::Gz | Self::Bz2 | Self::Xz | Self::Zst)
    }

    /// 是否能在不实际解压条目的情况下快速校验密码。
    ///
    /// 目前只有 ZIP/ZIPX 能可靠地只读取少量数据验证密码；RAR、7z、外部 7-Zip
    /// 后端等需要整条条目或整个归档参与校验，不应在打开阶段自动逐个试密。
    pub fn supports_quick_password_test(self) -> bool {
        matches!(self, Self::Zip | Self::Zipx)
    }

    pub fn can_create(self) -> bool {
        matches!(
            self,
            Self::Zip
                | Self::Zipx
                | Self::SevenZ
                | Self::Tar
                | Self::TarGz
                | Self::TarBz2
                | Self::TarXz
                | Self::TarZst
                | Self::Gz
                | Self::Bz2
                | Self::Xz
                | Self::Zst
                | Self::Iso
                | Self::Lzh
                | Self::Zpaq
        )
    }

    /// 按扩展名猜测格式；只作为 UI 默认值，打开归档仍以内容识别为准。
    pub fn from_extension(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
        let known: &[(&str, Self)] = &[
            (".tar.gz", Self::TarGz),
            (".tgz", Self::TarGz),
            (".tar.bz2", Self::TarBz2),
            (".tbz2", Self::TarBz2),
            (".tbz", Self::TarBz2),
            (".tar.xz", Self::TarXz),
            (".txz", Self::TarXz),
            (".tar.zst", Self::TarZst),
            (".tzst", Self::TarZst),
            (".tar", Self::Tar),
            (".zip", Self::Zip),
            (".zipx", Self::Zipx),
            (".7z", Self::SevenZ),
            (".rar", Self::Rar),
            (".gz", Self::Gz),
            (".bz2", Self::Bz2),
            (".xz", Self::Xz),
            (".zst", Self::Zst),
            (".iso", Self::Iso),
            (".wim", Self::Wim),
            (".swm", Self::Wim),
            (".cab", Self::Cab),
            (".lzh", Self::Lzh),
            (".lha", Self::Lzh),
            (".arj", Self::Arj),
            (".asar", Self::Asar),
            (".zpaq", Self::Zpaq),
            (".apk", Self::Zip),
            (".jar", Self::Zip),
            (".docx", Self::Zip),
            (".xlsx", Self::Zip),
            (".pptx", Self::Zip),
        ];
        known
            .iter()
            .find_map(|(suffix, format)| name.ends_with(suffix).then_some(*format))
    }

    /// 解析 CLI / 设置中的格式名。
    pub fn from_name(name: &str) -> Option<Self> {
        let value = name.trim().to_ascii_lowercase();
        Some(match value.as_str() {
            "zip" => Self::Zip,
            "zipx" => Self::Zipx,
            "7z" | "sevenz" | "seven-z" => Self::SevenZ,
            "rar" => Self::Rar,
            "tar" => Self::Tar,
            "tgz" | "tar.gz" | "targz" => Self::TarGz,
            "tbz" | "tbz2" | "tar.bz2" | "tarbz2" => Self::TarBz2,
            "txz" | "tar.xz" | "tarxz" => Self::TarXz,
            "tzst" | "tar.zst" | "tarzst" => Self::TarZst,
            "gz" | "gzip" => Self::Gz,
            "bz2" | "bzip2" => Self::Bz2,
            "xz" => Self::Xz,
            "zst" | "zstd" => Self::Zst,
            "iso" => Self::Iso,
            "wim" => Self::Wim,
            "cab" => Self::Cab,
            "lzh" | "lha" => Self::Lzh,
            "arj" => Self::Arj,
            "asar" => Self::Asar,
            "zpaq" => Self::Zpaq,
            _ => return None,
        })
    }
}

impl std::fmt::Display for ArchiveFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// 解压时用于解码 zip 非 UTF-8 文件名的代码页。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Codepage {
    /// 系统默认 ANSI 代码页（Windows 上由 GetACP 决定）。
    Default,
    #[default]
    Auto,
    Utf8,
    Gbk,
    Big5,
    Korean,
    ShiftJis,
    Windows1252,
    Windows1250,
    Windows1251,
    Windows1254,
    Windows1253,
    Windows1256,
    Windows1255,
    Windows1257,
    Windows874,
    Cp437,
}

impl Codepage {
    /// 工具栏/设置下拉中的显示顺序，与原型一致。
    pub const ALL: [Codepage; 16] = [
        Self::Default,
        Self::ShiftJis,
        Self::Gbk,
        Self::Big5,
        Self::Korean,
        Self::Windows1252,
        Self::Windows1250,
        Self::Windows1251,
        Self::Windows1254,
        Self::Windows1253,
        Self::Windows1256,
        Self::Windows1255,
        Self::Windows1257,
        Self::Windows874,
        Self::Utf8,
        Self::Auto,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "默认",
            Self::Auto => "自动检测",
            Self::Utf8 => "UTF-8",
            Self::Gbk => "简体中文",
            Self::Big5 => "繁体中文",
            Self::Korean => "朝鲜语",
            Self::ShiftJis => "日语",
            Self::Windows1252 => "西欧语言",
            Self::Windows1250 => "中欧语言",
            Self::Windows1251 => "西里尔语",
            Self::Windows1254 => "土耳其语",
            Self::Windows1253 => "希腊语",
            Self::Windows1256 => "阿拉伯语",
            Self::Windows1255 => "希伯来语",
            Self::Windows1257 => "波罗的语",
            Self::Windows874 => "泰语",
            Self::Cp437 => "DOS (CP437)",
        }
    }

    pub fn from_name(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "default" | "system" => Self::Default,
            "utf-8" | "utf8" => Self::Utf8,
            "gbk" | "gb18030" | "936" => Self::Gbk,
            "big5" | "950" => Self::Big5,
            "euc-kr" | "euckr" | "949" => Self::Korean,
            "shift-jis" | "shift_jis" | "sjis" | "932" => Self::ShiftJis,
            "windows-1252" | "1252" | "cp1252" => Self::Windows1252,
            "windows-1250" | "1250" | "cp1250" => Self::Windows1250,
            "windows-1251" | "1251" | "cp1251" => Self::Windows1251,
            "windows-1254" | "1254" | "cp1254" => Self::Windows1254,
            "windows-1253" | "1253" | "cp1253" => Self::Windows1253,
            "windows-1256" | "1256" | "cp1256" => Self::Windows1256,
            "windows-1255" | "1255" | "cp1255" => Self::Windows1255,
            "windows-1257" | "1257" | "cp1257" => Self::Windows1257,
            "windows-874" | "874" | "cp874" => Self::Windows874,
            "cp437" | "437" => Self::Cp437,
            _ => Self::Auto,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Auto => "auto",
            Self::Utf8 => "utf-8",
            Self::Gbk => "gbk",
            Self::Big5 => "big5",
            Self::Korean => "euc-kr",
            Self::ShiftJis => "shift-jis",
            Self::Windows1252 => "windows-1252",
            Self::Windows1250 => "windows-1250",
            Self::Windows1251 => "windows-1251",
            Self::Windows1254 => "windows-1254",
            Self::Windows1253 => "windows-1253",
            Self::Windows1256 => "windows-1256",
            Self::Windows1255 => "windows-1255",
            Self::Windows1257 => "windows-1257",
            Self::Windows874 => "windows-874",
            Self::Cp437 => "cp437",
        }
    }

    /// 把 Windows ANSI 代码页映射为具体编码。
    pub fn from_acp(acp: u32) -> Self {
        match acp {
            932 => Self::ShiftJis,
            936 => Self::Gbk,
            949 => Self::Korean,
            950 => Self::Big5,
            1250 => Self::Windows1250,
            1251 => Self::Windows1251,
            1252 => Self::Windows1252,
            1253 => Self::Windows1253,
            1254 => Self::Windows1254,
            1255 => Self::Windows1255,
            1256 => Self::Windows1256,
            1257 => Self::Windows1257,
            874 => Self::Windows874,
            65001 => Self::Utf8,
            _ => Self::Windows1252,
        }
    }

    /// 当前系统默认 ANSI 代码页。
    pub fn system() -> Self {
        #[cfg(windows)]
        {
            #[link(name = "kernel32")]
            extern "system" {
                fn GetACP() -> u32;
            }
            Self::from_acp(unsafe { GetACP() })
        }
        #[cfg(not(windows))]
        {
            Self::Utf8
        }
    }
}

#[cfg(test)]
mod codepage_tests {
    use super::Codepage;

    #[test]
    fn codepage_names_round_trip() {
        for codepage in Codepage::ALL {
            assert_eq!(
                Codepage::from_name(codepage.name()),
                codepage,
                "{}",
                codepage.name()
            );
            assert!(!codepage.label().is_empty());
        }
        assert_eq!(Codepage::ALL.len(), 16);
    }

    #[test]
    fn acp_mapping_covers_common_windows_codepages() {
        assert_eq!(Codepage::from_acp(936), Codepage::Gbk);
        assert_eq!(Codepage::from_acp(932), Codepage::ShiftJis);
        assert_eq!(Codepage::from_acp(950), Codepage::Big5);
        assert_eq!(Codepage::from_acp(949), Codepage::Korean);
        assert_eq!(Codepage::from_acp(1251), Codepage::Windows1251);
        assert_eq!(Codepage::from_acp(65001), Codepage::Utf8);
    }
}
