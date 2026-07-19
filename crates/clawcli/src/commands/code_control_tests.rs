use super::*;

#[test]
fn workspace_diff_args_only_include_explicit_machine_fields() {
    assert_eq!(workspace_diff_args(None, &[]), serde_json::json!({}));
    assert_eq!(
        workspace_diff_args(
            Some(" checkpoint-1 "),
            &["src/lib.rs".to_string(), "Cargo.toml".to_string()]
        ),
        serde_json::json!({
            "checkpoint_id": "checkpoint-1",
            "paths": ["src/lib.rs", "Cargo.toml"]
        })
    );
}

#[test]
fn workspace_rewind_args_carry_the_checkpoint_token() {
    assert_eq!(
        workspace_rewind_args(" checkpoint-2 "),
        serde_json::json!({"checkpoint_id": "checkpoint-2"})
    );
}

#[test]
fn workspace_diff_artifact_preserves_bounded_machine_patch() {
    let data = serde_json::json!({
        "result_json": {
            "task_journal": {
                "trace": {
                    "step_results": [{
                        "output_excerpt": serde_json::json!({
                            "extra": {
                                "schema_version": 1,
                                "source": "workspace_patch",
                                "action": "diff",
                                "checkpoint_id": "patch_checkpoint_1",
                                "patch_id": "sha256:patch-1",
                                "changed_files": ["src/lib.rs"],
                                "patch_bytes": 43,
                                "patch_truncated": false,
                                "patch": "diff --git a/src/lib.rs b/src/lib.rs\n"
                            }
                        }).to_string()
                    }]
                }
            }
        }
    });

    let artifact = workspace_diff_artifact_json(&data).expect("workspace diff");
    assert_eq!(artifact["checkpoint_id"], "patch_checkpoint_1");
    assert_eq!(artifact["patch_id"], "sha256:patch-1");
    assert_eq!(artifact["patch_truncated"], false);
    assert_eq!(artifact["patch"], "diff --git a/src/lib.rs b/src/lib.rs\n");
}

#[test]
fn workspace_diff_artifact_rejects_unbounded_patch() {
    let data = serde_json::json!({
        "result_json": {
            "task_journal": {
                "trace": {
                    "step_results": [{
                        "output_excerpt": serde_json::json!({
                            "extra": {
                                "source": "workspace_patch",
                                "action": "diff",
                                "patch": "x".repeat(MAX_WORKSPACE_DIFF_PATCH_BYTES + 1)
                            }
                        }).to_string()
                    }]
                }
            }
        }
    });

    assert!(workspace_diff_artifact_json(&data).is_none());
}
