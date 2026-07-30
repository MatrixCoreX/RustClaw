use super::*;
use crate::{DeprecatedEntry, RssMachineState, RssRuntime, SourceStateEntry};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempStorage {
    root: PathBuf,
}

impl TempStorage {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "agent-runtime-rss-storage-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("rss_fetch")).expect("create storage root");
        Self { root }
    }

    fn runtime(&self) -> RssRuntime {
        RssRuntime {
            storage_database_path: self.root.join("rss_fetch/state.db"),
            storage_busy_timeout_ms: 5_000,
        }
    }
}

impl Drop for TempStorage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn legacy_state() -> RssMachineState {
    RssMachineState {
        source_states: BTreeMap::from([(
            "general".to_string(),
            vec![SourceStateEntry {
                url: "https://example.com/feed".to_string(),
                failure_count: 1,
                last_error: "timeout".to_string(),
                last_failed_at: "1".to_string(),
            }],
        )]),
        candidates: BTreeMap::new(),
        deprecated: vec![DeprecatedEntry {
            url: "https://old.example.com/feed".to_string(),
            category: "general".to_string(),
            reason: "consecutive_fetch_failures".to_string(),
            failure_count: 3,
            last_error: "timeout".to_string(),
            deprecated_at: "2".to_string(),
        }],
        pending_categories: BTreeMap::new(),
    }
}

#[test]
fn legacy_machine_state_migrates_once_and_verifies() {
    let temp = TempStorage::new("migration");
    let runtime = temp.runtime();
    let legacy = legacy_state();

    let first = initialize_and_load(&runtime, &legacy).expect("migrate legacy state");
    assert_eq!(first.state, legacy);
    assert!(first.cleanup_legacy_config);

    let second = initialize_and_load(&runtime, &legacy).expect("repeat migration");
    assert_eq!(second.state, legacy);
    assert!(second.cleanup_legacy_config);
}

#[test]
fn private_state_save_rejects_stale_expected_snapshot() {
    let temp = TempStorage::new("conflict");
    let runtime = temp.runtime();
    let initial = RssMachineState::default();
    initialize_and_load(&runtime, &initial).expect("initialize storage");

    let updated = legacy_state();
    save_if_unchanged(&runtime, &initial, &updated).expect("first update");
    let stale_error =
        save_if_unchanged(&runtime, &initial, &RssMachineState::default()).unwrap_err();

    assert_eq!(stale_error, "storage_write_conflict");
}

#[test]
fn storage_requires_absolute_state_database_identity() {
    let relative = RssRuntime {
        storage_database_path: PathBuf::from("data/skills/rss_fetch/state.db"),
        storage_busy_timeout_ms: 5_000,
    };
    assert_eq!(
        initialize_and_load(&relative, &RssMachineState::default()).unwrap_err(),
        "storage_path_not_absolute"
    );

    let temp = TempStorage::new("identity");
    let invalid = RssRuntime {
        storage_database_path: temp.root.join("rss_fetch/rss.db"),
        storage_busy_timeout_ms: 5_000,
    };
    assert_eq!(
        initialize_and_load(&invalid, &RssMachineState::default()).unwrap_err(),
        "storage_database_identity_invalid"
    );
}

#[test]
fn compatible_state_from_older_schema_verifies_persisted_payload_bytes() {
    let temp = TempStorage::new("compatible-schema");
    let runtime = temp.runtime();
    let db = open(&runtime).expect("open storage");
    ensure_schema(&db).expect("create schema");

    // `pending_categories` was added with `#[serde(default)]`. An older
    // payload without that field remains valid, but decode + re-encode would
    // add the field and therefore must not be used for integrity validation.
    let payload = r#"{"source_states":{},"candidates":{},"deprecated":[]}"#;
    db.execute(
        "INSERT INTO rss_machine_state (id, payload_json, payload_sha256)
         VALUES (?1, ?2, ?3)",
        params![STATE_ROW_ID, payload, payload_digest(payload.as_bytes())],
    )
    .expect("seed compatible older payload");

    let loaded = initialize_and_load(&runtime, &RssMachineState::default())
        .expect("load compatible older payload");
    assert_eq!(loaded.state, RssMachineState::default());
}
