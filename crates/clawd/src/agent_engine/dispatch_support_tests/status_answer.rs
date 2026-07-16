use super::*;

#[test]
fn deterministic_status_answer_defers_for_agent_loop_rich_content() {
    let state = test_state_with_registry();
    let task = crate::ClaimedTask {
        task_id: "task-rich-content".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: String::new(),
    };
    let agent_run_context = AgentRunContext {
        output_contract: Some(crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::None,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        }),
        ..Default::default()
    };
    let mut loop_state = LoopState::new(4);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "run_cmd",
        "notes.txt\nnested/config.ini\n",
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_2", "run_cmd", "fixture archive notes\n"));
    loop_state
        .executed_step_results
        .push(ok_step("step_3", "run_cmd", "orders users\n"));
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_4".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some("binary file is not utf8".to_string()),
        started_at: 0,
        finished_at: 0,
    });

    assert!(deterministic_observed_execution_status_answer(
        &state,
        &task,
        "summarize archive and database observations",
        &loop_state,
        Some(&agent_run_context),
    )
    .is_none());
}
