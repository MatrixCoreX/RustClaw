use super::{execute_builtin_skill, test_state, TempDirGuard};
use serde_json::{json, Value};

fn recovery(error: &str) -> Value {
    let parsed = crate::skills::parse_structured_skill_error(error).expect("typed command error");
    assert_eq!(parsed.error_code, "invalid_input");
    let extra = parsed.extra.unwrap();
    assert_eq!(extra["failure_phase"], "pre_dispatch");
    assert_eq!(extra["side_effect_applied"], false);
    assert_eq!(extra["retryable"], true);
    assert_eq!(extra["recovery_action"], "replan_arguments");
    extra
}

#[tokio::test]
async fn overlong_command_replans_before_sync_async_or_terminal_start() {
    for (action, asynchronous) in [("exec", false), ("exec", true), ("terminal_start", false)] {
        let root = TempDirGuard::new("command-size-preflight");
        let mut state = test_state(root.path.clone());
        state.skill_rt.max_cmd_length = 16;
        let command = "printf should-not-run > unexpected-file";
        let error = execute_builtin_skill(
            &state,
            "run_cmd",
            &json!({
                "action":action, "command":command, "async_start":asynchronous,
            }),
        )
        .await
        .unwrap_err();
        let extra = recovery(&error);
        assert_eq!(extra["details"]["actual_bytes"], command.len());
        assert_eq!(extra["details"]["max_bytes"], 16);
        assert_eq!(std::fs::read_dir(&root.path).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn empty_command_is_effect_free_and_command_budget_counts_utf8_bytes() {
    let root = TempDirGuard::new("command-empty-preflight");
    let mut state = test_state(root.path.clone());
    state.skill_rt.max_cmd_length = 2;
    for command in ["", "  ", "界"] {
        let error = execute_builtin_skill(
            &state,
            "run_cmd",
            &json!({
                "action":"exec", "command":command,
            }),
        )
        .await
        .unwrap_err();
        let extra = recovery(&error);
        if command == "界" {
            assert_eq!(extra["details"]["actual_bytes"], 3);
        }
    }
    assert_eq!(std::fs::read_dir(&root.path).unwrap().count(), 0);
}

#[tokio::test]
async fn command_at_exact_byte_limit_executes_normally() {
    let root = TempDirGuard::new("command-size-boundary");
    let mut state = test_state(root.path.clone());
    let command = "printf preflight-ok";
    state.skill_rt.max_cmd_length = command.len();
    let output = execute_builtin_skill(
        &state,
        "run_cmd",
        &json!({
            "action":"exec", "command":command,
        }),
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["exit_code"], 0);
    assert_eq!(value["command_output"], "preflight-ok");
}

#[tokio::test]
async fn failure_after_process_start_cannot_claim_effect_free_argument_recovery() {
    let root = TempDirGuard::new("command-started-failure");
    let state = test_state(root.path.clone());
    let error = execute_builtin_skill(
        &state,
        "run_cmd",
        &json!({
            "action":"exec", "command":"printf started; exit 7",
        }),
    )
    .await
    .unwrap_err();
    let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
    assert_eq!(parsed.error_code, "nonzero_exit");
    let extra = parsed.extra.unwrap();
    assert_eq!(extra["exit_code"], 7);
    assert!(extra.get("side_effect_applied").is_none());
    assert!(extra.get("recovery_action").is_none());
}
