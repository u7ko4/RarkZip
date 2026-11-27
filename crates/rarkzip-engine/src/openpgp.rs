//! OpenPGP 签名生成、校验与本地密钥环。
//!
//! 支持分离签名（二进制或 ASCII armor）、明文签名（clearsigned）以及多签名
//! 文件。公钥和私钥由用户导入后保存在应用数据目录；私钥必须带口令。

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufReader, Cursor, Read};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use pgp::composed::{
    ArmorOptions, CleartextSignedMessage, Deserializable, DetachedSignature, SignedKeyDetails,
    SignedPublicKey, SignedPublicSubKey, SignedSecretKey, SignedSecretSubKey,
};
use pgp::crypto::hash::HashAlgorithm;
use pgp::packet::{KeyFlags, SecretKey, SecretSubkey, Signature, SignatureType, SubpacketData};
use pgp::ser::Serialize as PgpSerialize;
use pgp::types::{Fingerprint, KeyDetails, KeyId, Password, PublicKeyTrait};
use rand::{CryptoRng, Rng};

use crate::error::{EngineError, Result};
use crate::settings::data_dir;

const MAX_KEY_FILE: u64 = 16 * 1024 * 1024;
const MAX_SIGNATURE_FILE: u64 = 16 * 1024 * 1024;
const MAX_CLEARSIGNED_FILE: u64 = 64 * 1024 * 1024;

/// 本地公钥环目录。
pub fn keys_dir() -> PathBuf {
    data_dir().join("signature-keys")
}

/// 本地签名私钥目录。
pub fn secret_keys_dir() -> PathBuf {
    data_dir().join("signature-secret-keys")
}

/// 某个签名条目的校验状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureState {
    Valid,
    Invalid,
    MissingKey,
    ExpiredSignature,
    ExpiredKey,
    RevokedKey,
    InvalidKey,
    Error,
}

/// 单个签名的校验结果。
#[derive(Debug, Clone)]
pub struct VerifiedSignature {
    pub state: SignatureState,
    pub user_id: Option<String>,
    pub key_id: Option<String>,
    pub fingerprint: Option<String>,
    pub signed_at: Option<DateTime<Utc>>,
    pub key_created_at: Option<DateTime<Utc>>,
    pub key_expires_at: Option<DateTime<Utc>>,
    pub public_key_algorithm: Option<String>,
    pub hash_algorithm: Option<String>,
    pub detail: Option<String>,
}

/// 一个文件（可能包含多个签名）的校验结果。
#[derive(Debug, Clone)]
pub struct VerificationReport {
    pub overall: SignatureState,
    pub signatures: Vec<VerifiedSignature>,
    pub cleartext: bool,
}

impl VerificationReport {
    pub fn is_valid(&self) -> bool {
        self.overall == SignatureState::Valid
    }

    fn new(cleartext: bool, signatures: Vec<VerifiedSignature>) -> Self {
        let overall = overall_state(&signatures);
        Self {
            overall,
            signatures,
            cleartext,
        }
    }
}

/// 本地公钥环中的公钥摘要。
#[derive(Debug, Clone)]
pub struct KeyInfo {
    pub user_id: String,
    pub fingerprint: String,
    pub key_id: String,
    pub algorithm: String,
    pub created_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked: bool,
}

/// 公钥导入结果。
#[derive(Debug, Clone, Default)]
pub struct ImportReport {
    pub imported: Vec<KeyInfo>,
    pub errors: Vec<String>,
}

impl ImportReport {
    pub fn imported_count(&self) -> usize {
        self.imported.len()
    }
}

#[derive(Debug, Clone)]
struct KeyEntry {
    key: SignedPublicKey,
    info: KeyInfo,
    path: PathBuf,
}

/// 本地公钥环。公钥文件保存在应用数据目录，导入后即可离线校验签名。
#[derive(Debug, Clone)]
pub struct Keyring {
    dir: PathBuf,
    entries: Vec<KeyEntry>,
    warnings: Vec<String>,
}

impl Keyring {
    /// 从应用数据目录加载公钥环。
    pub fn load() -> Self {
        Self::load_from_dir(keys_dir())
    }

    /// 从指定目录加载公钥环，供测试或自定义部署使用。
    pub fn load_from_dir(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let (entries, warnings) = load_entries(&dir);
        Self {
            dir,
            entries,
            warnings,
        }
    }

    /// 公钥文件目录。
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 当前公钥列表。
    pub fn keys(&self) -> Vec<KeyInfo> {
        self.entries
            .iter()
            .map(|entry| entry.info.clone())
            .collect()
    }

    /// 当前公钥数量。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 加载或解析公钥文件时收集到的警告。
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// 导入一个或多个公钥。私钥文件会被拒绝。
    pub fn import(&mut self, path: &Path) -> Result<ImportReport> {
        if !path.is_file() {
            return Err(EngineError::NotFound(path.to_path_buf()));
        }
        let metadata = fs::metadata(path)?;
        if metadata.len() > MAX_KEY_FILE {
            return Err(EngineError::Other(format!(
                "公钥文件过大（上限 {} MiB）",
                MAX_KEY_FILE / 1024 / 1024
            )));
        }
        let bytes = fs::read(path)?;
        let keys = parse_public_keys(&bytes)?;
        fs::create_dir_all(&self.dir)?;

        let mut report = ImportReport::default();
        for key in keys {
            if let Err(error) = key.verify() {
                report.errors.push(format!("公钥自签名校验失败：{error}"));
                continue;
            }
            let info = key_info(&key);
            let target = self.dir.join(format!("{}.asc", info.fingerprint));
            match key.to_armored_string(ArmorOptions::default()) {
                Ok(armored) => match fs::write(&target, armored) {
                    Ok(()) => report.imported.push(info),
                    Err(error) => report
                        .errors
                        .push(format!("无法写入 {}：{error}", target.display())),
                },
                Err(error) => report.errors.push(format!("无法导出公钥：{error}")),
            }
        }
        self.reload();
        Ok(report)
    }

