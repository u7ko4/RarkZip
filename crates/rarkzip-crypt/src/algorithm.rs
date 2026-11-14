/// Block cipher algorithms used by the compatible file and text formats.
///
/// The numeric codes are the on-disk identifiers. Code 3 is a legacy
/// Blowfish-256 variant that is accepted for decryption but intentionally not
/// offered by the modern UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Algorithm {
    Aes256 = 0,
    Rc6_256 = 1,
    Serpent256 = 2,
    Blowfish256 = 3,
    Twofish256 = 4,
    Gost28147 = 5,
    Blowfish448 = 6,
    Threefish1024 = 7,
    Shacal2_512 = 8,
    Composite2048 = 9,
}

/// One layer of the composite 2048-bit algorithm.
#[derive(Debug, Clone, Copy)]
pub struct Layer {
    pub algorithm: Algorithm,
    pub key_bytes: usize,
    pub nonce_bytes: usize,
}

pub const COMPOSITE_LAYERS: [Layer; 4] = [
    Layer {
        algorithm: Algorithm::Threefish1024,
        key_bytes: 128,
        nonce_bytes: 128,
    },
    Layer {
        algorithm: Algorithm::Serpent256,
        key_bytes: 32,
        nonce_bytes: 16,
    },
    Layer {
        algorithm: Algorithm::Aes256,
        key_bytes: 32,
        nonce_bytes: 16,
    },
    Layer {
        algorithm: Algorithm::Shacal2_512,
        key_bytes: 64,
        nonce_bytes: 32,
    },
];

impl Algorithm {
    pub const ALL: [Self; 10] = [
        Self::Aes256,
        Self::Rc6_256,
        Self::Serpent256,
        Self::Blowfish256,
        Self::Twofish256,
        Self::Gost28147,
        Self::Blowfish448,
        Self::Threefish1024,
        Self::Shacal2_512,
        Self::Composite2048,
    ];

    /// Algorithms exposed in the modern UI, in display order.
    pub const UI: [Self; 9] = [
        Self::Aes256,
        Self::Rc6_256,
        Self::Serpent256,
        Self::Twofish256,
        Self::Gost28147,
        Self::Blowfish448,
        Self::Threefish1024,
        Self::Shacal2_512,
        Self::Composite2048,
    ];

    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|algorithm| *algorithm as u8 == code)
    }

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn key_bytes(self) -> usize {
        match self {
            Self::Aes256
            | Self::Rc6_256
            | Self::Serpent256
            | Self::Blowfish256
            | Self::Twofish256
            | Self::Gost28147 => 32,
            Self::Blowfish448 => 56,
            Self::Threefish1024 => 128,
            Self::Shacal2_512 => 64,
            Self::Composite2048 => 256,
        }
    }

    /// Nonce size used by the file/text formats.
    pub fn nonce_bytes(self) -> usize {
        match self {
            Self::Composite2048 => 192,
            _ => self.block_bytes(),
        }
    }

    /// Cipher block size in bytes. For the composite algorithm this is the
    /// total nonce size (192), not a single cipher block.
    pub fn block_bytes(self) -> usize {
        match self {
            Self::Aes256 | Self::Rc6_256 | Self::Serpent256 | Self::Twofish256 => 16,
            Self::Blowfish256 | Self::Gost28147 | Self::Blowfish448 => 8,
            Self::Threefish1024 => 128,
            Self::Shacal2_512 => 32,
            Self::Composite2048 => 192,
        }
    }

    /// AEAD tag size in bytes: `min(block_bits, 256) / 8`.
    pub fn tag_bytes(self) -> usize {
        match self {
            Self::Composite2048 => 32,
            Self::Threefish1024 | Self::Shacal2_512 => 32,
            Self::Blowfish256 | Self::Gost28147 | Self::Blowfish448 => 8,
            _ => 16,
        }
    }

    pub fn is_composite(self) -> bool {
        self == Self::Composite2048
    }

    pub fn layers(self) -> Option<&'static [Layer]> {
        if self.is_composite() {
            Some(&COMPOSITE_LAYERS)
        } else {
            None
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Aes256 => "AES-256",
            Self::Rc6_256 => "RC6-256",
            Self::Serpent256 => "Serpent-256",
            Self::Blowfish256 => "Blowfish-256",
            Self::Twofish256 => "Twofish-256",
            Self::Gost28147 => "GOST28147-256",
            Self::Blowfish448 => "Blowfish-448",
            Self::Threefish1024 => "Threefish-1024",
            Self::Shacal2_512 => "SHACAL-2-512",
            Self::Composite2048 => "C4-2048",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Aes256 => "AES (256 bit)",
            Self::Rc6_256 => "RC6 (256 bit)",
            Self::Serpent256 => "Serpent (256 bit)",
            Self::Blowfish256 => "Blowfish (256 bit)",
            Self::Twofish256 => "Twofish (256 bit)",
            Self::Gost28147 => "GOST28147 (256 bit)",
            Self::Blowfish448 => "Blowfish (448 bit)",
            Self::Threefish1024 => "Threefish (1024 bit)",
            Self::Shacal2_512 => "SHACAL-2 (512 bit)",
            Self::Composite2048 => "C4 (2048 bit)",
        }
    }
}
