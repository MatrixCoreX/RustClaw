use super::{
    document::LegacyDocument,
    tests::{MemoryStore, BACKUP_PASSWORD, PASSWORD},
    *,
};

fn legacy(directory: &std::path::Path, store: &MemoryStore) -> (Vec<Account>, Vec<u8>) {
    files::private_directory(directory).unwrap();
    let master = LockedKey::from_slice(&[7; 32]).unwrap();
    let id = Uuid::new_v4();
    let entries: Vec<_> = [1u8, 2]
        .into_iter()
        .map(|n| SecretAccount {
            account: Account {
                id: Uuid::new_v4(),
                name: format!("legacy {n}"),
                public_key: keys::public(&[n; 32]).unwrap(),
                backed_up: n == 1,
            },
            secret: [n; 32],
        })
        .collect();
    let accounts: Vec<_> = entries.iter().map(|e| e.account.clone()).collect();
    let aad = serde_json::to_vec(&("asset-vault-v1", id, &accounts)).unwrap();
    let doc = LegacyDocument {
        version: 1,
        id,
        accounts: accounts.clone(),
        sealed: crypto::seal(&master, &aad, &serde_json::to_vec(&entries).unwrap()).unwrap(),
    };
    store
        .save(
            id,
            &serde_json::to_string(
                &crypto::wrap(PASSWORD, id.as_bytes(), master.as_ref()).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let bytes = serde_json::to_vec(&doc).unwrap();
    files::write(&directory.join("vault-v1.json"), &bytes).unwrap();
    (accounts, bytes)
}

#[test]
fn legacy_upgrade_is_atomic_preserves_identity_and_reopens_without_rewriting() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("vault");
    let store = MemoryStore::default();
    let (accounts, old) = legacy(&directory, &store);
    let mut vault = Vault::with_store(directory.clone(), Box::new(store.clone())).unwrap();
    assert_eq!(vault.status().storage_version, 1);
    assert!(vault.unlock("wrong-password").is_err());
    assert_eq!(std::fs::read(&vault.path).unwrap(), old);
    vault.unlock_after = Instant::now();
    // Force a destination failure without modifying the real legacy file.
    let original = vault.path.clone();
    vault.path = root.path().join("missing/vault.json");
    assert!(vault.unlock(PASSWORD).is_err());
    assert!(vault.key.is_none());
    assert_eq!(vault.status().storage_version, 1);
    assert_eq!(std::fs::read(&original).unwrap(), old);
    vault.path = original;
    vault.unlock(PASSWORD).unwrap();
    assert_eq!(vault.status().storage_version, 2);
    assert_eq!(vault.status().accounts, accounts);
    assert_eq!(vault.status().backup_upgrade_accounts, vec![accounts[0].id]);
    assert_eq!(
        vault.sign_unlocked(accounts[0].id, b"migration").unwrap(),
        keys::sign(&[1; 32], b"migration").unwrap()
    );
    assert_eq!(*vault.selected_secret(accounts[1].id).unwrap(), [2; 32]);
    let upgraded = std::fs::read(&vault.path).unwrap();
    assert_ne!(upgraded, old);
    drop(vault);
    let mut vault = Vault::with_store(directory, Box::new(store)).unwrap();
    vault.unlock(PASSWORD).unwrap();
    assert_eq!(std::fs::read(&vault.path).unwrap(), upgraded);
    assert_eq!(vault.status().accounts, accounts);
}

#[test]
fn per_account_envelopes_bind_identity_and_only_selected_key_is_opened() {
    let master = LockedKey::random().unwrap();
    let mut doc = Document::empty(Uuid::new_v4());
    for n in [1u8, 2] {
        let secret = LockedKey::from_slice(&[n; 32]).unwrap();
        doc.insert(
            &master,
            Account {
                id: Uuid::new_v4(),
                name: n.to_string(),
                public_key: keys::public(&secret).unwrap(),
                backed_up: true,
            },
            &secret,
            2,
        )
        .unwrap();
    }
    doc.authenticate(&master).unwrap();
    doc.verify(&master).unwrap();
    let a = &doc.accounts[0];
    let b = &doc.accounts[1];
    let aad = |a: &Account| {
        serde_json::to_vec(&(
            "asset-vault-account-v2",
            doc.id,
            a.id,
            &a.public_key,
            "data-key",
        ))
        .unwrap()
    };
    let ka = crypto::open(&master, &aad(a), &doc.entries[0].wrapped_key).unwrap();
    let kb = crypto::open(&master, &aad(b), &doc.entries[1].wrapped_key).unwrap();
    assert_ne!(ka.as_ref(), kb.as_ref());
    assert!(crypto::open(&master, &aad(b), &doc.entries[0].wrapped_key).is_err());
    let id = a.id;
    let second_id = b.id;
    let before = serde_json::to_vec(&doc.entries).unwrap();
    assert_eq!(*doc.secret(&master, id).unwrap(), [1; 32]);
    assert_eq!(serde_json::to_vec(&doc.entries).unwrap(), before);
    // Even with an authenticated directory, accessing account 1 must never open
    // account 2. Only this test, which owns the master key, can authenticate this fault.
    doc.entries[1].secret.ciphertext = "00".repeat(48);
    doc.authenticate(&master).unwrap();
    doc.verify(&master).unwrap();
    assert_eq!(*doc.secret(&master, id).unwrap(), [1; 32]);
    assert!(doc.secret(&master, second_id).is_err());
    doc.entries.swap(0, 1);
    assert!(doc.verify(&master).is_err());
    assert!(Stored::read(&serde_json::to_vec(&doc).unwrap()).is_err());
}

#[test]
fn directory_rejects_changes_duplicates_unknown_fields_and_wrong_master() {
    let master = LockedKey::random().unwrap();
    let mut doc = Document::empty(Uuid::new_v4());
    let secret = LockedKey::from_slice(&[3; 32]).unwrap();
    doc.insert(
        &master,
        Account {
            id: Uuid::new_v4(),
            name: "protected".into(),
            public_key: keys::public(&secret).unwrap(),
            backed_up: false,
        },
        &secret,
        0,
    )
    .unwrap();
    doc.authenticate(&master).unwrap();
    let value = serde_json::to_value(&doc).unwrap();
    for path in ["name", "wrapped_key", "secret", "backup"] {
        let mut changed = value.clone();
        match path {
            "name" => changed["accounts"][0]["name"] = "changed".into(),
            "wrapped_key" | "secret" => {
                changed["entries"][0][path]["ciphertext"] = "00".repeat(48).into()
            }
            _ => {
                changed["accounts"][0]["backed_up"] = true.into();
                changed["entries"][0]["backup_version"] = 2.into();
            }
        }
        let loaded = Stored::read(&serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(loaded.current().unwrap().verify(&master).is_err(), "{path}");
    }
    assert!(doc.verify(&LockedKey::random().unwrap()).is_err());
    let bytes = serde_json::to_string(&doc).unwrap();
    assert!(Stored::read(bytes.replacen('{', "{\"version\":2,", 1).as_bytes()).is_err());
    assert!(Stored::read(bytes.replacen('{', "{\"unexpected\":true,", 1).as_bytes()).is_err());
}

#[test]
fn backup_strength_and_password_separation_prevent_export() {
    for weak in [
        "aaaaaaaaaaaaaaaa",
        "1234567890123456",
        "passwordpassword",
        "qwertyuiopasdfghjkl",
    ] {
        assert_eq!(
            backup_crypto::password_valid(weak, &[]).unwrap_err(),
            "wallet_backup_password_weak"
        );
    }
    for invalid in [
        "short".to_string(),
        "x".repeat(129),
        "sixteen-characters\n".into(),
    ] {
        assert_eq!(
            backup_crypto::password_valid(&invalid, &[]).unwrap_err(),
            "wallet_backup_password_length"
        );
    }
    backup_crypto::password_valid(BACKUP_PASSWORD, &[]).unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut vault =
        Vault::with_store(root.path().join("vault"), Box::<MemoryStore>::default()).unwrap();
    vault.initialize(PASSWORD).unwrap();
    let account = vault.create("example").unwrap();
    let dest = root.path().join("backup.json");
    files::write(&dest, b"keep previous backup").unwrap();
    for (pw, error) in [
        (PASSWORD, "wallet_backup_password_reused"),
        ("aaaaaaaaaaaaaaaa", "wallet_backup_password_weak"),
    ] {
        assert_eq!(
            vault.backup(account.id, PASSWORD, pw, &dest).unwrap_err(),
            error
        );
        assert_eq!(std::fs::read(&dest).unwrap(), b"keep previous backup");
        assert!(!vault.account(account.id).unwrap().backed_up);
        assert!(!vault.status().unlocked);
    }
}

#[test]
fn backup_v2_authenticates_profile_and_bounds_untrusted_work() {
    let secret = LockedKey::from_slice(&[4; 32]).unwrap();
    let public = keys::public(&secret).unwrap();
    let bytes = backup_crypto::seal(BACKUP_PASSWORD, &public, &secret).unwrap();
    let (restored, key, version) = backup_crypto::open(&bytes, BACKUP_PASSWORD).unwrap();
    assert_eq!(restored, public);
    assert_eq!(*key, *secret);
    assert_eq!(version, 2);
    assert!(backup_crypto::open(&bytes, "wrong-backup-password").is_err());
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for field in [
        "memory_kib",
        "iterations",
        "parallelism",
        "output_bytes",
        "version",
    ] {
        for invalid in [0u32, 1, u32::MAX] {
            let mut changed = value.clone();
            changed["kdf"][field] = invalid.into();
            assert!(
                backup_crypto::open(&serde_json::to_vec(&changed).unwrap(), BACKUP_PASSWORD)
                    .is_err(),
                "{field}:{invalid}"
            );
        }
    }
    for (field, replacement) in [
        ("public_key", keys::public(&[5; 32]).unwrap()),
        ("salt", "00".repeat(16)),
        ("ciphertext", "00".repeat(48)),
        ("nonce", "00".repeat(24)),
    ] {
        let mut changed = value.clone();
        match field {
            "public_key" => changed[field] = replacement.into(),
            "salt" => changed["kdf"][field] = replacement.into(),
            _ => changed["encrypted"][field] = replacement.into(),
        }
        assert!(
            backup_crypto::open(&serde_json::to_vec(&changed).unwrap(), BACKUP_PASSWORD).is_err(),
            "{field}"
        );
    }
    let text = String::from_utf8(bytes).unwrap();
    assert!(backup_crypto::open(
        text.replacen('{', "{\"format\":\"asset-account-backup-v2\",", 1)
            .as_bytes(),
        BACKUP_PASSWORD
    )
    .is_err());
    assert!(backup_crypto::open(
        text.replacen('{', "{\"extra\":1,", 1).as_bytes(),
        BACKUP_PASSWORD
    )
    .is_err());
    assert!(backup_crypto::open(&vec![b' '; 16385], BACKUP_PASSWORD).is_err());
    assert!(backup_crypto::open(&text.as_bytes()[..30], BACKUP_PASSWORD).is_err());
}

#[test]
fn legacy_backup_restores_with_original_password_and_requests_new_backup() {
    let root = tempfile::tempdir().unwrap();
    let secret = LockedKey::from_slice(&[6; 32]).unwrap();
    let public = keys::public(&secret).unwrap();
    let weak = "aaaaaaaaaaaa";
    let aad = format!("asset-account-backup-v1:{public}");
    let backup = serde_json::json!({"format":"asset-account-backup-v1","public_key":public,"encrypted":crypto::wrap(weak,aad.as_bytes(),secret.as_ref()).unwrap()});
    let bytes = serde_json::to_vec(&backup).unwrap();
    let path = root.path().join("legacy.json");
    files::write(&path, &bytes).unwrap();
    let mut vault =
        Vault::with_store(root.path().join("vault"), Box::<MemoryStore>::default()).unwrap();
    vault.initialize(PASSWORD).unwrap();
    let account = vault.restore(weak, &path, "legacy restore").unwrap();
    assert_eq!(account.public_key, public);
    assert!(account.backed_up);
    assert_eq!(vault.status().backup_upgrade_accounts, vec![account.id]);
    let new = root.path().join("new.json");
    vault
        .backup(account.id, PASSWORD, BACKUP_PASSWORD, &new)
        .unwrap();
    assert!(vault.status().backup_upgrade_accounts.is_empty());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    assert_eq!(
        backup_crypto::open(&std::fs::read(new).unwrap(), BACKUP_PASSWORD)
            .unwrap()
            .2,
        2
    );
}