    /// 从本地公钥环移除指定指纹的公钥。
    pub fn remove(&mut self, fingerprint: &str) -> Result<bool> {
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.info.fingerprint.eq_ignore_ascii_case(fingerprint))
        else {
            return Ok(false);
        };
        let target_path = self.entries[index].path.clone();
        let remaining = self
            .entries
            .iter()
            .enumerate()
            .filter(|(entry_index, entry)| *entry_index != index && entry.path == target_path)
            .map(|(_, entry)| entry.key.clone())
            .collect::<Vec<_>>();
        if remaining.is_empty() {
            if target_path.exists() {
                fs::remove_file(&target_path)?;
            }
        } else {
            let mut armored = String::new();
            for key in remaining {
                armored.push_str(
                    &key.to_armored_string(ArmorOptions::default())
                        .map_err(|error| EngineError::Other(format!("无法导出公钥：{error}")))?,
                );
            }
            fs::write(&target_path, armored)?;
        }
        self.reload();
        Ok(true)
    }

    /// 校验一个分离签名或明文签名。
    pub fn verify(
        &self,
        signature_path: &Path,
        data_path: Option<&Path>,
    ) -> Result<VerificationReport> {
        if !signature_path.is_file() {
            return Err(EngineError::NotFound(signature_path.to_path_buf()));
        }
        if is_clearsigned(signature_path) {
            let bytes = read_limited(signature_path, MAX_CLEARSIGNED_FILE)?;
            let text = String::from_utf8(bytes)
                .map_err(|_| EngineError::Other("明文签名文件不是有效的 UTF-8 文本".to_string()))?;
            let (message, _) = CleartextSignedMessage::from_string(&text)
                .map_err(|error| EngineError::Other(format!("无法解析明文签名：{error}")))?;
            let signed_text = message.signed_text();
            if message.signatures().is_empty() {
                return Err(EngineError::Other("文件中没有找到签名".to_string()));
            }
            let signatures = message
                .signatures()
                .iter()
                .map(|signature| self.verify_bytes(signature, signed_text.as_bytes()))
                .collect::<Vec<_>>();
            return Ok(VerificationReport::new(true, signatures));
        }

        let data_path = data_path
            .ok_or_else(|| EngineError::Other("分离签名需要选择被签名的原始文件".to_string()))?;
        if !data_path.is_file() {
            return Err(EngineError::NotFound(data_path.to_path_buf()));
        }
        let bytes = read_limited(signature_path, MAX_SIGNATURE_FILE)?;
        let signatures = parse_detached_signatures(&bytes)?;
        let results = signatures
            .iter()
            .map(|signature| self.verify_stream(&signature.signature, data_path))
            .collect::<Vec<_>>();
        Ok(VerificationReport::new(false, results))
    }

    fn reload(&mut self) {
        let (entries, warnings) = load_entries(&self.dir);
        self.entries = entries;
        self.warnings = warnings;
    }

    fn verify_bytes(&self, signature: &Signature, data: &[u8]) -> VerifiedSignature {
        let candidates = self.candidates(signature);
        let fallback = candidates.first().cloned();
        if candidates.is_empty() {
            return missing_key(signature);
        }
        let mut best: Option<(Candidate<'_>, SignatureState, Option<String>)> = None;
        let mut last_error = None;
        for candidate in candidates {
            let result = match candidate.key {
                KeyRef::Primary(key) => signature.verify(key, data),
                KeyRef::Subkey(key) => signature.verify(key, data),
            };
            match result {
                Ok(()) => {
                    let (state, detail) = successful_state(signature, &candidate);
                    if state == SignatureState::Valid {
                        return verified(signature, &candidate, state, detail);
                    }
                    if best.is_none() {
                        best = Some((candidate, state, detail));
                    }
                }
                Err(error) => last_error = Some(error.to_string()),
            }
        }
        if let Some((candidate, state, detail)) = best {
            return verified(signature, &candidate, state, detail);
        }
        invalid(signature, fallback.as_ref(), last_error)
    }

    fn verify_stream(&self, signature: &Signature, data_path: &Path) -> VerifiedSignature {
        let candidates = self.candidates(signature);
        let fallback = candidates.first().cloned();
        if candidates.is_empty() {
            return missing_key(signature);
        }
        let mut best: Option<(Candidate<'_>, SignatureState, Option<String>)> = None;
        let mut last_error = None;
        for candidate in candidates {
            match File::open(data_path) {
                Ok(file) => {
                    let reader = BufReader::new(file);
                    let result = match candidate.key {
                        KeyRef::Primary(key) => signature.verify(key, reader),
                        KeyRef::Subkey(key) => signature.verify(key, reader),
                    };
                    match result {
                        Ok(()) => {
                            let (state, detail) = successful_state(signature, &candidate);
                            if state == SignatureState::Valid {
                                return verified(signature, &candidate, state, detail);
                            }
                            if best.is_none() {
                                best = Some((candidate, state, detail));
                            }
                        }
                        Err(error) => last_error = Some(error.to_string()),
                    }
                }
                Err(error) => last_error = Some(error.to_string()),
            }
        }
        if let Some((candidate, state, detail)) = best {
            return verified(signature, &candidate, state, detail);
        }
        invalid(signature, fallback.as_ref(), last_error)
    }

    fn candidates<'a>(&'a self, signature: &Signature) -> Vec<Candidate<'a>> {
        let issuer_fingerprints = signature.issuer_fingerprint();
        let issuer_ids = signature.issuer();
        let mut candidates = Vec::new();
        for entry in &self.entries {
            let key = &entry.key;
            if matches_issuer(
                &issuer_fingerprints,
                &issuer_ids,
                &key.fingerprint(),
                &key.key_id(),
            ) {
                candidates.push(Candidate {
                    key: KeyRef::Primary(key),
                    user_id: entry.info.user_id.clone(),
                    key_id: key.key_id(),
                    fingerprint: key.fingerprint(),
                    algorithm: entry.info.algorithm.clone(),
                    created_at: Some(*key.created_at()),
                    expires_at: entry.info.expires_at,
                    revoked: entry.info.revoked,
                });
            }
            for subkey in &key.public_subkeys {
                if matches_issuer(
                    &issuer_fingerprints,
                    &issuer_ids,
                    &subkey.fingerprint(),
                    &subkey.key_id(),
                ) {
                    candidates.push(Candidate {
                        key: KeyRef::Subkey(subkey),
                        user_id: entry.info.user_id.clone(),
                        key_id: subkey.key_id(),
                        fingerprint: subkey.fingerprint(),
                        algorithm: format!("{:?}", subkey.algorithm()),
                        created_at: Some(*subkey.created_at()),
                        expires_at: subkey_expires_at(subkey),
                        revoked: subkey
                            .signatures
                            .iter()
                            .any(|sig| sig.typ() == Some(SignatureType::SubkeyRevocation)),
                    });
                }
            }
        }
        candidates
    }
}

