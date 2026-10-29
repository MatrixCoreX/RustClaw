use super::{execute_builtin_skill, test_state, TempDirGuard};
use serde_json::{json, Value};

#[tokio::test]
async fn shell_exit_status_parameter_is_not_treated_as_trailing_prose() {
    let root = TempDirGuard::new("command-literal-status");
    let state = test_state(root.path.clone());
    let command = "false; printf '%s' $?";
    let output = execute_builtin_skill(&state, "run_cmd", &json!({"command":command}))
        .await
        .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["command"], command);
    assert_eq!(value["stdout"], "1");
    assert_eq!(value["exit_code"], 0);
}

#[tokio::test]
async fn escaped_terminal_punctuation_survives_execution_and_evidence() {
    let root = TempDirGuard::new("command-literal-punctuation");
    let state = test_state(root.path.clone());
    for suffix in ['?', '!', ';', ':', ',', '。', '，', '；', '：', '！', '？'] {
        let command = format!("printf '%s' literal\\{suffix}");
        let output = execute_builtin_skill(&state, "run_cmd", &json!({"command":command}))
            .await
            .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["command"], command);
        assert_eq!(value["stdout"], format!("literal{suffix}"));
        assert_eq!(value["exit_code"], 0);
    }
}

#[tokio::test]
async fn leading_and_trailing_whitespace_are_literal_command_bytes() {
    let root = TempDirGuard::new("command-literal-whitespace");
    let state = test_state(root.path.clone());
    let command = "\n  printf '%s' literal  \n";
    let output = execute_builtin_skill(&state, "run_cmd", &json!({"command":command}))
        .await
        .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["command"], command);
    assert_eq!(value["stdout"], "literal");
}

#[tokio::test]
async fn wrapping_shell_quotes_and_backticks_are_not_unwrapped() {
    let root = TempDirGuard::new("command-literal-quoting");
    let state = test_state(root.path.clone());
    for command in ["\"printf literal\"", "`printf literal`"] {
        let error = execute_builtin_skill(&state, "run_cmd", &json!({"command":command}))
            .await
            .unwrap_err();
        let error = crate::skills::parse_structured_skill_error(&error).unwrap();
        assert_eq!(error.error_code, "nonzero_exit");
        let extra = error.extra.unwrap();
        assert_eq!(extra["command"], command);
        assert_eq!(extra["exit_code"], 127);
    }
}

#[tokio::test]
async fn raw_byte_budget_is_checked_before_any_command_normalization() {
    let root = TempDirGuard::new("command-literal-byte-budget");
    let mut state = test_state(root.path.clone());
    let command = " printf literal ";
    state.skill_rt.max_cmd_length = command.trim().len();
    let error = execute_builtin_skill(&state, "run_cmd", &json!({"command":command}))
        .await
        .unwrap_err();
    let error = crate::skills::parse_structured_skill_error(&error).unwrap();
    let extra = error.extra.unwrap();
    assert_eq!(extra["failure_phase"], "pre_dispatch");
    assert_eq!(extra["side_effect_applied"], false);
    assert_eq!(extra["details"]["actual_bytes"], command.len());
}

#[tokio::test]
async fn failed_execution_preserves_the_same_command_bytes_in_evidence() {
    let root = TempDirGuard::new("command-literal-failure");
    let state = test_state(root.path.clone());
    let command = "\n exit 7 \n";
    let error = execute_builtin_skill(&state, "run_cmd", &json!({"command":command}))
        .await
        .unwrap_err();
    let error = crate::skills::parse_structured_skill_error(&error).unwrap();
    let extra = error.extra.unwrap();
    assert_eq!(extra["command"], command);
    assert_eq!(extra["exit_code"], 7);
}
