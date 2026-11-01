use super::*;
use std::cell::Cell;

const TASK: &str = "owned-mutation-capacity";

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    filesystem: HostFilesystemPolicy,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        fs::write(root.join("a.txt"), b"before\n").unwrap();
        Self {
            _temporary: temporary,
            root,
            filesystem: HostFilesystemPolicy::without_private_authority(),
        }
    }

    fn base_manifest(&self) -> MutationCheckpointManifest {
        MutationCheckpointManifest {
            schema_version: CHECKPOINT_SCHEMA_VERSION,
            checkpoint_kind: CHECKPOINT_KIND.into(),
            checkpoint_id: format!("mutation_{}", "0".repeat(32)),
            task_id: TASK.into(),
            action: "write_text".into(),
            state: "prepared".into(),
            created_at: crate::now_ts_u64() as i64,
            target_path: "a.txt".into(),
            missing_parent_paths: Vec::new(),
            before: vec![SnapshotEntry {
                path: "a.txt".into(),
                kind: "file".into(),
                sha256: Some(sha256_label(b"before\n")),
                size_bytes: Some(7),
                backup_file: Some("before/0000.bin".into()),
            }],
            after: Vec::new(),
            mutation_id: None,
        }
    }

    fn sized_manifest(&self, size: usize) -> MutationCheckpointManifest {
        let mut manifest = self.base_manifest();
        manifest.task_id.clear();
        let overhead = serde_json::to_vec_pretty(&manifest).unwrap().len();
        manifest.task_id = "t".repeat(size.checked_sub(overhead).unwrap());
        assert_eq!(serde_json::to_vec_pretty(&manifest).unwrap().len(), size);
        manifest
    }

    fn run<F>(&self, task: &str, shape: MutationAfterShape, operation: F) -> Result<String, String>
    where
        F: FnOnce() -> Result<(), String>,
    {
        run_checkpointed_workspace_mutation_with_policy(
            &self.root,
            task,
            "write_text",
            &self.root.join("a.txt"),
            shape,
            &self.filesystem,
            operation,
        )
    }

    fn checkpoint(&self) -> (PathBuf, MutationCheckpointManifest) {
        let directories = fs::read_dir(
            claw_core::workspace_state::workspace_state_root(&self.root).join("checkpoints"),
        )
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
        assert_eq!(directories.len(), 1);
        let directory = directories.into_iter().next().unwrap();
        let manifest = read_manifest(&directory, &self.filesystem).unwrap();
        (directory, manifest)
    }

    fn assert_pre_dispatch(&self, error: String) {
        let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
        assert_eq!(parsed.error_code, "checkpoint_too_large");
        let extra = parsed.extra.unwrap();
        assert_eq!(extra["side_effect_applied"], false);
        assert_eq!(extra["failure_phase"], "pre_dispatch");
        assert_eq!(extra["recovery_action"], "replan_arguments");
        assert_eq!(fs::read(self.root.join("a.txt")).unwrap(), b"before\n");
    }

    fn assert_unknown(&self, error: String, phase: &str) {
        let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
        assert_eq!(parsed.error_code, "checkpoint_commit_unconfirmed");
        let extra = parsed.extra.unwrap();
        assert_eq!(extra["task_id"], TASK);
        assert_eq!(extra["retryable"], false);
        assert_eq!(extra["complete"], false);
        assert_eq!(extra["failure_phase"], phase);
        assert_eq!(extra["compensation_attempted"], false);
        assert_eq!(extra["checkpoint_cleanup_attempted"], false);
        assert!(extra.get("side_effect_applied").unwrap().is_null());
        let (directory, manifest) = self.checkpoint();
        assert_eq!(extra["checkpoint_id"], manifest.checkpoint_id);
        let backup = directory.join(manifest.before[0].backup_file.as_ref().unwrap());
        assert_eq!(fs::read(backup).unwrap(), b"before\n");
    }
}

