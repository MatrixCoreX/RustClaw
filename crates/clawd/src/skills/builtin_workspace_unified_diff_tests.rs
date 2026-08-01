use super::execute_workspace_patch_for_root;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "agent-runtime-pure-unified-diff-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).expect("create workspace");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn run(&self, value: Value) -> Result<Value, String> {
        let args: Map<String, Value> = value.as_object().expect("object").clone();
        execute_workspace_patch_for_root(&self.root, "task-unified-diff", &args)
            .and_then(|output| serde_json::from_str(&output).map_err(|error| error.to_string()))
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn non_git_workspace_uses_pure_rust_create_delete_and_rewind() {
    let workspace = Workspace::new();
    fs::write(workspace.path().join("old.txt"), "old\n").expect("old fixture");
    let patch = "diff --git a/old.txt b/old.txt\ndeleted file mode 100644\n--- a/old.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-old\ndiff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+new\n+file\n";

    let applied = workspace
        .run(json!({"action": "apply_patch", "patch": patch}))
        .expect("pure Rust patch");
    assert_eq!(applied["patch_engine"], "pure_rust");
    assert!(!workspace.path().join("old.txt").exists());
    assert_eq!(
        fs::read_to_string(workspace.path().join("new.txt")).expect("new file"),
        "new\nfile\n"
    );

    let diff = workspace
        .run(json!({
            "action": "diff",
            "checkpoint_id": applied["checkpoint_id"],
        }))
        .expect("checkpoint diff");
    assert_eq!(diff["patch"], patch);
    workspace
        .run(json!({
            "action": "rewind",
            "checkpoint_id": applied["checkpoint_id"],
        }))
        .expect("rewind");
    assert_eq!(
        fs::read_to_string(workspace.path().join("old.txt")).expect("restored old"),
        "old\n"
    );
    assert!(!workspace.path().join("new.txt").exists());
}

#[test]
fn git_workspace_keeps_git_check_and_apply_engine() {
    let workspace = Workspace::new();
    let initialized = Command::new("git")
        .args(["init", "-q"])
        .current_dir(workspace.path())
        .status()
        .expect("git executable");
    assert!(initialized.success());
    fs::write(workspace.path().join("note.txt"), "before\n").expect("fixture");
    let patch = "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1 +1 @@\n-before\n+after\n";

    let applied = workspace
        .run(json!({"action": "apply_patch", "patch": patch}))
        .expect("git patch");
    assert_eq!(applied["patch_engine"], "git");
    assert_eq!(
        fs::read_to_string(workspace.path().join("note.txt")).expect("patched"),
        "after\n"
    );
}

#[test]
fn pure_rust_parser_preserves_no_newline_marker() {
    let workspace = Workspace::new();
    fs::write(workspace.path().join("note.txt"), "before").expect("fixture");
    let patch = "--- a/note.txt\n+++ b/note.txt\n@@ -1 +1 @@\n-before\n\\ No newline at end of file\n+after\n\\ No newline at end of file\n";

    let applied = workspace
        .run(json!({"action": "apply_patch", "patch": patch}))
        .expect("patch without final newline");
    assert_eq!(applied["patch_engine"], "pure_rust");
    assert_eq!(
        fs::read(workspace.path().join("note.txt")).unwrap(),
        b"after"
    );
}

#[test]
fn pure_rust_parser_preserves_crlf_context() {
    let workspace = Workspace::new();
    fs::write(workspace.path().join("note.txt"), b"before\r\n").expect("fixture");
    let patch = "--- a/note.txt\r\n+++ b/note.txt\r\n@@ -1 +1 @@\r\n-before\r\n+after\r\n";

    workspace
        .run(json!({"action": "apply_patch", "patch": patch}))
        .expect("CRLF patch");
    assert_eq!(
        fs::read(workspace.path().join("note.txt")).unwrap(),
        b"after\r\n"
    );
}

#[test]
fn pure_rust_parser_rejects_inconsistent_new_hunk_position() {
    let workspace = Workspace::new();
    fs::write(workspace.path().join("note.txt"), "before\n").expect("fixture");
    let patch = "--- a/note.txt\n+++ b/note.txt\n@@ -1 +2 @@\n-before\n+after\n";

    let error = workspace
        .run(json!({"action": "apply_patch", "patch": patch}))
        .expect_err("malformed hunk must fail");
    assert!(error.contains("patch_context_mismatch"));
    assert_eq!(
        fs::read_to_string(workspace.path().join("note.txt")).unwrap(),
        "before\n"
    );
}
