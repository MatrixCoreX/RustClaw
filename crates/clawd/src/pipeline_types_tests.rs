use super::{plan_step_from_agent_action, AgentAction, IntentOutputContract, PlanStep};
use serde_json::json;

fn output_contract() -> IntentOutputContract {
    IntentOutputContract::default()
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
fn output_contract_exact_path_list_uses_structured_selector() {
    let mut contract = output_contract();
    contract.response_shape = crate::OutputResponseShape::Strict;
    contract.selection.structured_field_selector = Some("path".to_string());

    assert!(contract.requests_exact_path_list());
    assert!(contract.requests_exact_list());
    assert!(contract.does_not_request_exact_command_output());
}

#[test]
fn output_contract_path_selector_must_be_a_single_strict_field() {
    let mut contract = output_contract();
    contract.selection.structured_field_selector = Some("path".to_string());
    assert!(!contract.requests_exact_path_list());

    contract.response_shape = crate::OutputResponseShape::Strict;
    contract.selection.structured_field_selector = Some("path,resolved_path".to_string());
    assert!(!contract.requests_exact_path_list());
}

#[test]
fn single_file_delivery_requires_all_machine_fields() {
    let mut contract = output_contract();
    contract.delivery_required = true;
    contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    contract.response_shape = crate::OutputResponseShape::FileToken;
    assert!(contract.requests_single_file_delivery());

    contract.delivery_required = false;
    assert!(!contract.requests_single_file_delivery());
    contract.delivery_required = true;
    contract.delivery_intent = crate::OutputDeliveryIntent::None;
    assert!(!contract.requests_single_file_delivery());
    contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    contract.response_shape = crate::OutputResponseShape::Free;
    assert!(!contract.requests_single_file_delivery());
}

#[test]
fn default_output_contract_does_not_request_exact_command_output() {
    let contract = output_contract();

    assert!(contract.does_not_request_exact_command_output());
}

#[test]
fn machine_token_markers_read_values_with_shared_tokenization() {
    let markers = crate::MachineTokenMarkers::new(
        "first_token, clarify_reason_code:missing_locator | clarify_reason_code=missing_target",
    );

    assert!(markers.has_machine_marker("first_token"));
    assert_eq!(
        markers.machine_value("clarify_reason_code"),
        Some("missing_target")
    );
}
