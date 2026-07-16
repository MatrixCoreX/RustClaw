use super::{execute_workspace_patch_for_root, sha256_label};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "rustclaw-workspace-patch-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).expect("create workspace");
        Self { root }
    }

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().expect("object").clone()
    }

    fn run(&self, value: Value) -> Result<Value, String> {
        let text = execute_workspace_patch_for_root(
            &self.root,
            "task-workspace-patch-test",
            &Self::args(value),
        )?;
        serde_json::from_str(&text).map_err(|err| err.to_string())
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, content).expect("write fixture");
}

#[test]
fn applies_multi_file_patch_and_rewinds_checkpoint() {
    let workspace = TestWorkspace::new();
    write(&workspace.root.join("src/a.txt"), "alpha\n");
    write(&workspace.root.join("src/b.txt"), "one\n");
    let patch = "diff --git a/src/a.txt b/src/a.txt\n--- a/src/a.txt\n+++ b/src/a.txt\n@@ -1 +1 @@\n-alpha\n+beta\ndiff --git a/src/b.txt b/src/b.txt\n--- a/src/b.txt\n+++ b/src/b.txt\n@@ -1 +1,2 @@\n one\n+two\n";

    let applied = workspace
        .run(json!({"action":"apply_patch", "patch":patch}))
        .expect("apply patch");
    assert_eq!(applied["action"], "apply_patch");
    assert_eq!(applied["isolation_root"], "workspace://current");
    assert_eq!(applied["reversible"], true);
    assert_eq!(applied["changed_hunks"], 2);
    assert_eq!(applied["changed_files"].as_array().unwrap().len(), 2);
    assert_eq!(
        fs::read_to_string(workspace.root.join("src/a.txt")).unwrap(),
        "beta\n"
    );
    assert_eq!(
        fs::read_to_string(workspace.root.join("src/b.txt")).unwrap(),
        "one\ntwo\n"
    );

    let checkpoint_id = applied["checkpoint_id"].as_str().unwrap();
    let rewind = workspace
        .run(json!({"action":"rewind", "checkpoint_id":checkpoint_id}))
        .expect("rewind");
    assert_eq!(rewind["action"], "rewind");
    assert_eq!(rewind["reversible"], false);
    assert_eq!(
        fs::read_to_string(workspace.root.join("src/a.txt")).unwrap(),
        "alpha\n"
    );
    assert_eq!(
        fs::read_to_string(workspace.root.join("src/b.txt")).unwrap(),
        "one\n"
    );
}

#[test]
fn stale_precondition_rejects_patch_without_mutation() {
    let workspace = TestWorkspace::new();
    write(&workspace.root.join("note.txt"), "current\n");
    let patch = "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1 +1 @@\n-current\n+changed\n";
    let result = workspace.run(json!({
        "action":"apply_patch",
        "patch":patch,
        "precondition_hashes":{"note.txt":"sha256:stale"}
    }));
    assert!(result.unwrap_err().contains("patch_precondition_failed"));
    assert_eq!(
        fs::read_to_string(workspace.root.join("note.txt")).unwrap(),
        "current\n"
    );
}

#[test]
fn rewind_refuses_to_overwrite_later_edit() {
    let workspace = TestWorkspace::new();
    write(&workspace.root.join("note.txt"), "before\n");
    let patch = "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1 +1 @@\n-before\n+after\n";
    let applied = workspace
        .run(json!({"action":"apply_patch", "patch":patch}))
        .expect("apply patch");
    write(&workspace.root.join("note.txt"), "later user edit\n");

    let result = workspace.run(json!({
        "action":"rewind",
        "checkpoint_id":applied["checkpoint_id"]
    }));
    assert!(result.unwrap_err().contains("rewind_precondition_failed"));
    assert_eq!(
        fs::read_to_string(workspace.root.join("note.txt")).unwrap(),
        "later user edit\n"
    );
}

#[test]
fn patch_rejects_parent_traversal_and_internal_state_paths() {
    let workspace = TestWorkspace::new();
    for patch in [
        "diff --git a/../escape.txt b/../escape.txt\nnew file mode 100644\n--- /dev/null\n+++ b/../escape.txt\n@@ -0,0 +1 @@\n+bad\n",
        "diff --git a/.rustclaw/state b/.rustclaw/state\nnew file mode 100644\n--- /dev/null\n+++ b/.rustclaw/state\n@@ -0,0 +1 @@\n+bad\n",
    ] {
        let result = workspace.run(json!({"action":"apply_patch", "patch":patch}));
        assert!(result.is_err());
    }
    assert!(!workspace.root.join(".rustclaw/state").exists());
}

