use super::*;

#[tokio::test]
async fn finalize_loop_reply_attaches_machine_clarify_envelope_for_deferred_route_respond() {
    let state = test_state();
    let task = claimed_task("task-deferred-clarify-envelope");
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.needs_clarify = true;
    route.route_reason =
        "ordinary_clarify_deferred_to_agent_loop; clarify_reason_code:missing_read_target"
            .to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    let model_question = "Which file should I read from?";
    loop_state
        .delivery_messages
        .push(model_question.to_string());
    loop_state.last_user_visible_respond = Some(model_question.to_string());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "read the first line of that file",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should attach clarify envelope");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert!(reply.text.contains(model_question));
    let envelope = reply
        .messages
        .iter()
        .find_map(|message| {
            let payload = serde_json::from_str::<serde_json::Value>(message.trim()).ok()?;
            (payload
                .get("owner_layer")
                .and_then(serde_json::Value::as_str)
                == Some("agent_loop_clarify"))
            .then_some(payload)
        })
        .expect("clarify envelope");
    assert_eq!(
        envelope
            .pointer("/terminal_intent")
            .and_then(serde_json::Value::as_str),
        Some("clarify")
    );
    assert_eq!(
        envelope
            .pointer("/clarify_reason_code")
            .and_then(serde_json::Value::as_str),
        Some("missing_read_target")
    );
    assert_eq!(
        envelope
            .pointer("/missing_slot")
            .and_then(serde_json::Value::as_str),
        Some("locator")
    );
    assert_eq!(
        envelope
            .pointer("/field_path")
            .and_then(serde_json::Value::as_str),
        Some("output_contract.locator_hint")
    );
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Clarify)
    );
}

#[tokio::test]
async fn finalize_loop_reply_does_not_attach_clarify_envelope_after_completed_act_delivery() {
    let state = test_state();
    let task = claimed_task("task-deferred-clarify-act-complete");
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.needs_clarify = true;
    route.route_reason =
        "ordinary_clarify_deferred_to_agent_loop; clarify_reason_code:missing_read_target"
            .to_string();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    let act_envelope = serde_json::json!({
        "control_intent": "act",
        "decision": "call_capability",
        "capability_ref": "subagent",
        "terminal_intent": "continue"
    })
    .to_string();
    loop_state.output_vars.insert(
        "agent_loop.first_act_decision_envelope".to_string(),
        act_envelope,
    );
    let completed_delivery = serde_json::json!({
        "review_target": ["AGENTS.md", "plan/codex_claude_parity_convergence_plan_20260623.md"],
        "alignment_verdict": "consistent_and_complementary"
    })
    .to_string();
    loop_state
        .delivery_messages
        .push(completed_delivery.clone());
    loop_state.last_user_visible_respond = Some(completed_delivery);

    let reply = finalize_loop_reply(
        &state,
        &task,
        "review AGENTS.md and the active plan with a read-only subagent",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should preserve completed act delivery");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert!(
        !reply.messages.iter().any(|message| {
            serde_json::from_str::<serde_json::Value>(message.trim())
                .ok()
                .and_then(|payload| {
                    payload
                        .get("owner_layer")
                        .and_then(serde_json::Value::as_str)
                        .map(|owner| owner == "agent_loop_clarify")
                })
                .unwrap_or(false)
        }),
        "reply messages should not include clarify envelope: {:?}",
        reply.messages
    );
    assert_ne!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Clarify)
    );
}