/// 本地签名私钥摘要。
#[derive(Debug, Clone)]
pub struct SecretKeyInfo {
    pub user_id: String,
    pub fingerprint: String,
    pub key_id: String,
    pub algorithm: String,
    pub created_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked: bool,
    pub signing_fingerprint: Option<String>,
    pub signing_algorithm: Option<String>,
}

/// 私钥导入结果。
#[derive(Debug, Clone, Default)]
pub struct SecretImportReport {
    pub imported: Vec<SecretKeyInfo>,
    pub errors: Vec<String>,
}

impl SecretImportReport {
    pub fn imported_count(&self) -> usize {
        self.imported.len()
    }
}

/// 文件签名结果。
#[derive(Debug, Clone)]
pub struct SignReport {
    pub output: PathBuf,
    pub user_id: String,
    pub fingerprint: String,
    pub key_id: String,
    pub algorithm: String,
    pub armored: bool,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
struct SecretKeyEntry {
    key: SignedSecretKey,
    info: SecretKeyInfo,
    path: PathBuf,
}

/// 本地签名私钥环。私钥必须带口令，导入后保存在应用数据目录。
#[derive(Debug, Clone)]
pub struct SecretKeyring {
    dir: PathBuf,
    entries: Vec<SecretKeyEntry>,
    warnings: Vec<String>,
}

impl SecretKeyring {
    pub fn load() -> Self {
        Self::load_from_dir(secret_keys_dir())
    }

