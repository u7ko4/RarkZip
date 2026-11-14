#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Preparing,
    Compressing,
    Encrypting,
    Verifying,
    Decrypting,
    Extracting,
    Steganography,
    Finished,
}

#[derive(Debug, Clone)]
pub struct Progress {
    pub phase: Phase,
    pub done: u64,
    pub total: u64,
    pub current: Option<String>,
}

impl Progress {
    pub fn new(phase: Phase, done: u64, total: u64, current: Option<String>) -> Self {
        Self {
            phase,
            done,
            total,
            current,
        }
    }

    pub fn percent(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.done as f64 / self.total as f64 * 100.0).clamp(0.0, 100.0) as f32
        }
    }
}

/// Callback surface used by long-running operations.
pub trait Reporter {
    fn progress(&self, progress: Progress);
    fn cancelled(&self) -> bool {
        false
    }
}

pub struct NoopReporter;

impl Reporter for NoopReporter {
    fn progress(&self, _progress: Progress) {}
}

pub(crate) fn check_cancelled(reporter: &dyn Reporter) -> crate::error::Result<()> {
    if reporter.cancelled() {
        Err(crate::error::CryptError::Cancelled)
    } else {
        Ok(())
    }
}
