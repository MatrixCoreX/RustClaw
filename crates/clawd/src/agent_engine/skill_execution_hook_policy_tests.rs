use std::path::PathBuf;

use crate::agent_engine::support::{
    AnswerVerifierRequiredEvidenceScope, RegistryIdempotencyGuardScope,
};
use serde_json::json;
use sha2::{Digest, Sha256};

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new_in_repository(prefix: &str) -> Self {
        let path = std::env::current_dir()
            .expect("current repository")
            .join("target/hook-runtime-tests")
            .join(format!("{prefix}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(path.join("configs")).expect("create config dir");
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn write_command_hook(
    temp: &TempDirGuard,
    name: &str,
    decision: &str,
    reason_code: &str,
) -> String {
    std::fs::create_dir_all(temp.path.join("hooks")).expect("create hooks dir");
    let body = format!(
        "#!/bin/sh\nIFS= read -r _event\nprintf '%s\\n' '{{\"schema_version\":1,\"decision\":\"{decision}\",\"reason_code\":\"{reason_code}\"}}'\n"
    );
    let hook_path = temp.path.join("hooks").join(name);
    std::fs::write(&hook_path, &body).expect("write hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&hook_path)
            .expect("hook metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&hook_path, permissions).expect("make hook executable");
    }
    format!("sha256:{:x}", Sha256::digest(body.as_bytes()))
}

fn claimed_task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "task-skill-exec".to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({"text": "run tool"}).to_string(),
    }
}

fn test_policy() -> super::AgentLoopGuardPolicy {
    super::AgentLoopGuardPolicy {
        max_actions_per_turn: 16,
        repeat_action_limit: 4,
        answer_verifier_enforce_required_scope: AnswerVerifierRequiredEvidenceScope::Off,
        registry_idempotency_guard_scope: RegistryIdempotencyGuardScope::Off,
        fast_read: Default::default(),
        grounded_summary: Default::default(),
        multi_step_workspace: Default::default(),
        ops_closed_loop: Default::default(),
    }
}

#[tokio::test]
async fn pre_tool_hook_background_wait_publishes_checkpoint() {
    let temp = TempDirGuard::new_in_repository("hook_background_wait");
    let hash = write_command_hook(
        &temp,
        "background-wait.sh",
        "background_wait",
        "fixture_background_wait",
    );
    std::fs::write(
        temp.path.join("configs/agent_guard.toml"),
        format!(
            r#"
[agent.hooks]

[[agent.hooks.handlers]]
id = "fixture_background_wait"
stage = "pre_tool_use"
kind = "command"
enabled = true
trusted = true
blocking = true
path = "hooks/background-wait.sh"
content_sha256 = "{hash}"
timeout_ms = 1000
max_input_bytes = 4096
max_output_bytes = 4096
max_attempts = 1
failure_policy = "deny"
"#
        ),
    )
    .expect("write agent guard");
    let mut state = super::tests::test_state();
    state.skill_rt.workspace_root = temp.path.clone();
    let task = claimed_task();
    let mut loop_state = super::LoopState::new();
    let exec_args = json!({"action": "read_text", "path": "README.md"});
    let action = crate::AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: exec_args.clone(),
    };
    let actions = vec![action.clone()];
    let round_steps = Vec::<String>::new();
    let policy = test_policy();
    let policy_args = exec_args.clone();

    let outcome = super::execute_prepared_skill_action(
        &state,
        &task,
        "run tool",
        "run tool",
        &actions,
        &round_steps,
        &mut loop_state,
        &policy,
        0,
        &action,
        "fp-hook-background-wait",
        1,
        1,
        "fs_basic",
        "fs_basic",
        &policy_args,
        exec_args,
        None,
        None,
        None,
        "fs_basic.read_text".to_string(),
        "call_tool",
    )
    .await
    .expect("skill action outcome");

    assert!(!outcome.ended_with_user_visible_output);
    assert_eq!(outcome.stop_signal.as_deref(), Some("hook_background_wait"));
    assert!(!outcome.continue_in_round);
    assert_eq!(
        loop_state
            .task_lifecycle
            .as_ref()
            .and_then(|payload| payload.get("state"))
            .and_then(serde_json::Value::as_str),
        Some("waiting")
    );
    assert_eq!(
        loop_state
            .task_lifecycle
            .as_ref()
            .and_then(|payload| payload.get("resume_reason"))
            .and_then(serde_json::Value::as_str),
        Some("hook_background_wait")
    );
    assert!(loop_state
        .task_observations
        .iter()
        .any(|observation| observation
            .get("owner_layer")
            .and_then(serde_json::Value::as_str)
            == Some("agent_hooks")
            && observation.get("stage").and_then(serde_json::Value::as_str)
                == Some("pre_tool_use")
            && observation
                .get("decision")
                .and_then(serde_json::Value::as_str)
                == Some("background_wait")));
    assert!(loop_state.executed_step_results.is_empty());
}

#[test]
fn post_tool_hook_records_safe_run_cmd_machine_args() {
    let mut loop_state = super::LoopState::new();
    loop_state.round_no = 1;

    super::record_post_tool_use_observation(
        &mut loop_state,
        "run_cmd",
        &json!({
            "command": "python3 test_calc_core.py",
            "cwd": "/tmp/rustclaw_live_resume",
            "timeout_seconds": 30,
            "api_key": "should-not-be-recorded"
        }),
        3,
        2,
        crate::executor::StepExecutionStatus::Ok,
    );

    let observation = loop_state
        .task_observations
        .iter()
        .find(|observation| {
            observation.get("stage").and_then(serde_json::Value::as_str) == Some("post_tool_use")
        })
        .expect("post tool observation");
    assert_eq!(
        observation
            .get("status")
            .and_then(serde_json::Value::as_str),
        Some("ok")
    );
    assert_eq!(
        observation
            .pointer("/args/command")
            .and_then(serde_json::Value::as_str),
        Some("python3 test_calc_core.py")
    );
    assert!(observation.pointer("/args/api_key").is_none());
}

#[tokio::test]
async fn trusted_command_hook_blocks_through_production_pre_tool_path() {
    let temp = TempDirGuard::new_in_repository("trusted_command_hook");
    let hash = write_command_hook(&temp, "policy-guard.sh", "deny", "fixture_policy_denied");
    std::fs::write(
        temp.path.join("configs/agent_guard.toml"),
        format!(
            r#"
[agent.hooks]

[[agent.hooks.handlers]]
id = "fixture_guard"
stage = "pre_tool_use"
kind = "command"
enabled = true
trusted = true
blocking = true
path = "hooks/policy-guard.sh"
content_sha256 = "{hash}"
timeout_ms = 1000
max_input_bytes = 4096
max_output_bytes = 4096
max_attempts = 1
failure_policy = "deny"
"#
        ),
    )
    .expect("write agent guard");
    let mut state = super::tests::test_state();
    state.skill_rt.workspace_root = temp.path.clone();
    let task = claimed_task();
    let mut loop_state = super::LoopState::new();
    let exec_args = json!({"action": "read_text", "path": "README.md"});
    let action = crate::AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: exec_args.clone(),
    };
    let actions = vec![action.clone()];
    let policy_args = exec_args.clone();

    let _ = super::execute_prepared_skill_action(
        &state,
        &task,
        "inspect",
        "inspect",
        &actions,
        &[],
        &mut loop_state,
        &test_policy(),
        0,
        &action,
        "fp-trusted-command-hook",
        1,
        1,
        "fs_basic",
        "fs_basic",
        &policy_args,
        exec_args,
        None,
        None,
        None,
        "fs_basic.read_text".to_string(),
        "call_tool",
    )
    .await
    .expect("skill action outcome");

    assert!(loop_state
        .executed_step_results
        .iter()
        .all(|step| !step.is_ok()));
    let handler = loop_state
        .task_observations
        .iter()
        .find(|observation| {
            observation
                .get("handler_id")
                .and_then(serde_json::Value::as_str)
                == Some("fixture_guard")
        })
        .expect("handler observation");
    assert_eq!(handler["status"], "ok", "handler={handler}");
    assert_eq!(handler["decision"], "deny");
    assert_eq!(handler["reason_code"], "fixture_policy_denied");
    assert_eq!(handler["trust_status"], "trusted");
    assert_eq!(handler["content_sha256"], hash);
}

