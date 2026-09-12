use super::{
    tests::{MemoryStore, BACKUP_PASSWORD, PASSWORD},
    *,
};

#[test]
fn unlocked_management_cannot_export_without_fresh_authentication() {
    let root = tempfile::tempdir().unwrap();
    let mut vault =
        Vault::with_store(root.path().join("vault"), Box::<MemoryStore>::default()).unwrap();
    vault.initialize(PASSWORD).unwrap();
    let account = vault.create("protected").unwrap();
    let backup = root.path().join("backup.json");
    let before = std::fs::read(&vault.path).unwrap();
    assert!(vault
        .backup(account.id, "wrong-password", BACKUP_PASSWORD, &backup)
        .is_err());
    assert!(!backup.exists());
    assert!(!vault.status().unlocked);
    assert_eq!(std::fs::read(&vault.path).unwrap(), before);
    assert_eq!(
        vault
            .backup(account.id, PASSWORD, BACKUP_PASSWORD, &backup)
            .unwrap_err(),
        "wallet_unlock_rate_limited"
    );
    vault.unlock_after = Instant::now();
    vault
        .backup(account.id, PASSWORD, BACKUP_PASSWORD, &backup)
        .unwrap();
    assert!(!vault.status().unlocked);
    assert!(vault.account(account.id).unwrap().backed_up);
    let export: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&backup).unwrap()).unwrap();
    assert_eq!(export["format"], "asset-account-backup-v2");
    assert_eq!(export["kdf"]["memory_kib"], 262144);
    let mut restored =
        Vault::with_store(root.path().join("restored"), Box::<MemoryStore>::default()).unwrap();
    restored.initialize(PASSWORD).unwrap();
    assert_eq!(
        restored
            .restore(BACKUP_PASSWORD, &backup, "restored")
            .unwrap()
            .public_key,
        account.public_key
    );
}

#[test]
fn activity_cannot_extend_the_five_minute_unlock_deadline() {
    let root = tempfile::tempdir().unwrap();
    let mut vault =
        Vault::with_store(root.path().join("vault"), Box::<MemoryStore>::default()).unwrap();
    vault.initialize(PASSWORD).unwrap();
    vault.unlocked_at = Instant::now() - Duration::from_secs(290);
    let deadline = vault.unlocked_at;
    let account = vault.create("last minute").unwrap();
    assert_eq!(vault.unlocked_at, deadline);
    assert!(vault.status().unlocked);
    vault.unlocked_at = Instant::now() - Duration::from_secs(301);
    assert_eq!(vault.create("too late").unwrap_err(), "wallet_locked");
    assert!(vault.key.is_none());
    assert_eq!(vault.status().accounts, vec![account]);
}

#[test]
fn wrong_passwords_back_off_and_success_resets_without_lockout() {
    let root = tempfile::tempdir().unwrap();
    let mut vault =
        Vault::with_store(root.path().join("vault"), Box::<MemoryStore>::default()).unwrap();
    vault.initialize(PASSWORD).unwrap();
    for expected in [2, 4, 8, 16, 32, 60, 60] {
        vault.unlock_after = Instant::now();
        assert!(vault.unlock("wrong-password").is_err());
        assert_eq!(vault.status().retry_after_seconds, expected);
        assert_eq!(
            vault.unlock(PASSWORD).unwrap_err(),
            "wallet_unlock_rate_limited"
        );
        assert!(vault.key.is_none());
    }
    vault.unlock_after = Instant::now();
    vault.unlock(PASSWORD).unwrap();
    assert_eq!(vault.status().retry_after_seconds, 0);
    assert_eq!(vault.failed_attempts, 0);
    vault.unlock("wrong-password").unwrap_err();
    assert_eq!(vault.status().retry_after_seconds, 2);
}

#[cfg(unix)]
#[test]
fn private_permissions_are_repaired_without_changing_contents() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("vault");
    std::fs::create_dir(&directory).unwrap();
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o777)).unwrap();
    files::private_directory(&directory).unwrap();
    assert_eq!(
        std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let path = directory.join("vault-v1.json");
    std::fs::write(&path, b"encrypted fixture").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert_eq!(files::read_private(&path).unwrap(), b"encrypted fixture");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let lock_path = directory.join("vault.lock");
    let lock = files::private_lock(&lock_path).unwrap();
    assert_eq!(lock.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    assert!(files::private_lock(&lock_path).is_err());
}

#[cfg(unix)]
#[test]
fn links_and_nonregular_files_never_become_private_storage() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("original");
    std::fs::write(&target, b"keep original").unwrap();
    let link = root.path().join("link");
    symlink(&target, &link).unwrap();
    assert!(files::private_lock(&link).is_err());
    assert!(files::read(&link).is_err());
    assert!(files::write(&link, b"replacement").is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"keep original");
    let hard = root.path().join("hard");
    std::fs::hard_link(&target, &hard).unwrap();
    assert!(files::read_private(&hard).is_err());
    assert!(files::private_lock(&hard).is_err());
    assert!(files::read(root.path()).is_err());
    assert!(files::private_directory(&link).is_err());
    let directory = root.path().join("vault");
    files::private_directory(&directory).unwrap();
    symlink(root.path().join("missing"), directory.join("vault-v1.json")).unwrap();
    assert!(Vault::with_store(directory, Box::<MemoryStore>::default()).is_err());
}

#[test]
fn oversized_write_preserves_the_previous_encrypted_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault.json");
    files::write(&path, b"previous encrypted content").unwrap();
    assert_eq!(
        files::write(&path, &vec![0; 1_000_001]).unwrap_err(),
        "wallet_data_invalid"
    );
    assert_eq!(std::fs::read(path).unwrap(), b"previous encrypted content");
}
