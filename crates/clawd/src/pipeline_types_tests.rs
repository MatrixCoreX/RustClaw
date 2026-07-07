use super::{
    plan_step_from_agent_action, AgentAction, AskMode, IntentOutputContract, PlanStep,
    ResumeBehavior, RiskCeiling, RouteResult, ScheduleKind,
};
use serde_json::json;

fn route_result_with_mode(ask_mode: AskMode) -> RouteResult {
    RouteResult {
        ask_mode,
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
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
fn route_result_gate_kind_uses_ask_mode() {
    let route = route_result_with_mode(crate::AskMode::respond_trace());

    assert_eq!(route.gate_kind(), crate::RouteGateKind::Chat);
    assert!(route.is_chat_gate());
    assert!(!route.is_execute_gate());
}

#[test]
fn route_result_set_execute_gate_updates_structured_trace_label() {
    let mut route = route_result_with_mode(crate::AskMode::respond_trace());

    route.set_execute_gate();

    assert_eq!(route.gate_kind(), crate::RouteGateKind::Execute);
    assert_eq!(route.route_trace_label_for_log(), "act_plain_finalizer");
    assert!(route.is_execute_gate());
}

#[test]
fn route_result_exposes_chat_wrapped_planner_mode_as_structured_state() {
    let route = route_result_with_mode(crate::AskMode::act_with_chat_finalizer());

    assert!(route.is_execute_gate());
    assert!(route.uses_chat_finalizer());
    assert!(route.uses_pure_chat_agent_loop_submode());
    assert_eq!(route.route_trace_label_for_log(), "act_chat_finalizer");
}

#[test]
fn route_result_legacy_pure_chat_marker_is_exact_machine_token_fallback() {
    let mut route = route_result_with_mode(crate::AskMode::act_plain());

    route.route_reason = "some_reason; mode:pure_chat_agent_loop_submode".to_string();

    assert!(route.has_route_reason_machine_marker("pure_chat_agent_loop_submode"));
    assert!(route.uses_pure_chat_agent_loop_submode());
}

#[test]
fn route_result_output_contract_marker_methods_accept_route_reason_tokens() {
    let mut route = route_result_with_mode(crate::AskMode::act_plain());
    route.route_reason = "contract:file_paths; contract:service_status".to_string();

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(route.output_contract_marker_is(crate::OutputSemanticKind::FilePaths));
    assert!(route.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::DirectoryNames,
        crate::OutputSemanticKind::ServiceStatus,
    ]));
    assert!(route.has_any_output_contract_marker());
    assert!(!route.output_contract_is_unclassified());
}

#[test]
fn route_result_effective_output_contract_uses_route_reason_marker() {
    let mut route = route_result_with_mode(crate::AskMode::act_plain());
    route.route_reason = "contract:workspace_project_summary".to_string();

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert_eq!(
        route.effective_output_contract_semantic_kind(),
        crate::OutputSemanticKind::WorkspaceProjectSummary
    );
    assert_eq!(
        route.effective_output_contract().semantic_kind,
        crate::OutputSemanticKind::WorkspaceProjectSummary
    );
    assert!(!route.output_contract_marker_is(crate::OutputSemanticKind::FilePaths));
    assert!(route.output_contract_marker_is(crate::OutputSemanticKind::WorkspaceProjectSummary));
}

#[test]
fn route_result_explicit_contract_marker_overrides_legacy_raw_semantic() {
    let mut route = route_result_with_mode(crate::AskMode::act_plain());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    route.route_reason = "contract:workspace_project_summary".to_string();

    assert_eq!(
        route.effective_output_contract_semantic_kind(),
        crate::OutputSemanticKind::WorkspaceProjectSummary
    );
    assert_eq!(
        route.effective_output_contract().semantic_kind,
        crate::OutputSemanticKind::WorkspaceProjectSummary
    );
}
