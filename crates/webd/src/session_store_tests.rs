use std::collections::HashMap;
use std::fs;

use super::*;

fn test_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "agent-runtime-webd-session-{label}-{}-{}.json",
        std::process::id(),
        uuid::Uuid::new_v4()
    ))
}

#[test]
fn persisted_sessions_survive_reload_and_expired_entries_are_dropped() {
    let path = test_path("reload");
    let active_id = uuid::Uuid::new_v4().to_string();
    let expired_id = uuid::Uuid::new_v4().to_string();
    let sessions = HashMap::from([
        (
            active_id.clone(),
            SessionEntry {
                user_key: "rk-active".to_string(),
                expires_unix: 200,
            },
        ),
        (
            expired_id,
            SessionEntry {
                user_key: "rk-expired".to_string(),
                expires_unix: 99,
            },
        ),
    ]);

    persist_sessions(&path, &sessions).expect("persist sessions");
    let restored = load_sessions(&path, 100).expect("load sessions");
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[&active_id].user_key, "rk-active");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    fs::remove_file(path).ok();
}

#[test]
fn malformed_session_store_is_rejected_without_partial_restore() {
    let path = test_path("malformed");
    fs::write(&path, b"not-json").expect("write malformed store");
    assert!(load_sessions(&path, 100).is_err());
    fs::remove_file(path).ok();
}
