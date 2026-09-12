#[cfg(windows)]
#[path = "peer_memory_windows.rs"]
mod peer_memory_windows;
// Run against the installed executable, with disposable native credentials.
use super::*;
use crate::wallet::{backup_crypto, tests::PASSWORD};
use serde_json::{json, Value};
use std::{
    fs,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
const BACKUP: &str = "Jasper!flume7-Pebble4-Orbit9-velvet";

fn client(directory: &Path) -> VaultClient {
    let exe =
        std::env::var_os("DESKTOP_WALLET_TEST_EXE").expect("installed test executable required");
    let mut value = VaultClient {
        worker: None,
        input: None,
        output: None,
        #[cfg(windows)]
        guard: None,
        next: 0,
        failure: None,
        was_unlocked: false,
        revoked: false,
    };
    value
        .start_executable(directory.into(), Path::new(&exe))
        .unwrap();
    value
}
fn evidence(name: &str, report: Value) {
    let root = PathBuf::from(
        std::env::var_os("DESKTOP_WALLET_TEST_OUTPUT").expect("evidence path required"),
    );
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join(name), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}
fn challenge(
    account: &Account,
    recipient: &Account,
    service: &str,
) -> (Capabilities, Intent, String) {
    let cap = json!({"schema_version":1,"protocol":"asset_owner_v1","ledger_id":"native-fixture",
        "node_url":"https://ledger.example.test","service":service,"actions":[if service=="assets" {"transfer"} else {"bancor_trade"}]});
    let intent = if service == "assets" {
        json!({"kind":"transfer","asset":"AIC","amount_units":"100000000",
        "recipient":recipient.public_key,"memo":"fixture only","max_fee_bps":0})
    } else {
        json!({"kind":"bancor_trade","side":"buy","input_units":"100000000","slippage_bps":100,"max_fee_bps":0})
    };
    let mut terms = intent.clone();
    terms["fee_units"] = "0".into();
    if service == "bancor" {
        terms["quoted_output_units"] = "100000000".into();
        terms["min_output_units"] = "99000000".into();
    }
    let payload = json!({"schema_version":1,"protocol":"asset_owner_v1","ledger_id":"native-fixture",
        "node_url":"https://ledger.example.test","service":service,"account":account.public_key,
        "operation_id":Uuid::new_v4(),"challenge_id":Uuid::new_v4(),"nonce":"01".repeat(32),
        "expires_at_unix":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()+120,"terms":terms});
    (
        serde_json::from_value(cap).unwrap(),
        serde_json::from_value(intent).unwrap(),
        payload.to_string(),
    )
}
fn verify(account: &Account, payload: &str, signature: &str) {
    use k256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
    let public = bs58::decode(&account.public_key).into_vec().unwrap();
    let key = VerifyingKey::from_sec1_bytes(&public[..33]).unwrap();
    let signature = Signature::from_slice(&hex::decode(signature).unwrap()).unwrap();
    assert!(signature.normalize_s().is_none());
    key.verify(payload.as_bytes(), &signature).unwrap();
}

#[test]
#[ignore = "Requires packaged binary and disposable unlocked OS credential store"]
fn installed_worker_native_vault_backups_and_signatures() {
    #[cfg(any(windows, target_os = "macos"))]
    assert!(
        !crate::wallet::native_session::locked(),
        "native CI session must permit wallet interaction"
    );
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("vault 测试");
    let mut worker = client(&directory);
    assert!(!worker.status().unwrap().initialized);
    worker.initialize(PASSWORD).unwrap();
    let first = worker.create("native first").unwrap();
    let second = worker.create("native second").unwrap();
    assert_ne!(first.public_key, second.public_key);
    let vault_path = directory.join("vault-v1.json");
    let before: Value = serde_json::from_slice(&fs::read(&vault_path).unwrap()).unwrap();
    assert_eq!(before["version"], 2);
    peer_memory_denied(worker.worker.as_ref().unwrap().id());
    let backup_path = root.path().join("native.backup.json");
    let started = Instant::now();
    worker
        .backup(first.id, PASSWORD, BACKUP, &backup_path)
        .unwrap();
    let backup_seconds = started.elapsed().as_secs_f64();
    assert!(!worker.status().unwrap().unlocked);
    let after: Value = serde_json::from_slice(&fs::read(&vault_path).unwrap()).unwrap();
    assert_eq!(before["entries"][1], after["entries"][1]);
    let encrypted = fs::read(&backup_path).unwrap();
    assert_eq!(
        backup_crypto::open(&encrypted, BACKUP).unwrap().0,
        first.public_key
    );
    let mut restored = client(&root.path().join("restored"));
    restored.initialize(PASSWORD).unwrap();
    let restored_first = restored
        .restore(BACKUP, &backup_path, "native restored")
        .unwrap();
    assert_eq!(restored_first.public_key, first.public_key);
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut fixture_keys = Vec::new();
    for (file, password) in [
        ("wallet-linux-v2.json", BACKUP),
        ("wallet-linux-v1.json", "aaaaaaaaaaaa"),
    ] {
        let account = restored
            .restore(password, &fixtures.join(file), file)
            .unwrap();
        let expected: Value =
            serde_json::from_slice(&fs::read(fixtures.join(file)).unwrap()).unwrap();
        assert_eq!(account.public_key, expected["public_key"].as_str().unwrap());
        fixture_keys.push(account.public_key.clone());
        let (cap, intent, payload) = challenge(&account, &second, "assets");
        let signature = restored
            .sign_with_password(account.id, payload.as_bytes(), PASSWORD, cap, intent)
            .unwrap();
        verify(&account, &payload, &signature);
        restored.unlock(PASSWORD).unwrap();
    }
    for service in ["assets", "bancor"] {
        let (cap, intent, payload) = challenge(&first, &second, service);
        let mut tampered: Value = serde_json::from_str(&payload).unwrap();
        tampered["account"] = second.public_key.clone().into();
        assert!(worker
            .sign_with_password(
                first.id,
                tampered.to_string().as_bytes(),
                PASSWORD,
                cap.clone(),
                intent.clone()
            )
            .is_err());
        let signature = worker
            .sign_with_password(first.id, payload.as_bytes(), PASSWORD, cap, intent)
            .unwrap();
        verify(&first, &payload, &signature);
        assert!(!worker.status().unwrap().unlocked);
    }
    worker.unlock(PASSWORD).unwrap();
    let original = fs::read(&vault_path).unwrap();
    drop(worker);
    let mut reopened = client(&directory);
    assert!(!reopened.status().unwrap().unlocked);
    reopened.unlock(PASSWORD).unwrap();
    assert_eq!(fs::read(&vault_path).unwrap(), original);
    reopened.lock();
    assert!(!reopened.status().unwrap().unlocked);
    drop(reopened);
    let mut altered: Value = serde_json::from_slice(&original).unwrap();
    altered["accounts"][0]["name"] = "tampered".into();
    fs::write(&vault_path, serde_json::to_vec(&altered).unwrap()).unwrap();
    let mut tampered = client(&directory);
    assert!(tampered.unlock(PASSWORD).is_err());
    assert!(!tampered.status().unwrap().unlocked);
    let output = PathBuf::from(std::env::var_os("DESKTOP_WALLET_TEST_OUTPUT").unwrap());
    fs::create_dir_all(&output).unwrap();
    fs::copy(&backup_path, output.join("native.backup.json")).unwrap();
    evidence(
        "wallet-native-security.json",
        json!({"ok":true,"os":std::env::consts::OS,
        "arch":std::env::consts::ARCH,"backup_seconds":backup_seconds,"fixture_public_keys":fixture_keys,
        "checks":["packaged_worker_os_keystore","two_independent_account_envelopes","peer_memory_read_denied",
        "v2_backup_and_readback","v2_restore_native","linux_v1_v2_restore_and_signature",
        "fresh_password_transfer_bancor_signatures","tampered_challenge_rejected","sign_and_export_lock",
        "locked_restart_stable_ciphertext","tampered_directory_rejected"]}),
    );
}

fn peer_memory_denied(pid: u32) {
    #[cfg(windows)]
    peer_memory_windows::check(pid);
    #[cfg(target_os = "linux")]
    assert!(fs::File::open(format!("/proc/{pid}/mem")).is_err());
    #[cfg(target_os = "macos")]
    unsafe {
        let mut task = 0;
        assert_ne!(
            libc::task_for_pid(libc::mach_task_self(), pid as i32, &mut task),
            0
        );
    }
}

#[test]
#[ignore = "Requires packaged binary and disposable OS session"]
fn installed_worker_rejects_unframed_unknown_replayed_and_oversized_input() {
    let root = tempfile::tempdir().unwrap();
    for (n, raw) in [
        b"{\"version\":1,\"id\":2,\"request\":{\"operation\":\"export_private_key\"}}".as_slice(),
        b"{\"version\":1,\"id\":2,\"request\":{\"operation\":\"status\",\"password\":\"extra\"}}",
        b"{\"version\":1,\"id\":1,\"request\":{\"operation\":\"status\"}}",
    ]
    .iter()
    .enumerate()
    {
        let mut worker = client(&root.path().join(n.to_string()));
        transport::write(worker.input.as_mut().unwrap(), raw, Duration::from_secs(2)).unwrap();
        assert!(transport::read(worker.output.as_mut().unwrap(), Duration::from_secs(5)).is_err());
    }
    let mut worker = client(&root.path().join("oversized"));
    use std::io::Write;
    worker
        .input
        .as_mut()
        .unwrap()
        .write_all(&131073u32.to_be_bytes())
        .unwrap();
    assert!(transport::read(worker.output.as_mut().unwrap(), Duration::from_secs(5)).is_err());
    evidence(
        "wallet-native-ipc.json",
        json!({"ok":true,"checks":["no_private_key_export_command",
        "unknown_field_rejected","replay_rejected","oversized_frame_rejected"]}),
    );
}

#[path = "parent_tests.rs"]
mod parent_tests;
