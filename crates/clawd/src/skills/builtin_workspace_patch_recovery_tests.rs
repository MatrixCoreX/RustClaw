use super::*;
use std::os::unix::fs::DirBuilderExt;

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
        let private = root.join("private");
        fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
        let filesystem = HostFilesystemPolicy::pin_directories(&[private], &root).unwrap();
        fs::write(root.join("public.txt"), b"before\n").unwrap();
        let checkpoint =
            create_checkpoint_dir(&root, "patch_recovery_fixture", &filesystem).unwrap();
        let files = snapshot_files(
            &root,
            &checkpoint,
            &[PatchStat {
                path: "public.txt".into(),
                additions: Some(1),
                deletions: Some(1),
            }],
            &filesystem,
        )
        .unwrap();
        write_manifest(
            &checkpoint,
            &PatchCheckpoint {
                schema_version: CHECKPOINT_SCHEMA_VERSION,
                checkpoint_id: "patch_recovery_fixture".into(),
                task_id: "owned-recovery-fixture".into(),
                patch_id: "sha256:owned-fixture".into(),
                state: "prepared".into(),
                created_at: 0,
                files: files.clone(),
            },
            &filesystem,
        )
        .unwrap();
        fs::write(root.join("public.txt"), b"partially-applied\n").unwrap();
        Self {
            _temporary: temporary,
            root,
            checkpoint,
            files,
            filesystem,
        }
    }

    fn compensate(&self, filesystem: &HostFilesystemPolicy) -> String {
        compensate_failed_patch(
            &self.root,
            &self.checkpoint,
            &self.files,
            patch_error(
                "fixture_execution_failed",
                "workspace.patch.fixture_execution_failed",
                Value::Null,
            ),
            filesystem,
        )
    }
}

#[test]
fn native_mutation_compensation_keeps_backups_when_restore_verification_fails() {
    let fixture = Fixture::new();
    fs::write(
        fixture.checkpoint.join("before/0.bin"),
        b"corrupt-owned-backup",
    )
    .unwrap();
    let error = fixture.compensate(&fixture.filesystem);
    let retained = fixture.checkpoint.join("before/0.bin").exists();
    eprintln!("NATIVE_MUTATION_COMPENSATION case=corrupt_backup actual_helper=true checkpoint_retained={retained} llm_calls=0");
    assert!(
        retained,
        "failed compensation must not discard recovery evidence"
    );
    assert_eq!(
        read_manifest(&fixture.checkpoint, &fixture.filesystem)
            .unwrap()
            .state,
        "restore_failed"
    );
    assert_eq!(
        fs::read(fixture.root.join("public.txt")).unwrap(),
        b"partially-applied\n"
    );
    let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
    assert_eq!(parsed.error_code, "patch_apply_and_restore_failed");
    let extra = parsed.extra.unwrap();
    assert_eq!(extra["checkpoint_retained"], true);
    assert_ne!(extra["side_effect_applied"], false);
}

#[test]
fn native_mutation_compensation_keeps_backups_after_target_becomes_private() {
    let fixture = Fixture::new();
    let data = fixture.root.join("data");
    fs::DirBuilder::new().mode(0o700).create(&data).unwrap();
    fs::rename(fixture.root.join("public.txt"), data.join("public.txt")).unwrap();
    let mut files = fixture.files.clone();
    files[0].path = "data/public.txt".into();
    let filesystem = HostFilesystemPolicy::pin_directories(&[data.clone()], &fixture.root).unwrap();
    let error = compensate_failed_patch(
        &fixture.root,
        &fixture.checkpoint,
        &files,
        patch_error(
            "fixture_execution_failed",
            "workspace.patch.fixture_execution_failed",
            Value::Null,
        ),
        &filesystem,
    );
    let retained = fixture.checkpoint.join("before/0.bin").exists();
    eprintln!("NATIVE_MUTATION_COMPENSATION case=private_target actual_helper=true checkpoint_retained={retained} llm_calls=0");
    assert!(retained);
    assert_eq!(
        fs::read(data.join("public.txt")).unwrap(),
        b"partially-applied\n"
    );
    assert_eq!(
        crate::skills::parse_structured_skill_error(&error)
            .unwrap()
            .error_code,
        "patch_apply_and_restore_failed"
    );
}

#[test]
fn native_mutation_compensation_preserves_failure_after_successful_restore() {
    let fixture = Fixture::new();
    let error = fixture.compensate(&fixture.filesystem);
    assert_eq!(
        fs::read(fixture.root.join("public.txt")).unwrap(),
        b"before\n"
    );
    assert!(!fixture.checkpoint.exists());
    assert_eq!(
        crate::skills::parse_structured_skill_error(&error)
            .unwrap()
            .error_code,
        "fixture_execution_failed"
    );
}
