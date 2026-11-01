use super::*;

const TASK: &str = "owned-restore-preflight";

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    checkpoint: PathBuf,
    filesystem: HostFilesystemPolicy,
}

impl Fixture {
    fn file(before: &[u8]) -> Self {
        let temporary = tempfile::Builder::new()
            .prefix("agent-runtime-restore-budget-")
            .tempdir()
            .unwrap();
        let root = temporary.path().canonicalize().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let filesystem = HostFilesystemPolicy::without_private_authority();
        let target = root.join("a.txt");
        fs::write(&target, before).unwrap();
        let output = run_checkpointed_workspace_mutation_with_policy(
            &root,
            TASK,
            "write_text",
            &target,
            MutationAfterShape::RegularFile,
            &filesystem,
            || fs::write(&target, b"after\n").map_err(|error| error.to_string()),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        let checkpoint =
            resolve_checkpoint_dir(&root, value["checkpoint_id"].as_str().unwrap(), &filesystem)
                .unwrap();
        Self {
            _temporary: temporary,
            root,
            checkpoint,
            filesystem,
        }
    }

    fn manifest(&self) -> MutationCheckpointManifest {
        read_manifest(&self.checkpoint, &self.filesystem).unwrap()
    }

    fn removed_directory() -> Self {
        let mut fixture = Self::file(b"before\n");
        let target = fixture.root.join("tree");
        fs::create_dir_all(target.join("nested")).unwrap();
        fs::write(target.join("a"), b"alpha").unwrap();
        fs::write(target.join("nested/b"), b"beta").unwrap();
        let output = run_checkpointed_workspace_mutation_with_policy(
            &fixture.root,
            TASK,
            "remove_path",
            &target,
            MutationAfterShape::Missing,
            &fixture.filesystem,
            || fs::remove_dir_all(&target).map_err(|error| error.to_string()),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        fixture.checkpoint = resolve_checkpoint_dir(
            &fixture.root,
            value["checkpoint_id"].as_str().unwrap(),
            &fixture.filesystem,
        )
        .unwrap();
        assert!(!target.exists());
        fixture
    }

    fn publish(&self, manifest: &MutationCheckpointManifest) {
        write_manifest(&self.checkpoint, manifest, &self.filesystem).unwrap();
    }

    fn rewind(&self) -> Result<String, String> {
        let id = self.checkpoint.file_name().unwrap().to_str().unwrap();
        rewind_structured_mutation_with_policy(&self.root, id, &self.filesystem)
    }

    fn assert_current(&self) {
        assert_eq!(fs::read(self.root.join("a.txt")).unwrap(), b"after\n");
        assert_eq!(self.manifest().state, "applied");
    }

    async fn apply_builtin(&mut self, skill: &str, args: Value) -> Value {
        let state = super::super::tests::test_state(self.root.clone());
        let output =
            super::super::execute_builtin_skill_for_task(&state, &removal_task(), skill, &args)
                .await
                .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["state"], "applied");
        self.checkpoint = resolve_checkpoint_dir(
            &self.root,
            value["checkpoint_id"].as_str().unwrap(),
            &self.filesystem,
        )
        .unwrap();
        value
    }
}

fn error_code(result: Result<String, String>) -> String {
    let error = match result {
        Ok(_) => panic!("invalid restore metadata was accepted"),
        Err(error) => error,
    };
    crate::skills::parse_structured_skill_error(&error)
        .unwrap()
        .error_code
}

fn removal_task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        claim_attempt: 1,
        task_id: TASK.into(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "ui".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".into(),
        payload_json: "{}".into(),
    }
}

