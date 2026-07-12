use super::{
    plan_step_from_agent_action, AgentAction, AskMode, IntentOutputContract, OutputContractRef,
    PlanStep, ResumeBehavior, RiskCeiling, RouteResult, ScheduleKind,
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
fn route_reason_marker_facade_parses_machine_tokens_without_call_site_splitting() {
    let markers = crate::RouteReasonMarkers::new(
        "normalizer_hint; contract:scalar_count; mode:pure_chat_agent_loop_submode",
    );

    assert!(markers.has_machine_marker("pure_chat_agent_loop_submode"));
    assert!(markers.has_any_machine_marker(&["workspace_project_summary", "scalar_count",]));
    assert_eq!(
        markers.explicit_output_contract_marker_kind(),
        Some(crate::OutputSemanticKind::ScalarCount)
    );
}

#[test]
fn route_reason_marker_facade_parses_explicit_output_contract_kind() {
    let markers = crate::RouteReasonMarkers::new("output_contract_kind=scalar_count");

    assert_eq!(
        markers.explicit_output_contract_marker_kind(),
        Some(crate::OutputSemanticKind::ScalarCount)
    );
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
fn output_contract_ref_wraps_effective_contract_kind() {
    let mut route = route_result_with_mode(crate::AskMode::act_plain());
    route.route_reason = "contract:workspace_project_summary".to_string();
    let contract_ref = route.effective_output_contract_ref();

    assert_eq!(
        contract_ref.semantic_kind(),
        crate::OutputSemanticKind::WorkspaceProjectSummary
    );
}

#[test]
fn output_contract_ref_named_constructors_apply_to_contract() {
    let mut contract = IntentOutputContract::default();

    contract.apply_output_contract_ref(OutputContractRef::workspace_project_summary());
    assert_eq!(
        contract.semantic_kind,
        crate::OutputSemanticKind::WorkspaceProjectSummary
    );

    contract.apply_output_contract_ref(OutputContractRef::file_paths());
    assert_eq!(contract.semantic_kind, crate::OutputSemanticKind::FilePaths);
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