#[tokio::test]
async fn configured_post_tool_hook_runs_through_production_owner() {
    let temp = TempDirGuard::new_in_repository("post_tool_hook");
    let hash = write_command_hook(
        &temp,
        "post-tool-observer.sh",
        "allow",
        "fixture_post_tool_observed",
    );
    std::fs::write(
        temp.path.join("configs/agent_guard.toml"),
        format!(
            r#"
[agent.hooks]

[[agent.hooks.handlers]]
id = "fixture_post_tool"
stage = "post_tool_use"
kind = "command"
enabled = true
trusted = true
blocking = false
path = "hooks/post-tool-observer.sh"
content_sha256 = "{hash}"
timeout_ms = 1000
max_input_bytes = 4096
max_output_bytes = 4096
max_attempts = 1
failure_policy = "deny"
"#
        ),
    )
    .expect("write agent guard");
    let mut state = super::tests::test_state();
    state.skill_rt.workspace_root = temp.path.clone();
    let task = claimed_task();
    let mut loop_state = super::LoopState::new();

    super::record_post_tool_use_hook_observations(
        &state,
        &task,
        &mut loop_state,
        "fs_basic",
        &json!({"action": "read_text", "path": "README.md"}),
        2,
        1,
        crate::executor::StepExecutionStatus::Ok,
    )
    .await;

    let handler = loop_state
        .task_observations
        .iter()
        .find(|observation| observation["handler_id"] == "fixture_post_tool")
        .expect("post-tool handler observation");
    assert_eq!(handler["stage"], "post_tool_use");
    assert_eq!(handler["decision"], "allow");
    assert_eq!(handler["reason_code"], "fixture_post_tool_observed");
    assert_eq!(handler["blocking"], false);
}

