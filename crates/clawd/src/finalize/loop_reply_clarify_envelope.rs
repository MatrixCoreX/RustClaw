use crate::agent_engine::{AgentRunContext, LoopState};
use crate::ClaimedTask;
use serde_json::{json, Value};

use super::log_deterministic_delivery_record;
use super::route_helpers::route_output_contract_machine_json;

const TERMINAL_CLARIFY_STATUS: &str = "needs_clarification";

pub(super) fn attach_agent_loop_clarify_machine_line(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    delivery_messages: &mut Vec<String>,
    _finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    if !agent_loop_nonblocking_clarify_answer(loop_state) {
        return false;
    }
    if completed_act_delivery_should_own_terminal_state(loop_state, delivery_messages) {
        return false;
    }
    if delivery_messages
        .iter()
        .any(|message| delivery_has_terminal_clarify_machine_fields(message))
    {
        return false;
    }

    let Some(machine_line) = agent_loop_clarify_machine_line(loop_state) else {
        return false;
    };
    if !has_agent_loop_clarify_machine_line_observation(loop_state) {
        loop_state
            .task_observations
            .push(agent_loop_clarify_machine_line_observation(
                loop_state,
                &machine_line,
            ));
    }
    log_deterministic_delivery_record(
        &task.task_id,
        "agent_loop_clarify_machine_line",
        "structured_only",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

pub(super) fn attach_route_clarify_machine_envelope(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    let agent_loop_terminal_clarify = loop_state_agent_loop_terminal_clarify(loop_state);
    if !agent_loop_terminal_clarify {
        return false;
    }
    if !route_allows_terminal_clarify_envelope(route, loop_state) {
        return false;
    }
    if completed_act_delivery_should_own_terminal_state(loop_state, delivery_messages) {
        return false;
    }
    let strict_machine_delivery = strict_clarify_machine_delivery_requested(agent_run_context);
    let has_terminal_clarify_delivery = delivery_messages
        .iter()
        .any(|message| delivery_has_terminal_clarify_machine_fields(message));
    if has_terminal_clarify_delivery && !strict_machine_delivery {
        mark_terminal_clarify(loop_state, finalizer_summary);
        return false;
    }
    if strict_machine_delivery
        && delivery_messages
            .iter()
            .any(|message| canonical_terminal_clarify_envelope(message))
    {
        mark_terminal_clarify(loop_state, finalizer_summary);
        return false;
    }

    let reason_code = output_var(loop_state, "agent_loop.clarify_reason_code")
        .unwrap_or_else(|| derived_missing_reason_code(route));
    let missing_slot = output_var(loop_state, "agent_loop.missing_slot")
        .unwrap_or_else(|| missing_slot_for_reason_code(&reason_code).to_string());
    let field_path = output_var(loop_state, "agent_loop.field_path")
        .or_else(|| derived_missing_field_path(&missing_slot).map(str::to_string));
    let locator_kind = output_var(loop_state, "agent_loop.locator_kind")
        .unwrap_or_else(|| route.locator_kind.as_str().to_string());
    let message_key = output_var(loop_state, "agent_loop.message_key")
        .unwrap_or_else(|| message_key_for_missing_slot(&missing_slot).to_string());

    ensure_output_var(loop_state, "agent_loop.terminal_intent", "clarify");
    ensure_output_var(loop_state, "agent_loop.clarify_reason_code", &reason_code);
    ensure_output_var(loop_state, "agent_loop.missing_slot", &missing_slot);
    ensure_output_var(loop_state, "agent_loop.message_key", &message_key);
    ensure_output_var(loop_state, "agent_loop.locator_kind", &locator_kind);
    if let Some(field_path) = field_path.as_deref() {
        ensure_output_var(loop_state, "agent_loop.field_path", field_path);
    }
    mark_terminal_clarify(loop_state, finalizer_summary);
    if !strict_machine_delivery && !clarify_machine_delivery_requested(agent_run_context) {
        return false;
    }

    let envelope = serde_json::json!({
        "status": TERMINAL_CLARIFY_STATUS,
        "output_format": "machine_json",
        "owner_layer": "agent_loop_clarify",
        "terminal_intent": "clarify",
        "message_key": &message_key,
        "clarify_reason_code": &reason_code,
        "missing_slot": &missing_slot,
        "field_path": &field_path,
        "locator_kind": &locator_kind,
        "clarify": {
            "needs_clarify": true,
            "clarify_reason_code": &reason_code,
            "missing_slot": &missing_slot,
            "field_path": &field_path,
            "locator_kind": &locator_kind
        },
        "output_contract": route_output_contract_machine_json(route)
    })
    .to_string();
    if strict_machine_delivery {
        delivery_messages.clear();
        delivery_messages.push(envelope.clone());
        loop_state.delivery_messages.clear();
        loop_state.delivery_messages.push(envelope.clone());
        loop_state.last_user_visible_respond = Some(envelope);
    } else {
        delivery_messages.push(envelope);
    }
    log_deterministic_delivery_record(
        &task.task_id,
        "agent_loop_clarify_machine_envelope",
        if strict_machine_delivery {
            "replaced"
        } else {
            "attached"
        },
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn clarify_machine_delivery_requested(agent_run_context: Option<&AgentRunContext>) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.turn_analysis.as_ref())
        .and_then(|analysis| analysis.state_patch.as_ref())
        .and_then(|state_patch| state_patch.get("required_machine_fields"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(is_clarify_machine_field)
}

fn strict_clarify_machine_delivery_requested(agent_run_context: Option<&AgentRunContext>) -> bool {
    agent_run_context
        .and_then(AgentRunContext::output_contract)
        .is_some_and(|route| route.response_shape == crate::OutputResponseShape::Strict)
}

fn is_clarify_machine_field(raw: &str) -> bool {
    let field = raw
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | ';' | ':' | ')' | '('));
    matches!(
        field,
        "clarify"
            | "agent_loop_clarify"
            | "agent_loop.clarify"
            | "clarify_reason_code"
            | "agent_loop.clarify_reason_code"
            | "missing_slot"
            | "agent_loop.missing_slot"
            | "field_path"
            | "agent_loop.field_path"
            | "locator_kind"
            | "agent_loop.locator_kind"
            | "message_key"
            | "agent_loop.message_key"
    )
}

fn route_allows_terminal_clarify_envelope(
    _route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> bool {
    loop_state.pending_user_input_required
}

fn loop_state_agent_loop_terminal_clarify(loop_state: &LoopState) -> bool {
    output_var(loop_state, "agent_loop.terminal_intent").as_deref() == Some("clarify")
}

fn agent_loop_nonblocking_clarify_answer(loop_state: &LoopState) -> bool {
    output_var(loop_state, "agent_loop.nonblocking_clarify_answer").as_deref() == Some("true")
        || (output_var(loop_state, "agent_loop.recovered_terminal_intent").as_deref()
            == Some("clarify")
            && output_var(loop_state, "agent_loop.terminal_intent").as_deref() == Some("answer"))
}

fn completed_act_delivery_should_own_terminal_state(
    loop_state: &LoopState,
    delivery_messages: &[String],
) -> bool {
    !loop_state.pending_user_input_required
        && loop_state
            .output_vars
            .contains_key("agent_loop.first_act_decision_envelope")
        && delivery_messages
            .iter()
            .any(|message| !delivery_has_terminal_clarify_machine_fields(message))
}

fn machine_text_has_token(machine_text: &str, expected: &str) -> bool {
    crate::MachineTokenMarkers::new(machine_text).has_machine_marker(expected)
}

pub(super) fn delivery_has_terminal_clarify_machine_fields(message: &str) -> bool {
    if let Ok(payload) = serde_json::from_str::<serde_json::Value>(message.trim()) {
        if payload
            .get("owner_layer")
            .and_then(serde_json::Value::as_str)
            == Some("agent_loop_clarify")
        {
            return true;
        }
        if payload
            .get("terminal_intent")
            .and_then(serde_json::Value::as_str)
            == Some("clarify")
        {
            return true;
        }
    }
    let markers = crate::MachineTokenMarkers::new(message);
    machine_text_has_token(message, "agent_loop.terminal_intent=clarify")
        || markers.machine_value("agent_loop.terminal_intent") == Some("clarify")
        || markers.machine_value("terminal_intent") == Some("clarify")
}

fn canonical_terminal_clarify_envelope(message: &str) -> bool {
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(message.trim()) else {
        return false;
    };
    payload.get("status").and_then(serde_json::Value::as_str) == Some(TERMINAL_CLARIFY_STATUS)
        && payload
            .get("output_format")
            .and_then(serde_json::Value::as_str)
            == Some("machine_json")
        && payload
            .get("owner_layer")
            .and_then(serde_json::Value::as_str)
            == Some("agent_loop_clarify")
        && payload
            .get("terminal_intent")
            .and_then(serde_json::Value::as_str)
            == Some("clarify")
}

fn output_var(loop_state: &LoopState, key: &str) -> Option<String> {
    loop_state
        .output_vars
        .get(key)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn output_var_or_decision_envelope(loop_state: &LoopState, field: &str) -> Option<String> {
    let mut decision_field_key = String::from("agent_loop.decision_envelope.");
    decision_field_key.push_str(field);
    output_var(loop_state, &format!("agent_loop.{field}"))
        .or_else(|| output_var(loop_state, &decision_field_key))
        .or_else(|| {
            loop_state
                .output_vars
                .get("agent_loop.decision_envelope")
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                .and_then(|payload| {
                    payload
                        .get(field)
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
}

fn agent_loop_clarify_machine_line(loop_state: &LoopState) -> Option<String> {
    let mut parts = vec!["terminal_intent=clarify".to_string()];
    for (key, value) in [
        (
            "clarify_reason_code",
            output_var_or_decision_envelope(loop_state, "clarify_reason_code"),
        ),
        (
            "missing_slot",
            output_var_or_decision_envelope(loop_state, "missing_slot"),
        ),
        (
            "message_key",
            output_var_or_decision_envelope(loop_state, "message_key"),
        ),
        (
            "field_path",
            output_var_or_decision_envelope(loop_state, "field_path"),
        ),
        (
            "locator_kind",
            output_var_or_decision_envelope(loop_state, "locator_kind"),
        ),
    ] {
        if let Some(value) = value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            parts.push(format!("{key}={value}"));
        }
    }
    (parts.len() > 1).then(|| parts.join(" "))
}

fn has_agent_loop_clarify_machine_line_observation(loop_state: &LoopState) -> bool {
    loop_state.task_observations.iter().any(|observation| {
        observation
            .get("kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind == "terminal_clarify_machine_line")
            && observation
                .get("owner_layer")
                .and_then(Value::as_str)
                .is_some_and(|owner| owner == "agent_loop")
    })
}

fn agent_loop_clarify_machine_line_observation(
    loop_state: &LoopState,
    machine_line: &str,
) -> Value {
    let mut observation = json!({
        "kind": "terminal_clarify_machine_line",
        "owner_layer": "agent_loop",
        "schema_version": 1,
        "terminal_intent": "clarify",
        "machine_line": machine_line
    });
    let Some(object) = observation.as_object_mut() else {
        return observation;
    };
    for field in [
        "clarify_reason_code",
        "missing_slot",
        "message_key",
        "field_path",
        "locator_kind",
    ] {
        if let Some(value) = output_var_or_decision_envelope(loop_state, field) {
            object.insert(field.to_string(), json!(value));
        }
    }
    observation
}

fn ensure_output_var(loop_state: &mut LoopState, key: &str, value: &str) {
    loop_state
        .output_vars
        .entry(key.to_string())
        .or_insert_with(|| value.to_string());
}

fn derived_missing_reason_code(route: &crate::IntentOutputContract) -> String {
    if route.locator_hint.trim().is_empty()
        && (route.requires_content_evidence || route.delivery_required)
    {
        "missing_locator".to_string()
    } else {
        "missing_input".to_string()
    }
}

fn missing_slot_for_reason_code(reason_code: &str) -> &'static str {
    match reason_code {
        "missing_count_target" => "target",
        "missing_delivery_locator"
        | "missing_file_locator"
        | "missing_locator"
        | "missing_read_target"
        | "missing_search_locator"
        | "missing_service_target"
        | "missing_target" => "locator",
        _ => "input",
    }
}

fn derived_missing_field_path(missing_slot: &str) -> Option<&'static str> {
    match missing_slot {
        "locator" => Some("output_contract.locator_hint"),
        "target" => Some("resolved_intent.target"),
        _ => None,
    }
}

fn message_key_for_missing_slot(missing_slot: &str) -> &'static str {
    match missing_slot {
        "locator" => "clawd.clarify.missing_locator",
        "target" => "clawd.clarify.missing_target",
        _ => "clawd.clarify.missing_input",
    }
}

fn mark_terminal_clarify(
    loop_state: &mut LoopState,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) {
    loop_state.pending_user_input_required = true;
    let summary = finalizer_summary.get_or_insert_with(Default::default);
    summary.stage = Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric);
    summary.disposition = Some(crate::finalize::FinalizerDisposition::AllowFallback);
    summary.parsed = true;
    summary.format_ok = Some(true);
    summary.needs_clarify = Some(true);
}
