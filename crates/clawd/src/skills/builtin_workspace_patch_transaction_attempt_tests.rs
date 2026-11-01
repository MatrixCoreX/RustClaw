use super::super::tests::{error_code, Fixture, FIRST};
use super::*;
use std::fs;

fn manifest() -> PatchTransactionManifest {
    PatchTransactionManifest {
        schema_version: TRANSACTION_SCHEMA_VERSION,
        transaction_id: "owned_shards".into(),
        task_id: "owned-shard-task".into(),
        shard_count: 2,
        state: "planned".into(),
        shards: Vec::new(),
    }
}

fn reserve(root: &Path, filesystem: &HostFilesystemPolicy) -> ClaimedAttempt {
    let args = json!({"patch":FIRST}).as_object().unwrap().clone();
    ClaimedAttempt::reserve(
        root,
        &manifest(),
        0,
        &request_digest(&args).unwrap(),
        &super::super::super::sha256_label(FIRST.as_bytes()),
        filesystem,
    )
    .unwrap()
}

#[test]
fn retained_attempt_blocks_execution_even_when_the_transaction_manifest_is_missing() {
    let fixture = Fixture::new();
    let filesystem = HostFilesystemPolicy::without_private_authority();
    let attempt = reserve(&fixture.root, &filesystem);
    let checkpoint_id = attempt.checkpoint_id().to_owned();
    drop(attempt);
    assert_eq!(
        error_code(fixture.run(fixture.request(FIRST))),
        "transaction_attempt_unresolved"
    );
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"before\n");
    let status = fixture
        .run(json!({"action":"transaction_status","transaction_id":"owned_shards"}))
        .unwrap();
    assert_eq!(status["manifest_available"], false);
    assert_eq!(status["requires_reconciliation"], true);
    assert_eq!(status["attempt"]["checkpoint_id"], checkpoint_id);
    assert_eq!(status["execution_outcome"], "unknown");
    assert!(status["continuation"].is_null());
}

#[test]
fn retained_attempt_survives_manifest_loss_after_a_completed_execution() {
    let fixture = Fixture::new();
    fixture.apply();
    fs::remove_file(fixture.manifest_path()).unwrap();
    assert_eq!(
        error_code(fixture.run(fixture.request(FIRST))),
        "transaction_attempt_unresolved"
    );
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"after\n");
}

#[test]
fn retained_attempt_is_not_retired_from_unstructured_failure_or_file_absence() {
    let fixture = Fixture::new();
    let filesystem = HostFilesystemPolicy::without_private_authority();
    assert!(reserve(&fixture.root, &filesystem)
        .retire_rejected(&fixture.root, &filesystem, "unstructured failure")
        .is_err());
    assert!(lookup(
        &fixture.root,
        "owned-shard-task",
        "owned_shards",
        0,
        &filesystem
    )
    .unwrap()
    .is_some());
    assert_eq!(
        error_code(fixture.run(fixture.request(FIRST))),
        "transaction_attempt_unresolved"
    );
}

#[test]
fn typed_pre_dispatch_rejection_retires_only_its_current_receipt_and_allows_replanning() {
    let fixture = Fixture::new();
    let error = fixture.run(fixture.request("not a patch")).unwrap_err();
    let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
    let extra = parsed.extra.unwrap();
    assert_eq!(extra["failure_phase"], "pre_dispatch");
    assert_eq!(extra["side_effect_applied"], false);
    assert_eq!(extra["receipt_retained"], false);
    assert_eq!(extra["retryable"], true);
    assert!(!receipt_path(&fixture.root, "owned_shards", 0).exists());
    fixture.apply();
    assert!(receipt_path(&fixture.root, "owned_shards", 0).exists());
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"after\n");
}

#[test]
fn corrupt_misbound_and_foreign_owner_receipts_do_not_reopen_execution() {
    for case in ["corrupt", "identity", "index", "owner", "digest"] {
        let fixture = Fixture::new();
        let filesystem = HostFilesystemPolicy::without_private_authority();
        drop(reserve(&fixture.root, &filesystem));
        let path = receipt_path(&fixture.root, "owned_shards", 0);
        let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        match case {
            "corrupt" => fs::write(&path, b"owned invalid receipt").unwrap(),
            _ => {
                match case {
                    "identity" => value["transaction_id"] = json!("other_transaction"),
                    "index" => value["shard_index"] = json!(1),
                    "owner" => value["task_id"] = json!("another-task"),
                    "digest" => value["request_digest"] = json!("invalid"),
                    _ => unreachable!(),
                }
                fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            }
        }
        assert!(fixture.run(fixture.request(FIRST)).is_err(), "{case}");
        assert_eq!(
            fs::read(fixture.root.join("a.txt")).unwrap(),
            b"before\n",
            "{case}"
        );
    }
}