#[cfg(unix)]
#[test]
fn restore_operation_failure_does_not_leave_permission_changes_under_no_effect() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::file(b"seed\n");
    let target = fixture.root.join("script.sh");
    fs::write(&target, b"owned-script\n").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
    let error = run_checkpointed_workspace_mutation_with_policy(
        &fixture.root,
        TASK,
        "write_text",
        &target,
        MutationAfterShape::RegularFile,
        &fixture.filesystem,
        || {
            fs::set_permissions(&target, fs::Permissions::from_mode(0o600))
                .map_err(|error| io_error("owned_chmod_failed", error))?;
            Err(mutation_error(
                "owned_callback_failed",
                "workspace.test.callback_failed",
                Value::Null,
            ))
        },
    )
    .err()
    .expect("operation intentionally fails after changing only its mode");
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o755,
        "a no-effect result must not leave a permission change"
    );
    assert_eq!(fs::read(&target).unwrap(), b"owned-script\n");
    let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
    assert_eq!(parsed.error_code, "owned_callback_failed");
    assert_eq!(parsed.extra.unwrap()["side_effect_applied"], false);
    fixture.assert_current();
}

#[cfg(unix)]
#[tokio::test]
async fn restore_rewind_preserves_native_file_and_directory_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let mut fixture = Fixture::file(b"seed\n");
    let tree = fixture.root.join("tree");
    let nested = tree.join("nested");
    fs::create_dir_all(&nested).unwrap();
    let script = nested.join("script.sh");
    fs::write(&script, b"owned-script\n").unwrap();
    for (path, mode) in [(&tree, 0o700), (&nested, 0o711), (&script, 0o751)] {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }
    fixture
        .apply_builtin(
            "remove_file",
            json!({
                "path": "tree", "target_kind": "directory", "recursive": true,
            }),
        )
        .await;
    assert!(!tree.exists());
    fixture.rewind().unwrap();
    for (path, mode) in [(&tree, 0o700), (&nested, 0o711), (&script, 0o751)] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            mode
        );
    }
    assert_eq!(fs::read(script).unwrap(), b"owned-script\n");
}

async fn assert_reserved_descendant_is_not_removed(name: &str) {
    let fixture = Fixture::file(b"before\n");
    let target = fixture.root.join("tree");
    let reserved = target.join("nested").join(name).join("owned.txt");
    fs::create_dir_all(reserved.parent().unwrap()).unwrap();
    fs::write(&reserved, b"keep-owned\n").unwrap();
    let state = super::super::tests::test_state(fixture.root.clone());
    let result = super::super::execute_builtin_skill_for_task(
        &state,
        &removal_task(),
        "remove_file",
        &json!({"path":"tree","target_kind":"directory","recursive":true}),
    )
    .await;
    assert_eq!(
        fs::read(&reserved).unwrap(),
        b"keep-owned\n",
        "a reversible mutation must refuse unsupported descendants before dispatch"
    );
    let error = result.err().expect("reserved descendant must be rejected");
    let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
    assert_eq!(parsed.error_code, "invalid_target_path");
    let extra = parsed.extra.unwrap();
    assert_eq!(extra["side_effect_applied"], false);
    assert_eq!(extra["failure_phase"], "pre_dispatch");
    let checkpoints =
        claw_core::workspace_state::workspace_state_root(&fixture.root).join("checkpoints");
    assert_eq!(fs::read_dir(checkpoints).unwrap().count(), 1);
    fixture.assert_current();
}

#[tokio::test]
async fn restore_producer_nested_git_state_is_refused_before_removal() {
    assert_reserved_descendant_is_not_removed(".git").await;
}

#[tokio::test]
async fn restore_producer_nested_runtime_state_is_refused_before_removal() {
    assert_reserved_descendant_is_not_removed(claw_core::workspace_state::WORKSPACE_STATE_DIR_NAME)
        .await;
}

