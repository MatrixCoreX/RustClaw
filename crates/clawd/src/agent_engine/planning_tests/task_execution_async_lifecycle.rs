use super::*;

#[test]
fn structured_async_start_keeps_planner_action_when_route_contract_is_generic_content() {
    let state = test_state_with_enabled_skills(&["fs_basic", "run_cmd"]);
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "rustclaw".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({
            "command": "sleep 2 && echo RUSTCLAW_ASYNC_LIFECYCLE",
            "async_start": true,
            "poll_after_seconds": 2,
            "expires_in_seconds": 600,
            "capture_checkpoint": true
        }),
    }];

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &LoopState::new(1),
        "start async lifecycle",
        Some("start async lifecycle"),
        Some("start async lifecycle"),
        Some("rustclaw"),
        actions,
    );

    let Some(AgentAction::CallSkill { skill, args }) = normalized.first() else {
        panic!("expected planner async run_cmd as first action, got {normalized:?}");
    };
    assert_eq!(skill, "run_cmd");
    assert_eq!(
        args.get("command").and_then(Value::as_str),
        Some("sleep 2 && echo RUSTCLAW_ASYNC_LIFECYCLE")
    );
    assert_eq!(args.get("async_start").and_then(Value::as_bool), Some(true));
    assert_eq!(
        args.get("poll_after_seconds").and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        args.get("expires_in_seconds").and_then(Value::as_u64),
        Some(600)
    );
}

#[test]
fn async_job_protocol_dry_run_does_not_inject_real_run_cmd_async_start() {
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let mut route = base_route_result();
    route.resolved_intent =
        "async_job_protocol=version:1 mode=dry_run adapter_result_key=async_poll_adapter_result"
            .to_string();
    route.route_reason =
        "async_job_protocol=version:1 mode=dry_run would_mutate=false required_job_fields=job_id|status|poll_after_seconds|expires_at|cancel_ref|message_key"
            .to_string();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command":"sleep 2 && echo RUSTCLAW_ASYNC_DRY_RUN"}),
    }];

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &LoopState::new(1),
        "dry-run runtime async job",
        Some("dry-run runtime async job"),
        Some(&route.route_reason),
        None,
        actions,
    );

    assert!(
        !normalized.is_empty(),
        "planner action should remain visible for preflight classification"
    );
    for action in normalized {
        let AgentAction::CallSkill { skill, args } = action else {
            panic!("expected run_cmd call(s), got {action:?}");
        };
        assert_eq!(skill, "run_cmd");
        assert!(args.get("async_start").is_none());
        assert!(args
            .get(super::super::super::CLAWD_RUNTIME_ASYNC_JOB_START_ARG)
            .is_none());
    }
}

#[test]
fn execution_recipe_async_plan_hint_preserves_planner_run_cmd_without_route_async_marker() {
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "rustclaw".to_string();
    let mut loop_state = LoopState::new(1);
    loop_state.output_vars.insert(
        "route_execution_recipe_plan_kind".to_string(),
        "async_job_start".to_string(),
    );
    loop_state.output_vars.insert(
        "route_execution_recipe_plan_command".to_string(),
        "sleep 2 && echo RUSTCLAW_ASYNC_LIFECYCLE".to_string(),
    );
    loop_state.output_vars.insert(
        "route_execution_recipe_plan_execution_mode".to_string(),
        "async_preferred".to_string(),
    );
    loop_state.output_vars.insert(
        "route_execution_recipe_plan_async_adapter_kind".to_string(),
        "local_process_poll".to_string(),
    );
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({
            "command": "sleep 2 && echo RUSTCLAW_ASYNC_LIFECYCLE",
            "async_start": true,
            "poll_after_seconds": 2,
            "expires_in_seconds": 600,
            super::super::super::CLAWD_RUNTIME_ASYNC_JOB_START_ARG: "async_job_protocol"
        }),
    }];

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "start async lifecycle",
        Some("start async lifecycle"),
        Some("start async lifecycle"),
        Some("rustclaw"),
        actions,
    );

    let Some(AgentAction::CallSkill { skill, args }) = normalized.first() else {
        panic!("expected run_cmd action, got {normalized:?}");
    };
    assert_eq!(skill, "run_cmd");
    assert_eq!(
        args.get("command").and_then(Value::as_str),
        Some("sleep 2 && echo RUSTCLAW_ASYNC_LIFECYCLE")
    );
    assert_eq!(args.get("async_start").and_then(Value::as_bool), Some(true));
    assert_eq!(
        args.get(super::super::super::CLAWD_RUNTIME_ASYNC_JOB_START_ARG)
            .and_then(Value::as_str),
        Some("async_job_protocol")
    );
}