    pub fn load_from_dir(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let (entries, warnings) = load_secret_entries(&dir);
        Self {
            dir,
            entries,
            warnings,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn keys(&self) -> Vec<SecretKeyInfo> {
        self.entries
            .iter()
            .map(|entry| entry.info.clone())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// 导入一个或多个带口令的私钥。
    pub fn import(&mut self, path: &Path) -> Result<SecretImportReport> {
        if !path.is_file() {
            return Err(EngineError::NotFound(path.to_path_buf()));
        }
        let metadata = fs::metadata(path)?;
        if metadata.len() > MAX_KEY_FILE {
            return Err(EngineError::Other(format!(
                "私钥文件过大（上限 {} MiB）",
                MAX_KEY_FILE / 1024 / 1024
            )));
        }
        let bytes = fs::read(path)?;
        let keys = parse_secret_keys(&bytes)?;
        fs::create_dir_all(&self.dir)?;

        let mut report = SecretImportReport::default();
        for key in keys {
            if let Err(error) = key.verify() {
                report.errors.push(format!("私钥自签名校验失败：{error}"));
                continue;
            }
            if !secret_key_encrypted(&key) {
                report
                    .errors
                    .push("私钥未设置口令，出于安全考虑未导入；请先为私钥设置口令".to_string());
                continue;
            }
            if signing_component(&key).is_none() {
                report.errors.push("私钥中没有可用于签名的密钥".to_string());
                continue;
            }
            let info = secret_key_info(&key);
            let target = self.dir.join(format!("{}.asc", info.fingerprint));
            match key.to_armored_string(ArmorOptions::default()) {
                Ok(armored) => match write_private_key(&target, armored.as_bytes()) {
                    Ok(()) => report.imported.push(info),
                    Err(error) => report
                        .errors
                        .push(format!("无法写入 {}：{error}", target.display())),
                },
                Err(error) => report.errors.push(format!("无法导出私钥：{error}")),
            }
        }
        self.reload();
        Ok(report)
    }

    pub fn remove(&mut self, fingerprint: &str) -> Result<bool> {
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.info.fingerprint.eq_ignore_ascii_case(fingerprint))
        else {
            return Ok(false);
        };
        let target_path = self.entries[index].path.clone();
        let remaining = self
            .entries
            .iter()
            .enumerate()
            .filter(|(entry_index, entry)| *entry_index != index && entry.path == target_path)
            .map(|(_, entry)| entry.key.clone())
            .collect::<Vec<_>>();
        if remaining.is_empty() {
            if target_path.exists() {
                fs::remove_file(&target_path)?;
            }
        } else {
            let mut armored = String::new();
            for key in remaining {
                armored.push_str(
                    &key.to_armored_string(ArmorOptions::default())
                        .map_err(|error| EngineError::Other(format!("无法导出私钥：{error}")))?,
                );
            }
            write_private_key(&target_path, armored.as_bytes())?;
        }
        self.reload();
        Ok(true)
    }

    /// 使用选中的私钥为文件生成分离签名。
    #[allow(clippy::too_many_arguments)]
    pub fn sign(
        &self,
        data: &Path,
        output: &Path,
        fingerprint: &str,
        passphrase: &str,
        armored: bool,
    ) -> Result<SignReport> {
        if !data.is_file() {
            return Err(EngineError::NotFound(data.to_path_buf()));
        }
        if output == data {
            return Err(EngineError::Other(
                "输出文件不能与被签名文件相同".to_string(),
            ));
        }
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.info.fingerprint.eq_ignore_ascii_case(fingerprint))
            .ok_or_else(|| EngineError::Other("未找到选中的签名私钥".to_string()))?;
        if entry.info.revoked {
            return Err(EngineError::Other(
                "签名私钥已被吊销，不能用于签名".to_string(),
            ));
        }
        let component = signing_component(&entry.key)
            .ok_or_else(|| EngineError::Other("该私钥没有可用于签名的密钥".to_string()))?;
        let reader = BufReader::new(File::open(data)?);
        let password = Password::from(passphrase);
        let signature = component
            .key
            .sign(rand::thread_rng(), &password, HashAlgorithm::Sha256, reader)
            .map_err(map_sign_error)?;
        let encoded = if armored {
            signature
                .to_armored_bytes(ArmorOptions::default())
                .map_err(|error| EngineError::Other(format!("无法生成签名：{error}")))?
        } else {
            signature
                .to_bytes()
                .map_err(|error| EngineError::Other(format!("无法生成签名：{error}")))?
        };
        if let Some(parent) = output.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(output, &encoded)?;
        Ok(SignReport {
            output: output.to_path_buf(),
            user_id: entry.info.user_id.clone(),
            fingerprint: component.fingerprint,
            key_id: component.key_id,
            algorithm: component.algorithm,
            armored,
            bytes: encoded.len() as u64,
        })
    }

    fn reload(&mut self) {
        let (entries, warnings) = load_secret_entries(&self.dir);
        self.entries = entries;
        self.warnings = warnings;
    }
}

#[derive(Clone, Copy)]
enum SigningKeyRef<'a> {
    Primary(&'a SecretKey),
    Subkey(&'a SecretSubkey),
}

impl SigningKeyRef<'_> {
    fn sign<RNG, R>(
        self,
        rng: RNG,
        password: &Password,
        hash: HashAlgorithm,
        data: R,
    ) -> pgp::errors::Result<DetachedSignature>
    where
        RNG: Rng + CryptoRng,
        R: Read,
    {
        match self {
            Self::Primary(key) => {
                DetachedSignature::sign_binary_data(rng, key, password, hash, data)
            }
            Self::Subkey(key) => {
                DetachedSignature::sign_binary_data(rng, key, password, hash, data)
            }
        }
    }
}

struct SigningComponent<'a> {
    key: SigningKeyRef<'a>,
    fingerprint: String,
    key_id: String,
    algorithm: String,
    expires_at: Option<DateTime<Utc>>,
    revoked: bool,
}

fn signing_component(key: &SignedSecretKey) -> Option<SigningComponent<'_>> {
    if primary_can_sign(key) {
        let public = key.primary_key.public_key();
        return Some(SigningComponent {
            key: SigningKeyRef::Primary(&key.primary_key),
            fingerprint: public.fingerprint().to_string(),
            key_id: public.key_id().to_string().to_uppercase(),
            algorithm: format!("{:?}", public.algorithm()),
            expires_at: key.expires_at(),
            revoked: !key.details.revocation_signatures.is_empty(),
        });
    }
    key.secret_subkeys
        .iter()
        .find(|subkey| secret_subkey_can_sign(subkey))
        .map(|subkey| {
            let public = subkey.key.public_key();
            SigningComponent {
                key: SigningKeyRef::Subkey(&subkey.key),
                fingerprint: public.fingerprint().to_string(),
                key_id: public.key_id().to_string().to_uppercase(),
                algorithm: format!("{:?}", public.algorithm()),
                expires_at: secret_subkey_expires_at(subkey).or_else(|| key.expires_at()),
                revoked: subkey
                    .signatures
                    .iter()
                    .any(|signature| signature.typ() == Some(SignatureType::SubkeyRevocation)),
            }
        })
}

fn primary_can_sign(key: &SignedSecretKey) -> bool {
    let flags = key
        .details
        .users
        .iter()
        .flat_map(|user| &user.signatures)
        .chain(key.details.direct_signatures.iter())
        .filter_map(key_flags_of)
        .collect::<Vec<_>>();
    if flags.is_empty() {
        key.primary_key.public_key().is_signing_key()
    } else {
        flags.iter().any(KeyFlags::sign)
    }
}

fn secret_subkey_can_sign(subkey: &SignedSecretSubKey) -> bool {
    let flags = subkey
        .signatures
        .iter()
        .filter(|signature| signature.typ() == Some(SignatureType::SubkeyBinding))
        .filter_map(key_flags_of)
        .collect::<Vec<_>>();
    if flags.is_empty() {
        subkey.key.public_key().is_signing_key()
    } else {
        flags.iter().any(KeyFlags::sign)
    }
}

fn key_flags_of(signature: &Signature) -> Option<KeyFlags> {
    signature.config().and_then(|config| {
        config
            .hashed_subpackets()
            .find_map(|subpacket| match &subpacket.data {
                SubpacketData::KeyFlags(flags) => Some(flags.clone()),
                _ => None,
            })
    })
}

fn secret_key_encrypted(key: &SignedSecretKey) -> bool {
    key.primary_key.secret_params().is_encrypted()
        && key
            .secret_subkeys
            .iter()
            .all(|subkey| subkey.key.secret_params().is_encrypted())
}

fn secret_key_info(key: &SignedSecretKey) -> SecretKeyInfo {
    let component = signing_component(key);
    let revoked = !key.details.revocation_signatures.is_empty()
        || component
            .as_ref()
            .is_some_and(|component| component.revoked);
    SecretKeyInfo {
        user_id: user_id_from_details(&key.details),
        fingerprint: key.primary_key.public_key().fingerprint().to_string(),
        key_id: key
            .primary_key
            .public_key()
            .key_id()
            .to_string()
            .to_uppercase(),
        algorithm: format!("{:?}", key.primary_key.public_key().algorithm()),
        created_at: Some(*key.primary_key.public_key().created_at()),
        expires_at: component
            .as_ref()
            .and_then(|component| component.expires_at)
            .or_else(|| key.expires_at()),
        revoked,
        signing_fingerprint: component
            .as_ref()
            .map(|component| component.fingerprint.clone()),
        signing_algorithm: component.map(|component| component.algorithm),
    }
}