#[tokio::test]
async fn restore_producer_ordinary_hidden_names_remain_removable_and_rewindable() {
    let mut fixture = Fixture::file(b"before\n");
    let paths = [
        "tree/nested/.git-not-reserved/owned.txt".to_string(),
        format!(
            "tree/nested/{}-not-reserved/owned.txt",
            claw_core::workspace_state::WORKSPACE_STATE_DIR_NAME
        ),
        "tree/nested/.cache/owned.txt".to_string(),
    ];
    for path in &paths {
        let path = fixture.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"restore-owned\n").unwrap();
    }
    let state = super::super::tests::test_state(fixture.root.clone());
    let output = super::super::execute_builtin_skill_for_task(
        &state,
        &removal_task(),
        "remove_file",
        &json!({"path":"tree","target_kind":"directory","recursive":true}),
    )
    .await
    .unwrap();
    assert!(!fixture.root.join("tree").exists());
    let value: Value = serde_json::from_str(&output).unwrap();
    fixture.checkpoint = resolve_checkpoint_dir(
        &fixture.root,
        value["checkpoint_id"].as_str().unwrap(),
        &fixture.filesystem,
    )
    .unwrap();
    fixture.rewind().unwrap();
    for path in paths {
        assert_eq!(
            fs::read(fixture.root.join(path)).unwrap(),
            b"restore-owned\n"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn restore_paths_native_backslash_target_does_not_rewind_a_different_file() {
    let mut fixture = Fixture::file(b"before\n");
    let target = fixture.root.join(r"owned\file.txt");
    let sibling = fixture.root.join("owned/file.txt");
    fs::create_dir(sibling.parent().unwrap()).unwrap();
    fs::write(&target, b"native-before\n").unwrap();
    fs::write(&sibling, b"different-before\n").unwrap();
    fixture
        .apply_builtin(
            "write_file",
            json!({"path":r"./owned\file.txt","content":"native-after\n"}),
        )
        .await;
    assert_eq!(fs::read(&target).unwrap(), b"native-after\n");
    assert_eq!(fs::read(&sibling).unwrap(), b"different-before\n");
    fixture.rewind().unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"native-before\n");
    assert_eq!(fs::read(sibling).unwrap(), b"different-before\n");
    assert_eq!(fixture.manifest().target_path, r"owned\file.txt");
}

#[cfg(unix)]
#[tokio::test]
async fn restore_paths_native_backslash_descendant_does_not_collide_with_a_directory() {
    let mut fixture = Fixture::file(b"before\n");
    let target = fixture.root.join("tree");
    let native = target.join(r"nested\file.txt");
    let nested = target.join("nested/file.txt");
    fs::create_dir_all(nested.parent().unwrap()).unwrap();
    fs::write(&native, b"native-before\n").unwrap();
    fs::write(&nested, b"nested-before\n").unwrap();
    fixture
        .apply_builtin(
            "remove_file",
            json!({"path":"tree","target_kind":"directory","recursive":true}),
        )
        .await;
    assert!(!target.exists());
    fixture.rewind().unwrap();
    assert_eq!(fs::read(native).unwrap(), b"native-before\n");
    assert_eq!(fs::read(nested).unwrap(), b"nested-before\n");
    let paths = fixture
        .manifest()
        .before
        .into_iter()
        .map(|entry| entry.path)
        .collect::<Vec<_>>();
    assert!(paths.contains(&r"tree/nested\file.txt".to_string()));
    assert!(paths.contains(&"tree/nested/file.txt".to_string()));
}

#[cfg(unix)]
#[tokio::test]
async fn restore_paths_native_backslash_missing_parent_is_removed_without_touching_siblings() {
    let mut fixture = Fixture::file(b"before\n");
    let parent = fixture.root.join(r"new\parent");
    let sibling = fixture.root.join("new/parent/keep.txt");
    fs::create_dir_all(sibling.parent().unwrap()).unwrap();
    fs::write(&sibling, b"keep-sibling\n").unwrap();
    fixture
        .apply_builtin("make_dir", json!({"path":r"new\parent/nested"}))
        .await;
    assert!(parent.join("nested").is_dir());
    assert_eq!(
        fixture.manifest().missing_parent_paths,
        vec![r"new\parent".to_string()]
    );
    fixture.rewind().unwrap();
    assert!(!parent.exists());
    assert_eq!(fs::read(sibling).unwrap(), b"keep-sibling\n");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn restore_paths_non_utf8_descendant_is_refused_before_recursive_removal() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let fixture = Fixture::file(b"before\n");
    let target = fixture.root.join("tree");
    fs::create_dir(&target).unwrap();
    let invalid = target.join(OsString::from_vec(b"native-\xff.txt".to_vec()));
    let replacement = target.join("native-\u{fffd}.txt");
    fs::write(&invalid, b"native-bytes\n").unwrap();
    fs::write(&replacement, b"valid-unicode\n").unwrap();
    let state = super::super::tests::test_state(fixture.root.clone());
    let result = super::super::execute_builtin_skill_for_task(
        &state,
        &removal_task(),
        "remove_file",
        &json!({"path":"tree","target_kind":"directory","recursive":true}),
    )
    .await;
    assert_eq!(fs::read(invalid).unwrap(), b"native-bytes\n");
    assert_eq!(fs::read(replacement).unwrap(), b"valid-unicode\n");
    let parsed = crate::skills::parse_structured_skill_error(
        &result.err().expect("unrepresentable path must be refused"),
    )
    .unwrap();
    assert_eq!(parsed.error_code, "invalid_target_path");
    let extra = parsed.extra.unwrap();
    assert_eq!(extra["side_effect_applied"], false);
    assert_eq!(extra["failure_phase"], "pre_dispatch");
    let checkpoints =
        claw_core::workspace_state::workspace_state_root(&fixture.root).join("checkpoints");
    assert_eq!(fs::read_dir(checkpoints).unwrap().count(), 1);
    fixture.assert_current();
}

#[cfg(unix)]
#[test]
fn restore_paths_non_utf8_target_is_refused_before_dispatch() {
    use std::cell::Cell;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let fixture = Fixture::file(b"before\n");
    let target = fixture
        .root
        .join(OsString::from_vec(b"native-\xff.txt".to_vec()));
    let called = Cell::new(false);
    let result = run_checkpointed_workspace_mutation_with_policy(
        &fixture.root,
        TASK,
        "write_text",
        &target,
        MutationAfterShape::RegularFile,
        &fixture.filesystem,
        || {
            called.set(true);
            fs::write(fixture.root.join("a.txt"), b"unexpected\n")
                .map_err(|error| error.to_string())
        },
    );
    assert!(!called.get());
    assert_eq!(error_code(result), "invalid_target_path");
    fixture.assert_current();
}

#[tokio::test]
async fn restore_paths_unicode_names_remain_removable_and_rewindable() {
    let mut fixture = Fixture::file(b"before\n");
    let paths = [
        "tree/\u{6587}\u{4ef6}/caf\u{e9}.txt",
        "tree/\u{3b4}\u{3bf}\u{3ba}\u{3b9}\u{3bc}\u{3ae}.txt",
    ];
    for path in paths {
        let path = fixture.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"unicode-before\n").unwrap();
    }
    fixture
        .apply_builtin(
            "remove_file",
            json!({"path":"tree","target_kind":"directory","recursive":true}),
        )
        .await;
    assert!(!fixture.root.join("tree").exists());
    fixture.rewind().unwrap();
    for path in paths {
        assert_eq!(
            fs::read(fixture.root.join(path)).unwrap(),
            b"unicode-before\n"
        );
    }
}

