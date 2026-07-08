use serde_json::json;

fn turn_analysis_with_alias(alias: &str, target: &str) -> crate::intent_router::TurnAnalysis {
    crate::intent_router::TurnAnalysis {
        turn_type: None,
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(json!({
            "alias_bindings": [{
                "alias": alias,
                "target": target
            }]
        })),
        attachment_processing_required: false,
    }
}

fn turn_analysis_with_alias_map(alias: &str, target: &str) -> crate::intent_router::TurnAnalysis {
    crate::intent_router::TurnAnalysis {
        turn_type: None,
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(json!({
            "alias_bindings": {
                alias: target
            }
        })),
        attachment_processing_required: false,
    }
}

fn route_for_test() -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: String::new(),
        needs_clarify: true,
        clarify_question: "old clarify".to_string(),
        route_reason: "executable_contract_preserved_for_agent_loop".to_string(),
        route_confidence: Some(0.7),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            requires_content_evidence: true,
            delivery_required: true,
            ..crate::IntentOutputContract::default()
        },
    }
}

fn ack_route_for_test() -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "executionless_finalize_trace_plain".to_string(),
        route_confidence: Some(0.7),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    }
}

#[test]
fn alias_only_state_patch_ack_route_clears_execution_contract() {
    let turn_analysis = turn_analysis_with_alias(
        "ALPHA_DOC",
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
    );
    let mut route = ack_route_for_test();

    super::apply_alias_state_patch_ack_route(&mut route, Some(&turn_analysis), None);

    assert_eq!(route.ask_mode, crate::AskMode::state_patch_ack());
    assert!(!route.needs_clarify);
    assert!(!route.wants_file_delivery);
    assert!(!route.output_contract.requires_content_evidence);
    assert!(!route.output_contract.delivery_required);
    assert!(route.route_reason.contains("alias_state_patch_ack"));
}

#[test]
fn alias_only_state_patch_ack_route_accepts_alias_map() {
    let turn_analysis = turn_analysis_with_alias_map(
        "ALPHA_DOC",
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
    );
    let mut route = ack_route_for_test();

    super::apply_alias_state_patch_ack_route(&mut route, Some(&turn_analysis), None);

    assert_eq!(route.ask_mode, crate::AskMode::state_patch_ack());
    assert!(route.route_reason.contains("alias_state_patch_ack"));
}

#[test]
fn alias_only_state_patch_ack_route_allows_locator_kind_without_evidence_contract() {
    let turn_analysis = turn_analysis_with_alias(
        "ALPHA_DOC",
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
    );
    let mut route = ack_route_for_test();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;

    super::apply_alias_state_patch_ack_route(&mut route, Some(&turn_analysis), None);

    assert_eq!(route.ask_mode, crate::AskMode::state_patch_ack());
    assert!(route.route_reason.contains("alias_state_patch_ack"));
}

#[test]
fn alias_only_state_patch_ack_route_does_not_mask_content_evidence_contract() {
    let turn_analysis = turn_analysis_with_alias_map(
        "ALPHA_DOC",
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
    );
    let mut route = route_for_test();

    super::apply_alias_state_patch_ack_route(&mut route, Some(&turn_analysis), None);

    assert_eq!(route.ask_mode, crate::AskMode::act_with_chat_finalizer());
    assert!(!route.route_reason.contains("alias_state_patch_ack"));
}

#[test]
fn alias_only_state_patch_ack_route_does_not_mask_agent_loop_execution_boundary() {
    let turn_analysis =
        turn_analysis_with_alias_map("DEVICE_LOCAL", "scripts/nl_tests/fixtures/device_local");
    let mut route = ack_route_for_test();
    route.route_reason = "executable_contract_preserved_for_agent_loop".to_string();
    let boundary = crate::intent_router::BoundaryEnvelope {
        explicit_locators: vec!["scripts/nl_tests/fixtures/device_local".to_string()],
        ..Default::default()
    };

    super::apply_alias_state_patch_ack_route(&mut route, Some(&turn_analysis), Some(&boundary));

    assert_eq!(route.ask_mode, crate::AskMode::act_with_chat_finalizer());
    assert!(!route.route_reason.contains("alias_state_patch_ack"));
}

#[test]
fn alias_state_patch_ack_payload_marks_update_when_alias_existed() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "task-alias-ack".to_string(),
        user_id: 41,
        chat_id: 42,
        user_key: Some("user-key".to_string()),
        channel: "telegram".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    crate::conversation_state::replace_active_conversation_state_with_pointers(
        &state,
        &task,
        None,
        crate::conversation_state::ActiveSessionPointers {
            active_followup_task_id: None,
            active_clarify_task_id: None,
            active_observed_facts_task_id: None,
        },
    );
    let prior_turn = turn_analysis_with_alias(
        "ALPHA_DOC",
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
    );
    let route = route_for_test();
    let journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "set alias");
    crate::conversation_state::update_active_session_from_ask_outcome(
        &state,
        &task,
        None,
        "set alias",
        &route,
        Some(&prior_turn),
        "set alias",
        "ack",
        &[],
        false,
        &[],
        &journal,
        None,
    );
    let update_turn = turn_analysis_with_alias(
        "ALPHA_DOC",
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
    );
    let route = ack_route_for_test();

    let reply = super::alias_state_patch_ack_reply(
        &state,
        &task,
        "update alias",
        &route,
        Some(&update_turn),
        None,
    )
    .expect("ack reply");

    assert!(!reply.is_llm_reply);
    assert!(
        reply.text == "已更新这个临时指代。"
            || reply.text == "I have updated that temporary reference."
            || reply.text.contains("memory_alias_updated"),
        "reply text: {}",
        reply.text
    );
}
