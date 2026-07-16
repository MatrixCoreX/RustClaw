use serde_json::{json, Value};

use super::{TaskJournal, TaskJournalRoundTrace};

fn route_for_round_envelope() -> crate::RouteResult {
    crate::RouteResult {
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::None,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            ..Default::default()
        },
    }
}

#[test]
fn trace_json_includes_round_decision_envelope() {
    let route = route_for_round_envelope();
    let plan = crate::PlanResult {
        goal: "read a field".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        output_contract: Some(route.output_contract.clone()),
        steps: vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "call_capability".to_string(),
            skill: "fs.read_text_range".to_string(),
            args: json!({"path": "README.md"}),
            depends_on: Vec::new(),
            why: String::new(),
        }],
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: String::new(),
    };
    let mut journal = TaskJournal::for_task("task-round-envelope", "ask", "prompt");
    journal.record_output_contract(&route.effective_output_contract());
    journal.rounds.push(TaskJournalRoundTrace {
        round_no: 2,
        goal: "read a field".to_string(),
        plan_result: Some(plan),
        ..Default::default()
    });

    let trace = journal.to_trace_json();
    assert_eq!(
        trace
            .pointer("/rounds/0/decision_envelope/schema_version")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        trace
            .pointer("/rounds/0/decision_envelope/source")
            .and_then(Value::as_str),
        Some("planner_round_action")
    );
    assert!(trace
        .pointer("/rounds/0/decision_envelope/initial_gate_ref")
        .is_none());
    assert!(trace
        .pointer("/rounds/0/decision_envelope/initial_hint_ref")
        .is_none());
    assert!(trace
        .pointer("/rounds/0/decision_envelope/fallback_gate_policy")
        .is_none());
    assert_eq!(
        trace
            .pointer("/rounds/0/decision_envelope/semantic_authority")
            .and_then(Value::as_str),
        Some("planner_loop_runtime")
    );
    assert_eq!(
        trace
            .pointer("/rounds/0/decision_envelope/decision")
            .and_then(Value::as_str),
        Some("call_capability")
    );
    assert_eq!(
        trace
            .pointer("/rounds/0/decision_envelope/terminal_intent")
            .and_then(Value::as_str),
        Some("continue")
    );
    assert_eq!(
        trace
            .pointer("/rounds/0/decision_envelope/control_intent")
            .and_then(Value::as_str),
        Some("act")
    );
    assert_eq!(
        trace
            .pointer("/rounds/0/decision_envelope/control_reason_code")
            .and_then(Value::as_str),
        Some("agent_loop_control_act_first_action")
    );
    assert_eq!(
        trace
            .pointer("/rounds/0/decision_envelope/capability_ref")
            .and_then(Value::as_str),
        Some("fs.read_text_range")
    );
    assert_eq!(
        trace
            .pointer("/rounds/0/decision_envelope/evidence_needed")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );
    assert_eq!(
        trace
            .pointer("/rounds/0/decision_envelope/answer_shape")
            .and_then(Value::as_str),
        Some("free")
    );
}

#[test]
fn output_contract_ref_uses_evidence_policy_shape() {
    let plan = crate::PlanResult {
        goal: "summarize workspace".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        output_contract: Some(crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::WorkspaceProjectSummary,
            ..Default::default()
        }),
        steps: Vec::new(),
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: String::new(),
    };
    let envelope = super::decision_envelope::agent_loop_round_plan_contract_envelope_json(&plan);
    let output_contract_ref = envelope
        .get("output_contract_ref")
        .and_then(Value::as_str)
        .expect("output contract ref");

    assert!(output_contract_ref.contains("final_answer_shape=project_summary_grounded_in_files"));
    assert!(output_contract_ref.contains("final_answer_shape_class=grounded_summary"));
}

#[test]
fn planner_contract_envelope_uses_plan_slots_and_contract_evidence_without_route() {
    let plan = crate::PlanResult {
        goal: "count matching entries".to_string(),
        missing_slots: vec!["root".to_string()],
        needs_confirmation: false,
        output_contract: Some(crate::IntentOutputContract {
            response_shape: crate::OutputResponseShape::Scalar,
            requires_content_evidence: true,
            semantic_kind: crate::OutputSemanticKind::ScalarCount,
            ..Default::default()
        }),
        steps: vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "call_capability".to_string(),
            skill: "filesystem.count_entries".to_string(),
            args: json!({}),
            depends_on: Vec::new(),
            why: String::new(),
        }],
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: String::new(),
    };

    let envelope = super::decision_envelope::agent_loop_round_plan_contract_envelope_json(&plan);

    assert_eq!(
        envelope.get("missing_slot").and_then(Value::as_str),
        Some("root")
    );
    assert_eq!(
        envelope.get("risk_level").and_then(Value::as_str),
        Some("unknown")
    );
    assert_eq!(
        envelope.get("capability_ref").and_then(Value::as_str),
        Some("filesystem.count_entries")
    );
    assert!(envelope
        .get("required_evidence")
        .and_then(Value::as_array)
        .is_some_and(|fields| fields.iter().any(|field| field.as_str() == Some("count"))));
}

#[test]
fn agent_loop_decision_envelope_schema_accepts_round_runtime_source() {
    const SCHEMA_RAW: &str =
        include_str!("../../../prompts/schemas/agent_loop_decision_envelope.schema.json");
    let schema: Value =
        serde_json::from_str(SCHEMA_RAW).expect("agent_loop_decision_envelope schema json");
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("schema properties");
    let sources = properties
        .get("source")
        .and_then(|value| value.get("enum"))
        .and_then(Value::as_array)
        .expect("source enum");
    assert!(sources
        .iter()
        .any(|value| value.as_str() == Some("planner_first_action_shadow")));
    assert!(sources
        .iter()
        .any(|value| value.as_str() == Some("planner_round_action")));
    let semantic_authorities = properties
        .get("semantic_authority")
        .and_then(|value| value.get("enum"))
        .and_then(Value::as_array)
        .expect("semantic authority enum");
    assert!(semantic_authorities
        .iter()
        .any(|value| value.as_str() == Some("planner_loop_shadow")));
    assert!(semantic_authorities
        .iter()
        .any(|value| value.as_str() == Some("planner_loop_runtime")));
}