fn secret_subkey_expires_at(subkey: &SignedSecretSubKey) -> Option<DateTime<Utc>> {
    subkey
        .signatures
        .iter()
        .filter(|signature| signature.typ() == Some(SignatureType::SubkeyBinding))
        .filter_map(|signature| {
            signature
                .key_expiration_time()
                .map(|duration| *subkey.key.public_key().created_at() + *duration)
        })
        .max()
}

fn load_secret_entries(dir: &Path) -> (Vec<SecretKeyEntry>, Vec<String>) {
    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    let mut seen = HashSet::new();
    let Ok(read_dir) = fs::read_dir(dir) else {
        return (entries, warnings);
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_file() || !is_secret_key_file(&path) {
            continue;
        }
        match read_secret_key_file(&path) {
            Ok(keys) => {
                for key in keys {
                    let info = secret_key_info(&key);
                    if !seen.insert(info.fingerprint.clone()) {
                        warnings.push(format!("重复私钥：{}", info.fingerprint));
                        continue;
                    }
                    entries.push(SecretKeyEntry {
                        key,
                        info,
                        path: path.clone(),
                    });
                }
            }
            Err(error) => warnings.push(format!("{}：{error}", path.display())),
        }
    }
    entries.sort_by(|left, right| left.info.user_id.cmp(&right.info.user_id));
    (entries, warnings)
}

fn read_secret_key_file(path: &Path) -> Result<Vec<SignedSecretKey>> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_KEY_FILE {
        return Err(EngineError::Other("文件过大，已跳过".to_string()));
    }
    let bytes = fs::read(path)?;
    let keys = parse_secret_keys(&bytes)?;
    keys.into_iter()
        .map(|key| {
            key.verify()
                .map_err(|error| EngineError::Other(format!("私钥自签名无效：{error}")))?;
            if !secret_key_encrypted(&key) {
                return Err(EngineError::Other("私钥未设置口令，已跳过".to_string()));
            }
            if signing_component(&key).is_none() {
                return Err(EngineError::Other("私钥中没有可用于签名的密钥".to_string()));
            }
            Ok(key)
        })
        .collect()
}

fn parse_secret_keys(bytes: &[u8]) -> Result<Vec<SignedSecretKey>> {
    if contains_armor_header(bytes, "-----BEGIN PGP PUBLIC KEY BLOCK-----") {
        return Err(EngineError::Other(
            "请选择私钥文件，不能导入公钥".to_string(),
        ));
    }
    let armored = contains_armor_header(bytes, "-----BEGIN PGP PRIVATE KEY BLOCK-----");
    let parsed = if armored {
        let (iter, _) = SignedSecretKey::from_armor_many(Cursor::new(bytes))
            .map_err(|error| EngineError::Other(format!("无法解析私钥：{error}")))?;
        iter.collect::<std::result::Result<Vec<_>, _>>()
    } else {
        let iter = SignedSecretKey::from_bytes_many(Cursor::new(bytes))
            .map_err(|error| EngineError::Other(format!("无法解析私钥：{error}")))?;
        iter.collect::<std::result::Result<Vec<_>, _>>()
    };
    let keys = parsed.map_err(|error| EngineError::Other(format!("私钥文件格式无效：{error}")))?;
    if keys.is_empty() {
        return Err(EngineError::Other("私钥文件中没有私钥".to_string()));
    }
    Ok(keys)
}

fn is_secret_key_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref(),
        Some("asc" | "gpg" | "pgp" | "key" | "sec" | "skr" | "private")
    )
}

fn user_id_from_details(details: &SignedKeyDetails) -> String {
    details
        .users
        .first()
        .map(|user| String::from_utf8_lossy(user.id.id()).into_owned())
        .filter(|user_id| !user_id.is_empty())
        .unwrap_or_else(|| "（无用户 ID）".to_string())
}

