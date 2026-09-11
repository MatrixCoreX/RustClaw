mod backup;
#[cfg(feature = "gui")]
pub mod commands;
pub(crate) mod crypto;
pub(crate) mod files;
pub(crate) mod keys;
mod keystore;
#[cfg(feature = "gui")]
pub mod lifecycle;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use crate::Result;
use crypto::Sealed;
use keystore::{KeyStore, NativeKeyStore};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    path::PathBuf,
    time::{Duration, Instant},
};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

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
struct Document {
    version: u32,
    id: Uuid,
    accounts: Vec<Account>,
    sealed: Sealed,
}
impl Document {
    fn aad(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&("asset-vault-v1", self.id, &self.accounts))
            .map_err(|_| "wallet_data_invalid".into())
    }
}

#[derive(Serialize)]
pub struct Status {
    pub initialized: bool,
    pub unlocked: bool,
    pub accounts: Vec<Account>,
}

pub struct Vault {
    path: PathBuf,
    document: Option<Document>,
    key: Option<Zeroizing<[u8; 32]>>,
    last_used: Instant,
    unlock_after: Instant,
    store: Box<dyn KeyStore>,
    _file_lock: File,
}

impl Vault {
    pub fn new(directory: PathBuf) -> Result<Self> {
        Self::with_store(directory, Box::new(NativeKeyStore))
    }
    fn with_store(directory: PathBuf, store: Box<dyn KeyStore>) -> Result<Self> {
        files::private_directory(&directory)?;
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("vault.lock"))
            .map_err(|_| "wallet_storage_unavailable")?;
        lock.try_lock().map_err(|_| "wallet_already_open")?;
        let path = directory.join("vault-v1.json");
        let document: Option<Document> = if path.exists() {
            Some(serde_json::from_slice(&files::read(&path)?).map_err(|_| "wallet_data_invalid")?)
        } else {
            None
        };
        if document
            .as_ref()
            .is_some_and(|d| d.version != 1 || d.accounts.len() > 100)
        {
            return Err("wallet_data_invalid".into());
        }
        if let Some(doc) = &document {
            let mut ids = std::collections::HashSet::new();
            let mut public_keys = std::collections::HashSet::new();
            for account in &doc.accounts {
                keys::validate_public(&account.public_key)?;
                if account.id.is_nil()
                    || !ids.insert(account.id)
                    || !public_keys.insert(&account.public_key)
                    || account.name.is_empty()
                    || account.name.chars().count() > 50
                    || account.name.chars().any(char::is_control)
                {
                    return Err("wallet_data_invalid".into());
                }
            }
        }
        Ok(Self {
            path,
            document,
            key: None,
            last_used: Instant::now(),
            unlock_after: Instant::now(),
            store,
            _file_lock: lock,
        })
    }
    pub fn lock(&mut self) {
        self.key = None;
    }
    pub fn expire(&mut self) -> bool {
        let was_unlocked = self.key.is_some();
        if self.last_used.elapsed() >= Duration::from_secs(300) {
            self.lock();
        }
        was_unlocked && self.key.is_none()
    }
    pub fn status(&mut self) -> Status {
        self.expire();
        Status {
            initialized: self.document.is_some(),
            unlocked: self.key.is_some(),
            accounts: self
                .document
                .as_ref()
                .map(|d| d.accounts.clone())
                .unwrap_or_default(),
        }
    }
    pub fn initialize(&mut self, password: &str) -> Result<()> {
        if self.document.is_some() {
            return Err("wallet_already_initialized".into());
        }
        let key = Zeroizing::new(crypto::random::<32>()?);
        let id = Uuid::new_v4();
        let wrapper = crypto::wrap(password, id.as_bytes(), key.as_ref())?;
        self.store.save(
            id,
            &serde_json::to_string(&wrapper).map_err(|_| "wallet_data_invalid")?,
        )?;
        let mut doc = Document {
            version: 1,
            id,
            accounts: vec![],
            sealed: Sealed {
                nonce: String::new(),
                ciphertext: String::new(),
            },
        };
        doc.sealed = crypto::seal(&key, &doc.aad()?, b"[]")?;
        files::write(
            &self.path,
            &serde_json::to_vec(&doc).map_err(|_| "wallet_data_invalid")?,
        )?;
        self.document = Some(doc);
        self.key = Some(key);
        self.last_used = Instant::now();
        Ok(())
    }
    pub fn unlock(&mut self, password: &str) -> Result<()> {
        self.lock();
        if Instant::now() < self.unlock_after {
            return Err("wallet_unlock_rate_limited".into());
        }
        let result = self.unlock_inner(password);
        self.unlock_after = Instant::now() + Duration::from_secs(2);
        result
    }
    fn unlock_inner(&mut self, password: &str) -> Result<()> {
        let doc = self.document.as_ref().ok_or("wallet_not_initialized")?;
        let wrapper: crypto::Wrapped =
            serde_json::from_str(&self.store.load(doc.id)?).map_err(|_| "wallet_data_invalid")?;
        let bytes = crypto::unwrap(password, doc.id.as_bytes(), &wrapper)?;
        let mut key = Zeroizing::new([0; 32]);
        if bytes.len() != 32 {
            return Err("wallet_data_invalid".into());
        }
        key.copy_from_slice(&bytes);
        let clear = crypto::open(&key, &doc.aad()?, &doc.sealed)?;
        Self::decode(&clear, doc)?;
        self.key = Some(key);
        self.last_used = Instant::now();
        Ok(())
    }
    fn decode(clear: &[u8], doc: &Document) -> Result<Vec<SecretAccount>> {
        let entries: Vec<SecretAccount> =
            serde_json::from_slice(clear).map_err(|_| "wallet_data_invalid")?;
        if entries.len() != doc.accounts.len() {
            return Err("wallet_data_invalid".into());
        }
        for (entry, meta) in entries.iter().zip(&doc.accounts) {
            if &entry.account != meta || keys::public(&entry.secret)? != meta.public_key {
                return Err("wallet_data_invalid".into());
            }
        }
        Ok(entries)
    }
    fn entries(&mut self) -> Result<Vec<SecretAccount>> {
        self.expire();
        let key = self.key.as_ref().ok_or("wallet_locked")?;
        let doc = self.document.as_ref().ok_or("wallet_not_initialized")?;
        let clear = crypto::open(key, &doc.aad()?, &doc.sealed)?;
        Self::decode(&clear, doc)
    }
    fn save(&mut self, entries: &[SecretAccount]) -> Result<()> {
        let key = self.key.as_ref().ok_or("wallet_locked")?;
        let old = self.document.as_ref().ok_or("wallet_not_initialized")?;
        let mut doc = Document {
            version: 1,
            id: old.id,
            accounts: entries.iter().map(|e| e.account.clone()).collect(),
            sealed: old.sealed.clone(),
        };
        let clear = Zeroizing::new(serde_json::to_vec(entries).map_err(|_| "wallet_data_invalid")?);
        doc.sealed = crypto::seal(key, &doc.aad()?, &clear)?;
        files::write(
            &self.path,
            &serde_json::to_vec(&doc).map_err(|_| "wallet_data_invalid")?,
        )?;
        self.document = Some(doc);
        self.last_used = Instant::now();
        Ok(())
    }
    pub fn create(&mut self, name: &str) -> Result<Account> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 50 || name.chars().any(char::is_control) {
            return Err("wallet_name_invalid".into());
        }
        let mut entries = self.entries()?;
        if entries.len() >= 100 {
            return Err("wallet_account_limit".into());
        }
        let secret = keys::generate()?;
        let account = Account {
            id: Uuid::new_v4(),
            name: name.into(),
            public_key: keys::public(&secret)?,
            backed_up: false,
        };
        entries.push(SecretAccount {
            account: account.clone(),
            secret: *secret,
        });
        self.save(&entries)?;
        Ok(account)
    }
    pub fn account(&self, id: Uuid) -> Result<Account> {
        self.document
            .as_ref()
            .and_then(|d| d.accounts.iter().find(|a| a.id == id))
            .cloned()
            .ok_or("wallet_account_missing".into())
    }
    // Only the validated operation module gets signing access. Never registered as IPC.
    #[cfg(any(feature = "gui", test))]
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
    #[cfg(any(feature = "gui", test))]
    fn sign_unlocked(&mut self, id: Uuid, bytes: &[u8]) -> Result<String> {
        let entries = self.entries()?;
        let entry = entries
            .iter()
            .find(|e| e.account.id == id)
            .ok_or("wallet_account_missing")?;
        if !entry.account.backed_up {
            return Err("wallet_backup_required".into());
        }
        // Background balance/status requests must never prolong an unlocked vault.
        keys::sign(&entry.secret, bytes)
    }
}
