use super::{backup_crypto, files, validate_name, Account, Vault};
use crate::Result;
use std::path::Path;
use uuid::Uuid;

impl Vault {
    pub fn backup(
        &mut self,
        id: Uuid,
        vault_password: &str,
        password: &str,
        path: &Path,
    ) -> Result<()> {
        self.lock();
        let result = (|| {
            self.unlock(vault_password)?;
            let account = self.account(id)?;
            if password == vault_password {
                return Err("wallet_backup_password_reused".into());
            }
            backup_crypto::password_valid(password, &[&account.name, &account.public_key])?;
            self.backup_unlocked(id, password, path)
        })();
        self.lock();
        result
    }
    fn backup_unlocked(&mut self, id: Uuid, password: &str, path: &Path) -> Result<()> {
        let destination = std::fs::canonicalize(path.parent().ok_or("wallet_backup_path_invalid")?)
            .map_err(|_| "wallet_backup_path_invalid")?;
        let directory = std::fs::canonicalize(self.path.parent().unwrap())
            .map_err(|_| "wallet_storage_invalid")?;
        if destination.starts_with(&directory) {
            return Err("wallet_backup_path_invalid".into());
        }
        let account = self.account(id)?;
        let secret = self.selected_secret(id)?;
        let bytes = backup_crypto::seal(password, &account.public_key, &secret)?;
        files::write(path, &bytes)?;
        let (public, verified, version) = backup_crypto::open(&files::read(path)?, password)?;
        if public != account.public_key || *verified != *secret || version != 2 {
            return Err("wallet_backup_verify_failed".into());
        }
        let mut next = self.current()?.clone();
        let index = next
            .accounts
            .iter()
            .position(|a| a.id == id)
            .ok_or("wallet_account_missing")?;
        next.accounts[index].backed_up = true;
        next.entries[index].backup_version = 2;
        self.save_document(next)
    }
    pub fn restore(&mut self, password: &str, path: &Path, name: &str) -> Result<Account> {
        let name = name.trim();
        validate_name(name)?;
        if self.current()?.accounts.len() >= 100 {
            return Err("wallet_account_limit".into());
        }
        // Password strength is an export policy, never a barrier to old recovery.
        let (public, secret, version) = backup_crypto::open(&files::read(path)?, password)?;
        let account = Account {
            id: Uuid::new_v4(),
            name: name.into(),
            public_key: public,
            backed_up: true,
        };
        self.insert_account(account, &secret, version)
    }
}
