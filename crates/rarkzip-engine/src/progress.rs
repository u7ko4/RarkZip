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

/// 引擎向调用方上报进度、警告与取消状态。
pub trait ProgressSink: Send + Sync {
    fn update(&self, _progress: &Progress) {}

    fn warn(&self, _message: &str) {}

    fn is_cancelled(&self) -> bool {
        false
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
