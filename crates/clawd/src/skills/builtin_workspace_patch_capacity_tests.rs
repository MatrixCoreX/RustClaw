use super::*;

const TASK: &str = "owned-checkpoint-capacity";
const CHECKPOINT: &str = "patch_owned_capacity";
const PATCH: &str =
    "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-before\n+after\n";

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    filesystem: HostFilesystemPolicy,
}

impl Fixture {
    fn new(git: bool) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        fs::write(root.join("a.txt"), b"before\n").unwrap();
        if git {
            assert!(Command::new("git")
                .args(["init", "-q"])
                .current_dir(&root)
                .status()
                .unwrap()
                .success());
        }
        Self {
            _temporary: temporary,
            root,
            filesystem: HostFilesystemPolicy::without_private_authority(),
        }
    }

    fn manifest(&self, size: usize) -> PatchCheckpoint {
        let mut manifest = prepared_manifest(vec![CheckpointFile {
            path: "a.txt".into(),
            existed: true,
            before_sha256: Some(sha256_label(b"before\n")),
            after_sha256: None,
            backup_file: Some("before/0.bin".into()),
            additions: Some(1),
            deletions: Some(1),
        }]);
        manifest.task_id.clear();
        let overhead = serde_json::to_vec_pretty(&manifest).unwrap().len();
        manifest.task_id = "t".repeat(size.checked_sub(overhead).unwrap());
        assert_eq!(serde_json::to_vec_pretty(&manifest).unwrap().len(), size);
        manifest
    }

    fn apply(&self, task: &str, patch: &str) -> Result<String, String> {
        let args = serde_json::json!({"action":"apply_patch","patch":patch})
            .as_object()
            .unwrap()
            .clone();
        apply_patch_at_checkpoint(
            &self.root,
            task,
            &args,
            &self.filesystem,
            ToolSandboxBackend::Auto,
            CHECKPOINT,
        )
    }

    fn assert_rejected_before_dispatch(&self, result: Result<String, String>) {
        assert_eq!(fs::read(self.root.join("a.txt")).unwrap(), b"before\n");
        let error = match result {
            Ok(_) => panic!("oversized checkpoint was accepted"),
            Err(error) => error,
        };
        let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
        assert_eq!(parsed.error_code, "checkpoint_too_large");
        let extra = parsed.extra.unwrap();
        assert_eq!(extra["side_effect_applied"], false);
        assert_eq!(extra["failure_phase"], "pre_dispatch");
        assert_eq!(extra["recovery_action"], "replan_arguments");
        assert_eq!(extra["details"]["metadata_scope"], "projected_metadata");
        assert!(!checkpoint_root(&self.root).join(CHECKPOINT).exists());
    }
}

fn prepared_manifest(files: Vec<CheckpointFile>) -> PatchCheckpoint {
    PatchCheckpoint {
        schema_version: CHECKPOINT_SCHEMA_VERSION,
        checkpoint_id: CHECKPOINT.into(),
        task_id: TASK.into(),
        patch_id: sha256_label(PATCH.as_bytes()),
        state: "prepared".into(),
        created_at: crate::now_ts_u64() as i64,
        files,
    }
}

#[test]
fn checkpoint_capacity_writer_accepts_exact_reader_boundary() {
    let fixture = Fixture::new(false);
    let directory = create_checkpoint_dir(&fixture.root, CHECKPOINT, &fixture.filesystem).unwrap();
    let manifest = fixture.manifest(MAX_CHECKPOINT_METADATA_BYTES);
    write_manifest(&directory, &manifest, &fixture.filesystem).unwrap();
    assert_eq!(
        fs::metadata(directory.join("manifest.json")).unwrap().len(),
        MAX_CHECKPOINT_METADATA_BYTES as u64
    );
    let restored = read_manifest(&directory, &fixture.filesystem).unwrap();
    assert_eq!(restored.task_id, manifest.task_id);
}

#[test]
fn checkpoint_capacity_writer_refuses_overflow_without_replacing_valid_metadata() {
    let fixture = Fixture::new(false);
    let directory = create_checkpoint_dir(&fixture.root, CHECKPOINT, &fixture.filesystem).unwrap();
    let accepted = prepared_manifest(Vec::new());
    write_manifest(&directory, &accepted, &fixture.filesystem).unwrap();
    let path = directory.join("manifest.json");
    let before = fs::read(&path).unwrap();
    let rejected = fixture.manifest(MAX_CHECKPOINT_METADATA_BYTES + 1);
    let error = match write_manifest(&directory, &rejected, &fixture.filesystem) {
        Ok(()) => panic!("writer published metadata exceeding its reader limit"),
        Err(error) => error,
    };
    assert_eq!(
        crate::skills::parse_structured_skill_error(&error)
            .unwrap()
            .error_code,
        "checkpoint_too_large"
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        read_manifest(&directory, &fixture.filesystem)
            .unwrap()
            .task_id,
        TASK
    );
    fs::remove_file(&path).unwrap();
    assert!(write_manifest(&directory, &rejected, &fixture.filesystem).is_err());
    assert!(!path.exists());
}

#[test]
fn checkpoint_capacity_prepared_overflow_refuses_native_and_git_payloads() {
    for git in [false, true] {
        let fixture = Fixture::new(git);
        let manifest = fixture.manifest(MAX_CHECKPOINT_METADATA_BYTES + 1);
        fixture.assert_rejected_before_dispatch(fixture.apply(&manifest.task_id, PATCH));
    }
}

