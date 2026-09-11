use super::{crypto, files, keys, Account, SecretAccount, Vault};
use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Backup {
    format: String,
    public_key: String,
    encrypted: crypto::Wrapped,
}
const FORMAT: &str = "asset-account-backup-v1";
fn aad(public: &str) -> Vec<u8> {
    format!("{FORMAT}:{public}").into_bytes()
}

impl Vault {
    pub fn backup(&mut self, id: Uuid, password: &str, path: &Path) -> Result<()> {
        let destination = std::fs::canonicalize(path.parent().ok_or("wallet_backup_path_invalid")?)
            .map_err(|_| "wallet_backup_path_invalid")?;
        let vault_directory = std::fs::canonicalize(self.path.parent().unwrap())
            .map_err(|_| "wallet_storage_invalid")?;
        // Resolve directory aliases too: a symlink must not let an export replace
        // the live vault and then falsely report a recoverable portable backup.
        if destination.starts_with(&vault_directory) {
            return Err("wallet_backup_path_invalid".into());
        }
        let mut entries = self.entries()?;
        let entry = entries
            .iter_mut()
            .find(|e| e.account.id == id)
            .ok_or("wallet_account_missing")?;
        let backup = Backup {
            format: FORMAT.into(),
            public_key: entry.account.public_key.clone(),
            encrypted: crypto::wrap(password, &aad(&entry.account.public_key), &entry.secret)?,
        };
        files::write(
            path,
            &serde_json::to_vec_pretty(&backup).map_err(|_| "wallet_data_invalid")?,
        )?;
        let (public, secret) = read_backup(path, password)?;
        if public != entry.account.public_key || *secret != entry.secret {
            return Err("wallet_backup_verify_failed".into());
        }
        entry.account.backed_up = true;
        self.save(&entries)
    }
    pub fn restore(&mut self, password: &str, path: &Path, name: &str) -> Result<Account> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 50 || name.chars().any(char::is_control) {
            return Err("wallet_name_invalid".into());
        }
        let mut entries = self.entries()?;
        if entries.len() >= 100 {
            return Err("wallet_account_limit".into());
        }
        let (public, secret) = read_backup(path, password)?;
        if entries.iter().any(|e| e.account.public_key == public) {
            return Err("wallet_account_duplicate".into());
        }
        let account = Account {
            id: Uuid::new_v4(),
            name: name.into(),
            public_key: public,
            backed_up: true,
        };
        entries.push(SecretAccount {
            account: account.clone(),
            secret: *secret,
        });
        self.save(&entries)?;
        Ok(account)
    }
}

fn read_backup(path: &Path, password: &str) -> Result<(String, Zeroizing<[u8; 32]>)> {
    let backup: Backup =
        serde_json::from_slice(&files::read(path)?).map_err(|_| "wallet_backup_invalid")?;
    if backup.format != FORMAT {
        return Err("wallet_backup_invalid".into());
    }
    keys::validate_public(&backup.public_key)?;
    let clear = crypto::unwrap(password, &aad(&backup.public_key), &backup.encrypted)?;
    if clear.len() != 32 {
        return Err("wallet_backup_invalid".into());
    }
    let mut secret = Zeroizing::new([0; 32]);
    secret.copy_from_slice(&clear);
    if keys::public(&secret)? != backup.public_key {
        return Err("wallet_backup_invalid".into());
    }
    Ok((backup.public_key, secret))
}