#[test]
fn transaction_operation_lock_protects_actual_apply_and_rewind_but_not_status_reads() {
    let fixture = Fixture::new();
    fixture.apply();
    let filesystem = HostFilesystemPolicy::without_private_authority();
    let lock = filesystem
        .try_lock_operation_file(
            Some(&fixture.root),
            &fixture.manifest_path().with_extension("lock"),
        )
        .unwrap();
    assert_eq!(
        error_code(fixture.run(fixture.request(FIRST))),
        "transaction_busy"
    );
    assert_eq!(
        error_code(
            fixture.run(json!({"action":"transaction_rewind","transaction_id":"owned_shards"}))
        ),
        "transaction_busy"
    );
    let status = fixture
        .run(json!({"action":"transaction_status","transaction_id":"owned_shards"}))
        .unwrap();
    assert_eq!(status["applied_shard_count"], 1);
    drop(lock);
    fixture
        .run(json!({"action":"transaction_rewind","transaction_id":"owned_shards"}))
        .unwrap();
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"before\n");
}

#[test]
fn transaction_status_is_read_only_and_retained_attempts_require_the_original_task() {
    let fixture = Fixture::new();
    assert!(fixture
        .run(json!({"action":"transaction_status","transaction_id":"owned_shards"}))
        .is_err());
    assert!(!super::super::super::checkpoint_root(&fixture.root).exists());
    let filesystem = HostFilesystemPolicy::without_private_authority();
    drop(reserve(&fixture.root, &filesystem));
    let error = lookup(
        &fixture.root,
        "different-task",
        "owned_shards",
        0,
        &filesystem,
    )
    .err()
    .unwrap();
    assert_eq!(
        crate::skills::parse_structured_skill_error(&error)
            .unwrap()
            .error_code,
        "transaction_owner_mismatch"
    );
    let status = fixture
        .run(json!({"action":"transaction_status","transaction_id":"owned_shards","shard_index":0}))
        .unwrap();
    assert_eq!(status["state"], "attempt_evidence");
    assert_eq!(status["execution_outcome"], "unknown");
}

#[test]
fn retained_attempt_process_exit_before_and_after_native_application_does_not_replay() {
    const CHILD: &str = "APP_TEST_TRANSACTION_ATTEMPT_CHILD";
    const ROOT: &str = "APP_TEST_TRANSACTION_ATTEMPT_ROOT";
    if let Some(phase) = std::env::var_os(CHILD) {
        let root = PathBuf::from(std::env::var_os(ROOT).unwrap())
            .canonicalize()
            .unwrap();
        let filesystem = HostFilesystemPolicy::without_private_authority();
        let path = transaction_manifest_path(&root, "owned_shards", &filesystem).unwrap();
        let _operation = lock_transaction(&root, &path, "owned_shards", &filesystem).unwrap();
        let attempt = reserve(&root, &filesystem);
        if phase == std::ffi::OsStr::new("applied") {
            let args = json!({"action":"apply_patch","patch":FIRST})
                .as_object()
                .unwrap()
                .clone();
            super::super::super::apply_patch_at_checkpoint(
                &root,
                "owned-shard-task",
                &args,
                &filesystem,
                ToolSandboxBackend::Auto,
                attempt.checkpoint_id(),
            )
            .unwrap();
        }
        // Only this independently executed owned test child exits here. No
        // destructor removes the receipt or simulates full-daemon recovery.
        std::process::exit(0);
    }
    for phase in ["reserved", "applied"] {
        let fixture = Fixture::new();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "skills::builtin::builtin_workspace_patch::transaction::attempt::tests::retained_attempt_process_exit_before_and_after_native_application_does_not_replay", "--nocapture"])
            .env(CHILD, phase).env(ROOT, &fixture.root).output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let expected: &[u8] = if phase == "applied" {
            b"after\n"
        } else {
            b"before\n"
        };
        assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), expected);
        assert_eq!(
            error_code(fixture.run(fixture.request(FIRST))),
            "transaction_attempt_unresolved"
        );
        assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), expected);
        let status = fixture
            .run(json!({"action":"transaction_status","transaction_id":"owned_shards"}))
            .unwrap();
        assert_eq!(status["execution_outcome"], "unknown");
        assert!(status["continuation"].is_null());
    }
}