#[test]
fn checkpoint_capacity_reserves_after_hash_growth_before_native_and_git_payloads() {
    for git in [false, true] {
        let fixture = Fixture::new(git);
        let prepared = fixture.manifest(MAX_CHECKPOINT_METADATA_BYTES);
        let mut applied = prepared.clone();
        applied.state = "applied".into();
        applied.files[0].after_sha256 = Some(sha256_label(b"after\n"));
        assert!(serde_json::to_vec_pretty(&applied).unwrap().len() > MAX_CHECKPOINT_METADATA_BYTES);
        fixture.assert_rejected_before_dispatch(fixture.apply(&prepared.task_id, PATCH));
    }
}

#[test]
fn checkpoint_capacity_real_git_many_file_patch_refuses_before_any_payload() {
    let fixture = Fixture::new(true);
    let mut patch = String::new();
    let mut files = Vec::new();
    for index in 0..9000 {
        let path = format!("f{index:05}.txt");
        patch.push_str(&format!(
            "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1 @@\n+x\n"
        ));
        files.push(CheckpointFile {
            path,
            existed: false,
            before_sha256: None,
            after_sha256: None,
            backup_file: None,
            additions: Some(1),
            deletions: Some(0),
        });
    }
    let prepared = prepared_manifest(files);
    let mut applied = prepared.clone();
    applied.state = "applied".into();
    for file in &mut applied.files {
        file.after_sha256 = Some(sha256_label(b"x\n"));
    }
    assert!(patch.len() < MAX_PATCH_BYTES);
    assert!(serde_json::to_vec_pretty(&prepared).unwrap().len() < MAX_CHECKPOINT_METADATA_BYTES);
    assert!(serde_json::to_vec_pretty(&applied).unwrap().len() > MAX_CHECKPOINT_METADATA_BYTES);
    let result = fixture.apply(TASK, &patch);
    let count = fs::read_dir(&fixture.root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_str().unwrap().starts_with('f'))
        .count();
    assert_eq!(
        count, 0,
        "new payload files were created before capacity refusal"
    );
    fixture.assert_rejected_before_dispatch(result);
}

#[test]
fn checkpoint_capacity_small_native_and_git_patch_still_diff_and_rewind() {
    for git in [false, true] {
        let fixture = Fixture::new(git);
        let output = fixture.apply(TASK, PATCH).unwrap();
        let result: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(result["status"], "ok");
        assert_eq!(
            result["patch_engine"],
            if git { "git" } else { "pure_rust" }
        );
        let directory = checkpoint_root(&fixture.root).join(CHECKPOINT);
        assert_eq!(
            read_manifest(&directory, &fixture.filesystem)
                .unwrap()
                .state,
            "applied"
        );
        let args = serde_json::json!({"action":"diff","checkpoint_id":CHECKPOINT})
            .as_object()
            .unwrap()
            .clone();
        let output = diff(
            &fixture.root,
            &args,
            &fixture.filesystem,
            ToolSandboxBackend::Auto,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&output).unwrap()["patch"],
            PATCH
        );
        let args = serde_json::json!({"action":"rewind","checkpoint_id":CHECKPOINT})
            .as_object()
            .unwrap()
            .clone();
        rewind(&fixture.root, &args, &fixture.filesystem).unwrap();
        assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"before\n");
    }
}

#[test]
fn checkpoint_capacity_reserves_recovery_state_growth_before_dispatch() {
    for git in [false, true] {
        let fixture = Fixture::new(git);
        let mut prepared = fixture.manifest(MAX_CHECKPOINT_METADATA_BYTES);
        let mut applied = prepared.clone();
        applied.state = "applied".into();
        applied.files[0].after_sha256 = Some(sha256_label(b"after\n"));
        let growth = serde_json::to_vec_pretty(&applied).unwrap().len()
            - serde_json::to_vec_pretty(&prepared).unwrap().len();
        prepared.task_id.truncate(prepared.task_id.len() - growth);
        applied.task_id.clone_from(&prepared.task_id);
        assert_eq!(
            serde_json::to_vec_pretty(&applied).unwrap().len(),
            MAX_CHECKPOINT_METADATA_BYTES
        );
        applied.state = CHECKPOINT_RESTORE_FAILED_STATE.into();
        assert!(serde_json::to_vec_pretty(&applied).unwrap().len() > MAX_CHECKPOINT_METADATA_BYTES);
        fixture.assert_rejected_before_dispatch(fixture.apply(&prepared.task_id, PATCH));
    }
}

#[test]
fn checkpoint_capacity_exact_projected_boundary_remains_usable() {
    for git in [false, true] {
        let fixture = Fixture::new(git);
        let mut prepared = fixture.manifest(MAX_CHECKPOINT_METADATA_BYTES);
        let mut projected = prepared.clone();
        projected.state = CHECKPOINT_RESTORE_FAILED_STATE.into();
        projected.files[0].after_sha256 = Some(sha256_label(b"after\n"));
        let growth = serde_json::to_vec_pretty(&projected).unwrap().len()
            - serde_json::to_vec_pretty(&prepared).unwrap().len();
        prepared.task_id.truncate(prepared.task_id.len() - growth);
        projected.task_id.clone_from(&prepared.task_id);
        assert_eq!(
            serde_json::to_vec_pretty(&projected).unwrap().len(),
            MAX_CHECKPOINT_METADATA_BYTES
        );
        fixture.apply(&prepared.task_id, PATCH).unwrap();
        let directory = checkpoint_root(&fixture.root).join(CHECKPOINT);
        let applied = read_manifest(&directory, &fixture.filesystem).unwrap();
        assert_eq!(applied.task_id, prepared.task_id);
        assert_eq!(applied.state, "applied");
        assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"after\n");
        let args = serde_json::json!({"action":"rewind","checkpoint_id":CHECKPOINT})
            .as_object()
            .unwrap()
            .clone();
        rewind(&fixture.root, &args, &fixture.filesystem).unwrap();
        assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"before\n");
    }
}
