use super::*;

const CHECKPOINT: &str = "patch_restore_metadata_owned";
const FIRST_BEFORE: &[u8] = b"one-before\n";
const SECOND_BEFORE: &[u8] = b"two-before\n";
const FIRST_AFTER: &[u8] = b"one-after\n";
const SECOND_AFTER: &[u8] = b"two-after\n";

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    checkpoint: PathBuf,
    files: Vec<CheckpointFile>,
    filesystem: HostFilesystemPolicy,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let filesystem = HostFilesystemPolicy::without_private_authority();
        fs::write(root.join("first.txt"), FIRST_BEFORE).unwrap();
        fs::write(root.join("second.txt"), SECOND_BEFORE).unwrap();
        let checkpoint = create_checkpoint_dir(&root, CHECKPOINT, &filesystem).unwrap();
        let mut files = snapshot_files(
            &root,
            &checkpoint,
            &[stat("first.txt"), stat("second.txt")],
            &filesystem,
        )
        .unwrap();
        fs::write(root.join("first.txt"), FIRST_AFTER).unwrap();
        fs::write(root.join("second.txt"), SECOND_AFTER).unwrap();
        files[0].after_sha256 = Some(sha256_label(FIRST_AFTER));
        files[1].after_sha256 = Some(sha256_label(SECOND_AFTER));
        let fixture = Self {
            _temporary: temporary,
            root,
            checkpoint,
            files,
            filesystem,
        };
        fixture.persist(&fixture.files);
        fixture
    }

    fn persist(&self, files: &[CheckpointFile]) {
        write_manifest(
            &self.checkpoint,
            &PatchCheckpoint {
                schema_version: CHECKPOINT_SCHEMA_VERSION,
                checkpoint_id: CHECKPOINT.into(),
                task_id: "owned-restore-metadata".into(),
                patch_id: sha256_label(b"owned-patch"),
                state: "applied".into(),
                created_at: 0,
                files: files.to_vec(),
            },
            &self.filesystem,
        )
        .unwrap();
    }

    fn assert_payloads_untouched(&self) {
        assert_eq!(fs::read(self.root.join("first.txt")).unwrap(), FIRST_AFTER);
        assert_eq!(
            fs::read(self.root.join("second.txt")).unwrap(),
            SECOND_AFTER
        );
        assert_eq!(
            fs::read(self.checkpoint.join("before/0.bin")).unwrap(),
            FIRST_BEFORE
        );
        assert_eq!(
            fs::read(self.checkpoint.join("before/1.bin")).unwrap(),
            SECOND_BEFORE
        );
    }

    fn refuse(&self, files: &[CheckpointFile], code: &str, reason: &str) {
        let manifest = fs::read(self.checkpoint.join("manifest.json")).unwrap();
        let result = restore_snapshot_strict(&self.root, &self.checkpoint, files, &self.filesystem);
        assert_error(result, code, reason);
        self.assert_payloads_untouched();
        assert_eq!(
            fs::read(self.checkpoint.join("manifest.json")).unwrap(),
            manifest
        );
    }
}

fn stat(path: &str) -> PatchStat {
    PatchStat {
        path: path.into(),
        additions: Some(1),
        deletions: Some(1),
    }
}

fn assert_error(result: Result<(), String>, code: &str, reason: &str) {
    let error = result
        .err()
        .expect("malformed restore metadata must be refused");
    let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
    assert_eq!(parsed.error_code, code);
    let extra = parsed.extra.unwrap();
    assert_eq!(extra["details"]["reason"], reason);
    assert_eq!(
        extra["message_key"],
        match code {
            "checkpoint_backup_invalid" => "workspace.patch.backup_invalid",
            "checkpoint_invalid" => "workspace.patch.checkpoint_invalid",
            _ => panic!("unexpected restore metadata error code"),
        }
    );
}

#[test]
fn patch_restore_metadata_empty_file_set_cannot_claim_successful_rewind() {
    for public in [false, true] {
        let fixture = Fixture::new();
        let result = if public {
            fixture.persist(&[]);
            rewind(
                &fixture.root,
                serde_json::json!({"action":"rewind","checkpoint_id":CHECKPOINT})
                    .as_object()
                    .unwrap(),
                &fixture.filesystem,
            )
            .map(|_| ())
        } else {
            restore_snapshot_strict(&fixture.root, &fixture.checkpoint, &[], &fixture.filesystem)
        };
        assert_error(result, "checkpoint_invalid", "empty_file_set");
        fixture.assert_payloads_untouched();
        assert_eq!(
            read_manifest(&fixture.checkpoint, &fixture.filesystem)
                .unwrap()
                .state,
            "applied"
        );
    }
}

#[test]
fn patch_restore_metadata_refuses_normalized_duplicate_targets_before_changes() {
    for duplicate in ["first.txt", "./first.txt", ".//first.txt"] {
        let fixture = Fixture::new();
        let mut files = fixture.files.clone();
        files[1].path = duplicate.into();
        fixture.refuse(&files, "checkpoint_invalid", "duplicate_target");
    }
}

