use super::*;

#[test]
fn command_output_summary_kb_machine_chain_uses_kb_actions() {
    let state = test_state_with_enabled_skills(&["kb", "git_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.route_reason = "ordered KB skill actions: kb.ingest -> kb.search 'service status' -> kb.stats; namespace=nl_codex_resume_smoke".to_string();
    route.resolved_intent =
        "kb.ingest source_path=scripts/nl_tests/fixtures/device_local/docs/service_notes.md; kb.search query='service status'; kb.stats".to_string();
    let loop_state = LoopState::new(1);

    let plan = kb_chain_deterministic_plan_result(
        &state,
        "kb local cycle",
        Some(&route),
        &loop_state,
        "namespace=nl_codex_resume_smoke scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
        Some("/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/service_notes.md"),
    )
    .expect("machine KB chain should produce an ordered kb plan");

    assert_eq!(plan.steps.len(), 5);
    let ingest = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&ingest, "kb", "ingest");
    assert_eq!(
        args.get("namespace").and_then(Value::as_str),
        Some("nl_codex_resume_smoke")
    );
    assert_eq!(args.get("overwrite").and_then(Value::as_bool), Some(true));
    assert_eq!(
        args.get("paths")
            .and_then(Value::as_array)
            .and_then(|paths| paths.first())
            .and_then(Value::as_str),
        Some("/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/service_notes.md")
    );

    let search = plan.steps[1].to_agent_action().expect("agent action");
    let args = expect_planned_call(&search, "kb", "search");
    assert_eq!(
        args.get("namespace").and_then(Value::as_str),
        Some("nl_codex_resume_smoke")
    );
    assert_eq!(
        args.get("query").and_then(Value::as_str),
        Some("service status")
    );

    let stats = plan.steps[2].to_agent_action().expect("agent action");
    let args = expect_planned_call(&stats, "kb", "stats");
    assert_eq!(
        args.get("namespace").and_then(Value::as_str),
        Some("nl_codex_resume_smoke")
    );
}

#[test]
fn direct_answer_kb_machine_chain_uses_kb_actions_from_contract() {
    let state = test_state_with_enabled_skills(&["kb"]);
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::direct_answer();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.route_reason =
        "kb.ingest -> kb.search 'service status' -> kb.stats; namespace=agent_loop_contract"
            .to_string();
    route.resolved_intent =
        "kb.ingest source_path=/tmp/service_notes.md; kb.search query='service status'; kb.stats"
            .to_string();
    let loop_state = LoopState::new(1);

    let plan = kb_chain_deterministic_plan_result(
        &state,
        "kb local cycle",
        Some(&route),
        &loop_state,
        "namespace=agent_loop_contract /tmp/service_notes.md",
        Some("/tmp/service_notes.md"),
    )
    .expect("direct-answer compatibility trace should use the kb chain contract");

    assert_eq!(plan.steps.len(), 5);
    let ingest = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&ingest, "kb", "ingest");
    assert_eq!(
        args.get("namespace").and_then(Value::as_str),
        Some("agent_loop_contract")
    );

    let search = plan.steps[1].to_agent_action().expect("agent action");
    let args = expect_planned_call(&search, "kb", "search");
    assert_eq!(
        args.get("query").and_then(Value::as_str),
        Some("service status")
    );
}
