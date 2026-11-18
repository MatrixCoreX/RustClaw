use super::*;
use std::fs;

pub(super) const FIRST: &str =
    "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-before\n+after\n";
const DIFFERENT: &str =
    "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-before\n+different\n";
const NEXT: &str =
    "diff --git a/b.txt b/b.txt\n--- a/b.txt\n+++ b/b.txt\n@@ -1 +1 @@\n-before\n+after\n";

pub(super) struct Fixture {
    _temporary: tempfile::TempDir,
    pub(super) root: PathBuf,
    filesystem: HostFilesystemPolicy,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        fs::write(root.join("a.txt"), b"before\n").unwrap();
        fs::write(root.join("b.txt"), b"before\n").unwrap();
        Self {
            _temporary: temporary,
            root,
            filesystem: HostFilesystemPolicy::without_private_authority(),
        }
    }

    pub(super) fn run(&self, args: Value) -> Result<Value, String> {
        let output = super::super::execute_workspace_patch_with_policy(
            &self.root,
            "owned-shard-task",
            args.as_object().unwrap(),
            &self.filesystem,
            ToolSandboxBackend::Auto,
        )?;
        Ok(serde_json::from_str(&output).unwrap())
    }

    pub(super) fn request(&self, patch: &str) -> Value {
        json!({"action":"apply_patch_shard","transaction_id":"owned_shards",
            "shard_index":0,"shard_count":2,"patch":patch})
    }

    pub(super) fn apply(&self) -> Value {
        self.run(self.request(FIRST)).unwrap()
    }

    fn rewind(&self) -> Result<Value, String> {
        self.run(json!({"action":"transaction_rewind","transaction_id":"owned_shards"}))
    }

    pub(super) fn manifest_path(&self) -> PathBuf {
        super::super::checkpoint_root(&self.root).join("patch-transactions/owned_shards.json")
    }
}

pub(super) fn error_code(result: Result<Value, String>) -> String {
    let error = result.expect_err("must not report an unapplied shard as applied");
    crate::skills::parse_structured_skill_error(&error)
        .unwrap()
        .error_code
}

#[test]
fn shard_replay_identical_request_is_idempotent_without_another_checkpoint() {
    let fixture = Fixture::new();
    let first = fixture.apply();
    let before = fs::read(fixture.manifest_path()).unwrap();
    let repeated = fixture.run(fixture.request(FIRST)).unwrap();
    assert_eq!(repeated["outcome"], "already_applied");
    assert_eq!(repeated["current_shard"], first["current_shard"]);
    assert_eq!(fs::read(fixture.manifest_path()).unwrap(), before);
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"after\n");
}

#[test]
fn shard_replay_refuses_a_rewound_transaction() {
    let fixture = Fixture::new();
    fixture.apply();
    fixture.rewind().unwrap();
    let before = fs::read(fixture.manifest_path()).unwrap();
    assert_eq!(
        error_code(fixture.run(fixture.request(FIRST))),
        "transaction_not_active"
    );
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"before\n");
    assert_eq!(fs::read(fixture.manifest_path()).unwrap(), before);
}

#[test]
fn shard_replay_refuses_a_different_patch_for_the_same_position() {
    let fixture = Fixture::new();
    fixture.apply();
    assert_eq!(
        error_code(fixture.run(fixture.request(DIFFERENT))),
        "shard_request_conflict"
    );
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"after\n");
}

#[test]
fn shard_replay_refuses_changed_preconditions_for_the_same_position() {
    let fixture = Fixture::new();
    fixture.apply();
    let mut request = fixture.request(FIRST);
    request["precondition_hashes"] = json!({"a.txt":"missing"});
    assert_eq!(error_code(fixture.run(request)), "shard_request_conflict");
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"after\n");
}

#[test]
fn shard_replay_refuses_user_changes_without_overwriting_them() {
    let fixture = Fixture::new();
    fixture.apply();
    fs::write(fixture.root.join("a.txt"), b"user-owned edit\n").unwrap();
    assert_eq!(
        error_code(fixture.run(fixture.request(FIRST))),
        "shard_state_conflict"
    );
    assert_eq!(
        fs::read(fixture.root.join("a.txt")).unwrap(),
        b"user-owned edit\n"
    );
}

#[test]
fn shard_replay_refuses_an_independently_rewound_checkpoint() {
    let fixture = Fixture::new();
    let applied = fixture.apply();
    fixture
        .run(json!({"action":"rewind", "checkpoint_id":applied["current_shard"]["checkpoint_id"]}))
        .unwrap();
    assert_eq!(
        error_code(fixture.run(fixture.request(FIRST))),
        "shard_state_conflict"
    );
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"before\n");
}

#[test]
fn shard_replay_refuses_missing_checkpoint_evidence() {
    let fixture = Fixture::new();
    let applied = fixture.apply();
    let checkpoint = super::super::checkpoint_root(&fixture.root)
        .join(applied["current_shard"]["checkpoint_id"].as_str().unwrap());
    fs::remove_file(checkpoint.join("manifest.json")).unwrap();
    assert!(fixture.run(fixture.request(FIRST)).is_err());
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"after\n");
}

