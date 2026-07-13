use super::*;

fn normalize_test_actions(
    route: &RouteResult,
    loop_state: &LoopState,
    goal: &str,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    normalize_planned_actions(&test_state(), Some(route), loop_state, goal, None, actions)
}

#[test]
fn task_control_dry_run_contract_tokens_return_structured_cancel_projection() {
    let mut route = base_route_result();
    route.route_reason =
        "capability_ref=task_control.cancel_one field=task_id field=state field=can_cancel dry_run"
            .to_string();
    route.resolved_intent =
        "task_control task_id state can_cancel cancel_requested would_mutate=false".to_string();

    let normalized = normalize_test_actions(
        &route,
        &LoopState::new(1),
        "dry-run task cancel contract",
        vec![AgentAction::Respond {
            content: json!({
                "contract_marker": "task_control_cancel_dry_run",
                "execution_policy": {
                    "call_task_cancel_api": false
                }
            })
            .to_string(),
        }],
    );

    assert_eq!(normalized.len(), 1);
    let action = normalized[0].clone();
    let AgentAction::Respond { content } = action else {
        panic!("unexpected action: {action:?}");
    };
    let value: Value = serde_json::from_str(&content).expect("structured response json");
    assert_eq!(
        value.get("contract_marker").and_then(Value::as_str),
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

    let normalized = normalize_test_actions(
        &route,
        &LoopState::new(1),
        "dry-run task cancel contract",
        vec![],
    );

    assert!(normalized.is_empty());
}

#[test]
fn task_control_cancel_dry_run_requires_capability_ref_assignment() {
    let mut route = base_route_result();
    route.route_reason = "task_control.cancel_one cancel_one dry_run=true".to_string();
    route.resolved_intent =
        "task_control.cancel_one action=cancel_one would_mutate=false".to_string();

    let normalized = normalize_test_actions(
        &route,
        &LoopState::new(1),
        "dry-run task cancel contract",
        vec![],
    );

    assert!(normalized.is_empty());
}

#[test]
fn task_control_dry_run_ignores_prompt_only_capability_refs() {
    let route = base_route_result();
    let normalized = normalize_test_actions(
        &route,
        &LoopState::new(1),
        "capability_ref=task_control.cancel_one capability_ref=task_control.resume dry_run=true",
        vec![],
    );

    assert!(normalized.is_empty());
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

    let normalized = normalize_test_actions(
        &route,
        &LoopState::new(1),
        "task-control lifecycle dry-run contract",
        vec![
            AgentAction::CallSkill {
                skill: "task_control".to_string(),
                args: json!({
                    "action": "resume",
                    "task_id": "00000000-0000-4000-8000-000000000010",
                    "checkpoint_id": "ckpt-1",
                    "dry_run": true
                }),
            },
            AgentAction::CallSkill {
                skill: "task_control".to_string(),
                args: json!({
                    "action": "pause",
                    "task_id": "00000000-0000-4000-8000-000000000010",
                    "pause_seconds": 120,
                    "dry_run": true
                }),
            },
            AgentAction::Respond {
                content: json!({
                    "message_keys": [
                        "task_control.resume.dry_run",
                        "task_control.pause.dry_run"
                    ],
                    "checkpoint_id": "ckpt-1",
                    "would_mutate": false
                })
                .to_string(),
            },
        ],
    );

    assert_eq!(normalized.len(), 2);
    let action = normalized[0].clone();
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

    let action = normalized[1].clone();
    let AgentAction::CallSkill { skill, args } = action else {
        panic!("unexpected action: {action:?}");
    };
    assert_eq!(skill, "task_control");
    assert_eq!(args.get("action").and_then(Value::as_str), Some("pause"));
    assert_eq!(args.get("dry_run").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("pause_seconds").and_then(Value::as_u64), Some(120));
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

    let normalized = normalize_test_actions(
        &route,
        &LoopState::new(1),
        "task-control lifecycle dry-run contract",
        vec![],
    );

    assert!(normalized.is_empty());
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

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "preview config change and guard",
        None,
        vec![
            AgentAction::CallSkill {
                skill: "git_basic".to_string(),
                args: json!({"action": "status"}),
            },
            AgentAction::CallSkill {
                skill: "config_edit".to_string(),
                args: json!({
                    "action": "plan_config_change",
                    "path": "configs/config.toml",
                    "field_path": "llm.selected_vendor",
                    "value": "minimax"
                }),
            },
            AgentAction::CallSkill {
                skill: "config_basic".to_string(),
                args: json!({
                    "action": "guard_rustclaw_config",
                    "path": "configs/config.toml"
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec![
                    "step_1".to_string(),
                    "step_2".to_string(),
                    "step_3".to_string(),
                ],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ],
    );

    assert_eq!(normalized.len(), 4);
    let git = &normalized[0];
    assert_eq!(
        expect_planned_call(&git, "git_basic", "status")
            .as_object()
            .map(|obj| obj.len()),
        Some(1)
    );
    let guard = &normalized[1];
    let guard_args = expect_planned_call(&guard, "config_basic", "guard_rustclaw_config");
    assert_eq!(
        guard_args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
    let synth = normalized[2].clone();
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

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "preview config change and guard",
        None,
        vec![
            AgentAction::CallSkill {
                skill: "git_basic".to_string(),
                args: json!({"action": "status"}),
            },
            AgentAction::CallSkill {
                skill: "config_edit".to_string(),
                args: json!({
                    "action": "plan_config_change",
                    "path": "configs/config.toml",
                    "field_path": "llm.selected_vendor",
                    "value": "minimax"
                }),
            },
            AgentAction::CallSkill {
                skill: "config_basic".to_string(),
                args: json!({
                    "action": "guard_rustclaw_config",
                    "path": "configs/config.toml"
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec![
                    "step_1".to_string(),
                    "step_2".to_string(),
                    "step_3".to_string(),
                ],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ],
    );

    assert_eq!(normalized.len(), 4);
    let guard = &normalized[1];
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

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "preview config change and guard",
        None,
        vec![],
    );

    assert!(
        normalized.is_empty(),
        "runtime must not inject config risk preview without machine field/value: {normalized:?}"
    );
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

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "summarize main config",
        None,
        vec![
            AgentAction::CallSkill {
                skill: "config_basic".to_string(),
                args: json!({
                    "action": "guard_rustclaw_config",
                    "path": "/home/guagua/rustclaw/configs/config.toml"
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["step_1".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ],
    );

    assert_eq!(normalized.len(), 3);
    let guard_args = expect_planned_call(&normalized[0], "config_basic", "guard_rustclaw_config");
    assert_eq!(
        guard_args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
    let synth = normalized[1].clone();
    let AgentAction::SynthesizeAnswer { evidence_refs } = synth else {
        panic!("unexpected synthesis action: {synth:?}");
    };
    assert_eq!(evidence_refs, vec!["step_1"]);
}

#[test]
fn chat_wrapped_text_loop_terminal_respond_does_not_force_plan_repair() {
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
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

#[test]
fn boundary_observation_clarify_terminal_respond_does_not_force_plan_repair() {
    let mut loop_state = LoopState::new(1);
    loop_state.boundary_observation_needs_clarify = true;
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.needs_clarify = false;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.delivery_required = false;
    route.wants_file_delivery = false;
    let actions = vec![AgentAction::Respond {
        content: r#"{"message_key":"clarify_missing_target","missing_slot":"target"}"#.to_string(),
    }];

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn pending_user_boundary_terminal_respond_does_not_force_plan_repair() {
    let mut loop_state = LoopState::new(1);
    loop_state.pending_user_boundary_present = true;
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.needs_clarify = false;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.delivery_required = false;
    route.wants_file_delivery = false;
    let actions = vec![AgentAction::Respond {
        content: r#"{"message_key":"deferred_clarify_question","missing_slot":"target"}"#
            .to_string(),
    }];

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions
    ));
}