#[test]
fn restore_preflight_file_and_empty_file_roundtrip() {
    for before in [b"before\n".as_slice(), b"".as_slice()] {
        let fixture = Fixture::file(before);
        let output: Value = serde_json::from_str(&fixture.rewind().unwrap()).unwrap();
        assert_eq!(output["state"], "rewound");
        assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), before);
    }
}

#[test]
fn restore_preflight_directory_roundtrip_preserves_all_files() {
    let fixture = Fixture::removed_directory();
    let target = fixture.root.join("tree");
    fixture.rewind().unwrap();
    assert_eq!(fs::read(target.join("a")).unwrap(), b"alpha");
    assert_eq!(fs::read(target.join("nested/b")).unwrap(), b"beta");
}

#[test]
fn restore_preflight_size_metadata_must_match_backup() {
    for declared in [None, Some(0), Some(1), Some(u64::MAX)] {
        let fixture = Fixture::file(b"before\n");
        let mut manifest = fixture.manifest();
        manifest.before[0].size_bytes = declared;
        fixture.publish(&manifest);
        let result = fixture.rewind();
        fixture.assert_current();
        assert_eq!(error_code(result), "checkpoint_backup_invalid");
    }
}

#[test]
fn restore_preflight_total_budget_is_checked_before_reading_backups() {
    let fixture = Fixture::removed_directory();
    let mut manifest = fixture.manifest();
    let mut sizes = [MAX_SNAPSHOT_BYTES, 1].into_iter();
    for (index, entry) in manifest
        .before
        .iter_mut()
        .filter(|entry| entry.kind == "file")
        .enumerate()
    {
        entry.size_bytes = Some(sizes.next().unwrap());
        entry.backup_file = Some(format!("before/absent-{index}.bin"));
    }
    assert!(sizes.next().is_none());
    fixture.publish(&manifest);
    let result = fixture.rewind();
    fixture.assert_current();
    assert!(!fixture.root.join("tree").exists());
    assert_eq!(error_code(result), "checkpoint_backup_invalid");
}

