use super::execute_workspace_replace_for_root;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn new(content: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "agent-runtime-workspace-batch-edit-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).expect("create workspace");
        fs::write(root.join("sample.txt"), content).expect("fixture");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn content(&self) -> String {
        fs::read_to_string(self.root.join("sample.txt")).expect("read fixture")
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn args(value: Value) -> Map<String, Value> {
    value.as_object().expect("object").clone()
}

fn run(workspace: &Workspace, value: Value) -> Result<Value, String> {
    execute_workspace_replace_for_root(workspace.path(), "task-batch-edit", &args(value))
        .and_then(|output| serde_json::from_str(&output).map_err(|error| error.to_string()))
}

fn error_code(error: String) -> String {
    crate::skills::parse_structured_skill_error(&error)
        .expect("structured error")
        .error_code
}

#[test]
fn replace_all_honors_exact_occurrence_constraint_and_rewinds() {
    let workspace = Workspace::new("same same same\n");
    let applied = run(
        &workspace,
        json!({
            "action": "replace_text",
            "path": "sample.txt",
            "old_text": "same",
            "new_text": "new",
            "replace_all": true,
            "expected_occurrences": 3,
        }),
    )
    .expect("replace all");
    assert_eq!(applied["replacement_count"], 3);
    assert_eq!(applied["edit_count"], 1);
    assert_eq!(workspace.content(), "new new new\n");

    let checkpoint_id = applied["checkpoint_id"].as_str().expect("checkpoint");
    let diff = super::super::builtin_workspace_patch::execute_workspace_patch_for_root(
        workspace.path(),
        "task-batch-edit",
        &args(json!({"action": "diff", "checkpoint_id": checkpoint_id})),
    )
    .expect("checkpoint diff");
    let diff: Value = serde_json::from_str(&diff).expect("diff json");
    assert_eq!(diff["action"], "diff");
    assert_eq!(diff["state"], "applied");

    super::super::builtin_workspace_patch::execute_workspace_patch_for_root(
        workspace.path(),
        "task-batch-edit",
        &args(json!({"action": "rewind", "checkpoint_id": checkpoint_id})),
    )
    .expect("rewind");
    assert_eq!(workspace.content(), "same same same\n");
}

#[test]
fn replace_all_without_count_is_explicit_and_bounded() {
    let workspace = Workspace::new("a a a\n");
    let output = run(
        &workspace,
        json!({
            "action": "preview_replace_text",
            "path": "sample.txt",
            "old_text": "a",
            "new_text": "b",
            "replace_all": true,
        }),
    )
    .expect("preview replace all");
    assert_eq!(output["occurrence_count"], 3);
    assert_eq!(workspace.content(), "a a a\n");
}

#[test]
fn batch_edits_apply_sequentially_in_one_checkpoint() {
    let workspace = Workspace::new("left left right\n");
    let hash = format!(
        "sha256:{:x}",
        Sha256::digest(workspace.content().as_bytes())
    );
    let applied = run(
        &workspace,
        json!({
            "action": "replace_text",
            "path": "sample.txt",
            "expected_sha256": hash,
            "edits": [
                {"old_text": "left", "new_text": "middle", "replace_all": true, "expected_occurrences": 2},
                {"old_text": "middle middle right", "new_text": "done"}
            ],
        }),
    )
    .expect("batch edit");
    assert_eq!(applied["edit_count"], 2);
    assert_eq!(applied["replacement_count"], 3);
    assert_eq!(workspace.content(), "done\n");
    assert!(applied["checkpoint_id"].is_string());
}

#[test]
fn failed_batch_edit_never_writes_partial_content() {
    let workspace = Workspace::new("alpha beta\n");
    let error = run(
        &workspace,
        json!({
            "action": "replace_text",
            "path": "sample.txt",
            "edits": [
                {"old_text": "alpha", "new_text": "changed"},
                {"old_text": "missing", "new_text": "never"}
            ],
        }),
    )
    .expect_err("second edit must fail");
    assert_eq!(error_code(error), "replacement_target_not_found");
    assert_eq!(workspace.content(), "alpha beta\n");
}

#[test]
fn multiple_occurrences_require_explicit_replace_all() {
    let workspace = Workspace::new("x x\n");
    let error = run(
        &workspace,
        json!({
            "action": "replace_text",
            "path": "sample.txt",
            "old_text": "x",
            "new_text": "y",
            "expected_occurrences": 2,
        }),
    )
    .expect_err("replace_all required");
    assert_eq!(error_code(error), "invalid_expected_occurrences");
    assert_eq!(workspace.content(), "x x\n");
}

#[test]
fn replace_all_rejects_stale_occurrence_count() {
    let workspace = Workspace::new("x x\n");
    let error = run(
        &workspace,
        json!({
            "action": "replace_text",
            "path": "sample.txt",
            "old_text": "x",
            "new_text": "y",
            "replace_all": true,
            "expected_occurrences": 3,
        }),
    )
    .expect_err("occurrence mismatch");
    assert_eq!(error_code(error), "replacement_occurrence_mismatch");
    assert_eq!(workspace.content(), "x x\n");
}
