use super::*;

const TASK: &str = "owned-checkpoint-finalization";
const PATCH: &str =
    "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-before\n+after\n";

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

    fn applied_patch(&self) -> (PathBuf, PatchCheckpoint) {
        let args = serde_json::json!({"action":"apply_patch","patch":PATCH})
            .as_object()
            .unwrap()
            .clone();
        let result: Value = serde_json::from_str(
            &apply_patch(
                &self.root,
                TASK,
                &args,
                &self.filesystem,
                ToolSandboxBackend::Auto,
            )
            .unwrap(),
        )
        .unwrap();
        let directory = resolve_checkpoint_dir(
            &self.root,
            result["checkpoint_id"].as_str().unwrap(),
            &self.filesystem,
        )
        .unwrap();
        let manifest = read_manifest(&directory, &self.filesystem).unwrap();
        (directory, manifest)
    }
}

fn block_publication(directory: &Path) {
    fs::remove_file(directory.join("manifest.json")).unwrap();
    fs::create_dir(directory.join("manifest.json")).unwrap();
}

fn assert_unknown_commit(error: &str, checkpoint_id: &str) {
    let parsed = crate::skills::parse_structured_skill_error(error).unwrap();
    assert_eq!(parsed.error_code, "checkpoint_commit_unconfirmed");
    let extra = parsed.extra.unwrap();
    assert_eq!(extra["checkpoint_id"], checkpoint_id);
    assert_eq!(extra["task_id"], TASK);
    assert_eq!(extra["retryable"], false);
    assert_eq!(extra["failure_phase"], "commit");
    assert_eq!(extra["commit_state"], "unknown");
    assert!(extra.get("side_effect_applied").unwrap().is_null());
    assert_eq!(extra["compensation_attempted"], false);
    assert_eq!(extra["checkpoint_cleanup_attempted"], false);
    assert!(extra.get("checkpoint_retained").is_none());
    assert_eq!(extra["complete"], false);
    assert_eq!(
        extra["artifact_refs"][0]["ref"],
        format!("workspace_checkpoint:{checkpoint_id}")
    );
    assert_eq!(extra["recovery"]["tool"], "workspace_patch");
    assert_eq!(extra["recovery"]["action"], "diff");
    assert_eq!(extra["recovery"]["auto_retry"], false);
}

#[test]
fn checkpoint_finalization_patch_publication_control_keeps_normal_success() {
    let fixture = Fixture::new();
    let (directory, manifest) = fixture.applied_patch();
    publish_applied_checkpoint(&directory, &manifest, &fixture.filesystem).unwrap();
    assert_eq!(
        read_manifest(&directory, &fixture.filesystem)
            .unwrap()
            .state,
        "applied"
    );
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"after\n");
}

#[test]
fn checkpoint_finalization_patch_publication_refusal_does_not_undo_applied_files() {
    let fixture = Fixture::new();
    let (directory, manifest) = fixture.applied_patch();
    let backup = directory.join(manifest.files[0].backup_file.as_ref().unwrap());
    block_publication(&directory);
    let error = publish_applied_checkpoint(&directory, &manifest, &fixture.filesystem).unwrap_err();
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"after\n");
    assert_eq!(fs::read(backup).unwrap(), b"before\n");
    assert_unknown_commit(&error, &manifest.checkpoint_id);
}

#[test]
fn checkpoint_finalization_patch_refusal_does_not_overwrite_a_later_workspace_edit() {
    let fixture = Fixture::new();
    let (directory, manifest) = fixture.applied_patch();
    fs::write(fixture.root.join("a.txt"), b"later-owned-edit\n").unwrap();
    block_publication(&directory);
    let error = publish_applied_checkpoint(&directory, &manifest, &fixture.filesystem).unwrap_err();
    assert_eq!(
        fs::read(fixture.root.join("a.txt")).unwrap(),
        b"later-owned-edit\n"
    );
    assert!(directory.exists());
    assert_unknown_commit(&error, &manifest.checkpoint_id);
}

#[test]
fn checkpoint_finalization_structured_publication_control_keeps_normal_success() {
    let fixture = Fixture::new();
    let target = fixture.root.join("a.txt");
    let result =
        super::super::builtin_workspace_mutation::run_checkpointed_workspace_mutation_with_policy(
            &fixture.root,
            TASK,
            "write_text",
            &target,
            super::super::builtin_workspace_mutation::MutationAfterShape::RegularFile,
            &fixture.filesystem,
            || fs::write(&target, b"after\n").map_err(|error| error.to_string()),
        )
        .unwrap();
    let result: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["status"], "ok");
    assert_eq!(result["state"], "applied");
    assert_eq!(fs::read(&target).unwrap(), b"after\n");
}

#[test]
fn checkpoint_finalization_structured_actual_entry_refusal_keeps_payload_and_evidence() {
    let fixture = Fixture::new();
    let target = fixture.root.join("a.txt");
    let mut retained = None;
    let error =
        super::super::builtin_workspace_mutation::run_checkpointed_workspace_mutation_with_policy(
            &fixture.root,
            TASK,
            "write_text",
            &target,
            super::super::builtin_workspace_mutation::MutationAfterShape::RegularFile,
            &fixture.filesystem,
            || {
                fs::write(&target, b"after\n").unwrap();
                let directories = fs::read_dir(checkpoint_root(&fixture.root))
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .collect::<Vec<_>>();
                assert_eq!(directories.len(), 1);
                let directory = directories.into_iter().next().unwrap();
                let manifest: Value =
                    serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap())
                        .unwrap();
                assert_eq!(manifest["task_id"], TASK);
                let backup = directory.join(manifest["before"][0]["backup_file"].as_str().unwrap());
                retained = Some((
                    directory.clone(),
                    backup,
                    manifest["checkpoint_id"].as_str().unwrap().to_string(),
                ));
                block_publication(&directory);
                Ok(())
            },
        )
        .unwrap_err();
    let (directory, backup, checkpoint_id) = retained.unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"after\n");
    assert_eq!(fs::read(backup).unwrap(), b"before\n");
    assert!(directory.exists());
    assert_unknown_commit(&error, &checkpoint_id);
}