fn projected_file(manifest: &MutationCheckpointManifest) -> MutationCheckpointManifest {
    let mut projected = manifest.clone();
    projected.after = vec![SnapshotEntry {
        path: manifest.target_path.clone(),
        kind: "file".into(),
        sha256: Some(sha256_label(&[])),
        size_bytes: Some(u64::MAX),
        backup_file: None,
    }];
    projected.mutation_id = Some(sha256_label(&[]));
    projected
}

#[test]
fn mutation_capacity_writer_accepts_exact_read_boundary() {
    let fixture = Fixture::new();
    let manifest = fixture.sized_manifest(MAX_CHECKPOINT_METADATA_BYTES);
    let directory =
        create_checkpoint_dir(&fixture.root, &manifest.checkpoint_id, &fixture.filesystem).unwrap();
    write_manifest(&directory, &manifest, &fixture.filesystem).unwrap();
    assert_eq!(
        fs::metadata(directory.join("manifest.json")).unwrap().len(),
        MAX_CHECKPOINT_METADATA_BYTES as u64
    );
    assert_eq!(
        read_manifest(&directory, &fixture.filesystem)
            .unwrap()
            .task_id,
        manifest.task_id
    );
}

#[test]
fn mutation_capacity_writer_refuses_overflow_without_replacing_or_publishing() {
    let fixture = Fixture::new();
    let accepted = fixture.base_manifest();
    let directory =
        create_checkpoint_dir(&fixture.root, &accepted.checkpoint_id, &fixture.filesystem).unwrap();
    write_manifest(&directory, &accepted, &fixture.filesystem).unwrap();
    let path = directory.join("manifest.json");
    let before = fs::read(&path).unwrap();
    let rejected = fixture.sized_manifest(MAX_CHECKPOINT_METADATA_BYTES + 1);
    let error = match write_manifest(&directory, &rejected, &fixture.filesystem) {
        Ok(()) => panic!("oversized mutation metadata was published"),
        Err(error) => error,
    };
    assert_eq!(
        crate::skills::parse_structured_skill_error(&error)
            .unwrap()
            .error_code,
        "checkpoint_too_large"
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::remove_file(&path).unwrap();
    assert!(write_manifest(&directory, &rejected, &fixture.filesystem).is_err());
    assert!(!path.exists());
}

#[test]
fn mutation_capacity_prepared_overflow_refuses_before_callback() {
    for shape in [
        MutationAfterShape::RegularFile,
        MutationAfterShape::Directory,
        MutationAfterShape::Missing,
    ] {
        let fixture = Fixture::new();
        let manifest = fixture.sized_manifest(MAX_CHECKPOINT_METADATA_BYTES + 1);
        let ran = Cell::new(false);
        let result = fixture.run(&manifest.task_id, shape, || {
            ran.set(true);
            fs::write(fixture.root.join("a.txt"), b"after\n").map_err(|error| error.to_string())
        });
        assert!(
            !ran.get(),
            "callback ran despite oversized prepared metadata"
        );
        fixture.assert_pre_dispatch(result.err().unwrap());
    }
}

#[test]
fn mutation_capacity_after_growth_refuses_before_callback() {
    let fixture = Fixture::new();
    let manifest = fixture.sized_manifest(MAX_CHECKPOINT_METADATA_BYTES);
    assert!(
        serde_json::to_vec_pretty(&projected_file(&manifest))
            .unwrap()
            .len()
            > MAX_CHECKPOINT_METADATA_BYTES
    );
    let ran = Cell::new(false);
    let result = fixture.run(&manifest.task_id, MutationAfterShape::RegularFile, || {
        ran.set(true);
        fs::write(fixture.root.join("a.txt"), b"after\n").map_err(|error| error.to_string())
    });
    assert!(!ran.get(), "callback ran before reserving after metadata");
    fixture.assert_pre_dispatch(result.err().unwrap());
}

#[test]
fn mutation_capacity_real_recursive_removal_refuses_before_deleting_tree() {
    let fixture = Fixture::new();
    let target = fixture.root.join("tree");
    let nested = target.join("n".repeat(200));
    fs::create_dir_all(&nested).unwrap();
    for index in 0..3500 {
        fs::write(nested.join(format!("{index:04}{}", "f".repeat(220))), b"x").unwrap();
    }
    let mut manifest = fixture.base_manifest();
    manifest.action = "remove_path".into();
    manifest.target_path = "tree".into();
    manifest.before = capture_snapshot(&fixture.root, "tree", None, &fixture.filesystem).unwrap();
    for (index, entry) in manifest.before.iter_mut().enumerate() {
        if entry.kind == "file" {
            entry.backup_file = Some(format!("before/{index:04}.bin"));
        }
    }
    assert!(manifest.before.len() <= MAX_SNAPSHOT_ENTRIES);
    assert!(serde_json::to_vec_pretty(&manifest).unwrap().len() > MAX_CHECKPOINT_METADATA_BYTES);
    let result = run_checkpointed_workspace_mutation_with_policy(
        &fixture.root,
        TASK,
        "remove_path",
        &target,
        MutationAfterShape::Missing,
        &fixture.filesystem,
        || fs::remove_dir_all(&target).map_err(|error| error.to_string()),
    );
    assert!(
        target.exists(),
        "oversized checkpoint allowed removal of its payload tree"
    );
    assert_eq!(fs::read_dir(nested).unwrap().count(), 3500);
    fixture.assert_pre_dispatch(result.err().unwrap());
}

#[test]
fn mutation_capacity_unexpected_after_shape_preserves_payload_and_backup() {
    let fixture = Fixture::new();
    let target = fixture.root.join("a.txt");
    let result = fixture.run(TASK, MutationAfterShape::RegularFile, || {
        fs::remove_file(&target).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("later.txt"), b"later-owned-edit").unwrap();
        Ok(())
    });
    assert_eq!(
        fs::read(target.join("later.txt")).unwrap(),
        b"later-owned-edit"
    );
    let error = match result {
        Ok(_) => panic!("unexpected after shape was accepted"),
        Err(error) => error,
    };
    fixture.assert_unknown(error, "after_state_capture");
}

