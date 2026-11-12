use std::path::PathBuf;

/// 引擎统一错误模型。
///
/// 密码错误、数据损坏、取消等需要被 UI/CLI 区分处理，因此使用独立变体而非
/// 统一的字符串错误。
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("不支持的归档格式：{0}")]
    UnsupportedFormat(String),

    #[error("归档已加密，需要密码")]
    PasswordRequired,

    #[error("密码错误或缺失")]
    WrongPassword,

    #[error("归档数据损坏：{0}")]
    DataError(String),

    #[error("不安全的条目路径：{0}")]
    UnsafePath(String),

    #[error("操作已取消")]
    Cancelled,

    #[error("文件不存在：{}", .0.display())]
    NotFound(PathBuf),

    #[error("目标已存在：{}", .0.display())]
    AlreadyExists(PathBuf),

    #[error("I/O 错误：{0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

impl EngineError {
    pub fn other(message: impl Into<String>) -> Self {
        Self::Other(message.into())
    }

    /// 是否是密码相关错误，便于 UI 决定是否弹出密码框。
    pub fn is_password_related(&self) -> bool {
        matches!(self, Self::PasswordRequired | Self::WrongPassword)
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;
