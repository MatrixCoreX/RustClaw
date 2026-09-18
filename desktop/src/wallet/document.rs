use super::{
    crypto::{self, Sealed},
    keys,
    secure_memory::LockedKey,
    Account, SecretAccount,
};
use crate::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EncryptedAccount {
    pub account_id: Uuid,
    pub wrapped_key: Sealed,
    pub secret: Sealed,
    pub backup_version: u32,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Document {
    pub version: u32,
    pub id: Uuid,
    pub accounts: Vec<Account>,
    pub entries: Vec<EncryptedAccount>,
    pub manifest: Sealed,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LegacyDocument {
    pub version: u32,
    pub id: Uuid,
    pub accounts: Vec<Account>,
    pub sealed: Sealed,
}
pub(super) enum Stored {
    Legacy(LegacyDocument),
    Current(Document),
}
impl Stored {
    pub fn read(bytes: &[u8]) -> Result<Self> {
        #[derive(Deserialize)]
        struct Header {
            version: u32,
        }
        let header: Header = serde_json::from_slice(bytes).map_err(|_| "wallet_data_invalid")?;
        let stored = match header.version {
            1 => Self::Legacy(serde_json::from_slice(bytes).map_err(|_| "wallet_data_invalid")?),
            2 => Self::Current(serde_json::from_slice(bytes).map_err(|_| "wallet_data_invalid")?),
            _ => return Err("wallet_data_invalid".into()),
        };
        validate_accounts(stored.id(), stored.accounts())?;
        if let Self::Current(doc) = &stored {
            if doc.entries.len() != doc.accounts.len()
                || doc.entries.iter().zip(&doc.accounts).any(|(e, a)| {
                    e.account_id != a.id
                        || e.backup_version > 2
                        || (e.backup_version > 0) != a.backed_up
                })
            {
                return Err("wallet_data_invalid".into());
            }
        }
        Ok(stored)
    }
    pub fn id(&self) -> Uuid {
        match self {
            Self::Legacy(d) => d.id,
            Self::Current(d) => d.id,
        }
    }
    pub fn version(&self) -> u32 {
        match self {
            Self::Legacy(_) => 1,
            Self::Current(_) => 2,
        }
    }
    pub fn accounts(&self) -> &[Account] {
        match self {
            Self::Legacy(d) => &d.accounts,
            Self::Current(d) => &d.accounts,
        }
    }
    pub fn current(&self) -> Result<&Document> {
        match self {
            Self::Current(d) => Ok(d),
            _ => Err("wallet_migration_required".into()),
        }
    }
}
fn validate_accounts(id: Uuid, accounts: &[Account]) -> Result<()> {
    if id.is_nil() || accounts.len() > 100 {
        return Err("wallet_data_invalid".into());
    }
    let mut ids = std::collections::HashSet::new();
    let mut public = std::collections::HashSet::new();
    for a in accounts {
        keys::validate_public(&a.public_key)?;
        if a.id.is_nil()
            || !ids.insert(a.id)
            || !public.insert(&a.public_key)
            || a.name.is_empty()
            || a.name.chars().count() > 50
            || a.name.chars().any(char::is_control)
        {
            return Err("wallet_data_invalid".into());
        }
    }
    Ok(())
}
impl Document {
    pub fn empty(id: Uuid) -> Self {
        Self {
            version: 2,
            id,
            accounts: vec![],
            entries: vec![],
            manifest: Sealed {
                nonce: String::new(),
                ciphertext: String::new(),
            },
        }
    }
    fn aad(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&(
            "asset-vault-directory-v2",
            self.version,
            self.id,
            &self.accounts,
            &self.entries,
        ))
        .map_err(|_| "wallet_data_invalid".into())
    }
    fn key_aad(&self, account: &Account, purpose: &str) -> Result<Vec<u8>> {
        serde_json::to_vec(&(
            "asset-vault-account-v2",
            self.id,
            account.id,
            &account.public_key,
            purpose,
        ))
        .map_err(|_| "wallet_data_invalid".into())
    }
    pub fn authenticate(&mut self, master: &LockedKey) -> Result<()> {
        self.manifest = crypto::seal(master, &self.aad()?, b"")?;
        Ok(())
    }
    pub fn verify(&self, master: &LockedKey) -> Result<()> {
        if !crypto::open(master, &self.aad()?, &self.manifest)?.is_empty() {
            return Err("wallet_data_invalid".into());
        }
        Ok(())
    }
    pub fn insert(
        &mut self,
        master: &LockedKey,
        account: Account,
        secret: &LockedKey,
        backup_version: u32,
    ) -> Result<()> {
        if self.accounts.len() >= 100 {
            return Err("wallet_account_limit".into());
        }
        if self
            .accounts
            .iter()
            .any(|a| a.id == account.id || a.public_key == account.public_key)
        {
            return Err("wallet_account_duplicate".into());
        }
        if keys::public(secret)? != account.public_key {
            return Err("wallet_data_invalid".into());
        }
        let key = LockedKey::random()?;
        self.entries.push(EncryptedAccount {
            account_id: account.id,
            wrapped_key: crypto::seal(master, &self.key_aad(&account, "data-key")?, key.as_ref())?,
            secret: crypto::seal(
                &key,
                &self.key_aad(&account, "private-key")?,
                secret.as_ref(),
            )?,
            backup_version,
        });
        self.accounts.push(account);
        Ok(())
    }
    pub fn secret(&self, master: &LockedKey, id: Uuid) -> Result<LockedKey> {
        let index = self
            .accounts
            .iter()
            .position(|a| a.id == id)
            .ok_or("wallet_account_missing")?;
        let account = &self.accounts[index];
        let entry = &self.entries[index];
        let key = LockedKey::from_slice(&crypto::open(
            master,
            &self.key_aad(account, "data-key")?,
            &entry.wrapped_key,
        )?)?;
        let secret = LockedKey::from_slice(&crypto::open(
            &key,
            &self.key_aad(account, "private-key")?,
            &entry.secret,
        )?)?;
        if keys::public(&secret)? != account.public_key {
            return Err("wallet_data_invalid".into());
        }
        Ok(secret)
    }
}
impl LegacyDocument {
    pub fn migrate(&self, master: &LockedKey) -> Result<Document> {
        let aad = serde_json::to_vec(&("asset-vault-v1", self.id, &self.accounts))
            .map_err(|_| "wallet_data_invalid")?;
        let clear = crypto::open(master, &aad, &self.sealed)?;
        let entries: Vec<SecretAccount> =
            serde_json::from_slice(&clear).map_err(|_| "wallet_data_invalid")?;
        if entries.len() != self.accounts.len() {
            return Err("wallet_data_invalid".into());
        }
        let mut next = Document::empty(self.id);
        for (entry, account) in entries.iter().zip(&self.accounts) {
            if &entry.account != account {
                return Err("wallet_data_invalid".into());
            }
            let secret = LockedKey::from_slice(&entry.secret)?;
            next.insert(
                master,
                account.clone(),
                &secret,
                u32::from(account.backed_up),
            )?;
            if *next.secret(master, account.id)? != *secret {
                return Err("wallet_migration_failed".into());
            }
        }
        next.authenticate(master)?;
        next.verify(master)?;
        Ok(next)
    }
}
