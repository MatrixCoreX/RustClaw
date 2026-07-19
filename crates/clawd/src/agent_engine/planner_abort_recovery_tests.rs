use super::*;
use crate::providers::fixture_replay::{
    clear_cache_for_test, RecordedCall, FIXTURE_CALLS_FILENAME, FIXTURE_LLM_CASE_ENV,
    FIXTURE_LLM_ROOT_ENV, FIXTURE_LLM_SEQUENCE_FALLBACK_ENV,
};

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::fixture_replay_e2e::fixture_env_lock()
}

struct FixtureEnv {
    root: std::path::PathBuf,
}

impl FixtureEnv {
    fn install(clean_response: &str) -> Self {
        clear_cache_for_test();
        let root = std::env::temp_dir().join(format!(
            "rustclaw_planner_abort_recovery_{}",
            uuid::Uuid::new_v4()
        ));
        let case = "compact_retry";
        let case_dir = root.join(case);
        std::fs::create_dir_all(&case_dir).expect("create compact retry fixture dir");
        let rec = RecordedCall {
            prompt_hash: "0000000000000000".to_string(),
            prompt_source: Some("planner_abort_compact_retry".to_string()),
            prompt_preview: None,
            clean_response: clean_response.to_string(),
            raw_response: None,
            usage: None,
        };
        std::fs::write(
            case_dir.join(FIXTURE_CALLS_FILENAME),
            serde_json::to_string(&rec).expect("serialize fixture call"),
        )
        .expect("write compact retry fixture");
        std::env::set_var(FIXTURE_LLM_ROOT_ENV, &root);
        std::env::set_var(FIXTURE_LLM_CASE_ENV, case);
        std::env::set_var(FIXTURE_LLM_SEQUENCE_FALLBACK_ENV, "1");
        Self { root }
    }
}

impl Drop for FixtureEnv {
    fn drop(&mut self) {
        std::env::remove_var(FIXTURE_LLM_ROOT_ENV);
        std::env::remove_var(FIXTURE_LLM_CASE_ENV);
        std::env::remove_var(FIXTURE_LLM_SEQUENCE_FALLBACK_ENV);
        clear_cache_for_test();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn compact_retry_requires_planner_contract_or_active_recipe() {
    let mut loop_state = LoopState::default();
    assert!(!should_try_compact_planner_abort_recovery(&loop_state));

    loop_state.output_contract = Some(crate::IntentOutputContract::default());
    assert!(should_try_compact_planner_abort_recovery(&loop_state));

    loop_state.output_contract = None;
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
        crate::execution_recipe::ExecutionRecipeSpec {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            ..Default::default()
        },
    );
    assert!(should_try_compact_planner_abort_recovery(&loop_state));
}

#[test]
fn planner_contract_summary_uses_machine_fields() {
    let mut contract = crate::IntentOutputContract::default();
    contract.response_shape = crate::OutputResponseShape::Scalar;
    contract.locator_hint = "src/main.rs".to_string();

    let summary = planner_contract_summary(Some(&contract));
    assert!(!summary.contains("\"gate_kind\""));
    assert!(!summary.contains("\"ask_mode\""));
    assert!(!summary.contains("\"route_reason\""));
    assert!(summary.contains("\"response_shape\": \"scalar\""));
    assert!(summary.contains("\"locator_hint\": \"src/main.rs\""));
}

#[tokio::test]
async fn compact_retry_uses_fixture_response_as_executable_steps() {
    let _lock = env_lock();
    let _fixture = FixtureEnv::install(
        r#"{"steps":[{"type":"call_tool","tool":"fs_basic","args":{"action":"list_dir","path":"."}}]}"#,
    );
    let state =
        crate::AppState::test_default_with_fixture_provider().with_prompt_layers_installed();
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "compact-retry-task".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut loop_state = LoopState::new(1);
    loop_state.output_contract = Some(crate::IntentOutputContract::default());

    let Some((actions, raw)) = compact_retry_plan_actions(
        &state,
        &task,
        PlannerAbortRecoveryInput {
            goal: "machine_goal",
            turn_analysis: "{}",
            user_text: "create project",
            tool_spec: "fs_basic",
            skill_playbooks: "",
            attempt_ledger: "[]",
            first_raw_plan: "",
            latest_raw_plan: Some(""),
            round_no: 1,
            loop_state: &loop_state,
        },
    )
    .await
    .expect("compact retry should run") else {
        panic!("compact retry should return actions");
    };

    assert!(raw.contains("\"steps\""));
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list_dir"));
        }
        other => panic!("expected call_tool action, got {other:?}"),
    }
}