#[test]
fn mutation_capacity_after_capture_failure_never_undoes_successful_payload() {
    let fixture = Fixture::new();
    let target = fixture.root.join("a.txt");
    let result = fixture.run(TASK, MutationAfterShape::RegularFile, || {
        fs::write(&target, b"after\n").unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&target)
            .unwrap()
            .set_len(MAX_SNAPSHOT_BYTES + 1)
            .unwrap();
        Ok(())
    });
    assert_eq!(fs::metadata(&target).unwrap().len(), MAX_SNAPSHOT_BYTES + 1);
    fixture.assert_unknown(result.err().unwrap(), "after_state_capture");
}

#[test]
fn mutation_capacity_exact_projected_fit_remains_readable_and_rewindable() {
    let fixture = Fixture::new();
    let mut manifest = fixture.sized_manifest(MAX_CHECKPOINT_METADATA_BYTES);
    let mut projected = projected_file(&manifest);
    let growth =
        serde_json::to_vec_pretty(&projected).unwrap().len() - MAX_CHECKPOINT_METADATA_BYTES;
    manifest.task_id.truncate(manifest.task_id.len() - growth);
    projected.task_id.clone_from(&manifest.task_id);
    assert_eq!(
        serde_json::to_vec_pretty(&projected).unwrap().len(),
        MAX_CHECKPOINT_METADATA_BYTES
    );
    let output = fixture
        .run(&manifest.task_id, MutationAfterShape::RegularFile, || {
            fs::write(fixture.root.join("a.txt"), b"after\n").map_err(|error| error.to_string())
        })
        .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    let (directory, applied) = fixture.checkpoint();
    assert_eq!(applied.state, "applied");
    assert!(
        fs::metadata(directory.join("manifest.json")).unwrap().len()
            <= MAX_CHECKPOINT_METADATA_BYTES as u64
    );
    rewind_structured_mutation_with_policy(
        &fixture.root,
        value["checkpoint_id"].as_str().unwrap(),
        &fixture.filesystem,
    )
    .unwrap();
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"before\n");
}

