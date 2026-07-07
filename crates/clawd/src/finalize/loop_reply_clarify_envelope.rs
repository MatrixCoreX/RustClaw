use crate::agent_engine::{AgentRunContext, LoopState};
use crate::ClaimedTask;
use serde_json::Value;

use super::log_deterministic_delivery_record;
use super::route_helpers::{route_clarify_reason_code, route_output_contract_machine_json};

pub(super) fn attach_route_clarify_machine_envelope(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .filter(|route| route.needs_clarify)
        .filter(|route| route_allows_terminal_clarify_envelope(route, loop_state))
    else {
        return false;
    };
    if completed_act_delivery_should_own_terminal_state(loop_state, delivery_messages) {
        return false;
    }
    if delivery_messages
        .iter()
        .any(|message| delivery_has_terminal_clarify_machine_fields(message))
    {
        mark_terminal_clarify(loop_state, finalizer_summary);
        return false;
    }

    let reason_code = output_var(loop_state, "agent_loop.clarify_reason_code")
        .or_else(|| route_clarify_reason_code(&route.route_reason).map(str::to_string))
        .unwrap_or_else(|| derived_missing_reason_code(route));
    let missing_slot = output_var(loop_state, "agent_loop.missing_slot")
        .unwrap_or_else(|| missing_slot_for_reason_code(&reason_code).to_string());
    let field_path = output_var(loop_state, "agent_loop.field_path")
        .or_else(|| derived_missing_field_path(&missing_slot).map(str::to_string));
    let locator_kind = output_var(loop_state, "agent_loop.locator_kind")
        .unwrap_or_else(|| route.output_contract.locator_kind.as_str().to_string());
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
    if !clarify_machine_delivery_requested(agent_run_context) {
        return false;
    }

    let envelope = serde_json::json!({
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
    delivery_messages.push(envelope);
    log_deterministic_delivery_record(
        &task.task_id,
        "agent_loop_clarify_machine_envelope",
        "attached",
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
    _route: &crate::RouteResult,
    loop_state: &LoopState,
) -> bool {
    loop_state.pending_user_input_required
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

fn route_reason_has_machine_token(route_reason: &str, expected: &str) -> bool {
    route_reason
        .split(|ch: char| {
            ch.is_whitespace() || matches!(ch, ';' | ',' | '|' | '[' | ']' | '(' | ')')
        })
        .any(|token| token.trim() == expected)
}

fn delivery_has_terminal_clarify_machine_fields(message: &str) -> bool {
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
    route_reason_has_machine_token(message, "agent_loop.terminal_intent=clarify")
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

fn ensure_output_var(loop_state: &mut LoopState, key: &str, value: &str) {
    loop_state
        .output_vars
        .entry(key.to_string())
        .or_insert_with(|| value.to_string());
}

fn derived_missing_reason_code(route: &crate::RouteResult) -> String {
    if route.output_contract.locator_hint.trim().is_empty()
        && (route.output_contract.requires_content_evidence
            || route.output_contract.delivery_required)
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
