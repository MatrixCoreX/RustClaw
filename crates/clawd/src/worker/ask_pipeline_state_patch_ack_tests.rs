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

fn repo_i18n_state() -> crate::AppState {
    let mut state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.skill_rt.workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("repo root")
        .to_path_buf();
    state.policy.schedule.i18n_dir = "configs/i18n".to_string();
    state
}

fn ask_task_with_payload_text(task_id: &str, text: &str) -> crate::ClaimedTask {
    crate::ClaimedTask {
        task_id: task_id.to_string(),
        user_id: 41,
        chat_id: 42,
        user_key: Some("user-key".to_string()),
        channel: "telegram".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({ "text": text }).to_string(),
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
fn alias_state_patch_ack_accepts_required_content_literal_metadata_for_japanese() {
    let state = repo_i18n_state();
    let task = ask_task_with_payload_text(
        "task-alias-ack-ja-required-content",
        "この会話では、参照名「資料A」を scripts/nl_tests/fixtures/device_local/docs/service_notes.md として覚えてください。返答は「記憶しました」だけ。",
    );
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: None,
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(json!({
            "alias_bindings": {
                "alias": "資料A",
                "target_kind": "path",
                "target": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
            },
            "required_content_literals": ["記憶しました"]
        })),
        attachment_processing_required: false,
    };
    let mut route = ack_route_for_test();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md".to_string();

    super::apply_alias_state_patch_ack_route(&mut route, Some(&turn_analysis), None);
    let reply = super::alias_state_patch_ack_reply(
        &state,
        &task,
        "Bind session alias '資料A' to path 'scripts/nl_tests/fixtures/device_local/docs/service_notes.md'.",
        &route,
        Some(&turn_analysis),
        None,
    )
    .expect("ack reply");

    assert_eq!(route.ask_mode, crate::AskMode::state_patch_ack());
    assert!(route.route_reason.contains("alias_state_patch_ack"));
    assert!(!reply.is_llm_reply);
    assert_eq!(reply.text, "記憶しました");
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
fn alias_only_state_patch_ack_route_allows_agent_loop_marker_without_execution_contract() {
    let turn_analysis =
        turn_analysis_with_alias_map("DEVICE_LOCAL", "scripts/nl_tests/fixtures/device_local");
    let mut route = ack_route_for_test();
    route.route_reason = "executable_contract_preserved_for_agent_loop".to_string();
    let boundary = crate::intent_router::BoundaryEnvelope {
        explicit_locators: vec!["scripts/nl_tests/fixtures/device_local".to_string()],
        ..Default::default()
    };

    super::apply_alias_state_patch_ack_route(&mut route, Some(&turn_analysis), Some(&boundary));

    assert_eq!(route.ask_mode, crate::AskMode::state_patch_ack());
    assert!(route.route_reason.contains("alias_state_patch_ack"));
}

#[test]
fn alias_state_patch_ack_reply_uses_current_request_locale_for_korean_remember() {
    let state = repo_i18n_state();
    let task = ask_task_with_payload_text(
        "task-alias-ack-ko-remember",
        "이 대화에서는 참조 이름 \"자료A\"를 scripts/nl_tests/fixtures/device_local/docs/service_notes.md 로 기억해 주세요. 답장은 \"기억했습니다\"만 해 주세요.",
    );
    let turn = turn_analysis_with_alias(
        "자료A",
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
    );
    let route = ack_route_for_test();

    let reply = super::alias_state_patch_ack_reply(
        &state,
        &task,
        "Bind session alias '자료A' to path 'scripts/nl_tests/fixtures/device_local/docs/service_notes.md'.",
        &route,
        Some(&turn),
        None,
    )
    .expect("ack reply");

    assert!(!reply.is_llm_reply);
    assert_eq!(reply.text, "기억했습니다");
}

#[test]
fn alias_state_patch_ack_accepts_korean_object_map_value_field() {
    let state = repo_i18n_state();
    let task = ask_task_with_payload_text(
        "task-alias-ack-ko-object-map-value",
        "이 대화에서는 참조 이름 \"자료A\"를 scripts/nl_tests/fixtures/device_local/docs/service_notes.md 로 기억해 주세요. 답장은 \"기억했습니다\"만 해 주세요.",
    );
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: None,
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(json!({
            "alias_bindings": {
                "자료A": {
                    "kind": "path",
                    "value": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
                    "scope": "session",
                    "created_by": "user_request"
                }
            },
            "required_content_literals": ["기억했습니다"]
        })),
        attachment_processing_required: false,
    };
    let mut route = ack_route_for_test();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md".to_string();

    super::apply_alias_state_patch_ack_route(&mut route, Some(&turn_analysis), None);
    let reply = super::alias_state_patch_ack_reply(
        &state,
        &task,
        "Bind session alias",
        &route,
        Some(&turn_analysis),
        Some(&crate::intent_router::BoundaryEnvelope {
            language_hint: Some("ko".to_string()),
            ..Default::default()
        }),
    )
    .expect("ack reply");

    assert_eq!(route.ask_mode, crate::AskMode::state_patch_ack());
    assert!(route.route_reason.contains("alias_state_patch_ack"));
    assert!(!reply.is_llm_reply);
    assert_eq!(reply.text, "기억했습니다");
}

#[test]
fn alias_state_patch_ack_accepts_korean_add_or_update_schema() {
    let state = repo_i18n_state();
    let task = ask_task_with_payload_text(
        "task-alias-ack-ko-add-or-update",
        "정정합니다. \"자료A\"는 이제 scripts/nl_tests/fixtures/device_local/docs/release_checklist.md 를 가리킵니다. 답장은 \"업데이트했습니다\"만 해 주세요.",
    );
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
        "자료A",
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
    );
    let prior_route = route_for_test();
    let journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "set alias");
    crate::conversation_state::update_active_session_from_ask_outcome(
        &state,
        &task,
        None,
        "set alias",
        &prior_route,
        Some(&prior_turn),
        "set alias",
        "ack",
        &[],
        false,
        &[],
        &journal,
        None,
    );
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: None,
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(json!({
            "alias_bindings": {
                "add_or_update": [{
                    "alias": "자료A",
                    "target": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
                    "scope": "session"
                }],
                "remove": []
            },
            "required_content_literals": ["업데이트했습니다"]
        })),
        attachment_processing_required: false,
    };
    let mut route = ack_route_for_test();

    super::apply_alias_state_patch_ack_route(&mut route, Some(&turn_analysis), None);
    let reply = super::alias_state_patch_ack_reply(
        &state,
        &task,
        "Update session alias '자료A' to path 'scripts/nl_tests/fixtures/device_local/docs/release_checklist.md'.",
        &route,
        Some(&turn_analysis),
        Some(&crate::intent_router::BoundaryEnvelope {
            language_hint: Some("ko".to_string()),
            ..Default::default()
        }),
    )
    .expect("ack reply");

    assert_eq!(route.ask_mode, crate::AskMode::state_patch_ack());
    assert!(route.route_reason.contains("alias_state_patch_ack"));
    assert!(!reply.is_llm_reply);
    assert_eq!(reply.text, "업데이트했습니다");
}

