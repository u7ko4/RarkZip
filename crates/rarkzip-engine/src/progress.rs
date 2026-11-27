use std::path::PathBuf;

use crate::error::{EngineError, Result};

/// 任务阶段，用于 UI 显示“正在做什么”。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Preparing,
    Extracting,
    Compressing,
    Testing,
    Password,
    Nested,
    Scanning,
    Hashing,
    Finished,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preparing => "准备中",
            Self::Extracting => "解压中",
            Self::Compressing => "压缩中",
            Self::Testing => "校验中",
            Self::Password => "尝试密码",
            Self::Nested => "嵌套解压",
            Self::Scanning => "安全扫描",
            Self::Hashing => "计算哈希",
            Self::Finished => "完成",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Progress {
    pub phase: Phase,
    pub done: u64,
    pub total: u64,
    /// 当前条目或文件的展示名。
    pub current: String,
}

impl Progress {
    pub fn new(phase: Phase, done: u64, total: u64, current: impl Into<String>) -> Self {
        Self {
            phase,
            done,
            total,
            current: current.into(),
        }
    }

    pub fn percent(&self) -> f32 {
        if self.total == 0 {
            if self.done > 0 {
                100.0
            } else {
                0.0
            }
        } else {
            (self.done as f64 / self.total as f64 * 100.0).clamp(0.0, 100.0) as f32
        }
    }
}

/// 引擎请求调用方输入密码。
///
/// 首层归档只有能快速校验密码的格式才会自动尝试密码本；其余格式以及
/// 嵌套层沿用密码失败时，引擎通过 `ProgressSink::request_password` 请求
/// UI / CLI 弹窗。返回 `None` 表示用户取消，该归档将按“需要密码”跳过。
#[derive(Debug, Clone)]
pub struct PasswordPrompt {
    /// 需要密码的归档路径（伪装/分卷已还原为可读副本时仍保留原路径）。
    pub archive: PathBuf,
    /// 是否是嵌套压缩包。
    pub nested: bool,
    /// 第几次请求，从 1 开始；大于 1 表示上一次输入的密码不正确。
    pub attempt: u32,
    /// 可直接展示给用户的提示文本。
    pub message: String,
}

/// 引擎向调用方上报进度、警告、取消状态与密码请求。
pub trait ProgressSink: Send + Sync {
    fn update(&self, _progress: &Progress) {}

    fn warn(&self, _message: &str) {}

    fn is_cancelled(&self) -> bool {
        false
    }

    /// 请求用户输入密码。默认实现不弹窗（调用方得不到密码时归档会被跳过）。
    fn request_password(&self, _prompt: &PasswordPrompt) -> Option<String> {
        None
    }
}

#[derive(Debug, Default)]
pub struct NoopSink;

impl ProgressSink for NoopSink {}

pub fn check_cancelled(sink: &dyn ProgressSink) -> Result<()> {
    if sink.is_cancelled() {
        Err(EngineError::Cancelled)
    } else {
        Ok(())
    }
}
