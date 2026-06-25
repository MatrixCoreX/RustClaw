use super::*;

#[test]
fn task_control_dry_run_contract_tokens_return_structured_cancel_projection() {
    let mut route = base_route_result();
    route.route_reason =
        "capability_ref=task_control field=task_id field=state field=can_cancel dry_run"
            .to_string();
    route.resolved_intent =
        "task_control task_id state can_cancel cancel_requested would_mutate=false".to_string();

    let plan = structured_dry_run_response_deterministic_plan_result(
        "dry-run task cancel contract",
        Some(&route),
        &LoopState::new(1),
    )
    .expect("task_control dry-run contract should return structured response");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let AgentAction::Respond { content } = action else {
        panic!("unexpected action: {action:?}");
    };
    let value: Value = serde_json::from_str(&content).expect("structured response json");
    assert_eq!(
        value.get("semantic_kind").and_then(Value::as_str),
        Some("task_control_cancel_dry_run")
    );
    assert_eq!(
        value
            .pointer("/execution_policy/call_task_cancel_api")
            .and_then(Value::as_bool),
        Some(false)
    );
}

#[test]
fn config_risk_preview_uses_git_plan_change_and_guard_observations() {
    let state = test_state_with_enabled_skills(&["git_basic", "config_edit", "config_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let loop_state = LoopState::new(1);

    let plan = config_risk_preview_deterministic_plan_result(
        &state,
        "preview config change and guard",
        Some(&route),
        &loop_state,
        "configs/config.toml llm.selected_vendor minimax",
        None,
    )
    .expect("config risk preview should use config_edit and guard tools");

    assert_eq!(plan.steps.len(), 5);
    let git = plan.steps[0].to_agent_action().expect("git action");
    assert_eq!(
        expect_planned_call(&git, "git_basic", "status")
            .as_object()
            .map(|obj| obj.len()),
        Some(1)
    );
    let preview = plan.steps[1].to_agent_action().expect("preview action");
    let preview_args = expect_planned_call(&preview, "config_edit", "plan_config_change");
    assert_eq!(
        preview_args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
    assert_eq!(
        preview_args.get("field_path").and_then(Value::as_str),
        Some("llm.selected_vendor")
    );
    assert_eq!(
        preview_args.get("value").and_then(Value::as_str),
        Some("minimax")
    );
    let guard = plan.steps[2].to_agent_action().expect("guard action");
    let guard_args = expect_planned_call(&guard, "config_basic", "guard_rustclaw_config");
    assert_eq!(
        guard_args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
    let synth = plan.steps[3].to_agent_action().expect("synthesis action");
    let AgentAction::SynthesizeAnswer { evidence_refs } = synth else {
        panic!("unexpected synthesis action: {synth:?}");
    };
    assert_eq!(evidence_refs, vec!["step_1", "step_2", "step_3"]);
}

#[test]
fn main_config_content_excerpt_deterministic_fast_path_uses_guard_observation() {
    let state = test_state_with_enabled_skills(&["fs_basic", "config_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let loop_state = LoopState::new(1);

    let plan = content_excerpt_explicit_file_targets_deterministic_plan_result(
        &state,
        "summarize main config",
        Some(&route),
        &loop_state,
        "configs/config.toml",
        None,
        Some("/home/guagua/rustclaw/configs/config.toml"),
    )
    .expect("main config broad content summary should prefer config guard");

    assert_eq!(plan.steps.len(), 3);
    assert_eq!(plan.steps[0].skill, "config_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("guard_rustclaw_config")
    );
    assert_eq!(
        plan.steps[0].args.get("path").and_then(Value::as_str),
        Some("/home/guagua/rustclaw/configs/config.toml")
    );
    let synth = plan.steps[1].to_agent_action().expect("synthesis action");
    let AgentAction::SynthesizeAnswer { evidence_refs } = synth else {
        panic!("unexpected synthesis action: {synth:?}");
    };
    assert_eq!(evidence_refs, vec!["step_1"]);
}

#[test]
fn browser_http_summary_uses_both_observations_and_explicit_evidence_refs() {
    let state = test_state_with_enabled_skills(&["browser_web", "http_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::WebPageSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Url;
    route.output_contract.locator_hint = "https://example.com".to_string();
    route.route_reason =
        "capability_ref=browser_web.open_extract capability_ref=http_basic.get".to_string();
    let loop_state = LoopState::new(1);

    let plan = browser_http_url_deterministic_plan_result(
        &state,
        "observe web title and http status",
        Some(&route),
        &loop_state,
        "https://example.com",
    )
    .expect("browser/http route should use both observations");

    assert_eq!(plan.steps.len(), 4);
    let browser = plan.steps[0].to_agent_action().expect("browser action");
    assert_eq!(
        expect_planned_call(&browser, "browser_web", "open_extract")
            .get("url")
            .and_then(Value::as_str),
        Some("https://example.com")
    );
    let http = plan.steps[1].to_agent_action().expect("http action");
    assert_eq!(
        expect_planned_call(&http, "http_basic", "get")
            .get("url")
            .and_then(Value::as_str),
        Some("https://example.com")
    );
    let synth = plan.steps[2].to_agent_action().expect("synthesis action");
    let AgentAction::SynthesizeAnswer { evidence_refs } = synth else {
        panic!("unexpected synthesis action: {synth:?}");
    };
    assert_eq!(evidence_refs, vec!["step_1", "step_2"]);
}

#[test]
fn web_search_summary_prefers_quoted_query_over_full_instruction() {
    let state = test_state_with_enabled_skills(&["web_search_extract"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::WebSearchSummary;
    route.resolved_intent =
        "Search the web for \"Rust async tutorial\" with top_k=3 and return titles".to_string();
    route.output_contract.self_extension.list_selector.limit = Some(3);
    let loop_state = LoopState::new(1);

    let plan = web_search_summary_deterministic_plan_result(
        &state,
        "search web",
        Some(&route),
        &loop_state,
        "Search the web for \"Rust async tutorial\" with top_k=3",
    )
    .expect("web search summary should use search_extract");

    assert_eq!(plan.steps.len(), 3);
    let action = plan.steps[0].to_agent_action().expect("search action");
    let args = expect_planned_call(&action, "web_search_extract", "search_extract");
    assert_eq!(
        args.get("query").and_then(Value::as_str),
        Some("Rust async tutorial")
    );
    assert_eq!(args.get("top_k").and_then(Value::as_u64), Some(3));
}

#[test]
fn chat_wrapped_text_loop_terminal_respond_does_not_force_plan_repair() {
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        false,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.delivery_required = false;
    route.wants_file_delivery = false;
    let actions = vec![AgentAction::Respond {
        content:
            r#"{"status":"ok","message_key":"provider_blocker","category":"external_blocker"}"#
                .to_string(),
    }];

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions
    ));
}
