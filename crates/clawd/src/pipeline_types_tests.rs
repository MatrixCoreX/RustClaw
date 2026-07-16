use super::{
    plan_step_from_agent_action, AgentAction, IntentOutputContract, PlanStep, ResumeBehavior,
    RiskCeiling, RouteResult, ScheduleKind,
};
use serde_json::json;

fn route_result() -> RouteResult {
    RouteResult {
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract::default(),
    }
}

#[test]
fn plan_step_to_agent_action_parses_synthesize_answer() {
    let step = PlanStep {
        step_id: "step_1".to_string(),
        action_type: "synthesize_answer".to_string(),
        skill: "synthesize_answer".to_string(),
        args: json!({ "evidence_refs": ["last_output", "step_1"] }),
        depends_on: vec![],
        why: "synthesize".to_string(),
    };

    assert!(matches!(
        step.to_agent_action(),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == vec!["last_output".to_string(), "step_1".to_string()]
    ));
}

#[test]
fn plan_step_to_agent_action_normalizes_terminal_call_tool_wrappers() {
    let synthesize_step = PlanStep {
        step_id: "step_3".to_string(),
        action_type: "call_tool".to_string(),
        skill: "synthesize_answer".to_string(),
        args: json!({ "evidence_refs": ["step_1", "step_2"] }),
        depends_on: vec![],
        why: String::new(),
    };
    assert!(matches!(
        synthesize_step.to_agent_action(),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == vec!["step_1".to_string(), "step_2".to_string()]
    ));

    let respond_step = PlanStep {
        step_id: "step_4".to_string(),
        action_type: "call_tool".to_string(),
        skill: "respond".to_string(),
        args: json!({ "content": "{{last_output}}" }),
        depends_on: vec![],
        why: String::new(),
    };
    assert!(matches!(
        respond_step.to_agent_action(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn plan_step_from_agent_action_serializes_synthesize_answer() {
    let action = AgentAction::SynthesizeAnswer {
        evidence_refs: vec!["last_output".to_string()],
    };
    let step = plan_step_from_agent_action(
        &action,
        "step_2".to_string(),
        vec!["step_1".to_string()],
        "why".to_string(),
    );

    assert_eq!(step.action_type, "synthesize_answer");
    assert_eq!(step.skill, "synthesize_answer");
    assert_eq!(step.args, json!({ "evidence_refs": ["last_output"] }));
    assert_eq!(step.depends_on, vec!["step_1".to_string()]);
}

#[test]
fn plan_step_round_trips_call_capability() {
    let step = PlanStep {
        step_id: "step_1".to_string(),
        action_type: "call_capability".to_string(),
        skill: "filesystem.list_entries".to_string(),
        args: json!({ "path": "." }),
        depends_on: vec![],
        why: "list workspace".to_string(),
    };

    let action = step
        .to_agent_action()
        .expect("call_capability step should parse");
    assert!(matches!(
        action,
        AgentAction::CallCapability { capability, args }
            if capability == "filesystem.list_entries" && args == json!({ "path": "." })
    ));

    let serialized = plan_step_from_agent_action(
        &AgentAction::CallCapability {
            capability: "system.run_command".to_string(),
            args: json!({ "command": "pwd" }),
        },
        "step_2".to_string(),
        vec!["step_1".to_string()],
        "run command".to_string(),
    );
    assert_eq!(serialized.action_type, "call_capability");
    assert_eq!(serialized.skill, "system.run_command");
    assert_eq!(serialized.args, json!({ "command": "pwd" }));
}

#[test]
fn route_result_output_contract_methods_use_direct_contract() {
    let mut route = route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    route.route_reason = "contract:file_paths; contract:service_status".to_string();

    assert!(route.output_contract_marker_is(crate::OutputSemanticKind::FilePaths));
    assert!(!route.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::DirectoryNames,
        crate::OutputSemanticKind::ServiceStatus,
    ]));
    assert!(!route.output_contract_is_unclassified());
}

#[test]
fn route_result_route_reason_does_not_supply_output_contract() {
    let mut route = route_result();
    route.route_reason = "contract:workspace_project_summary".to_string();

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert_eq!(
        route.effective_output_contract_semantic_kind(),
        crate::OutputSemanticKind::None
    );
    assert_eq!(
        route.effective_output_contract().semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(!route.output_contract_marker_is(crate::OutputSemanticKind::FilePaths));
    assert!(!route.output_contract_marker_is(crate::OutputSemanticKind::WorkspaceProjectSummary));
}

#[test]
fn route_reason_marker_facade_reads_machine_values_with_shared_tokenization() {
    let markers = crate::RouteReasonMarkers::new(
        "first_token, clarify_reason_code:missing_locator | clarify_reason_code=missing_target",
    );

    assert!(markers.has_machine_marker("first_token"));
    assert_eq!(
        markers.machine_value("clarify_reason_code"),
        Some("missing_target")
    );
}

#[test]
fn route_result_route_reason_cannot_override_direct_semantic_kind() {
    let mut route = route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    route.route_reason = "contract:workspace_project_summary".to_string();

    assert_eq!(
        route.effective_output_contract_semantic_kind(),
        crate::OutputSemanticKind::FilePaths
    );
    assert_eq!(
        route.effective_output_contract().semantic_kind,
        crate::OutputSemanticKind::FilePaths
    );
}