#[test]
fn mutation_capacity_declared_shapes_keep_normal_apply_and_rewind() {
    for shape in [
        MutationAfterShape::RegularFile,
        MutationAfterShape::Directory,
        MutationAfterShape::Missing,
    ] {
        let fixture = Fixture::new();
        let target = fixture.root.join("a.txt");
        let output = fixture
            .run(TASK, shape, || {
                match shape {
                    MutationAfterShape::RegularFile => fs::write(&target, b"after\n"),
                    MutationAfterShape::Directory => {
                        fs::remove_file(&target).unwrap();
                        fs::create_dir(&target)
                    }
                    MutationAfterShape::Missing => fs::remove_file(&target),
                }
                .map_err(|error| error.to_string())
            })
            .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["state"], "applied");
        rewind_structured_mutation_with_policy(
            &fixture.root,
            value["checkpoint_id"].as_str().unwrap(),
            &fixture.filesystem,
        )
        .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"before\n");
    }
}

#[tokio::test]
async fn mutation_capacity_actual_builtin_producers_refuse_oversized_metadata() {
    for (skill, args) in [
        ("write_file", json!({"path":"./a.txt","content":"after\n"})),
        (
            "write_file",
            json!({"path":"./a.txt","content":"after\n","append":true}),
        ),
        ("make_dir", json!({"path":"new/nested"})),
        ("remove_file", json!({"path":"a.txt"})),
    ] {
        let fixture = Fixture::new();
        let state = super::super::tests::test_state(fixture.root.clone());
        let task = crate::ClaimedTask {
            claim_attempt: 1,
            task_id: "t".repeat(MAX_CHECKPOINT_METADATA_BYTES),
            user_id: 1,
            chat_id: 1,
            user_key: None,
            channel: "ui".into(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".into(),
            payload_json: "{}".into(),
        };
        let result =
            super::super::execute_builtin_skill_for_task(&state, &task, skill, &args).await;
        assert_eq!(
            fs::read(fixture.root.join("a.txt")).unwrap(),
            b"before\n",
            "{skill}"
        );
        assert!(!fixture.root.join("new").exists(), "{skill}");
        fixture.assert_pre_dispatch(result.err().unwrap());
    }
}

#[test]
fn mutation_capacity_actual_replace_producer_refuses_oversized_metadata() {
    let fixture = Fixture::new();
    let args =
        json!({"action":"replace_text","path":"a.txt","old_text":"before","new_text":"after"})
            .as_object()
            .unwrap()
            .clone();
    let result = super::super::builtin_workspace_replace::execute_workspace_replace_with_policy(
        &fixture.root,
        &"t".repeat(MAX_CHECKPOINT_METADATA_BYTES),
        &args,
        &fixture.filesystem,
    );
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"before\n");
    fixture.assert_pre_dispatch(result.err().unwrap());
}

#[test]
fn mutation_capacity_wrong_single_entry_kind_is_unknown_not_success() {
    let fixture = Fixture::new();
    let target = fixture.root.join("a.txt");
    let result = fixture.run(TASK, MutationAfterShape::Missing, || {
        fs::write(&target, b"after\n").unwrap();
        Ok(())
    });
    assert_eq!(fs::read(&target).unwrap(), b"after\n");
    fixture.assert_unknown(result.err().unwrap(), "after_state_validation");
}

#[test]
fn mutation_capacity_missing_entries_still_consume_the_snapshot_budget() {
    let fixture = Fixture::new();
    let mut entries = Vec::new();
    let mut budget = SnapshotBudget {
        pending_entries: 0,
        entries: 1,
        bytes: 0,
        max_entries: 1,
    };
    let result = capture_path(
        &fixture.root,
        &fixture.root.join("absent"),
        "absent",
        None,
        &mut entries,
        &mut budget,
        &fixture.filesystem,
    );
    assert!(result.is_err());
    assert!(entries.is_empty());
    assert_eq!(budget.entries, 2);
}