#[test]
fn patch_restore_metadata_binds_backups_to_the_produced_ordinal() {
    for backup in ["before/0.bin", "copies/1.bin", "before/01.bin"] {
        let fixture = Fixture::new();
        let mut files = fixture.files.clone();
        if backup == "before/0.bin" {
            files[1].before_sha256 = Some(sha256_label(FIRST_BEFORE));
        } else {
            let path = fixture.checkpoint.join(backup);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, SECOND_BEFORE).unwrap();
        }
        files[1].backup_file = Some(backup.into());
        fixture.refuse(
            &files,
            "checkpoint_backup_invalid",
            "backup_reference_invalid",
        );
    }
}

#[test]
fn patch_restore_metadata_missing_preimage_cannot_delete_a_file_with_payload_fields() {
    for keep_backup in [true, false] {
        let fixture = Fixture::new();
        let mut files = fixture.files.clone();
        files[1].existed = false;
        if !keep_backup {
            files[1].backup_file = None;
        }
        fixture.refuse(&files, "checkpoint_invalid", "missing_preimage_has_payload");
    }
}

#[test]
fn patch_restore_metadata_validates_later_entries_before_any_backup_io() {
    let fixture = Fixture::new();
    let mut files = fixture.files.clone();
    files[1].existed = false;
    fs::remove_file(fixture.checkpoint.join("before/0.bin")).unwrap();
    assert_error(
        restore_snapshot_strict(
            &fixture.root,
            &fixture.checkpoint,
            &files,
            &fixture.filesystem,
        ),
        "checkpoint_invalid",
        "missing_preimage_has_payload",
    );
    assert_eq!(
        fs::read(fixture.root.join("first.txt")).unwrap(),
        FIRST_AFTER
    );
    assert_eq!(
        fs::read(fixture.root.join("second.txt")).unwrap(),
        SECOND_AFTER
    );
    assert!(!fixture.checkpoint.join("before/0.bin").exists());
}

#[test]
fn patch_restore_metadata_rejects_invalid_hashes_before_any_backup_io() {
    for hash in [None, Some(String::new()), Some("sha256:invalid".into())] {
        let fixture = Fixture::new();
        let mut files = fixture.files.clone();
        files[1].before_sha256 = hash;
        fs::remove_file(fixture.checkpoint.join("before/0.bin")).unwrap();
        assert_error(
            restore_snapshot_strict(
                &fixture.root,
                &fixture.checkpoint,
                &files,
                &fixture.filesystem,
            ),
            "checkpoint_backup_invalid",
            "before_hash_invalid",
        );
        assert_eq!(
            fs::read(fixture.root.join("first.txt")).unwrap(),
            FIRST_AFTER
        );
        assert_eq!(
            fs::read(fixture.root.join("second.txt")).unwrap(),
            SECOND_AFTER
        );
    }
}

#[test]
fn patch_restore_metadata_rejects_malformed_optional_after_hashes() {
    let fixture = Fixture::new();
    let mut files = fixture.files.clone();
    files[1].after_sha256 = Some("invalid-after-digest".into());
    fixture.refuse(&files, "checkpoint_invalid", "after_hash_invalid");
}

#[test]
fn patch_restore_metadata_public_rewind_preserves_rejected_checkpoint_state() {
    let fixture = Fixture::new();
    let mut files = fixture.files.clone();
    files[1].backup_file = Some("before/0.bin".into());
    files[1].before_sha256 = Some(sha256_label(FIRST_BEFORE));
    fixture.persist(&files);
    let manifest = fs::read(fixture.checkpoint.join("manifest.json")).unwrap();
    let result = rewind(
        &fixture.root,
        serde_json::json!({"action":"rewind","checkpoint_id":CHECKPOINT})
            .as_object()
            .unwrap(),
        &fixture.filesystem,
    );
    assert_error(
        result.map(|_| ()),
        "checkpoint_backup_invalid",
        "backup_reference_invalid",
    );
    fixture.assert_payloads_untouched();
    assert_eq!(
        fs::read(fixture.checkpoint.join("manifest.json")).unwrap(),
        manifest
    );
}

#[test]
fn patch_restore_metadata_producer_refuses_duplicate_targets_before_backups() {
    let fixture = Fixture::new();
    let checkpoint =
        create_checkpoint_dir(&fixture.root, "patch_duplicate_owned", &fixture.filesystem).unwrap();
    let error = snapshot_files(
        &fixture.root,
        &checkpoint,
        &[stat("first.txt"), stat("./first.txt")],
        &fixture.filesystem,
    )
    .err()
    .expect("producer must refuse duplicate normalized targets");
    let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
    assert_eq!(parsed.error_code, "duplicate_patch_path");
    assert!(!checkpoint.join("before").exists());
    fixture.assert_payloads_untouched();
}

