use super::execute_workspace_replace_for_root;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "agent-runtime-workspace-replace-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).expect("create workspace");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn args(value: Value) -> Map<String, Value> {
    value.as_object().expect("object").clone()
}

fn parse_error(error: String) -> crate::skills::StructuredSkillError {
    crate::skills::parse_structured_skill_error(&error).expect("structured error")
}

#[test]
fn preview_requires_one_exact_match_without_writing() {
    let dir = TestWorkspace::new();
    let path = dir.path().join("sample.txt");
    fs::write(&path, "alpha\nbeta\ngamma\n").expect("fixture");

    let output = execute_workspace_replace_for_root(
        dir.path(),
        "task-preview",
        &args(json!({
            "action": "preview_replace_text",
            "path": "sample.txt",
            "old_text": "beta",
            "new_text": "delta",
            "expected_occurrences": 1,
        })),
    )
    .expect("preview");
    let output: Value = serde_json::from_str(&output).expect("json");

    assert_eq!(output["action"], "preview_replace_text");
    assert_eq!(output["occurrence_count"], 1);
    assert_eq!(output["would_change"], true);
    assert_eq!(
        fs::read_to_string(path).expect("unchanged"),
        "alpha\nbeta\ngamma\n"
    );
}

#[test]
fn replace_is_atomic_checkpointed_and_rewindable() {
    let dir = TestWorkspace::new();
    let path = dir.path().join("sample.txt");
    fs::write(&path, "alpha\nbeta\ngamma\n").expect("fixture");

    let output = execute_workspace_replace_for_root(
        dir.path(),
        "task-replace",
        &args(json!({
            "action": "replace_text",
            "path": "sample.txt",
            "old_text": "beta",
            "new_text": "delta",
            "expected_occurrences": 1,
        })),
    )
    .expect("replace");
    let output: Value = serde_json::from_str(&output).expect("json");

    assert_eq!(output["action"], "replace_text");
    assert_eq!(output["replacement_count"], 1);
    assert_eq!(output["state"], "applied");
    assert_eq!(output["reversible"], true);
    assert!(output["checkpoint_id"].is_string());
    assert_eq!(
        fs::read_to_string(&path).expect("changed"),
        "alpha\ndelta\ngamma\n"
    );

    let rewind = super::super::builtin_workspace_patch::execute_workspace_patch_for_root(
        dir.path(),
        "task-replace",
        &args(json!({
            "action": "rewind",
            "checkpoint_id": output["checkpoint_id"],
        })),
    )
    .expect("rewind");
    let rewind: Value = serde_json::from_str(&rewind).expect("rewind json");
    assert_eq!(rewind["state"], "rewound");
    assert_eq!(
        fs::read_to_string(path).expect("restored"),
        "alpha\nbeta\ngamma\n"
    );
}

#[test]
fn replace_rejects_missing_and_ambiguous_targets() {
    let dir = TestWorkspace::new();
    fs::write(dir.path().join("sample.txt"), "same\nsame\n").expect("fixture");

    let ambiguous = execute_workspace_replace_for_root(
        dir.path(),
        "task-ambiguous",
        &args(json!({
            "action": "replace_text",
            "path": "sample.txt",
            "old_text": "same",
            "new_text": "new",
        })),
    )
    .expect_err("ambiguous");
    assert_eq!(
        parse_error(ambiguous).error_code,
        "replacement_target_ambiguous"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("sample.txt")).expect("unchanged"),
        "same\nsame\n"
    );

    let missing = execute_workspace_replace_for_root(
        dir.path(),
        "task-missing",
        &args(json!({
            "action": "replace_text",
            "path": "sample.txt",
            "old_text": "absent",
            "new_text": "new",
        })),
    )
    .expect_err("missing");
    assert_eq!(
        parse_error(missing).error_code,
        "replacement_target_not_found"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("sample.txt")).expect("unchanged"),
        "same\nsame\n"
    );
}

