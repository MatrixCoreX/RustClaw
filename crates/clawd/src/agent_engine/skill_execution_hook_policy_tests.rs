use std::path::PathBuf;

use crate::agent_engine::support::{
    AnswerVerifierRequiredEvidenceScope, RegistryIdempotencyGuardScope,
};
use serde_json::json;

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rustclaw_{prefix}_{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(path.join("configs")).expect("create config dir");
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn claimed_task() -> crate::ClaimedTask {
    crate::ClaimedTask {
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
        max_steps: 16,
        max_rounds: 2,
        max_tool_calls: 12,
        recoverable_failure_extra_rounds: 1,
        repeat_action_limit: 4,
        no_progress_limit: 1,
        multi_round_enabled: true,
        answer_verifier_retry_limit: 2,
        answer_verifier_enforce_required_scope: AnswerVerifierRequiredEvidenceScope::Off,
        registry_idempotency_guard_scope: RegistryIdempotencyGuardScope::Off,
        structured_evidence_required_for_selected_contracts: false,
        fast_read: Default::default(),
        grounded_summary: Default::default(),
        multi_step_workspace: Default::default(),
        ops_closed_loop: Default::default(),
    }
}

#[tokio::test]
async fn pre_tool_hook_background_wait_publishes_checkpoint() {
    let temp = TempDirGuard::new("hook_background_wait");
    std::fs::write(
        temp.path.join("configs/agent_guard.toml"),
        r#"
[agent.hooks]
background_wait_action_refs = ["fs_basic.read_text"]
"#,
    )
    .expect("write agent guard");
    let mut state = super::tests::test_state();
    state.skill_rt.workspace_root = temp.path.clone();
    let task = claimed_task();
    let mut loop_state = super::LoopState::new(2);
    let exec_args = json!({"action": "read_text", "path": "README.md"});
    let action = crate::AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: exec_args.clone(),
    };
    let actions = vec![action.clone()];
    let round_steps = Vec::<String>::new();
    let policy = test_policy();

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
