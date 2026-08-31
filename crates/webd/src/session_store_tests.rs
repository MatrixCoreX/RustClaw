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
    let active_secret = uuid::Uuid::new_v4().to_string();
    let active_id = crate::session_id_digest(&active_secret);
    let expired_id = crate::session_id_digest(&uuid::Uuid::new_v4().to_string());
    let sessions = HashMap::from([
        (
            active_id.clone(),
            SessionEntry {
                user_key: "rk-active".to_string(),
                session_handle: uuid::Uuid::new_v4().to_string(),
                username: "active".to_string(),
                role: "admin".to_string(),
                client_ip: "192.0.2.10".to_string(),
                client_platform: "FixtureOS".to_string(),
                user_agent: "fixture-browser/1.0".to_string(),
                created_unix: 50,
                last_activity_unix: 100,
                expires_unix: 200,
                csrf_token: "01".repeat(16),
            },
        ),
        (
            expired_id,
            SessionEntry {
                user_key: "rk-expired".to_string(),
                session_handle: uuid::Uuid::new_v4().to_string(),
                username: "expired".to_string(),
                role: "user".to_string(),
                client_ip: "192.0.2.11".to_string(),
                client_platform: "FixtureOS".to_string(),
                user_agent: "fixture-browser/1.0".to_string(),
                created_unix: 50,
                last_activity_unix: 90,
                expires_unix: 99,
                csrf_token: "02".repeat(16),
            },
        ),
    ]);

    persist_sessions(&path, &sessions).expect("persist sessions");
    let persisted = fs::read_to_string(&path).expect("read persisted sessions");
    assert!(!persisted.contains(&active_secret));
    let restored = load_sessions(&path, 100).expect("load sessions");
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[&active_id].user_key, "rk-active");
    assert_eq!(restored[&active_id].csrf_token, "01".repeat(16));
    assert_eq!(restored[&active_id].client_ip, "192.0.2.10");

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
fn schema_three_sessions_load_without_device_metadata() {
    let path = test_path("schema-three");
    let session_id = crate::session_id_digest(&uuid::Uuid::new_v4().to_string());
    let mut sessions = serde_json::Map::new();
    sessions.insert(
        session_id.clone(),
        serde_json::json!({
            "user_key": "rk-existing",
            "session_handle": uuid::Uuid::new_v4().to_string(),
            "username": "existing",
            "role": "admin",
            "created_unix": 50,
            "last_activity_unix": 100,
            "expires_unix": 200,
            "csrf_token": "03".repeat(16)
        }),
    );
    fs::write(
        &path,
        serde_json::json!({
            "schema_version": 3,
            "sessions": sessions
        })
        .to_string(),
    )
    .expect("write schema-three store");

    let restored = load_sessions(&path, 100).expect("load schema-three store");
    assert_eq!(restored.len(), 1);
    assert!(restored[&session_id].client_ip.is_empty());
    assert!(restored[&session_id].client_platform.is_empty());
    assert!(restored[&session_id].user_agent.is_empty());
    fs::remove_file(path).ok();
}

#[test]
fn legacy_or_tokenless_session_stores_fail_closed() {
    let path = test_path("legacy");
    fs::write(
        &path,
        serde_json::json!({
            "schema_version": 2,
            "sessions": {
                "00000000-0000-4000-8000-000000000001": {
                    "user_key": "rk-legacy",
                    "expires_unix": 200
                }
            }
        })
        .to_string(),
    )
    .expect("write legacy store");
    assert!(load_sessions(&path, 100).is_err());
    fs::remove_file(path).ok();
}

#[test]
fn malformed_session_store_is_rejected_without_partial_restore() {
    let path = test_path("malformed");
    fs::write(&path, b"not-json").expect("write malformed store");
    assert!(load_sessions(&path, 100).is_err());
    fs::remove_file(path).ok();
}

#[cfg(unix)]
#[test]
fn session_store_rejects_symbolic_links_without_touching_the_target() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let path = test_path("symlink");
    let target = test_path("symlink-target");
    fs::write(&target, b"target").expect("write target");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o644))
        .expect("set target permissions");
    symlink(&target, &path).expect("create session-store symlink");

    assert!(load_sessions(&path, 100).is_err());
    assert!(persist_sessions(&path, &HashMap::new()).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"target");
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o644
    );
    fs::remove_file(path).ok();
    fs::remove_file(target).ok();
}
