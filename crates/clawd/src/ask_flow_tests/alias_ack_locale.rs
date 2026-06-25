#[test]
fn structural_alias_ack_defers_japanese_update_without_safe_candidate() {
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

    assert!(structural_alias_binding_ack(&state, Some(&ctx), "", "", "ja").is_none());
}

#[test]
fn structural_alias_ack_defers_korean_update_without_safe_candidate() {
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

    assert!(structural_alias_binding_ack(&state, Some(&ctx), "", "", "ko").is_none());
}