#[test]
fn restore_preflight_unrelated_target_entry_cannot_overwrite_a_sibling() {
    let fixture = Fixture::removed_directory();
    let sibling = fixture.root.join("tree-other/a");
    fs::create_dir(sibling.parent().unwrap()).unwrap();
    fs::write(&sibling, b"keep-owned").unwrap();
    let mut manifest = fixture.manifest();
    let mut escaped = manifest
        .before
        .iter()
        .find(|entry| entry.kind == "file")
        .unwrap()
        .clone();
    escaped.path = "tree-other/a".into();
    manifest.before.push(escaped);
    fixture.publish(&manifest);
    let result = fixture.rewind();
    assert_eq!(fs::read(&sibling).unwrap(), b"keep-owned");
    fixture.assert_current();
    assert!(!fixture.root.join("tree").exists());
    assert_eq!(error_code(result), "checkpoint_restore_invalid");
}

#[test]
fn restore_preflight_entry_count_limit_precedes_directory_creation() {
    let fixture = Fixture::removed_directory();
    let mut manifest = fixture.manifest();
    manifest.before.retain(|entry| entry.path == "tree");
    assert_eq!(manifest.before.len(), 1);
    for index in 0..MAX_SNAPSHOT_ENTRIES {
        manifest.before.push(SnapshotEntry {
            path: format!("tree/d{index:04}"),
            kind: "directory".into(),
            sha256: None,
            size_bytes: None,
            backup_file: None,
        });
    }
    assert_eq!(manifest.before.len(), MAX_SNAPSHOT_ENTRIES + 1);
    fixture.publish(&manifest);
    let result = fixture.rewind();
    assert!(!fixture.root.join("tree").exists());
    fixture.assert_current();
    assert_eq!(error_code(result), "checkpoint_restore_invalid");
}

#[test]
fn restore_preflight_duplicate_normalized_paths_are_rejected_before_mutation() {
    let fixture = Fixture::file(b"before\n");
    let mut manifest = fixture.manifest();
    let mut duplicate = manifest.before[0].clone();
    duplicate.path = "./a.txt".into();
    manifest.before.push(duplicate);
    fixture.publish(&manifest);
    let result = fixture.rewind();
    fixture.assert_current();
    assert_eq!(error_code(result), "checkpoint_restore_invalid");
}

#[test]
fn restore_preflight_empty_or_unknown_root_kind_is_not_success() {
    for empty in [false, true] {
        let fixture = Fixture::file(b"before\n");
        let mut manifest = fixture.manifest();
        if empty {
            manifest.before.clear();
        } else {
            manifest.before[0].kind = "unknown_owned_kind".into();
        }
        fixture.publish(&manifest);
        let result = fixture.rewind();
        fixture.assert_current();
        assert_eq!(error_code(result), "checkpoint_restore_invalid");
    }
}

#[test]
fn restore_preflight_missing_parent_metadata_cannot_remove_an_unrelated_directory() {
    let fixture = Fixture::file(b"before\n");
    let unrelated = fixture.root.join("empty-owned");
    fs::create_dir(&unrelated).unwrap();
    let mut manifest = fixture.manifest();
    manifest.missing_parent_paths.push("empty-owned".into());
    fixture.publish(&manifest);
    let result = fixture.rewind();
    assert!(unrelated.is_dir());
    fixture.assert_current();
    assert_eq!(error_code(result), "checkpoint_restore_invalid");
}