#[test]
fn patch_restore_metadata_native_alias_producer_refuses_before_dispatch() {
    let fixture = Fixture::new();
    let patch = concat!(
        "diff --git a/first.txt b/first.txt\n--- a/first.txt\n+++ b/first.txt\n@@ -1 +1 @@\n-one-after\n+updated-one\n",
        "diff --git a/./first.txt b/./first.txt\n--- a/./first.txt\n+++ b/./first.txt\n@@ -1 +1 @@\n-one-after\n+updated-two\n",
    );
    let error = execute_workspace_patch_with_policy(
        &fixture.root,
        "owned-duplicate-producer",
        serde_json::json!({"action":"apply_patch","patch":patch})
            .as_object()
            .unwrap(),
        &fixture.filesystem,
        ToolSandboxBackend::Auto,
    )
    .err()
    .expect("normalized alias must be refused before payload dispatch");
    let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
    assert_eq!(parsed.error_code, "duplicate_patch_path");
    let extra = parsed.extra.unwrap();
    assert_eq!(extra["failure_phase"], "pre_dispatch");
    assert_eq!(extra["side_effect_applied"], false);
    assert_eq!(extra["recovery_action"], "replan_arguments");
    fixture.assert_payloads_untouched();
    assert_eq!(
        fs::read_dir(checkpoint_root(&fixture.root))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn patch_restore_metadata_missing_backup_keeps_its_error_contract() {
    let fixture = Fixture::new();
    let mut files = fixture.files.clone();
    files[1].backup_file = None;
    let error = restore_snapshot_strict(
        &fixture.root,
        &fixture.checkpoint,
        &files,
        &fixture.filesystem,
    )
    .unwrap_err();
    let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
    assert_eq!(parsed.error_code, "checkpoint_backup_missing");
    assert_eq!(
        parsed.extra.unwrap()["message_key"],
        "workspace.patch.checkpoint_backup_missing"
    );
    fixture.assert_payloads_untouched();
}

#[test]
fn patch_restore_metadata_real_native_and_git_create_edit_delete_roundtrip() {
    let patch = concat!(
        "diff --git a/first.txt b/first.txt\n--- a/first.txt\n+++ b/first.txt\n@@ -1 +1 @@\n-one-before\n+one-after\n",
        "diff --git a/second.txt b/second.txt\ndeleted file mode 100644\n--- a/second.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-two-before\n",
        "diff --git a/created.txt b/created.txt\nnew file mode 100644\n--- /dev/null\n+++ b/created.txt\n@@ -0,0 +1 @@\n+created\n",
    );
    for git in [false, true] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        fs::write(root.join("first.txt"), FIRST_BEFORE).unwrap();
        fs::write(root.join("second.txt"), SECOND_BEFORE).unwrap();
        if git {
            assert!(Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(&root)
                .status()
                .unwrap()
                .success());
        }
        let filesystem = HostFilesystemPolicy::without_private_authority();
        let applied = execute_workspace_patch_with_policy(
            &root,
            "owned-restore-roundtrip",
            serde_json::json!({"action":"apply_patch","patch":patch})
                .as_object()
                .unwrap(),
            &filesystem,
            ToolSandboxBackend::Auto,
        )
        .unwrap();
        let applied: Value = serde_json::from_str(&applied).unwrap();
        assert_eq!(applied["status"], "ok");
        assert_eq!(fs::read(root.join("first.txt")).unwrap(), FIRST_AFTER);
        assert!(!root.join("second.txt").exists());
        assert_eq!(fs::read(root.join("created.txt")).unwrap(), b"created\n");
        let rewound = rewind(
            &root,
            serde_json::json!({"action":"rewind","checkpoint_id":applied["checkpoint_id"]})
                .as_object()
                .unwrap(),
            &filesystem,
        )
        .unwrap();
        let rewound: Value = serde_json::from_str(&rewound).unwrap();
        assert_eq!(rewound["state"], "rewound");
        assert_eq!(fs::read(root.join("first.txt")).unwrap(), FIRST_BEFORE);
        assert_eq!(fs::read(root.join("second.txt")).unwrap(), SECOND_BEFORE);
        assert!(!root.join("created.txt").exists());
    }
}

#[test]
fn patch_restore_metadata_empty_binary_and_missing_preimages_remain_distinct() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let filesystem = HostFilesystemPolicy::without_private_authority();
    fs::write(root.join("empty.bin"), []).unwrap();
    fs::write(root.join("binary.bin"), b"\x00\xff\n").unwrap();
    let checkpoint = create_checkpoint_dir(&root, CHECKPOINT, &filesystem).unwrap();
    let files = snapshot_files(
        &root,
        &checkpoint,
        &[stat("empty.bin"), stat("binary.bin"), stat("missing.bin")],
        &filesystem,
    )
    .unwrap();
    fs::write(root.join("empty.bin"), b"changed").unwrap();
    fs::remove_file(root.join("binary.bin")).unwrap();
    fs::write(root.join("missing.bin"), b"created").unwrap();
    restore_snapshot_strict(&root, &checkpoint, &files, &filesystem).unwrap();
    assert_eq!(fs::read(root.join("empty.bin")).unwrap(), b"");
    assert_eq!(fs::read(root.join("binary.bin")).unwrap(), b"\x00\xff\n");
    assert!(!root.join("missing.bin").exists());
}
