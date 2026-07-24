use super::*;
use crate::{DocMeta, KbRuntime, NamespaceIndex};
use std::collections::HashMap;

fn runtime(root: &Path, user_key: &str) -> KbRuntime {
    KbRuntime {
        scope_user_key: user_key.to_string(),
        workspace_root: root.to_path_buf(),
        storage_database_path: root.join("data/skills/kb/state.db"),
        storage_busy_timeout_ms: 5_000,
    }
}

#[test]
fn namespace_storage_is_user_scoped_inside_one_skill_database() {
    let root =
        std::env::temp_dir().join(format!("rustclaw-kb-storage-users-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let alpha = runtime(&root, "rk-alpha");
    let beta = runtime(&root, "rk-beta");
    initialize(&alpha).expect("initialize");
    save_namespace(
        &alpha,
        &NamespaceIndex {
            namespace: "docs".to_string(),
            owner_user_key: "rk-alpha".to_string(),
            updated_at_epoch: 1,
            next_chunk_seq: 1,
            docs: HashMap::<String, DocMeta>::new(),
            chunks: Vec::new(),
        },
    )
    .expect("save alpha");
    assert!(namespace_exists(&alpha, "docs").expect("alpha exists"));
    assert!(!namespace_exists(&beta, "docs").expect("beta isolated"));
    assert!(load_namespace(&beta, "docs").is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn legacy_json_migrates_once_and_is_physically_removed() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-kb-storage-migrate-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let legacy = root.join("data/kb/by_user/owner");
    fs::create_dir_all(&legacy).expect("legacy directory");
    let snapshot = NamespaceIndex {
        namespace: "manuals".to_string(),
        owner_user_key: "rk-owner".to_string(),
        updated_at_epoch: 7,
        next_chunk_seq: 1,
        docs: HashMap::new(),
        chunks: Vec::new(),
    };
    let legacy_file = legacy.join("manuals.json");
    fs::write(
        &legacy_file,
        serde_json::to_string_pretty(&snapshot).expect("snapshot"),
    )
    .expect("legacy file");
    let runtime = runtime(&root, "rk-owner");
    initialize(&runtime).expect("first migration");
    initialize(&runtime).expect("second start");
    assert!(!legacy_file.exists());
    assert_eq!(
        load_namespace(&runtime, "manuals")
            .expect("migrated namespace")
            .updated_at_epoch,
        7
    );
    let _ = fs::remove_dir_all(root);
}