fn write_private_key(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn map_sign_error(error: pgp::errors::Error) -> EngineError {
    let message = error.to_string();
    let lower = message.to_ascii_lowercase();
    if lower.contains("checksum") || lower.contains("password") || lower.contains("decrypt") {
        EngineError::Other("私钥口令错误或私钥已损坏".to_string())
    } else {
        EngineError::Other(format!("签名失败：{message}"))
    }
}

#[derive(Clone, Copy)]
enum KeyRef<'a> {
    Primary(&'a SignedPublicKey),
    Subkey(&'a SignedPublicSubKey),
}

#[derive(Clone)]
struct Candidate<'a> {
    key: KeyRef<'a>,
    user_id: String,
    key_id: KeyId,
    fingerprint: Fingerprint,
    algorithm: String,
    created_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
    revoked: bool,
}

fn matches_issuer(
    issuer_fingerprints: &[&Fingerprint],
    issuer_ids: &[&KeyId],
    fingerprint: &Fingerprint,
    key_id: &KeyId,
) -> bool {
    if !issuer_fingerprints.is_empty() {
        return issuer_fingerprints.contains(&fingerprint);
    }
    if !issuer_ids.is_empty() {
        return issuer_ids.contains(&key_id);
    }
    true
}

fn successful_state(
    signature: &Signature,
    candidate: &Candidate<'_>,
) -> (SignatureState, Option<String>) {
    if candidate.revoked {
        return (
            SignatureState::RevokedKey,
            Some("签名密钥已被吊销".to_string()),
        );
    }
    if let (Some(created), Some(expiration)) =
        (signature.created(), signature.signature_expiration_time())
    {
        if Utc::now() > *created + *expiration {
            return (
                SignatureState::ExpiredSignature,
                Some("签名已过期".to_string()),
            );
        }
    }
    if let Some(expires_at) = candidate.expires_at {
        if Utc::now() > expires_at {
            return (
                SignatureState::ExpiredKey,
                Some("签名密钥已过期".to_string()),
            );
        }
    }
    (SignatureState::Valid, None)
}

fn verified(
    signature: &Signature,
    candidate: &Candidate<'_>,
    state: SignatureState,
    detail: Option<String>,
) -> VerifiedSignature {
    VerifiedSignature {
        state,
        user_id: Some(candidate.user_id.clone()),
        key_id: Some(candidate.key_id.to_string().to_uppercase()),
        fingerprint: Some(candidate.fingerprint.to_string().to_uppercase()),
        signed_at: signature.created().cloned(),
        key_created_at: candidate.created_at,
        key_expires_at: candidate.expires_at,
        public_key_algorithm: Some(candidate.algorithm.clone()),
        hash_algorithm: signature
            .hash_alg()
            .map(|algorithm| format!("{algorithm:?}")),
        detail,
    }
}

fn missing_key(signature: &Signature) -> VerifiedSignature {
    VerifiedSignature {
        state: SignatureState::MissingKey,
        user_id: None,
        key_id: signature
            .issuer()
            .first()
            .map(|key_id| key_id.to_string().to_uppercase()),
        fingerprint: signature
            .issuer_fingerprint()
            .first()
            .map(|fingerprint| fingerprint.to_string().to_uppercase()),
        signed_at: signature.created().cloned(),
        key_created_at: None,
        key_expires_at: None,
        public_key_algorithm: None,
        hash_algorithm: signature
            .hash_alg()
            .map(|algorithm| format!("{algorithm:?}")),
        detail: Some("本地公钥环中没有匹配的公钥".to_string()),
    }
}

fn invalid(
    signature: &Signature,
    candidate: Option<&Candidate<'_>>,
    last_error: Option<String>,
) -> VerifiedSignature {
    VerifiedSignature {
        state: SignatureState::Invalid,
        user_id: candidate.map(|candidate| candidate.user_id.clone()),
        key_id: candidate
            .map(|candidate| candidate.key_id.to_string().to_uppercase())
            .or_else(|| {
                signature
                    .issuer()
                    .first()
                    .map(|key_id| key_id.to_string().to_uppercase())
            }),
        fingerprint: candidate
            .map(|candidate| candidate.fingerprint.to_string().to_uppercase())
            .or_else(|| {
                signature
                    .issuer_fingerprint()
                    .first()
                    .map(|fingerprint| fingerprint.to_string().to_uppercase())
            }),
        signed_at: signature.created().cloned(),
        key_created_at: candidate.and_then(|candidate| candidate.created_at),
        key_expires_at: candidate.and_then(|candidate| candidate.expires_at),
        public_key_algorithm: candidate.map(|candidate| candidate.algorithm.clone()),
        hash_algorithm: signature
            .hash_alg()
            .map(|algorithm| format!("{algorithm:?}")),
        detail: Some(last_error.unwrap_or_else(|| "签名校验失败".to_string())),
    }
}

fn overall_state(signatures: &[VerifiedSignature]) -> SignatureState {
    signatures
        .iter()
        .map(|signature| signature.state)
        .max_by_key(|state| state_rank(*state))
        .unwrap_or(SignatureState::Error)
}

fn state_rank(state: SignatureState) -> u8 {
    match state {
        SignatureState::Valid => 0,
        SignatureState::ExpiredKey => 1,
        SignatureState::ExpiredSignature => 2,
        SignatureState::MissingKey => 3,
        SignatureState::RevokedKey => 4,
        SignatureState::InvalidKey => 5,
        SignatureState::Error => 6,
        SignatureState::Invalid => 7,
    }
}

fn load_entries(dir: &Path) -> (Vec<KeyEntry>, Vec<String>) {
    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    let mut seen = HashSet::new();
    let Ok(read_dir) = fs::read_dir(dir) else {
        return (entries, warnings);
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_file() || !is_key_file(&path) {
            continue;
        }
        match read_key_file(&path) {
            Ok(keys) => {
                for key in keys {
                    let info = key_info(&key);
                    if !seen.insert(info.fingerprint.clone()) {
                        warnings.push(format!("重复公钥：{}", info.fingerprint));
                        continue;
                    }
                    entries.push(KeyEntry {
                        key,
                        info,
                        path: path.clone(),
                    });
                }
            }
            Err(error) => warnings.push(format!("{}：{error}", path.display())),
        }
    }
    entries.sort_by(|left, right| left.info.user_id.cmp(&right.info.user_id));
    (entries, warnings)
}

fn read_key_file(path: &Path) -> Result<Vec<SignedPublicKey>> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_KEY_FILE {
        return Err(EngineError::Other("文件过大，已跳过".to_string()));
    }
    let bytes = fs::read(path)?;
    let keys = parse_public_keys(&bytes)?;
    keys.into_iter()
        .map(|key| {
            key.verify()
                .map_err(|error| EngineError::Other(format!("公钥自签名无效：{error}")))?;
            Ok(key)
        })
        .collect()
}

fn is_key_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref(),
        Some("asc" | "gpg" | "pgp" | "key" | "pub")
    )
}