#[test]
fn failed_context_check_preserves_all_target_files() {
    let workspace = TestWorkspace::new();
    write(&workspace.root.join("a.txt"), "actual\n");
    write(&workspace.root.join("b.txt"), "before\n");
    let patch = "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-expected\n+changed\ndiff --git a/b.txt b/b.txt\n--- a/b.txt\n+++ b/b.txt\n@@ -1 +1 @@\n-before\n+after\n";

    let result = workspace.run(json!({"action":"apply_patch", "patch":patch}));
    assert!(result.unwrap_err().contains("patch_context_mismatch"));
    assert_eq!(
        fs::read_to_string(workspace.root.join("a.txt")).unwrap(),
        "actual\n"
    );
    assert_eq!(
        fs::read_to_string(workspace.root.join("b.txt")).unwrap(),
        "before\n"
    );
}

#[test]
fn patch_and_rewind_preserve_preexisting_dirty_content() {
    let workspace = TestWorkspace::new();
    write(&workspace.root.join("source.txt"), "user edit\ntarget\n");
    write(
        &workspace.root.join("unrelated.txt"),
        "dirty and unrelated\n",
    );
    let patch = "diff --git a/source.txt b/source.txt\n--- a/source.txt\n+++ b/source.txt\n@@ -1,2 +1,2 @@\n user edit\n-target\n+patched\n";

    let applied = workspace
        .run(json!({"action":"apply_patch", "patch":patch}))
        .expect("apply patch");
    assert_eq!(
        fs::read_to_string(workspace.root.join("source.txt")).unwrap(),
        "user edit\npatched\n"
    );
    assert_eq!(
        fs::read_to_string(workspace.root.join("unrelated.txt")).unwrap(),
        "dirty and unrelated\n"
    );
    workspace
        .run(json!({"action":"rewind", "checkpoint_id":applied["checkpoint_id"]}))
        .expect("rewind");
    assert_eq!(
        fs::read_to_string(workspace.root.join("source.txt")).unwrap(),
        "user edit\ntarget\n"
    );
}

#[cfg(unix)]
#[test]
fn checkpoint_state_symlink_is_rejected() {
    use std::os::unix::fs::symlink;

    let workspace = TestWorkspace::new();
    let external = std::env::temp_dir().join(format!(
        "rustclaw-workspace-patch-external-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&external).unwrap();
    symlink(&external, workspace.root.join(".rustclaw")).unwrap();
    let patch = "diff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+new\n";
    let result = workspace.run(json!({"action":"apply_patch", "patch":patch}));
    assert!(result.unwrap_err().contains("checkpoint_symlink_denied"));
    assert!(fs::read_dir(&external).unwrap().next().is_none());
    let _ = fs::remove_dir_all(external);
}

#[cfg(unix)]
#[test]
fn patch_created_symlink_is_rolled_back() {
    let workspace = TestWorkspace::new();
    let patch = "diff --git a/link.txt b/link.txt\nnew file mode 120000\n--- /dev/null\n+++ b/link.txt\n@@ -0,0 +1 @@\n+../outside\n\\ No newline at end of file\n";
    let result = workspace.run(json!({"action":"apply_patch", "patch":patch}));
    assert!(result.unwrap_err().contains("symlink_path_denied"));
    assert!(fs::symlink_metadata(workspace.root.join("link.txt")).is_err());
}

#[test]
fn checkpoint_diff_returns_the_original_patch() {
    let workspace = TestWorkspace::new();
    write(&workspace.root.join("note.txt"), "before\n");
    let before_hash = sha256_label(b"before\n");
    let patch = "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1 +1 @@\n-before\n+after\n";
    let applied = workspace
        .run(json!({
            "action":"apply_patch",
            "patch":patch,
            "precondition_hashes":{"note.txt":before_hash}
        }))
        .expect("apply patch");
    let diff = workspace
        .run(json!({
            "action":"diff",
            "checkpoint_id":applied["checkpoint_id"]
        }))
        .expect("checkpoint diff");
    assert_eq!(diff["patch"], patch);
    assert_eq!(diff["patch_id"], applied["patch_id"]);
}
