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

#[test]
fn registry_capability_keeps_kb_visible_to_planner() {
    let state = test_state_with_registry();
    let registry = state.get_skills_registry().expect("registry loaded");
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = Arc::new(SkillViewsSnapshot {
        registry: Some(registry),
        skills_list: Arc::new(HashSet::from(["fs_basic".to_string(), "kb".to_string()])),
    });
    let state = state.with_prompt_layers_installed();
    let task = test_task();

    let quick_index = build_lightweight_skill_quick_index_text(&state, &task, None);

    assert!(quick_index.contains("kb"), "{quick_index}");
    assert!(quick_index.contains("kb.ingest"), "{quick_index}");
    assert!(quick_index.contains("kb.search"), "{quick_index}");
    assert!(quick_index.contains("kb.stats"), "{quick_index}");
}

#[test]
fn open_scope_lightweight_skill_notes_use_compact_registry_index() {
    let state = test_state_with_registry();
    let registry = state.get_skills_registry().expect("registry loaded");
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = Arc::new(SkillViewsSnapshot {
        registry: Some(registry),
        skills_list: Arc::new(HashSet::from([
            "archive_basic".to_string(),
            "browser_web".to_string(),
            "config_basic".to_string(),
            "config_edit".to_string(),
            "db_basic".to_string(),
            "doc_parse".to_string(),
            "fs_basic".to_string(),
            "git_basic".to_string(),
            "health_check".to_string(),
            "http_basic".to_string(),
            "kb".to_string(),
            "log_analyze".to_string(),
            "package_manager".to_string(),
            "process_basic".to_string(),
            "system_basic".to_string(),
            "task_control".to_string(),
        ])),
    });
    let state = state.with_prompt_layers_installed();
    let task = test_task();

    let playbooks = build_lightweight_skill_playbooks_text(&state, &task, None);
    let quick_index = build_lightweight_skill_quick_index_text(&state, &task, None);

    assert!(
        playbooks.starts_with("open_scope_lightweight_skill_index_v1"),
        "{playbooks}"
    );
    assert!(playbooks.contains("skill=kb"), "{playbooks}");
    assert!(playbooks.contains("kb.ingest"), "{playbooks}");
    assert!(playbooks.contains("kb.search"), "{playbooks}");
    assert!(playbooks.contains("kb.stats"), "{playbooks}");
    assert!(playbooks.contains("req=namespace"), "{playbooks}");
    assert!(
        !playbooks.contains("filesystem_write=false"),
        "open-scope index should omit repeated policy booleans: {playbooks}"
    );
    assert!(
        !playbooks.contains("runtime_availability:"),
        "open-scope index should omit verbose availability metadata: {playbooks}"
    );
    assert!(
        !playbooks.contains("Requests that semantically mean"),
        "{playbooks}"
    );
    assert!(
        playbooks.chars().count() < 24_000,
        "compact index should stay bounded, got {} chars",
        playbooks.chars().count()
    );
    assert!(
        quick_index.starts_with("open_scope_lightweight_quick_index_ref_v1"),
        "{quick_index}"
    );
    assert!(
        quick_index.contains("visible_skill_count=16"),
        "{quick_index}"
    );
    assert!(
        !quick_index.contains("planner_capabilities:"),
        "open scope quick index should not duplicate registry metadata: {quick_index}"
    );
    assert!(
        playbooks.chars().count() + quick_index.chars().count() < 24_500,
        "combined open-scope lightweight skill context should stay bounded"
    );
}