#[test]
fn replace_checks_hash_and_preserves_crlf() {
    let dir = TestWorkspace::new();
    fs::write(dir.path().join("sample.txt"), b"alpha\r\nbeta\r\n").expect("fixture");

    let stale = execute_workspace_replace_for_root(
        dir.path(),
        "task-stale",
        &args(json!({
            "action": "replace_text",
            "path": "sample.txt",
            "old_text": "beta",
            "new_text": "one\ntwo",
            "expected_sha256": "sha256:0000",
        })),
    )
    .expect_err("stale");
    assert_eq!(
        parse_error(stale).error_code,
        "replacement_precondition_failed"
    );
    assert_eq!(
        fs::read(dir.path().join("sample.txt")).expect("unchanged"),
        b"alpha\r\nbeta\r\n"
    );

    execute_workspace_replace_for_root(
        dir.path(),
        "task-crlf",
        &args(json!({
            "action": "replace_text",
            "path": "sample.txt",
            "old_text": "beta",
            "new_text": "one\ntwo",
        })),
    )
    .expect("replace");
    assert_eq!(
        fs::read(dir.path().join("sample.txt")).expect("read"),
        b"alpha\r\none\r\ntwo\r\n"
    );
}

#[test]
fn replace_supports_unicode_and_empty_replacement() {
    let dir = TestWorkspace::new();
    let path = dir.path().join("unicode.txt");
    fs::write(&path, "prefix\n你好，世界\nsuffix\n").expect("fixture");

    let output = execute_workspace_replace_for_root(
        dir.path(),
        "task-unicode",
        &args(json!({
            "action": "replace_text",
            "path": "./unicode.txt",
            "old_text": "你好，世界\n",
            "new_text": "",
        })),
    )
    .expect("replace");
    let output: Value = serde_json::from_str(&output).expect("json");

    assert_eq!(output["path"], "unicode.txt");
    assert_eq!(output["replacement_count"], 1);
    assert_eq!(
        fs::read_to_string(path).expect("changed"),
        "prefix\nsuffix\n"
    );
}

#[test]
fn replace_accepts_an_absolute_path_inside_the_workspace() {
    let dir = TestWorkspace::new();
    let path = dir.path().join("absolute.txt");
    fs::write(&path, "before old after").expect("fixture");

    let output = execute_workspace_replace_for_root(
        dir.path(),
        "task-absolute-inside",
        &args(json!({
            "action": "replace_text",
            "path": path.to_string_lossy(),
            "old_text": "old",
            "new_text": "new",
        })),
    )
    .expect("replace absolute workspace path");
    let output: Value = serde_json::from_str(&output).expect("json");

    assert_eq!(output["path"], "absolute.txt");
    assert_eq!(
        fs::read_to_string(path).expect("changed"),
        "before new after"
    );
}

#[cfg(unix)]
#[test]
fn replace_accepts_an_absolute_path_through_the_configured_root_alias() {
    use std::os::unix::fs::symlink;

    let dir = TestWorkspace::new();
    let alias_parent = TestWorkspace::new();
    let alias_root = alias_parent.path().join("workspace");
    symlink(dir.path(), &alias_root).expect("create configured workspace alias");
    fs::write(dir.path().join("absolute.txt"), "before old after").expect("fixture");

    let output = execute_workspace_replace_for_root(
        &alias_root,
        "task-absolute-configured-alias",
        &args(json!({
            "action": "replace_text",
            "path": alias_root.join("absolute.txt").to_string_lossy(),
            "old_text": "old",
            "new_text": "new",
        })),
    )
    .expect("replace absolute path through configured workspace alias");
    let output: Value = serde_json::from_str(&output).expect("json");

    assert_eq!(output["path"], "absolute.txt");
    assert_eq!(
        fs::read_to_string(dir.path().join("absolute.txt")).expect("changed"),
        "before new after"
    );
}

