use super::*;
use std::fs;

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    path: PathBuf,
    manifest: PatchTransactionManifest,
    attempt: attempt::ClaimedAttempt,
    filesystem: HostFilesystemPolicy,
}

impl Fixture {
    fn applied() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let filesystem = HostFilesystemPolicy::without_private_authority();
        fs::write(root.join("a.txt"), b"before\n").unwrap();
        let patch =
            "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-before\n+after\n";
        let args = json!({"action":"apply_patch","patch":patch})
            .as_object()
            .unwrap()
            .clone();
        let mut manifest = PatchTransactionManifest {
            schema_version: TRANSACTION_SCHEMA_VERSION,
            transaction_id: "commit_fixture".into(),
            task_id: "owned-commit-task".into(),
            shard_count: 1,
            state: "planned".into(),
            shards: Vec::new(),
        };
        let path = transaction_manifest_path(&root, &manifest.transaction_id, &filesystem).unwrap();
        let attempt = attempt::ClaimedAttempt::reserve(
            &root,
            &manifest,
            0,
            &request_digest(&args).unwrap(),
            &super::super::super::sha256_label(patch.as_bytes()),
            &filesystem,
        )
        .unwrap();
        let applied = super::super::super::apply_patch_at_checkpoint(
            &root,
            "owned-commit-task",
            &args,
            &filesystem,
            ToolSandboxBackend::Auto,
            attempt.checkpoint_id(),
        )
        .unwrap();
        let applied: Value = serde_json::from_str(&applied).unwrap();
        let record = PatchShardRecord {
            shard_index: 0,
            checkpoint_id: applied["checkpoint_id"].as_str().unwrap().to_owned(),
            patch_id: applied["patch_id"].as_str().unwrap().to_owned(),
            state: "applied".into(),
            request_digest: Some(request_digest(&args).unwrap()),
        };
        manifest.state = "applied".into();
        manifest.shards.push(record);
        Self {
            _temporary: temporary,
            root,
            path,
            manifest,
            attempt,
            filesystem,
        }
    }

    fn checkpoint(&self) -> PathBuf {
        super::super::super::checkpoint_root(&self.root)
            .join(&self.manifest.shards[0].checkpoint_id)
    }

    fn publish(&self) -> Result<String, String> {
        super::publish(
            &self.root,
            &self.path,
            &self.manifest,
            &self.manifest.shards[0],
            &self.attempt,
            &self.filesystem,
        )
    }

    fn refuse_publication(&self) {
        fs::create_dir(&self.path).unwrap();
    }

    fn assert_uncertain(&self, error: &str) {
        let parsed = crate::skills::parse_structured_skill_error(error).unwrap();
        assert_eq!(parsed.error_code, "transaction_commit_unconfirmed");
        let extra = parsed.extra.unwrap();
        assert_eq!(extra["retryable"], false);
        assert_eq!(extra["compensation_attempted"], false);
        assert_eq!(
            extra["checkpoint_id"],
            self.manifest.shards[0].checkpoint_id
        );
        assert_eq!(extra["transaction_id"], self.manifest.transaction_id);
        assert_ne!(extra["side_effect_applied"], false);
    }
}

#[test]
fn commit_publication_failure_retains_the_applied_checkpoint_without_blind_rollback() {
    let fixture = Fixture::applied();
    fixture.refuse_publication();
    let error = fixture.publish().unwrap_err();
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"after\n");
    let checkpoint: Value =
        serde_json::from_slice(&fs::read(fixture.checkpoint().join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(checkpoint["state"], "applied");
    fixture.assert_uncertain(&error);
}

#[test]
fn commit_publication_failure_retains_corrupt_backup_evidence_without_unasked_repair() {
    let fixture = Fixture::applied();
    fs::write(
        fixture.checkpoint().join("before/0.bin"),
        b"owned corrupt backup",
    )
    .unwrap();
    fixture.refuse_publication();
    let error = fixture.publish().unwrap_err();
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"after\n");
    assert_eq!(
        fs::read(fixture.checkpoint().join("before/0.bin")).unwrap(),
        b"owned corrupt backup"
    );
    fixture.assert_uncertain(&error);
}

#[test]
fn commit_publication_failure_does_not_undo_a_new_user_edit() {
    let fixture = Fixture::applied();
    fs::write(fixture.root.join("a.txt"), b"user-owned edit\n").unwrap();
    fixture.refuse_publication();
    let error = fixture.publish().unwrap_err();
    assert_eq!(
        fs::read(fixture.root.join("a.txt")).unwrap(),
        b"user-owned edit\n"
    );
    fixture.assert_uncertain(&error);
}

#[test]
fn commit_publication_success_keeps_the_normal_result_and_shard_binding() {
    let fixture = Fixture::applied();
    let result: Value = serde_json::from_str(&fixture.publish().unwrap()).unwrap();
    assert_eq!(result["outcome"], "shard_applied");
    assert_eq!(result["complete"], true);
    assert_eq!(
        read_manifest(&fixture.root, &fixture.path, &fixture.filesystem)
            .unwrap()
            .shards[0]
            .checkpoint_id,
        fixture.manifest.shards[0].checkpoint_id
    );
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"after\n");
}