#[tokio::test]
async fn configured_permission_hook_can_deny_at_production_owner() {
    let temp = TempDirGuard::new_in_repository("permission_hook");
    let hash = write_command_hook(
        &temp,
        "permission-guard.sh",
        "deny",
        "fixture_permission_denied",
    );
    std::fs::write(
        temp.path.join("configs/agent_guard.toml"),
        format!(
            r#"
[agent.hooks]

[[agent.hooks.handlers]]
id = "fixture_permission_guard"
stage = "permission_request"
kind = "command"
enabled = true
trusted = true
blocking = true
path = "hooks/permission-guard.sh"
content_sha256 = "{hash}"
timeout_ms = 1000
max_input_bytes = 4096
max_output_bytes = 4096
max_attempts = 1
failure_policy = "deny"
"#
        ),
    )
    .expect("write agent guard");
    let mut state = super::tests::test_state();
    state.skill_rt.workspace_root = temp.path.clone();
    let task = claimed_task();
    let mut loop_state = super::LoopState::new();

    let evaluation = super::record_permission_request_hook(
        &state,
        &task,
        &mut loop_state,
        "fs_basic",
        "fs_basic.write_text",
        3,
        2,
    )
    .await;

    assert_eq!(
        evaluation.outcome.decision_kind(),
        Some(crate::policy_decision::PolicyDecision::Deny)
    );
    let handler = loop_state
        .task_observations
        .iter()
        .find(|observation| observation["handler_id"] == "fixture_permission_guard")
        .expect("permission handler observation");
    assert_eq!(handler["stage"], "permission_request");
    assert_eq!(handler["reason_code"], "fixture_permission_denied");
    assert_eq!(handler["blocking"], true);
}

#[tokio::test]
async fn verifier_confirmation_runs_permission_hook_before_approval_creation() {
    let temp = TempDirGuard::new_in_repository("verifier_permission_hook");
    let hash = write_command_hook(
        &temp,
        "verifier-permission-guard.sh",
        "deny",
        "fixture_verifier_permission_denied",
    );
    std::fs::write(
        temp.path.join("configs/agent_guard.toml"),
        format!(
            r#"
[agent.hooks]

[[agent.hooks.handlers]]
id = "fixture_verifier_permission_guard"
stage = "permission_request"
kind = "command"
enabled = true
trusted = true
blocking = true
path = "hooks/verifier-permission-guard.sh"
content_sha256 = "{hash}"
timeout_ms = 1000
max_input_bytes = 4096
max_output_bytes = 4096
max_attempts = 1
failure_policy = "deny"
"#
        ),
    )
    .expect("write agent guard");
    let mut state = super::tests::test_state();
    state.skill_rt.workspace_root = temp.path.clone();
    let task = claimed_task();

    let (_text, resume_context) = crate::agent_engine::build_confirmation_required_resume_context(
        &state,
        &task,
        &[],
        "inspect",
        "inspect",
        &[],
        &[],
        "verification_confirmation_required",
        &[],
    )
    .await;

    assert_eq!(resume_context["required_decision"], "deny");
    assert_eq!(resume_context["permission_hook_decision"], "deny");
    assert!(resume_context.get("approval_request").is_none());
    assert!(resume_context["agent_hook_events"]
        .as_array()
        .is_some_and(|events| events.iter().any(|event| {
            event["handler_id"] == "fixture_verifier_permission_guard"
                && event["reason_code"] == "fixture_verifier_permission_denied"
        })));
}
