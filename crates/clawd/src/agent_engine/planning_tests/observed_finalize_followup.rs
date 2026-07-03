use super::*;

#[test]
fn content_evidence_route_keeps_terminal_discussion_followup_for_planned_synthesis() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "read_range",
                "path": "README.md",
                "mode": "head",
                "n": 20
            }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let kept = strip_terminal_discussion_for_observed_finalize(
        Some(&route_result(
            crate::AskMode::planner_execute_with_chat_finalizer(),
            true,
            OutputResponseShape::Free,
        )),
        &loop_state,
        actions.clone(),
    );
    assert_eq!(kept.len(), 2);
    assert!(matches!(
        &kept[0],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
    assert!(matches!(
        &kept[1],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn content_evidence_route_keeps_terminal_synthesize_followup_for_planned_synthesis() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "read_range",
                "path": "README.md",
                "mode": "head",
                "n": 20
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];
    let kept = strip_terminal_discussion_for_observed_finalize(
        Some(&route_result(
            crate::AskMode::planner_execute_with_chat_finalizer(),
            true,
            OutputResponseShape::Free,
        )),
        &loop_state,
        actions.clone(),
    );
    assert_eq!(kept.len(), 2);
    assert!(matches!(
        &kept[0],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
    assert!(matches!(
        &kept[1],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["last_output".to_string()]
    ));
}

#[test]
fn content_evidence_route_keeps_multi_evidence_synthesize_followup() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "read_range",
                "path": "service_notes.md",
                "mode": "head",
                "n": 20
            }),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "README.md" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["s1".to_string(), "s2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let kept = strip_terminal_discussion_for_observed_finalize(
        Some(&route_result(
            crate::AskMode::planner_execute_with_chat_finalizer(),
            true,
            OutputResponseShape::Free,
        )),
        &loop_state,
        actions.clone(),
    );
    assert_eq!(kept.len(), 4);
    assert!(matches!(
        &kept[0],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
    assert!(matches!(
        &kept[1],
        AgentAction::CallSkill { skill, .. } if skill == "read_file"
    ));
    assert!(matches!(
        &kept[2],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["s1".to_string(), "s2".to_string()]
    ));
    assert!(matches!(
        &kept[3],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn recent_scalar_pair_strips_terminal_synthesis_for_runtime_finalizer() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: serde_json::json!({
                "action": "read_field",
                "path": "UI/package.json",
                "field_path": "name"
            }),
        },
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: serde_json::json!({
                "action": "read_field",
                "path": "crates/clawd/Cargo.toml",
                "field_path": "package.name"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;

    let stripped =
        strip_terminal_discussion_for_observed_finalize(Some(&route), &loop_state, actions);

    assert_eq!(stripped.len(), 2);
    assert!(matches!(
        &stripped[0],
        AgentAction::CallTool { tool, args }
            if tool == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_field")
    ));
    assert!(matches!(
        &stripped[1],
        AgentAction::CallTool { tool, args }
            if tool == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_field")
    ));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &loop_state,
        &stripped
    ));
}
