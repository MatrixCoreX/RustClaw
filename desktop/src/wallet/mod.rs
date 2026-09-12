mod backup;
mod backup_crypto;
#[cfg(feature = "gui")]
pub mod commands;
pub(crate) mod crypto;
mod document;
pub(crate) mod files;
pub(crate) mod keys;
mod keystore;
#[cfg(feature = "gui")]
pub mod lifecycle;
#[cfg(test)]
mod protection_tests;
pub(crate) mod secure_memory;
#[cfg(test)]
mod security_tests;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
pub mod worker;

use crate::Result;
use document::{Document, Stored};
use keystore::{KeyStore, NativeKeyStore};
use secure_memory::LockedKey;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    path::PathBuf,
    time::{Duration, Instant},
};
use uuid::Uuid;
use zeroize::Zeroize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub id: Uuid,
    pub name: String,
    pub public_key: String,
    pub backed_up: bool,
}
#[derive(Serialize, Deserialize, Zeroize)]
#[zeroize(drop)]
struct SecretAccount {
    #[zeroize(skip)]
    account: Account,
    secret: [u8; 32],
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub initialized: bool,
    pub unlocked: bool,
    pub accounts: Vec<Account>,
    pub retry_after_seconds: u64,
    pub storage_version: u32,
    pub backup_upgrade_accounts: Vec<Uuid>,
}
pub struct Vault {
    path: PathBuf,
    document: Option<Stored>,
    key: Option<LockedKey>,
    unlocked_at: Instant,
    unlock_after: Instant,
    failed_attempts: u32,
    store: Box<dyn KeyStore>,
    _file_lock: File,
}
impl Vault {
    pub fn new(directory: PathBuf) -> Result<Self> {
        Self::with_store(directory, Box::new(NativeKeyStore))
    }
    fn with_store(directory: PathBuf, store: Box<dyn KeyStore>) -> Result<Self> {
        files::private_directory(&directory)?;
        let lock = files::private_lock(&directory.join("vault.lock"))?;
        // Keep the canonical storage location. The authenticated version controls decoding.
        let path = directory.join("vault-v1.json");
        let document = if std::fs::symlink_metadata(&path).is_ok() {
            Some(Stored::read(&files::read_private(&path)?)?)
        } else {
            None
        };
        Ok(Self {
            path,
            document,
            key: None,
            unlocked_at: Instant::now(),
            unlock_after: Instant::now(),
            failed_attempts: 0,
            store,
            _file_lock: lock,
        })
    }
    pub fn lock(&mut self) {
        self.key = None;
    }
    pub fn expire(&mut self) -> bool {
        let was = self.key.is_some();
        if self.unlocked_at.elapsed() >= Duration::from_secs(300) {
            self.lock();
        }
        was && self.key.is_none()
    }
    pub fn status(&mut self) -> Status {
        self.expire();
        let accounts = self
            .document
            .as_ref()
            .map(|d| d.accounts().to_vec())
            .unwrap_or_default();
        let backup_upgrade_accounts = match self.document.as_ref() {
            Some(Stored::Legacy(d)) => d
                .accounts
                .iter()
                .filter(|a| a.backed_up)
                .map(|a| a.id)
                .collect(),
            Some(Stored::Current(d)) => d
                .entries
                .iter()
                .filter(|e| e.backup_version == 1)
                .map(|e| e.account_id)
                .collect(),
            _ => vec![],
        };
        Status {
            initialized: self.document.is_some(),
            unlocked: self.key.is_some(),
            accounts,
            retry_after_seconds: self
                .unlock_after
                .saturating_duration_since(Instant::now())
                .as_secs_f64()
                .ceil() as u64,
            storage_version: self.document.as_ref().map(Stored::version).unwrap_or(2),
            backup_upgrade_accounts,
        }
    }
    pub fn initialize(&mut self, password: &str) -> Result<()> {
        if self.document.is_some() {
            return Err("wallet_already_initialized".into());
        }
        let key = LockedKey::random()?;
        let id = Uuid::new_v4();
        let wrapper = crypto::wrap(password, id.as_bytes(), key.as_ref())?;
        self.store.save(
            id,
            &serde_json::to_string(&wrapper).map_err(|_| "wallet_data_invalid")?,
        )?;
        let mut doc = Document::empty(id);
        doc.authenticate(&key)?;
        files::write(
            &self.path,
            &serde_json::to_vec(&doc).map_err(|_| "wallet_data_invalid")?,
        )?;
        self.document = Some(Stored::Current(doc));
        self.key = Some(key);
        self.unlocked_at = Instant::now();
        Ok(())
    }
    pub fn unlock(&mut self, password: &str) -> Result<()> {
        self.lock();
        if Instant::now() < self.unlock_after {
            return Err("wallet_unlock_rate_limited".into());
        }
        let result = self.unlock_inner(password);
        if result.is_ok() {
            self.failed_attempts = 0;
            self.unlock_after = Instant::now();
        } else if result.as_ref().err().is_some_and(|e| {
            matches!(
                e.as_str(),
                "wallet_unlock_failed" | "wallet_password_length"
            )
        }) {
            self.failed_attempts = self.failed_attempts.saturating_add(1).min(6);
            self.unlock_after =
                Instant::now() + Duration::from_secs((1u64 << self.failed_attempts).min(60));
        }
        result
    }
    fn unlock_inner(&mut self, password: &str) -> Result<()> {
        let doc = self.document.as_ref().ok_or("wallet_not_initialized")?;
        let wrapper: crypto::Wrapped =
            serde_json::from_str(&self.store.load(doc.id())?).map_err(|_| "wallet_data_invalid")?;
        let key = LockedKey::from_slice(&crypto::unwrap(password, doc.id().as_bytes(), &wrapper)?)?;
        match doc {
            Stored::Current(d) => d.verify(&key)?,
            Stored::Legacy(d) => {
                let next = d.migrate(&key)?;
                let bytes = serde_json::to_vec(&next).map_err(|_| "wallet_data_invalid")?;
                Stored::read(&bytes)?.current()?.verify(&key)?;
                files::write(&self.path, &bytes)?;
                self.document = Some(Stored::Current(next));
            }
        }
        self.key = Some(key);
        self.unlocked_at = Instant::now();
        Ok(())
    }
    fn current(&mut self) -> Result<&Document> {
        self.expire();
        self.key.as_ref().ok_or("wallet_locked")?;
        self.document
            .as_ref()
            .ok_or("wallet_not_initialized")?
            .current()
    }
    fn selected_secret(&mut self, id: Uuid) -> Result<LockedKey> {
        self.current()?;
        self.document
            .as_ref()
            .unwrap()
            .current()?
            .secret(self.key.as_ref().unwrap(), id)
    }
    fn save_document(&mut self, mut doc: Document) -> Result<()> {
        let key = self.key.as_ref().ok_or("wallet_locked")?;
        doc.authenticate(key)?;
        doc.verify(key)?;
        files::write(
            &self.path,
            &serde_json::to_vec(&doc).map_err(|_| "wallet_data_invalid")?,
        )?;
        self.document = Some(Stored::Current(doc));
        Ok(())
    }
    fn insert_account(
        &mut self,
        account: Account,
        secret: &LockedKey,
        backup_version: u32,
    ) -> Result<Account> {
        let mut next = self.current()?.clone();
        next.insert(
            self.key.as_ref().unwrap(),
            account.clone(),
            secret,
            backup_version,
        )?;
        self.save_document(next)?;
        Ok(account)
    }
    pub fn create(&mut self, name: &str) -> Result<Account> {
        let name = name.trim();
        validate_name(name)?;
        self.current()?;
        let secret = keys::generate()?;
        let account = Account {
            id: Uuid::new_v4(),
            name: name.into(),
            public_key: keys::public(&secret)?,
            backed_up: false,
        };
        self.insert_account(account, &secret, 0)
    }
    pub fn account(&self, id: Uuid) -> Result<Account> {
        self.document
            .as_ref()
            .and_then(|d| d.accounts().iter().find(|a| a.id == id))
            .cloned()
            .ok_or("wallet_account_missing".into())
    }
    pub(crate) fn sign_with_password(
        &mut self,
        id: Uuid,
        bytes: &[u8],
        password: &str,
    ) -> Result<String> {
        self.lock();
        let result = (|| {
            self.unlock(password)?;
            self.sign_unlocked(id, bytes)
        })();
        self.lock();
        result
    }
    fn sign_unlocked(&mut self, id: Uuid, bytes: &[u8]) -> Result<String> {
        self.current()?;
        if !self.account(id)?.backed_up {
            return Err("wallet_backup_required".into());
        }
        keys::sign(&*self.selected_secret(id)?, bytes)
    }
    #[cfg(test)]
    fn entries(&mut self) -> Result<Vec<SecretAccount>> {
        let accounts = self.current()?.accounts.clone();
        accounts
            .into_iter()
            .map(|account| {
                Ok(SecretAccount {
                    secret: *self.selected_secret(account.id)?,
                    account,
                })
            })
            .collect()
    }
}
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.chars().count() > 50 || name.chars().any(char::is_control) {
        Err("wallet_name_invalid".into())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
mod windows_security;

#[cfg(all(test, windows))]
mod windows_tests;

#[cfg(target_os="macos")]
#[path="session_macos.rs"]
mod native_session;
#[cfg(target_os="windows")]
#[path="session_windows.rs"]
mod native_session;