fn parse_public_keys(bytes: &[u8]) -> Result<Vec<SignedPublicKey>> {
    if contains_armor_header(bytes, "-----BEGIN PGP PRIVATE KEY BLOCK-----") {
        return Err(EngineError::Other(
            "请选择公钥文件，不能导入私钥".to_string(),
        ));
    }
    let armored = contains_armor_header(bytes, "-----BEGIN PGP PUBLIC KEY BLOCK-----");
    let parsed = if armored {
        let (iter, _) = SignedPublicKey::from_armor_many(Cursor::new(bytes))
            .map_err(|error| EngineError::Other(format!("无法解析公钥：{error}")))?;
        iter.collect::<std::result::Result<Vec<_>, _>>()
    } else {
        let iter = SignedPublicKey::from_bytes_many(Cursor::new(bytes))
            .map_err(|error| EngineError::Other(format!("无法解析公钥：{error}")))?;
        iter.collect::<std::result::Result<Vec<_>, _>>()
    };
    let keys = parsed.map_err(|error| EngineError::Other(format!("公钥文件格式无效：{error}")))?;
    if keys.is_empty() {
        return Err(EngineError::Other("公钥文件中没有公钥".to_string()));
    }
    Ok(keys)
}

fn parse_detached_signatures(bytes: &[u8]) -> Result<Vec<DetachedSignature>> {
    let armored = contains_armor_header(bytes, "-----BEGIN PGP SIGNATURE-----");
    let parsed = if armored {
        let (iter, _) = DetachedSignature::from_armor_many(Cursor::new(bytes))
            .map_err(|error| EngineError::Other(format!("无法解析签名：{error}")))?;
        iter.collect::<std::result::Result<Vec<_>, _>>()
    } else {
        let iter = DetachedSignature::from_bytes_many(Cursor::new(bytes))
            .map_err(|error| EngineError::Other(format!("无法解析签名：{error}")))?;
        iter.collect::<std::result::Result<Vec<_>, _>>()
    };
    let signatures =
        parsed.map_err(|error| EngineError::Other(format!("签名文件格式无效：{error}")))?;
    if signatures.is_empty() {
        return Err(EngineError::Other("签名文件中没有签名".to_string()));
    }
    Ok(signatures)
}

fn contains_armor_header(bytes: &[u8], header: &str) -> bool {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    text.contains(header)
}

fn key_info(key: &SignedPublicKey) -> KeyInfo {
    KeyInfo {
        user_id: primary_user_id(key),
        fingerprint: key.fingerprint().to_string(),
        key_id: key.key_id().to_string().to_uppercase(),
        algorithm: format!("{:?}", key.algorithm()),
        created_at: Some(*key.created_at()),
        expires_at: key.expires_at(),
        revoked: !key.details.revocation_signatures.is_empty(),
    }
}

fn primary_user_id(key: &SignedPublicKey) -> String {
    user_id_from_details(&key.details)
}

fn subkey_expires_at(subkey: &SignedPublicSubKey) -> Option<DateTime<Utc>> {
    subkey
        .signatures
        .iter()
        .filter(|signature| signature.typ() == Some(SignatureType::SubkeyBinding))
        .filter_map(|signature| {
            signature
                .key_expiration_time()
                .map(|duration| *subkey.key.created_at() + *duration)
        })
        .max()
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > limit {
        return Err(EngineError::Other(format!(
            "文件过大（上限 {} MiB）",
            limit / 1024 / 1024
        )));
    }
    Ok(fs::read(path)?)
}

/// 判断文件是否为明文签名（clearsigned）。
pub fn is_clearsigned(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut buffer = [0u8; 512];
    let Ok(read) = file.read(&mut buffer) else {
        return false;
    };
    String::from_utf8_lossy(&buffer[..read]).contains("-----BEGIN PGP SIGNED MESSAGE-----")
}

/// 是否为可能的签名文件扩展名。
pub fn is_signature_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref(),
        Some("sig" | "asc" | "sign" | "gpg" | "pgp")
    )
}

