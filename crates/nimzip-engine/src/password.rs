use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::error::{EngineError, Result};
use crate::paths::to_extended;
use crate::settings::passwords_path;
use crate::util::now_unix;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasswordEntry {
    pub password: String,
    pub note: String,
    pub added_at: i64,
}

#[derive(Debug, Clone)]
pub struct PasswordBook {
    path: PathBuf,
    entries: Vec<PasswordEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredBook {
    version: u32,
    entries: Vec<StoredEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredEntry {
    secret: String,
    protected: bool,
    note: String,
    added_at: i64,
}

impl Default for PasswordBook {
    fn default() -> Self {
        Self::load_default()
    }
}

impl PasswordBook {
    /// 首次运行时导入程序目录/当前目录下的 `passwords.txt`。
    pub fn load_default() -> Self {
        let path = passwords_path();
        let existed = path.exists();
        let mut book = Self::load(path);
        // 旧版本会写入一批内置解压密码；升级后清理这些条目，密码本只保留
        // 用户手动添加/导入的密码。旧版本还为空备注自动写入“手动添加”，
        // 这里一并清空，界面上不再显示该自动备注。
        let changed = clean_legacy_notes(&mut book.entries);
        if !existed {
            for file in password_seed_files() {
                let Ok(text) = std::fs::read_to_string(to_extended(&file)) else {
                    continue;
                };
                for line in text.lines() {
                    let line = line.trim().trim_start_matches('\u{feff}');
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    book.add(line, "passwords.txt");
                }
            }
            let _ = book.save();
        } else if changed {
            let _ = book.save();
        }
        book
    }

    pub fn load(path: PathBuf) -> Self {
        let mut book = Self {
            path,
            entries: Vec::new(),
        };
        let Ok(bytes) = std::fs::read(to_extended(&book.path)) else {
            return book;
        };
        let Ok(stored) = serde_json::from_slice::<StoredBook>(&bytes) else {
            return book;
        };
        for item in stored.entries {
            let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(item.secret.as_bytes())
            else {
                continue;
            };
            let password = if item.protected {
                match unprotect(&raw) {
                    Ok(value) => value,
                    Err(_) => continue,
                }
            } else {
                raw
            };
            if let Ok(password) = String::from_utf8(password) {
                book.entries.push(PasswordEntry {
                    password,
                    note: item.note,
                    added_at: item.added_at,
                });
            }
        }
        book
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(to_extended(parent))?;
        }
        let mut entries = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            let raw = entry.password.as_bytes();
            let (secret, protected) = match protect(raw) {
                Ok(value) => (value, true),
                Err(_) => (raw.to_vec(), false),
            };
            entries.push(StoredEntry {
                secret: base64::engine::general_purpose::STANDARD.encode(secret),
                protected,
                note: entry.note.clone(),
                added_at: entry.added_at,
            });
        }
        let stored = StoredBook {
            version: 1,
            entries,
        };
        let bytes = serde_json::to_vec_pretty(&stored)
            .map_err(|error| EngineError::Other(error.to_string()))?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(to_extended(&tmp), bytes)?;
        std::fs::rename(to_extended(&tmp), to_extended(&self.path))?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entries(&self) -> &[PasswordEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn add(&mut self, password: impl Into<String>, note: impl Into<String>) {
        let password = password.into();
        if password.is_empty() || self.entries.iter().any(|entry| entry.password == password) {
            return;
        }
        self.entries.push(PasswordEntry {
            password,
            note: note.into(),
            added_at: now_unix(),
        });
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.entries.len() {
            self.entries.remove(index);
        }
    }

    pub fn move_up(&mut self, index: usize) {
        if index > 0 && index < self.entries.len() {
            self.entries.swap(index, index - 1);
        }
    }

    pub fn move_down(&mut self, index: usize) {
        if index + 1 < self.entries.len() {
            self.entries.swap(index, index + 1);
        }
    }

    /// 按列表顺序逐个试密，返回第一个命中的密码。
    ///
    /// 只有支持快速密码校验的格式才自动尝试；RAR/7z 等格式逐个试密需要完整
    /// 解压条目，直接返回 `None` 让调用方请求用户输入。
    pub fn try_all(&self, archive: &Path) -> Result<Option<String>> {
        if !crate::archive::supports_quick_password_test(archive) {
            return Ok(None);
        }
        for entry in &self.entries {
            let matched = crate::archive::test_password(archive, &entry.password)?;
            if matched {
                return Ok(Some(entry.password.clone()));
            }
        }
        Ok(None)
    }

    /// 手动输入成功后记入密码本末尾。
    pub fn remember(&mut self, password: &str) -> Result<()> {
        if password.is_empty() || self.entries.iter().any(|entry| entry.password == password) {
            return Ok(());
        }
        self.add(password, "手动输入");
        self.save()
    }
}

/// 清理旧版本自动写入的备注：删除“内置密码”种子条目，并把“手动添加”
/// 还原为空备注。返回是否有改动。
fn clean_legacy_notes(entries: &mut Vec<PasswordEntry>) -> bool {
    let before = entries.len();
    entries.retain(|entry| entry.note != "内置密码");
    let mut changed = entries.len() != before;
    for entry in entries {
        if entry.note == "手动添加" {
            entry.note.clear();
            changed = true;
        }
    }
    changed
}

fn password_seed_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            files.push(dir.join("passwords.txt"));
        }
    }
    files.push(PathBuf::from("passwords.txt"));
    files
}

#[cfg(windows)]
fn protect(data: &[u8]) -> Result<Vec<u8>> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        CryptProtectData(
            &input,
            PCWSTR::null(),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|error| EngineError::Other(format!("DPAPI 加密失败：{error}")))?;
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData as *mut _)));
        Ok(bytes)
    }
}

#[cfg(windows)]
fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|error| EngineError::Other(format!("DPAPI 解密失败：{error}")))?;
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData as *mut _)));
        Ok(bytes)
    }
}

#[cfg(not(windows))]
fn protect(_data: &[u8]) -> Result<Vec<u8>> {
    Err(EngineError::Other("当前平台没有 DPAPI".to_string()))
}

#[cfg(not(windows))]
fn unprotect(_data: &[u8]) -> Result<Vec<u8>> {
    Err(EngineError::Other("当前平台没有 DPAPI".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(password: &str, note: &str) -> PasswordEntry {
        PasswordEntry {
            password: password.to_string(),
            note: note.to_string(),
            added_at: 0,
        }
    }

    #[test]
    fn cleans_legacy_auto_notes() {
        let mut entries = vec![
            entry("builtin", "内置密码"),
            entry("manual", "手动添加"),
            entry("work", "工作"),
        ];

        assert!(clean_legacy_notes(&mut entries));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].password, "manual");
        assert!(entries[0].note.is_empty());
        assert_eq!(entries[1].note, "工作");

        assert!(!clean_legacy_notes(&mut entries));
    }

    #[test]
    fn try_all_skips_rar_without_quick_password_check() {
        let book = PasswordBook {
            path: PathBuf::new(),
            entries: vec![entry("unrar", "")],
        };
        let rar = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/crypted.rar");
        assert!(!crate::archive::supports_quick_password_test(&rar));
        // RAR 不能快速试密，即使密码本命中也不应在自动阶段返回密码。
        assert_eq!(book.try_all(&rar).unwrap(), None);
    }
}
