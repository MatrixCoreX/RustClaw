use serde_json::{json, Map, Value};

use crate::agent_engine::{AgentRunContext, LoopState};
use crate::ClaimedTask;

use super::log_deterministic_delivery_record;

pub(super) fn attach_requested_control_machine_envelope(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(ctx) = agent_run_context else {
        return false;
    };
    if delivery_messages
        .iter()
        .any(|message| delivery_has_control_machine_envelope(message))
    {
        return false;
    }
    let requested_fields = requested_machine_fields(ctx);
    if requested_fields.is_empty() {
        return false;
    }
    let Some(decision_envelope) = decision_envelope_for_delivery(loop_state, &requested_fields)
    else {
        return false;
    };
    let projected = projected_decision_envelope(&decision_envelope, &requested_fields);
    if projected.is_empty()
        && !requested_fields
            .iter()
            .any(|field| field == "decision_envelope")
    {
        return false;
    }
    let envelope = json!({
        "output_format": "machine_json",
        "owner_layer": "agent_loop_control",
        "required_machine_fields": requested_fields,
        "decision_envelope": if projected.is_empty() {
            decision_envelope
        } else {
            Value::Object(projected)
        },
        "output_contract": loop_state.output_contract.as_ref().map(output_contract_machine_json)
    })
    .to_string();
    delivery_messages.push(envelope.clone());
    loop_state.last_user_visible_respond = Some(envelope);
    mark_control_envelope_complete(loop_state, finalizer_summary);
    log_deterministic_delivery_record(
        &task.task_id,
        "agent_loop_control_machine_envelope",
        "attached",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn requested_machine_fields(ctx: &AgentRunContext) -> Vec<String> {
    ctx.turn_analysis
        .as_ref()
        .and_then(|analysis| analysis.state_patch.as_ref())
        .and_then(|state_patch| state_patch.get("required_machine_fields"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(normalize_required_machine_field)
        .fold(Vec::<String>::new(), push_unique)
}

fn normalize_required_machine_field(raw: &str) -> Option<String> {
    let field = raw
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | ';' | ':' | ')' | '('));
    if field.is_empty() {
        return None;
    }
    let normalized = match field {
        "decision_envelope" => "decision_envelope",
        "control_intent" | "agent_loop.control_intent" | "decision_envelope.control_intent" => {
            "decision_envelope.control_intent"
        }
        "terminal_intent" | "agent_loop.terminal_intent" | "decision_envelope.terminal_intent" => {
            "decision_envelope.terminal_intent"
        }
        "decision" | "agent_loop.decision" | "decision_envelope.decision" => {
            "decision_envelope.decision"
        }
        "capability_ref" | "agent_loop.capability_ref" | "decision_envelope.capability_ref" => {
            "decision_envelope.capability_ref"
        }
        "control_reason_code"
        | "agent_loop.control_reason_code"
        | "decision_envelope.control_reason_code" => "decision_envelope.control_reason_code",
        "reason_code" | "agent_loop.reason_code" | "decision_envelope.reason_code" => {
            "decision_envelope.reason_code"
        }
        "validation_status"
        | "agent_loop.validation_status"
        | "decision_envelope.validation_status" => "decision_envelope.validation_status",
        "validation_reason_code"
        | "agent_loop.validation_reason_code"
        | "decision_envelope.validation_reason_code" => "decision_envelope.validation_reason_code",
        "semantic_authority"
        | "agent_loop.semantic_authority"
        | "decision_envelope.semantic_authority" => "decision_envelope.semantic_authority",
        _ => return None,
    };
    Some(normalized.to_string())
}

fn push_unique(mut fields: Vec<String>, field: String) -> Vec<String> {
    if !fields.iter().any(|existing| existing == &field) {
        fields.push(field);
    }
    fields
}

fn decision_envelope_for_delivery(
    loop_state: &LoopState,
    requested_fields: &[String],
) -> Option<Value> {
    let preferred_key = requested_fields
        .iter()
        .any(|field| field == "decision_envelope.control_intent")
        .then_some("agent_loop.first_act_decision_envelope")
        .unwrap_or("agent_loop.decision_envelope");
    loop_state
        .output_vars
        .get(preferred_key)
        .or_else(|| loop_state.output_vars.get("agent_loop.decision_envelope"))
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .or_else(|| {
            let plan = loop_state
                .round_traces
                .iter()
                .rev()
                .find_map(|round| round.plan_result.as_ref())?;
            Some(crate::task_journal::agent_loop_round_plan_contract_envelope(plan))
        })
}

fn output_contract_machine_json(contract: &crate::IntentOutputContract) -> Value {
    json!({
        "response_shape": contract.response_shape.as_str(),
        "exact_sentence_count": contract.exact_sentence_count,
        "requires_content_evidence": contract.requires_content_evidence,
        "delivery_required": contract.delivery_required,
        "locator_kind": contract.locator_kind.as_str(),
        "delivery_intent": contract.delivery_intent.as_str(),
        "structured_field_selector": contract.selection.structured_field_selector,
        "final_answer_shape": crate::evidence_policy::final_answer_shape_for_output_contract(contract)
            .map(|shape| shape.as_str())
            .unwrap_or_else(|| contract.response_shape.as_str()),
    })
}

fn projected_decision_envelope(
    envelope: &Value,
    requested_fields: &[String],
) -> Map<String, Value> {
    let mut projected = Map::new();
    for field in requested_fields {
        if field == "decision_envelope" {
            return envelope.as_object().cloned().unwrap_or_default();
        }
        let Some(key) = field.strip_prefix("decision_envelope.") else {
            continue;
        };
        if let Some(value) = envelope.get(key) {
            projected.insert(key.to_string(), value.clone());
        }
    }
    projected
}

fn delivery_has_control_machine_envelope(message: &str) -> bool {
    serde_json::from_str::<Value>(message.trim())
        .ok()
        .and_then(|payload| {
            payload
                .get("owner_layer")
                .and_then(Value::as_str)
                .map(|owner| owner == "agent_loop_control")
        })
        .unwrap_or(false)
}

fn mark_control_envelope_complete(
    loop_state: &LoopState,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) {
    let summary = finalizer_summary.get_or_insert_with(Default::default);
    summary.stage = Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric);
    summary.disposition = Some(crate::finalize::FinalizerDisposition::QualifiedCompletion);
    summary.parsed = true;
    summary.contract_ok = true;
    summary.completion_ok = Some(true);
    summary.grounded_ok = Some(true);
    summary.format_ok = Some(true);
    summary.needs_clarify = Some(false);
    summary.used_evidence_ids_count = loop_state.executed_step_results.len().max(1);
}
