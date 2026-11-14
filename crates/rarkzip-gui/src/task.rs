use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::i18n::{tr, Language};
use crossbeam_channel::{RecvTimeoutError, Sender};
use rarkzip_engine::{
    ArchiveInfo, CreateOptions, CreateOutcome, EngineError, ExtractOptions, ExtractOutcome,
    HashAlgorithm, PasswordPrompt, Phase, Progress, ProgressSink, VerifyReport,
};

#[derive(Debug, Clone)]
pub enum RetrySpec {
    Open {
        path: PathBuf,
        password: Option<String>,
    },
    Extract {
        path: PathBuf,
        options: ExtractOptions,
    },
    Create {
        sources: Vec<PathBuf>,
        dest: PathBuf,
        options: CreateOptions,
    },
    Verify {
        path: PathBuf,
        password: Option<String>,
    },
    Hash {
        path: PathBuf,
        algorithm: HashAlgorithm,
    },
}

#[derive(Debug, Clone)]
pub enum TaskStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug)]
pub struct TaskItem {
    pub id: u64,
    pub title: String,
    pub detail: String,
    pub status: TaskStatus,
    pub progress: f32,
    pub phase: String,
    pub cancel: Arc<AtomicBool>,
    pub dest: Option<PathBuf>,
    pub retry: Option<RetrySpec>,
}

impl TaskItem {
    pub fn new(
        id: u64,
        title: impl Into<String>,
        retry: Option<RetrySpec>,
        language: Language,
    ) -> Self {
        Self {
            id,
            title: title.into(),
            detail: tr(language, "等待开始…").to_string(),
            status: TaskStatus::Running,
            progress: 0.0,
            phase: tr(language, Phase::Preparing.label()).to_string(),
            cancel: Arc::new(AtomicBool::new(false)),
            dest: None,
            retry,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.status, TaskStatus::Running)
    }
}

#[derive(Debug)]
pub enum JobResult {
    Open {
        path: PathBuf,
        password: Option<String>,
        result: Result<ArchiveInfo, EngineError>,
    },
    Extract {
        path: PathBuf,
        result: Result<ExtractOutcome, EngineError>,
    },
    Create {
        password: Option<String>,
        result: Result<CreateOutcome, EngineError>,
    },
    Verify {
        path: PathBuf,
        result: Result<VerifyReport, EngineError>,
    },
    Hash {
        path: PathBuf,
        algorithm: HashAlgorithm,
        result: Result<String, EngineError>,
    },
}

#[derive(Debug)]
pub enum TaskEvent {
    Progress {
        id: u64,
        phase: &'static str,
        percent: f32,
        detail: String,
    },
    Warn {
        id: u64,
        message: String,
    },
    PasswordPrompt {
        id: u64,
        prompt: PasswordPrompt,
        responder: Sender<Option<String>>,
    },
    Finished {
        id: u64,
        result: JobResult,
    },
}

pub struct GuiSink {
    sender: Sender<TaskEvent>,
    id: u64,
    cancel: Arc<AtomicBool>,
    language: Language,
}

impl GuiSink {
    pub fn new(
        sender: Sender<TaskEvent>,
        id: u64,
        cancel: Arc<AtomicBool>,
        language: Language,
    ) -> Self {
        Self {
            sender,
            id,
            cancel,
            language,
        }
    }
}

impl ProgressSink for GuiSink {
    fn update(&self, progress: &Progress) {
        let _ = self.sender.send(TaskEvent::Progress {
            id: self.id,
            phase: tr(self.language, progress.phase.label()),
            percent: progress.percent(),
            detail: progress.current.clone(),
        });
    }

    fn warn(&self, message: &str) {
        let _ = self.sender.send(TaskEvent::Warn {
            id: self.id,
            message: message.to_string(),
        });
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// 密码本/父层密码失效时，阻塞任务线程等待主窗口输入密码。
    fn request_password(&self, prompt: &PasswordPrompt) -> Option<String> {
        if self.cancel.load(Ordering::Relaxed) {
            return None;
        }
        let (responder, receiver) = crossbeam_channel::unbounded();
        if self
            .sender
            .send(TaskEvent::PasswordPrompt {
                id: self.id,
                prompt: prompt.clone(),
                responder,
            })
            .is_err()
        {
            return None;
        }
        loop {
            match receiver.recv_timeout(std::time::Duration::from_millis(120)) {
                Ok(value) => return value,
                Err(RecvTimeoutError::Timeout) => {
                    if self.cancel.load(Ordering::Relaxed) {
                        return None;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return None,
            }
        }
    }
}
