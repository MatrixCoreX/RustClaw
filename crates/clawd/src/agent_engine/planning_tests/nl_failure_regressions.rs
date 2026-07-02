use super::*;

#[test]
fn task_control_dry_run_contract_tokens_return_structured_cancel_projection() {
    let mut route = base_route_result();
    route.route_reason =
        "capability_ref=task_control.cancel_one field=task_id field=state field=can_cancel dry_run"
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
fn generic_task_control_capability_ref_does_not_trigger_cancel_dry_run_contract() {
    let mut route = base_route_result();
    route.route_reason =
        "capability_ref=task_control field=task_id field=state field=can_cancel dry_run"
            .to_string();
    route.resolved_intent =
        "task_control task_id state can_cancel cancel_requested would_mutate=false".to_string();

    assert!(structured_dry_run_response_deterministic_plan_result(
        "dry-run task cancel contract",
        Some(&route),
        &LoopState::new(1),
    )
    .is_none());
}

#[test]
fn task_control_cancel_dry_run_requires_capability_ref_assignment() {
    let mut route = base_route_result();
    route.route_reason = "task_control.cancel_one cancel_one dry_run=true".to_string();
    route.resolved_intent =
        "task_control.cancel_one action=cancel_one would_mutate=false".to_string();

    assert!(structured_dry_run_response_deterministic_plan_result(
        "dry-run task cancel contract",
        Some(&route),
        &LoopState::new(1),
    )
    .is_none());
}

#[test]
fn task_control_dry_run_ignores_prompt_only_capability_refs() {
    assert!(structured_dry_run_response_deterministic_plan_result(
        "capability_ref=task_control.cancel_one capability_ref=task_control.resume dry_run=true",
        None,
        &LoopState::new(1),
    )
    .is_none());
}

#[test]
fn task_control_lifecycle_dry_run_tokens_return_structured_resume_pause_projection() {
    let mut route = base_route_result();
    route.route_reason =
        "capability_ref=task_control.resume capability_ref=task_control.pause dry_run=true"
            .to_string();
    route.resolved_intent = concat!(
        "task_control.resume task_control.pause ",
        "task_id=00000000-0000-4000-8000-000000000010 ",
        "checkpoint_id=ckpt-1 pause_seconds=120 would_mutate=false"
    )
    .to_string();

    let plan = structured_dry_run_response_deterministic_plan_result(
        "task-control lifecycle dry-run contract",
        Some(&route),
        &LoopState::new(1),
    )
    .expect("task_control lifecycle dry-run contract should return structured response");

    assert_eq!(plan.steps.len(), 3);
    let action = plan.steps[0].to_agent_action().expect("resume action");
    let AgentAction::CallSkill { skill, args } = action else {
        panic!("unexpected action: {action:?}");
    };
    assert_eq!(skill, "task_control");
    assert_eq!(args.get("action").and_then(Value::as_str), Some("resume"));
    assert_eq!(args.get("dry_run").and_then(Value::as_bool), Some(true));
    assert_eq!(
        args.get("checkpoint_id").and_then(Value::as_str),
        Some("ckpt-1")
    );

    let action = plan.steps[1].to_agent_action().expect("pause action");
    let AgentAction::CallSkill { skill, args } = action else {
        panic!("unexpected action: {action:?}");
    };
    assert_eq!(skill, "task_control");
    assert_eq!(args.get("action").and_then(Value::as_str), Some("pause"));
    assert_eq!(args.get("dry_run").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("pause_seconds").and_then(Value::as_u64), Some(120));

    let action = plan.steps[2].to_agent_action().expect("respond action");
    let AgentAction::Respond { content } = action else {
        panic!("unexpected action: {action:?}");
    };
    assert!(content.contains("task_control.resume.dry_run"));
    assert!(content.contains("task_control.pause.dry_run"));
    assert!(content.contains("checkpoint_id=ckpt-1"));
    assert!(content.contains("would_mutate=false"));
}

#[test]
fn task_control_lifecycle_dry_run_requires_explicit_capability_refs() {
    let mut route = base_route_result();
    route.route_reason = "task_control.resume task_control.pause dry_run=true".to_string();
    route.resolved_intent = concat!(
        "action=resume action=pause ",
        "task_id=00000000-0000-4000-8000-000000000010 ",
        "checkpoint_id=ckpt-1 pause_seconds=120 would_mutate=false"
    )
    .to_string();

    assert!(structured_dry_run_response_deterministic_plan_result(
        "task-control lifecycle dry-run contract",
        Some(&route),
        &LoopState::new(1),
    )
    .is_none());
}

#[test]
fn config_risk_preview_uses_git_plan_change_and_guard_observations() {
    let state = test_state_with_enabled_skills(&["git_basic", "config_edit", "config_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    route.route_reason = "field_path=llm.selected_vendor value=minimax".to_string();
    let loop_state = LoopState::new(1);

    let plan = config_risk_preview_deterministic_plan_result(
        &state,
        "preview config change and guard",
        Some(&route),
        &loop_state,
        "configs/config.toml llm.selected_vendor wrong_user_text_value",
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
fn config_risk_preview_uses_capability_ref_without_semantic_kind() {
    let state = test_state_with_enabled_skills(&["git_basic", "config_edit", "config_basic"]);
    let mut route = base_route_result();
    route.route_reason =
        "capability_ref=config.guard_after_change field_path=llm.selected_vendor value=minimax"
            .to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let loop_state = LoopState::new(1);

    let plan = config_risk_preview_deterministic_plan_result(
        &state,
        "preview config change and guard",
        Some(&route),
        &loop_state,
        "configs/config.toml llm.selected_vendor wrong_user_text_value",
        None,
    )
    .expect("config risk preview should use config capability_ref");

    assert_eq!(plan.steps.len(), 5);
    let preview = plan.steps[1].to_agent_action().expect("preview action");
    let preview_args = expect_planned_call(&preview, "config_edit", "plan_config_change");
    assert_eq!(
        preview_args.get("field_path").and_then(Value::as_str),
        Some("llm.selected_vendor")
    );
    assert_eq!(
        preview_args.get("value").and_then(Value::as_str),
        Some("minimax")
    );
    let guard = plan.steps[2].to_agent_action().expect("guard action");
    expect_planned_call(&guard, "config_basic", "guard_rustclaw_config");
}

#[test]
fn config_risk_preview_without_machine_field_value_defers_to_planner() {
    let state = test_state_with_enabled_skills(&["git_basic", "config_edit", "config_basic"]);
    let mut route = base_route_result();
    route.route_reason = "capability_ref=config.guard_after_change".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
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
    );

    assert!(plan.is_none());
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
fn chat_wrapped_text_loop_terminal_respond_does_not_force_plan_repair() {
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
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
