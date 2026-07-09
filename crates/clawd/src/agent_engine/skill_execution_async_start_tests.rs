use std::fs;

use super::tests::{enable_test_skills, test_policy, test_state, test_task, unique_suffix};
use super::LoopState;

#[tokio::test]
async fn planner_run_cmd_async_start_publishes_waiting_checkpoint() {
    let workspace_root = std::env::temp_dir().join(format!(
        "rustclaw-run-cmd-async-start-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let mut state = test_state();
    state.skill_rt.workspace_root = workspace_root.clone();
    enable_test_skills(&state, &["run_cmd"]);
    let task = test_task();
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;
    let raw_args = serde_json::json!({
        "command": "sleep 0.05 && echo RUSTCLAW_ASYNC_SMOKE",
        "cwd": workspace_root.display().to_string(),
        "async_start": true,
        "poll_after_seconds": 1,
        "expires_in_seconds": 30,
        crate::agent_engine::CLAWD_RUNTIME_ASYNC_JOB_START_ARG: "async_job_protocol"
    });
    let mut exec_args = raw_args.clone();
    exec_args
        .as_object_mut()
        .expect("object args")
        .remove(crate::agent_engine::CLAWD_RUNTIME_ASYNC_JOB_START_ARG);
    let action = crate::AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: raw_args.clone(),
    };
    let actions = vec![action.clone()];
    let policy = test_policy();

    let outcome = super::execute_prepared_skill_action(
        &state,
        &task,
        "start async command",
        "start async command",
        &actions,
        &[],
        &mut loop_state,
        &policy,
        0,
        &action,
        "fp-run-cmd-async-start",
        1,
        1,
        "run_cmd",
        exec_args,
        Some(raw_args),
        None,
        None,
        "run_cmd".to_string(),
        "call_skill",
    )
    .await
    .expect("async run_cmd should publish checkpoint");

    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("async_job_checkpoint_waiting")
    );
    assert_eq!(
        loop_state
            .task_lifecycle
            .as_ref()
            .and_then(|value| value.get("state"))
            .and_then(serde_json::Value::as_str),
        Some("waiting")
    );
    assert!(loop_state
        .task_lifecycle
        .as_ref()
        .and_then(|value| value.get("checkpoint_id"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| value.starts_with("agent-loop:task-skill-exec:")));
    assert!(loop_state
        .task_checkpoint
        .as_ref()
        .and_then(|value| value.pointer("/pending_async_job/job_id"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| value.starts_with("local_process:")));
    let delivered = loop_state
        .executed_step_results
        .last()
        .and_then(|step| step.output.as_deref())
        .expect("visible checkpoint reply");
    let delivered: serde_json::Value = serde_json::from_str(delivered).expect("machine reply");
    assert_eq!(delivered["status"], "accepted");
    assert!(delivered.get("checkpoint_id").is_some());
    assert!(delivered.get("poll_ref").is_some());
    assert!(delivered.get("next_check_after").is_some());

    let _ = fs::remove_dir_all(&workspace_root);
}
