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