#[test]
fn shard_replay_rechecks_the_current_private_filesystem_policy() {
    use std::os::unix::fs::PermissionsExt;
    let mut fixture = Fixture::new();
    fixture.apply();
    let private = super::super::checkpoint_root(&fixture.root);
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
    fixture.filesystem = HostFilesystemPolicy::pin_directories(&[private], &fixture.root).unwrap();
    assert!(fixture.run(fixture.request(FIRST)).is_err());
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"after\n");
}

#[test]
fn shard_replay_does_not_continue_a_failed_rollback() {
    let fixture = Fixture::new();
    fixture.apply();
    fs::write(fixture.root.join("a.txt"), b"user-owned edit\n").unwrap();
    assert_eq!(error_code(fixture.rewind()), "transaction_rollback_failed");
    let request = json!({"action":"apply_patch_shard","transaction_id":"owned_shards",
        "shard_index":1,"shard_count":2,"patch":NEXT});
    assert_eq!(error_code(fixture.run(request)), "transaction_not_active");
    assert_eq!(
        fs::read(fixture.root.join("a.txt")).unwrap(),
        b"user-owned edit\n"
    );
    assert_eq!(fs::read(fixture.root.join("b.txt")).unwrap(), b"before\n");
}

#[test]
fn shard_replay_accepts_composed_state_after_later_shards_touch_the_same_file() {
    let fixture = Fixture::new();
    fixture.apply();
    let later = FIRST.replace("-before\n+after", "-after\n+final");
    let applied = fixture
        .run(
            json!({"action":"apply_patch_shard","transaction_id":"owned_shards",
        "shard_index":1,"shard_count":2,"patch":later}),
        )
        .unwrap();
    assert_eq!(applied["complete"], true);
    let repeated = fixture.run(fixture.request(FIRST)).unwrap();
    assert_eq!(repeated["outcome"], "already_applied");
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"final\n");
}

#[test]
fn shard_replay_missing_request_binding_keeps_historical_status_and_rewind() {
    let fixture = Fixture::new();
    fixture.apply();
    let path = fixture.manifest_path();
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["shards"][0]
        .as_object_mut()
        .unwrap()
        .remove("request_digest");
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let status = fixture
        .run(json!({"action":"transaction_status","transaction_id":"owned_shards"}))
        .unwrap();
    assert_eq!(status["action"], "transaction_status");
    assert_eq!(
        error_code(fixture.run(fixture.request(FIRST))),
        "shard_request_binding_missing"
    );
    let rewound = fixture.rewind().unwrap();
    assert_eq!(rewound["action"], "transaction_rewind");
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"before\n");
}

#[test]
fn shard_replay_equivalent_empty_preconditions_keep_the_same_request_binding() {
    let fixture = Fixture::new();
    fixture.apply();
    let mut request = fixture.request(FIRST);
    request["precondition_hashes"] = json!({});
    assert_eq!(fixture.run(request).unwrap()["outcome"], "already_applied");
}

#[test]
fn shard_replay_rechecks_newly_private_payload_files_not_only_the_manifest() {
    use std::os::unix::fs::PermissionsExt;
    let mut fixture = Fixture::new();
    let private = fixture.root.join("material");
    fs::create_dir(&private).unwrap();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(private.join("a.txt"), b"before\n").unwrap();
    let request = fixture.request(&FIRST.replace("a.txt", "material/a.txt"));
    fixture.run(request.clone()).unwrap();
    fixture.filesystem =
        HostFilesystemPolicy::pin_directories(&[private.clone()], &fixture.root).unwrap();
    assert_eq!(
        error_code(fixture.run(request)),
        "host_private_directory_access_denied"
    );
    assert_eq!(fs::read(private.join("a.txt")).unwrap(), b"after\n");
}

#[test]
fn shard_replay_known_pre_dispatch_refusal_allows_a_corrected_next_shard() {
    let fixture = Fixture::new();
    fixture.apply();
    assert!(fixture
        .run(
            json!({"action":"apply_patch_shard","transaction_id":"owned_shards",
        "shard_index":1,"shard_count":2,"patch":"not a patch"})
        )
        .is_err());
    let status = fixture
        .run(json!({"action":"transaction_status","transaction_id":"owned_shards"}))
        .unwrap();
    assert_eq!(status["state"], "partial_rejected");
    assert_eq!(status["continuation"]["next_shard_index"], 1);
    let corrected = fixture
        .run(
            json!({"action":"apply_patch_shard","transaction_id":"owned_shards",
        "shard_index":1,"shard_count":2,"patch":NEXT}),
        )
        .unwrap();
    assert_eq!(corrected["complete"], true);
}

#[test]
fn shard_replay_unknown_failure_exposes_no_automatic_continuation() {
    let fixture = Fixture::new();
    fixture.apply();
    let path = fixture.manifest_path();
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    // A retained earlier failure has no proven pre-dispatch/no-effect evidence.
    manifest["state"] = json!("partial_failed");
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let status = fixture
        .run(json!({"action":"transaction_status","transaction_id":"owned_shards"}))
        .unwrap();
    assert!(status["continuation"].is_null());
    let error = fixture.run(fixture.request(FIRST)).unwrap_err();
    let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
    assert_eq!(parsed.error_code, "transaction_not_active");
    let extra = parsed.extra.unwrap();
    assert_eq!(extra["retryable"], false);
    assert_eq!(extra["recovery"]["auto_retry"], false);
    assert_ne!(extra["side_effect_applied"], false);
}
