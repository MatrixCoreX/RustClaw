use super::*;

#[test]
fn command_output_summary_kb_machine_chain_exposes_kb_capability_actions() {
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.route_reason = "capability_ref=kb.ingest capability_ref=kb.search capability_ref=kb.stats; namespace=nl_codex_resume_smoke".to_string();
    route.resolved_intent =
        "kb.ingest source_path=scripts/nl_tests/fixtures/device_local/docs/service_notes.md; kb.search query='service status'; kb.stats".to_string();

    let ingest_policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "kb",
        &json!({
            "action": "ingest",
            "namespace": "nl_codex_resume_smoke",
            "paths": ["/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/service_notes.md"],
            "overwrite": true,
        }),
    )
    .expect("kb.ingest capability ref should expose ingest action");
    assert!(ingest_policy.is_allowed(), "{ingest_policy:?}");
    assert!(
        ingest_policy.action_matches_preferred(),
        "{ingest_policy:?}"
    );

    let search_policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "kb",
        &json!({
            "action": "search",
            "namespace": "nl_codex_resume_smoke",
            "query": "service status",
            "top_k": 5,
        }),
    )
    .expect("kb.search capability ref should expose search action");
    assert!(search_policy.is_allowed(), "{search_policy:?}");
    assert!(
        search_policy.action_matches_preferred(),
        "{search_policy:?}"
    );

    let stats_policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "kb",
        &json!({
            "action": "stats",
            "namespace": "nl_codex_resume_smoke",
        }),
    )
    .expect("kb.stats capability ref should expose stats action");
    assert!(stats_policy.is_allowed(), "{stats_policy:?}");
    assert!(stats_policy.action_matches_preferred(), "{stats_policy:?}");
}

#[test]
fn direct_answer_kb_machine_chain_exposes_kb_capability_actions_from_contract() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::respond_trace();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.route_reason =
        "capability_ref=kb.ingest capability_ref=kb.search capability_ref=kb.stats; namespace=agent_loop_contract"
            .to_string();
    route.resolved_intent =
        "kb.ingest source_path=/tmp/service_notes.md; kb.search query='service status'; kb.stats"
            .to_string();

    let ingest_policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "kb",
        &json!({
            "action": "ingest",
            "namespace": "agent_loop_contract",
            "paths": ["/tmp/service_notes.md"],
            "overwrite": true,
        }),
    )
    .expect("direct-answer KB contract should expose ingest action");
    assert!(ingest_policy.is_allowed(), "{ingest_policy:?}");

    let search_policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "kb",
        &json!({
            "action": "search",
            "namespace": "agent_loop_contract",
            "query": "service status",
        }),
    )
    .expect("direct-answer KB contract should expose search action");
    assert!(search_policy.is_allowed(), "{search_policy:?}");
}