#[test]
fn alias_state_patch_ack_accepts_korean_locator_hint_schema() {
    let state = repo_i18n_state();
    let task = ask_task_with_payload_text(
        "task-alias-ack-ko-locator-hint",
        "이 대화에서는 참조 이름 \"자료A\"를 scripts/nl_tests/fixtures/device_local/docs/service_notes.md 로 기억해 주세요. 답장은 \"기억했습니다\"만 해 주세요.",
    );
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: None,
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(json!({
            "alias_bindings": [{
                "alias": "자료A",
                "locator_kind": "path",
                "locator_hint": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
                "scope": "session"
            }],
            "required_machine_fields": [],
            "required_content_literals": []
        })),
        attachment_processing_required: false,
    };
    let mut route = ack_route_for_test();

    super::apply_alias_state_patch_ack_route(&mut route, Some(&turn_analysis), None);
    let reply = super::alias_state_patch_ack_reply(
        &state,
        &task,
        "Bind session alias '자료A'.",
        &route,
        Some(&turn_analysis),
        Some(&crate::intent_router::BoundaryEnvelope {
            language_hint: Some("ko".to_string()),
            ..Default::default()
        }),
    )
    .expect("ack reply");

    assert_eq!(route.ask_mode, crate::AskMode::state_patch_ack());
    assert!(route.route_reason.contains("alias_state_patch_ack"));
    assert!(!reply.is_llm_reply);
    assert_eq!(reply.text, "기억했습니다");
}

#[test]
fn alias_state_patch_ack_reply_uses_current_request_locale_for_korean_update() {
    let state = repo_i18n_state();
    let task = ask_task_with_payload_text(
        "task-alias-ack-ko-update",
        "정정합니다. \"자료A\"는 이제 scripts/nl_tests/fixtures/device_local/docs/release_checklist.md 를 가리킵니다. 답장은 \"업데이트했습니다\"만 해 주세요.",
    );
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
        "자료A",
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
        "자료A",
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
    );
    let route = ack_route_for_test();

    let reply = super::alias_state_patch_ack_reply(
        &state,
        &task,
        "Update session alias '자료A' to path 'scripts/nl_tests/fixtures/device_local/docs/release_checklist.md'.",
        &route,
        Some(&update_turn),
        None,
    )
    .expect("ack reply");

    assert!(!reply.is_llm_reply);
    assert_eq!(reply.text, "업데이트했습니다");
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

#[test]
fn alias_state_patch_ack_reply_prefers_boundary_language_hint_for_mixed_path_text() {
    let state = repo_i18n_state();
    let task = ask_task_with_payload_text(
        "task-alias-ack-zh-boundary",
        "不对，甲文件改成 scripts/nl_tests/fixtures/device_local/docs/release_checklist.md。只回复已更新。",
    );
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
        "甲文件",
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
        "甲文件",
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
    );
    let boundary = crate::intent_router::BoundaryEnvelope {
        language_hint: Some("zh".to_string()),
        ..Default::default()
    };
    let route = ack_route_for_test();

    let reply = super::alias_state_patch_ack_reply(
        &state,
        &task,
        "Update session alias",
        &route,
        Some(&update_turn),
        Some(&boundary),
    )
    .expect("ack reply");

    assert!(!reply.is_llm_reply);
    assert_eq!(reply.text, "已更新这个临时指代。");
}