/// 从签名文件名推断被签名的原始文件。
pub fn paired_data_path(signature: &Path) -> Option<PathBuf> {
    let name = signature.file_name()?.to_string_lossy();
    for suffix in [".sig", ".asc", ".sign", ".gpg", ".pgp"] {
        if name.to_ascii_lowercase().ends_with(suffix) {
            let base = &name[..name.len() - suffix.len()];
            if !base.is_empty() {
                let candidate = signature.with_file_name(base);
                if candidate != signature && candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    let stem = signature.file_stem()?;
    let candidate = signature.with_file_name(stem);
    (candidate != signature && candidate.is_file()).then_some(candidate)
}

/// 从原始文件名推断同名分离签名。
pub fn paired_signature_path(data: &Path) -> Option<PathBuf> {
    let name = data.file_name()?.to_string_lossy();
    let mut candidates = vec![
        data.with_file_name(format!("{name}.sig")),
        data.with_file_name(format!("{name}.asc")),
        data.with_file_name(format!("{name}.sign")),
        data.with_file_name(format!("{name}.gpg")),
    ];
    for extension in ["sig", "asc", "sign", "gpg"] {
        candidates.push(data.with_extension(extension));
    }
    candidates
        .into_iter()
        .find(|candidate| candidate != data && candidate.is_file())
}

/// 将指纹格式化为 4 字符一组的可读形式。
pub fn format_fingerprint(fingerprint: &str) -> String {
    let upper = fingerprint.to_ascii_uppercase();
    upper
        .as_bytes()
        .chunks(4)
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/openpgp")
            .join(name)
    }

    fn keyring_with_key() -> (tempfile::TempDir, Keyring) {
        let temp = tempfile::tempdir().unwrap();
        let source = fixture("public.asc");
        let target = temp.path().join("test-key.asc");
        fs::copy(source, target).unwrap();
        let keyring = Keyring::load_from_dir(temp.path());
        assert_eq!(keyring.len(), 1);
        (temp, keyring)
    }

    #[test]
    fn verifies_detached_armored_signature() {
        let (_temp, keyring) = keyring_with_key();
        let report = keyring
            .verify(&fixture("data.txt.asc"), Some(&fixture("data.txt")))
            .unwrap();
        assert!(report.is_valid());
        assert!(!report.cleartext);
        assert_eq!(report.signatures.len(), 1);
        assert_eq!(report.signatures[0].state, SignatureState::Valid);
        assert_eq!(
            report.signatures[0].user_id.as_deref(),
            Some("RarkZip Test Signer <test@example.com>")
        );
    }

    #[test]
    fn verifies_detached_binary_signature() {
        let (_temp, keyring) = keyring_with_key();
        let report = keyring
            .verify(&fixture("data.txt.sig"), Some(&fixture("data.txt")))
            .unwrap();
        assert!(report.is_valid());
    }

    #[test]
    fn verifies_clearsigned_message_without_data_file() {
        let (_temp, keyring) = keyring_with_key();
        let report = keyring.verify(&fixture("data.asc"), None).unwrap();
        assert!(report.is_valid());
        assert!(report.cleartext);
        assert_eq!(report.signatures[0].state, SignatureState::Valid);
    }

    #[test]
    fn rejects_tampered_data() {
        let (_temp, keyring) = keyring_with_key();
        let report = keyring
            .verify(&fixture("data.txt.asc"), Some(&fixture("tampered.txt")))
            .unwrap();
        assert_eq!(report.overall, SignatureState::Invalid);
        assert_eq!(report.signatures[0].state, SignatureState::Invalid);
    }

    #[test]
    fn reports_missing_key() {
        let temp = tempfile::tempdir().unwrap();
        let keyring = Keyring::load_from_dir(temp.path());
        let report = keyring
            .verify(&fixture("data.txt.asc"), Some(&fixture("data.txt")))
            .unwrap();
        assert_eq!(report.overall, SignatureState::MissingKey);
        assert!(report.signatures[0].key_id.is_some());
    }

    #[test]
    fn imports_and_removes_public_key() {
        let temp = tempfile::tempdir().unwrap();
        let mut keyring = Keyring::load_from_dir(temp.path());
        assert!(keyring.is_empty());
        let report = keyring.import(&fixture("public.asc")).unwrap();
        assert_eq!(report.imported_count(), 1);
        assert_eq!(keyring.len(), 1);
        let fingerprint = keyring.keys()[0].fingerprint.clone();
        assert!(keyring.remove(&fingerprint).unwrap());
        assert!(keyring.is_empty());
    }

    #[test]
    fn formats_fingerprint_in_groups() {
        assert_eq!(
            format_fingerprint("c7f1c188a08c75ac64735cfabe181a0f17ec6f00"),
            "C7F1 C188 A08C 75AC 6473 5CFA BE18 1A0F 17EC 6F00"
        );
    }

    #[test]
    fn detects_clearsigned_files() {
        assert!(is_clearsigned(&fixture("data.asc")));
        assert!(!is_clearsigned(&fixture("data.txt.asc")));
    }

    fn secret_keyring() -> (tempfile::TempDir, SecretKeyring) {
        let temp = tempfile::tempdir().unwrap();
        let mut keyring = SecretKeyring::load_from_dir(temp.path());
        let report = keyring.import(&fixture("signer-secret.asc")).unwrap();
        assert_eq!(report.imported_count(), 1);
        assert_eq!(keyring.len(), 1);
        (temp, keyring)
    }

    fn public_keyring_for_signer() -> (tempfile::TempDir, Keyring) {
        let temp = tempfile::tempdir().unwrap();
        fs::copy(
            fixture("signer-public.asc"),
            temp.path().join("signer-public.asc"),
        )
        .unwrap();
        let keyring = Keyring::load_from_dir(temp.path());
        assert_eq!(keyring.len(), 1);
        (temp, keyring)
    }

    #[test]
    fn imports_secret_key_and_signs_detached_signature() {
        let (_secret_temp, secret_keyring) = secret_keyring();
        let (_public_temp, public_keyring) = public_keyring_for_signer();
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("sign-data.sig");
        let fingerprint = secret_keyring.keys()[0].fingerprint.clone();
        let report = secret_keyring
            .sign(
                &fixture("sign-data.txt"),
                &output,
                &fingerprint,
                "test-passphrase",
                false,
            )
            .unwrap();
        assert!(output.exists());
        assert!(!report.armored);
        assert!(report.bytes > 0);
        let verification = public_keyring
            .verify(&output, Some(&fixture("sign-data.txt")))
            .unwrap();
        assert!(verification.is_valid());
        assert_eq!(
            verification.signatures[0].fingerprint.as_deref(),
            Some(report.fingerprint.to_uppercase().as_str())
        );
    }

    #[test]
    fn signs_armored_and_rejects_wrong_passphrase() {
        let (_secret_temp, secret_keyring) = secret_keyring();
        let (_public_temp, public_keyring) = public_keyring_for_signer();
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("sign-data.asc");
        let fingerprint = secret_keyring.keys()[0].fingerprint.clone();
        let report = secret_keyring
            .sign(
                &fixture("sign-data.txt"),
                &output,
                &fingerprint,
                "test-passphrase",
                true,
            )
            .unwrap();
        assert!(report.armored);
        let content = fs::read_to_string(&output).unwrap();
        assert!(content.starts_with("-----BEGIN PGP SIGNATURE-----"));
        assert!(public_keyring
            .verify(&output, Some(&fixture("sign-data.txt")))
            .unwrap()
            .is_valid());

        let wrong_output = temp.path().join("wrong.sig");
        assert!(secret_keyring
            .sign(
                &fixture("sign-data.txt"),
                &wrong_output,
                &fingerprint,
                "wrong-passphrase",
                false,
            )
            .is_err());
        assert!(!wrong_output.exists());
    }

    #[test]
    fn rejects_unencrypted_secret_key() {
        let temp = tempfile::tempdir().unwrap();
        let mut keyring = SecretKeyring::load_from_dir(temp.path());
        let report = keyring
            .import(&fixture("signer-secret-unencrypted.asc"))
            .unwrap();
        assert!(report.imported.is_empty());
        assert!(!report.errors.is_empty());
        assert!(keyring.is_empty());
    }
}
