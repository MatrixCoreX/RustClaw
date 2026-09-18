use super::*;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub(super) struct MemoryStore(Arc<Mutex<std::collections::HashMap<Uuid, String>>>);
impl KeyStore for MemoryStore {
    fn load(&self, id: Uuid) -> Result<String> {
        self.0
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or("wallet_keystore_unavailable".into())
    }
    fn save(&self, id: Uuid, wrapped: &str) -> Result<()> {
        self.0.lock().unwrap().insert(id, wrapped.into());
        Ok(())
    }
}
pub(super) const BACKUP_PASSWORD: &str = "Jasper!flume7-Pebble4-Orbit9-velvet";
pub(super) const PASSWORD: &str = "test-only-vault-password";

#[test]
fn backend_handoff_vectors_match_native_signatures_and_payload_validation() {
    use crate::asset_operations::protocol::{validate_challenge, Capabilities, Intent, Payload};
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/asset-owner-v1-vectors.json"
    ))
    .unwrap();
    for vector in fixture["vectors"].as_array().unwrap() {
        let raw = vector["signing_payload"].as_str().unwrap();
        let payload: Payload = serde_json::from_str(raw).unwrap();
        let cap: Capabilities = serde_json::from_value(vector["capabilities"].clone()).unwrap();
        let intent: Intent = serde_json::from_value(vector["intent"].clone()).unwrap();
        validate_challenge(
            raw,
            &cap,
            &payload.account,
            payload.operation_id,
            &intent,
            vector["now_unix"].as_i64().unwrap(),
        )
        .unwrap();
        assert_eq!(
            keys::sign(&[1; 32], raw.as_bytes()).unwrap(),
            vector["signature_hex"].as_str().unwrap()
        );
    }
}

#[test]
fn k1_signature_matches_existing_ui_and_rejects_invalid_public_keys() {
    let public = keys::public(&[1; 32]).unwrap();
    assert_eq!(public, "73MTSWz2Nks4Eaf8G8F7Nr6jbHorZSM774HFmtrdEuahUsc7To");
    assert_eq!(keys::sign(&[1;32],b"asset-owner-native-interop-v1").unwrap(),"0529bebcbed219a98f8e7e9a5de9627d18d50de40f57ebd4ef3440c323d6d78c1c259b2899e5ec302fd62413b5c1a296e9ae0d01db8ad387f76abdf7818bb8c2");
    keys::validate_public(&public).unwrap();
    assert!(keys::validate_public(&(public + "1")).is_err());
    assert!(keys::public(&[0; 32]).is_err());
}

