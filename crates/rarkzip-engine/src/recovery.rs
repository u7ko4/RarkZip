//! 密码恢复：字典 / 掩码 / 暴力三种模式，多线程搜索并支持暂停、取消。
//!
//! 恢复只调用引擎的快速试密（最小加密条目），不会整包解压；RAR/7Z/ZIP
//! 共用同一套搜索空间与调度。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::archive::open_archive;
use crate::error::{EngineError, Result};
use crate::progress::NoopSink;

#[derive(Debug, Clone)]
pub enum RecoveryMode {
    Dictionary(Vec<String>),
    Mask(String),
    Brute {
        charset: String,
        min_len: usize,
        max_len: usize,
    },
}

#[derive(Debug, Clone)]
pub struct RecoveryOptions {
    pub mode: RecoveryMode,
    pub threads: usize,
}

impl Default for RecoveryOptions {
    fn default() -> Self {
        Self {
            mode: RecoveryMode::Dictionary(Vec::new()),
            threads: std::thread::available_parallelism()
                .map(|value| value.get())
                .unwrap_or(4)
                .min(16),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RecoveryControl {
    pub cancel: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
}

impl RecoveryControl {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone)]
pub struct RecoveryOutcome {
    pub password: Option<String>,
    pub attempts: u64,
    pub total: u64,
    pub elapsed: Duration,
    pub cancelled: bool,
}

pub fn recover_password(
    path: &std::path::Path,
    options: &RecoveryOptions,
    control: &RecoveryControl,
    progress: impl Fn(u64, u64) + Send + Sync,
) -> Result<RecoveryOutcome> {
    let space = SearchSpace::new(&options.mode)?;
    if space.total() == 0 {
        return Err(EngineError::Other("恢复候选集为空".to_string()));
    }
    check_encrypted(path)?;
    let threads = options.threads.clamp(1, 64);
    let next = Arc::new(AtomicU64::new(0));
    let attempts = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let found = Arc::new(Mutex::new(None::<String>));
    let fatal = Arc::new(Mutex::new(None::<String>));
    let started = Instant::now();
    let mut cancelled = false;

    std::thread::scope(|scope| {
        for _ in 0..threads {
            let space = &space;
            let next = Arc::clone(&next);
            let attempts = Arc::clone(&attempts);
            let stop = Arc::clone(&stop);
            let found = Arc::clone(&found);
            let fatal = Arc::clone(&fatal);
            let control = control.clone();
            let progress = &progress;
            scope.spawn(move || loop {
                if stop.load(Ordering::Relaxed)
                    || control.cancel.load(Ordering::Relaxed)
                    || fatal.lock().map(|value| value.is_some()).unwrap_or(true)
                {
                    break;
                }
                while control.paused.load(Ordering::Relaxed) {
                    if control.cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(30));
                }
                let index = next.fetch_add(1, Ordering::Relaxed);
                if index >= space.total() {
                    break;
                }
                let Some(candidate) = space.candidate(index) else {
                    continue;
                };
                let done = attempts.fetch_add(1, Ordering::Relaxed) + 1;
                if done % 64 == 0 || done == space.total() {
                    progress(done, space.total());
                }
                match crate::test_password(path, &candidate) {
                    Ok(true) => {
                        if let Ok(mut slot) = found.lock() {
                            if slot.is_none() {
                                *slot = Some(candidate);
                            }
                        }
                        stop.store(true, Ordering::Relaxed);
                        break;
                    }
                    Ok(false) => {}
                    Err(EngineError::WrongPassword) => {}
                    Err(error) => {
                        if let Ok(mut slot) = fatal.lock() {
                            if slot.is_none() {
                                *slot = Some(error.to_string());
                            }
                        }
                        stop.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            });
        }
    });

    if let Some(message) = fatal.lock().ok().and_then(|value| value.clone()) {
        return Err(EngineError::Other(message));
    }
    let password = found.lock().ok().and_then(|value| value.clone());
    if password.is_none() && control.cancel.load(Ordering::Relaxed) {
        cancelled = true;
    }
    let attempts = attempts.load(Ordering::Relaxed);
    progress(attempts, space.total());
    Ok(RecoveryOutcome {
        password,
        attempts,
        total: space.total(),
        elapsed: started.elapsed(),
        cancelled,
    })
}

fn check_encrypted(path: &std::path::Path) -> Result<()> {
    match open_archive(path, &crate::archive::OpenOptions::default()) {
        Ok(info) if !info.encrypted && info.encrypted_entry_count() == 0 => {
            Err(EngineError::Other("该归档未加密，无需恢复密码".to_string()))
        }
        Ok(_) => Ok(()),
        Err(error) if error.is_password_related() => Ok(()),
        Err(error) => Err(error),
    }
}

enum SearchSpace {
    Dictionary(Vec<String>),
    Mask {
        positions: Vec<Vec<char>>,
        total: u64,
    },
    Brute {
        chars: Vec<char>,
        segments: Vec<(usize, u64, u64)>,
        total: u64,
    },
}

impl SearchSpace {
    fn total(&self) -> u64 {
        match self {
            Self::Dictionary(words) => words.len() as u64,
            Self::Mask { total, .. } => *total,
            Self::Brute { total, .. } => *total,
        }
    }

    fn new(mode: &RecoveryMode) -> Result<Self> {
        match mode {
            RecoveryMode::Dictionary(words) => {
                let words: Vec<String> = words
                    .iter()
                    .map(|word| word.trim_matches(['\r', '\n']).to_string())
                    .filter(|word| !word.is_empty())
                    .collect();
                Ok(Self::Dictionary(words))
            }
            RecoveryMode::Mask(pattern) => Self::mask(pattern),
            RecoveryMode::Brute {
                charset,
                min_len,
                max_len,
            } => Self::brute(charset, *min_len, *max_len),
        }
    }

    fn mask(pattern: &str) -> Result<Self> {
        let mut positions = Vec::new();
        let mut chars = pattern.chars();
        while let Some(ch) = chars.next() {
            if ch != '?' {
                positions.push(vec![ch]);
                continue;
            }
            let Some(token) = chars.next() else {
                positions.push(vec!['?']);
                break;
            };
            positions.push(match token {
                'l' => ('a'..='z').collect(),
                'u' => ('A'..='Z').collect(),
                'd' => ('0'..='9').collect(),
                's' => "!@#$%^&*()-_=+[]{};:,.?/\\|~`".chars().collect(),
                'a' => {
                    let mut values: Vec<char> = ('a'..='z').collect();
                    values.extend('A'..='Z');
                    values.extend('0'..='9');
                    values.extend("!@#$%^&*()-_=+[]{};:,.?/\\|~`".chars());
                    values
                }
                '?' => vec!['?'],
                other => {
                    return Err(EngineError::Other(format!(
                        "未知掩码占位符：?{other}（支持 ?l ?u ?d ?s ?a ??）"
                    )))
                }
            });
        }
        if positions.is_empty() {
            return Err(EngineError::Other("掩码不能为空".to_string()));
        }
        let total = positions.iter().try_fold(1u64, |acc, values| {
            acc.checked_mul(values.len() as u64)
                .ok_or_else(|| EngineError::Other("掩码组合数超出范围".to_string()))
        })?;
        Ok(Self::Mask { positions, total })
    }

    fn brute(charset: &str, min_len: usize, max_len: usize) -> Result<Self> {
        let chars: Vec<char> = charset.chars().collect();
        if chars.is_empty() {
            return Err(EngineError::Other("字符集不能为空".to_string()));
        }
        let min_len = min_len.max(1);
        let max_len = max_len.max(min_len);
        if max_len > 12 {
            return Err(EngineError::Other(
                "暴力搜索最大长度不能超过 12".to_string(),
            ));
        }
        let mut segments = Vec::new();
        let mut total = 0u64;
        for len in min_len..=max_len {
            let count = (chars.len() as u64)
                .checked_pow(len as u32)
                .ok_or_else(|| EngineError::Other("搜索空间超出范围".to_string()))?;
            let start = total;
            total = total
                .checked_add(count)
                .ok_or_else(|| EngineError::Other("搜索空间超出范围".to_string()))?;
            segments.push((len, start, count));
        }
        Ok(Self::Brute {
            chars,
            segments,
            total,
        })
    }

    fn candidate(&self, index: u64) -> Option<String> {
        match self {
            Self::Dictionary(words) => words.get(index as usize).cloned(),
            Self::Mask { positions, .. } => {
                let mut value = vec!['\0'; positions.len()];
                let mut rest = index;
                for position in (0..positions.len()).rev() {
                    let choices = &positions[position];
                    let choice = (rest % choices.len() as u64) as usize;
                    rest /= choices.len() as u64;
                    value[position] = choices[choice];
                }
                Some(value.into_iter().collect())
            }
            Self::Brute {
                chars, segments, ..
            } => {
                let (len, start, _) = segments
                    .iter()
                    .find(|(_, start, count)| index >= *start && index < start + count)?;
                let mut rest = index - start;
                let mut value = vec!['\0'; *len];
                for position in (0..*len).rev() {
                    let choice = (rest % chars.len() as u64) as usize;
                    rest /= chars.len() as u64;
                    value[position] = chars[choice];
                }
                Some(value.into_iter().collect())
            }
        }
    }
}

#[allow(dead_code)]
fn _progress_sink_is_send() {
    fn assert_send<T: Send + Sync>() {}
    assert_send::<NoopSink>();
}
