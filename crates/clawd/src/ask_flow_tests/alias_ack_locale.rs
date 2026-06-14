#[test]
fn structural_alias_ack_uses_locale_i18n_for_japanese_alias_update() {
    let state = crate::AppState::test_default_with_fixture_provider().with_prompt_layers_installed();
    let route = chat_route_for_gate();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        session_alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
            alias: "資料A".to_string(),
            target: "scripts/nl_tests/fixtures/device_local/docs/service_notes.md".to_string(),
            updated_at_ts: 1,
        }],
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "alias_bindings": [{
                    "alias": "資料A",
                    "target": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
                }]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let reply = structural_alias_binding_ack(&state, Some(&ctx), "", "", "ja")
        .expect("alias update ack");

    assert_eq!(reply.text, "更新しました");
    assert!(reply.messages.is_empty());
}

#[test]
fn structural_alias_ack_uses_locale_i18n_for_korean_alias_update() {
    let state = crate::AppState::test_default_with_fixture_provider().with_prompt_layers_installed();
    let route = chat_route_for_gate();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        session_alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
            alias: "자료A".to_string(),
            target: "scripts/nl_tests/fixtures/device_local/docs/service_notes.md".to_string(),
            updated_at_ts: 1,
        }],
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "alias_bindings": [{
                    "alias": "자료A",
                    "target": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
                }]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let reply = structural_alias_binding_ack(&state, Some(&ctx), "", "", "ko")
        .expect("alias update ack");

    assert_eq!(reply.text, "업데이트했습니다");
    assert!(reply.messages.is_empty());
}