#[test]
fn vault_encrypted_backup_restore_lock_and_metadata_authentication() {
    let root = tempfile::tempdir().unwrap();
    let store = MemoryStore::default();
    let directory = root.path().join("vault");
    let mut vault = Vault::with_store(directory.clone(), Box::new(store.clone())).unwrap();
    assert!(Vault::with_store(directory.clone(), Box::new(store.clone())).is_err());
    assert!(vault.initialize("short").is_err());
    vault.initialize(PASSWORD).unwrap();
    let account = vault.create("账户一").unwrap();
    #[cfg(unix)]
    {
        let alias = root.path().join("vault-alias");
        std::os::unix::fs::symlink(&directory, &alias).unwrap();
        assert_eq!(
            vault
                .backup(
                    account.id,
                    PASSWORD,
                    BACKUP_PASSWORD,
                    &alias.join("export.json")
                )
                .unwrap_err(),
            "wallet_backup_path_invalid"
        );
    }
    vault.unlock(PASSWORD).unwrap();
    assert_eq!(
        vault.sign_unlocked(account.id, b"test").unwrap_err(),
        "wallet_backup_required"
    );
    let secret = vault.entries().unwrap()[0].secret;
    let disk = std::fs::read_to_string(&vault.path).unwrap();
    assert!(!disk.contains(&hex::encode(secret)));
    assert!(!disk.contains(&serde_json::to_string(&secret).unwrap()));
    assert!(!disk.contains(PASSWORD));
    let backup = root.path().join("account.backup.json");
    vault
        .backup(account.id, PASSWORD, BACKUP_PASSWORD, &backup)
        .unwrap();
    assert!(!vault.status().unlocked);
    vault.unlock(PASSWORD).unwrap();
    let signature = vault.sign_unlocked(account.id, b"test").unwrap();
    assert!(vault.account(account.id).unwrap().backed_up);
    assert_eq!(
        vault
            .restore(BACKUP_PASSWORD, &backup, "duplicate")
            .unwrap_err(),
        "wallet_account_duplicate"
    );
    vault.lock();
    assert_eq!(
        vault.sign_unlocked(account.id, b"test").unwrap_err(),
        "wallet_locked"
    );
    drop(vault);
    let mut vault = Vault::with_store(directory.clone(), Box::new(store.clone())).unwrap();
    assert!(!vault.status().unlocked);
    assert!(vault.unlock("wrong-test-password").is_err());
    assert_eq!(
        vault.unlock(PASSWORD).unwrap_err(),
        "wallet_unlock_rate_limited"
    );
    vault.unlock_after = Instant::now();
    vault.unlock(PASSWORD).unwrap();
    vault.unlocked_at = Instant::now() - Duration::from_secs(301);
    assert_eq!(
        vault.sign_unlocked(account.id, b"test").unwrap_err(),
        "wallet_locked"
    );
    drop(vault);
    let mut replacement = Vault::with_store(
        root.path().join("replacement"),
        Box::<MemoryStore>::default(),
    )
    .unwrap();
    replacement.initialize(PASSWORD).unwrap();
    assert!(replacement
        .restore("wrong-test-password", &backup, "restored")
        .is_err());
    let restored = replacement
        .restore(BACKUP_PASSWORD, &backup, "restored")
        .unwrap();
    assert_eq!(restored.public_key, account.public_key);
    assert_eq!(
        replacement.sign_unlocked(restored.id, b"test").unwrap(),
        signature
    );
    let path = directory.join("vault-v1.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["accounts"][0]["name"] = "tampered".into();
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let mut tampered = Vault::with_store(directory, Box::new(store)).unwrap();
    assert!(tampered.unlock(PASSWORD).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(backup).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(root.path().join("replacement"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}

#[test]
fn every_signature_requires_a_fresh_password_even_when_management_is_unlocked() {
    let root = tempfile::tempdir().unwrap();
    let mut vault =
        Vault::with_store(root.path().join("vault"), Box::<MemoryStore>::default()).unwrap();
    vault.initialize(PASSWORD).unwrap();
    let account = vault.create("test account").unwrap();
    vault
        .backup(
            account.id,
            PASSWORD,
            BACKUP_PASSWORD,
            &root.path().join("backup.json"),
        )
        .unwrap();
    vault.unlock(PASSWORD).unwrap();
    assert!(vault.status().unlocked);
    assert!(vault
        .sign_with_password(account.id, b"first", "wrong-password")
        .is_err());
    assert!(!vault.status().unlocked);
    assert!(vault.account(account.id).unwrap().backed_up);
    assert_eq!(
        vault
            .sign_with_password(account.id, b"first", PASSWORD)
            .unwrap_err(),
        "wallet_unlock_rate_limited"
    );
    vault.unlock_after = Instant::now();
    let signature = vault
        .sign_with_password(account.id, b"first", PASSWORD)
        .unwrap();
    assert_eq!(signature.len(), 128);
    assert!(!vault.status().unlocked);
    vault.unlock_after = Instant::now();
    assert!(vault.sign_with_password(account.id, b"second", "").is_err());
    assert!(!vault.status().unlocked);
    assert!(
        !vault.expire(),
        "an already locked vault must not cancel pending confirmations"
    );
}

#[test]
fn ciphertext_aad_and_nonce_tampering_fail_closed() {
    let key = [7; 32];
    let sealed = crypto::seal(&key, b"account-one", b"secret").unwrap();
    assert!(crypto::open(&key, b"account-two", &sealed).is_err());
    let mut changed = sealed.clone();
    changed.ciphertext.replace_range(..2, "00");
    if changed.ciphertext == sealed.ciphertext {
        changed.ciphertext.replace_range(..2, "01");
    }
    assert!(crypto::open(&key, b"account-one", &changed).is_err());
    changed.nonce = "00".into();
    assert!(crypto::open(&key, b"account-one", &changed).is_err());
}

#[test]
fn unavailable_system_store_never_creates_plaintext_fallback() {
    struct Unavailable;
    impl KeyStore for Unavailable {
        fn load(&self, _: Uuid) -> Result<String> {
            Err("wallet_keystore_unavailable".into())
        }
        fn save(&self, _: Uuid, _: &str) -> Result<()> {
            Err("wallet_keystore_unavailable".into())
        }
    }
    let root = tempfile::tempdir().unwrap();
    let mut vault = Vault::with_store(root.path().join("vault"), Box::new(Unavailable)).unwrap();
    assert_eq!(
        vault.initialize(PASSWORD).unwrap_err(),
        "wallet_keystore_unavailable"
    );
    assert!(!vault.path.exists());
    assert!(!vault.status().initialized);
}