#[test]
fn replace_rejects_an_absolute_path_outside_the_workspace() {
    let dir = TestWorkspace::new();
    let outside = std::env::temp_dir().join(format!(
        "agent-runtime-workspace-replace-outside-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::write(&outside, "before old after").expect("outside fixture");

    let error = execute_workspace_replace_for_root(
        dir.path(),
        "task-absolute-outside",
        &args(json!({
            "action": "replace_text",
            "path": outside.to_string_lossy(),
            "old_text": "old",
            "new_text": "new",
        })),
    )
    .expect_err("outside path");

    assert_eq!(parse_error(error).error_code, "path_outside_workspace");
    assert_eq!(
        fs::read_to_string(&outside).expect("unchanged"),
        "before old after"
    );
    fs::remove_file(outside).expect("remove outside fixture");
}

#[test]
fn replace_rejects_binary_and_symlink_targets() {
    let dir = TestWorkspace::new();
    fs::write(dir.path().join("binary.bin"), b"old\0value").expect("fixture");
    let binary = execute_workspace_replace_for_root(
        dir.path(),
        "task-binary",
        &args(json!({
            "action": "preview_replace_text",
            "path": "binary.bin",
            "old_text": "old",
            "new_text": "new",
        })),
    )
    .expect_err("binary");
    assert_eq!(parse_error(binary).error_code, "binary_file_unsupported");

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("binary.bin", dir.path().join("link.txt")).expect("symlink");
        let symlink = execute_workspace_replace_for_root(
            dir.path(),
            "task-symlink",
            &args(json!({
                "action": "preview_replace_text",
                "path": "link.txt",
                "old_text": "old",
                "new_text": "new",
            })),
        )
        .expect_err("symlink");
        assert_eq!(parse_error(symlink).error_code, "symlink_path_denied");

        let absolute_symlink = execute_workspace_replace_for_root(
            dir.path(),
            "task-absolute-symlink",
            &args(json!({
                "action": "preview_replace_text",
                "path": dir.path().join("link.txt").to_string_lossy(),
                "old_text": "old",
                "new_text": "new",
            })),
        )
        .expect_err("absolute symlink");
        assert_eq!(
            parse_error(absolute_symlink).error_code,
            "symlink_path_denied"
        );
    }
}

#[test]
fn replace_rejects_non_utf8_and_workspace_escape() {
    let dir = TestWorkspace::new();
    fs::write(dir.path().join("non-utf8.txt"), [b'o', b'l', b'd', 0xff]).expect("fixture");

    let non_utf8 = execute_workspace_replace_for_root(
        dir.path(),
        "task-non-utf8",
        &args(json!({
            "action": "replace_text",
            "path": "non-utf8.txt",
            "old_text": "old",
            "new_text": "new",
        })),
    )
    .expect_err("non utf8");
    assert_eq!(
        parse_error(non_utf8).error_code,
        "non_utf8_file_unsupported"
    );

    let escaped = execute_workspace_replace_for_root(
        dir.path(),
        "task-escape",
        &args(json!({
            "action": "replace_text",
            "path": "../outside.txt",
            "old_text": "old",
            "new_text": "new",
        })),
    )
    .expect_err("workspace escape");
    assert_eq!(parse_error(escaped).error_code, "path_outside_workspace");
}

#[cfg(unix)]
#[test]
fn replace_permission_failure_preserves_content_and_cleans_temporary_file() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TestWorkspace::new();
    let protected = dir.path().join("protected");
    fs::create_dir(&protected).expect("protected directory");
    let path = protected.join("sample.txt");
    fs::write(&path, "before old after").expect("fixture");
    fs::set_permissions(&protected, fs::Permissions::from_mode(0o555))
        .expect("remove directory write permission");

    let result = execute_workspace_replace_for_root(
        dir.path(),
        "task-permission",
        &args(json!({
            "action": "replace_text",
            "path": "protected/sample.txt",
            "old_text": "old",
            "new_text": "new",
        })),
    );

    fs::set_permissions(&protected, fs::Permissions::from_mode(0o755))
        .expect("restore directory permission");
    let error = result.expect_err("permission failure");
    assert_eq!(parse_error(error).error_code, "replacement_write_failed");
    assert_eq!(
        fs::read_to_string(&path).expect("unchanged file"),
        "before old after"
    );
    assert_eq!(
        fs::read_dir(&protected)
            .expect("protected entries")
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".agent-runtime-write-"))
            .count(),
        0
    );
}