#[test]
fn restore_preflight_missing_backup_and_corrupt_hash_remain_distinct() {
    for missing in [false, true] {
        let fixture = Fixture::file(b"before\n");
        let manifest = fixture.manifest();
        let backup = fixture
            .checkpoint
            .join(manifest.before[0].backup_file.as_ref().unwrap());
        if missing {
            fs::remove_file(&backup).unwrap();
        } else {
            fs::write(&backup, b"corrupt").unwrap();
        }
        let result = fixture.rewind();
        fixture.assert_current();
        assert_eq!(
            error_code(result),
            if missing {
                "restore_read_failed"
            } else {
                "checkpoint_backup_invalid"
            }
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn restore_budget_large_backup_is_bounded_before_payload_mutation() {
    use std::os::unix::fs::MetadataExt;
    use std::process::Command;

    const CHILD_ROOT: &str = "APP_TEST_RESTORE_BUDGET_ROOT";
    const TEST: &str = "skills::builtin::builtin_workspace_mutation::restore_tests::restore_budget_large_backup_is_bounded_before_payload_mutation";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = PathBuf::from(root).canonicalize().unwrap();
        assert!(root.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        assert!(root
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("agent-runtime-restore-budget-"));
        let metadata = fs::metadata(&root).unwrap();
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.mode() & 0o7777, 0o700);
        let directories = fs::read_dir(
            claw_core::workspace_state::workspace_state_root(&root).join("checkpoints"),
        )
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
        assert_eq!(directories.len(), 1);
        let checkpoint = &directories[0];
        let filesystem = HostFilesystemPolicy::without_private_authority();
        let manifest = read_manifest(checkpoint, &filesystem).unwrap();
        let virtual_bytes = fs::read_to_string("/proc/self/status")
            .unwrap()
            .lines()
            .find_map(|line| {
                line.strip_prefix("VmSize:").map(|value| {
                    value
                        .split_whitespace()
                        .next()
                        .unwrap()
                        .parse::<u64>()
                        .unwrap()
                        * 1024
                })
            })
            .unwrap();
        let mut inherited = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_AS, &mut inherited) },
            0
        );
        let ceiling = virtual_bytes
            .checked_add(64 * 1024 * 1024)
            .unwrap()
            .min(inherited.rlim_cur);
        assert!(ceiling > virtual_bytes + 4 * 1024 * 1024);
        let no_core = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        let address = libc::rlimit {
            rlim_cur: ceiling,
            rlim_max: ceiling,
        };
        // Limits belong only to this independently executed owned fixture child.
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_CORE, &no_core) }, 0);
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_AS, &address) }, 0);
        let result =
            rewind_structured_mutation_with_policy(&root, &manifest.checkpoint_id, &filesystem);
        assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"after\n");
        assert_eq!(error_code(result), "checkpoint_backup_invalid");
        let completed = json!({"schema_version":1,"test_name":TEST,"error_code":"checkpoint_backup_invalid",
            "virtual_bytes_before":virtual_bytes,"address_limit_bytes":ceiling});
        fs::write(
            root.join("restore-result.json"),
            serde_json::to_vec(&completed).unwrap(),
        )
        .unwrap();
        return;
    }
    let fixture = Fixture::file(b"before\n");
    let manifest = fixture.manifest();
    let backup = fixture
        .checkpoint
        .join(manifest.before[0].backup_file.as_ref().unwrap());
    let length = 512 * 1024 * 1024;
    fs::OpenOptions::new()
        .write(true)
        .open(&backup)
        .unwrap()
        .set_len(length)
        .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--test-threads=1", "--nocapture"])
        .env(CHILD_ROOT, &fixture.root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "owned restore child failed: {}\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let completed: Value = serde_json::from_slice(
        &fs::read(fixture.root.join("restore-result.json"))
            .expect("selected child must finish its restore proof"),
    )
    .unwrap();
    assert_eq!(completed["schema_version"], 1);
    assert_eq!(completed["test_name"], TEST);
    assert_eq!(completed["error_code"], "checkpoint_backup_invalid");
    assert_eq!(fs::metadata(backup).unwrap().len(), length);
    fixture.assert_current();
    println!("restore_bounded_case={completed}");
}
