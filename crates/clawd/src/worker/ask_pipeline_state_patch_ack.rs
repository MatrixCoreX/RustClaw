use serde_json::{json, Value};

fn alias_only_state_patch_bindings(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Vec<crate::conversation_state::SessionAliasBinding> {
    let Some(state_patch) = turn_analysis.and_then(|analysis| analysis.state_patch.as_ref()) else {
        return Vec::new();
    };
    if !crate::conversation_state::state_patch_is_alias_bindings_only(state_patch) {
        return Vec::new();
    }
    crate::conversation_state::session_alias_bindings_from_state_patch(Some(state_patch))
}

pub(super) fn apply_alias_state_patch_ack_route(
    route_result: &mut crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    boundary_envelope: Option<&crate::intent_router::BoundaryEnvelope>,
) {
    let bindings = alias_only_state_patch_bindings(turn_analysis);
    if bindings.is_empty() {
        return;
    }
    if !route_allows_alias_state_patch_ack(route_result, boundary_envelope, true) {
        return;
    }
    route_result.needs_clarify = false;
    route_result.clarify_question.clear();
    route_result.wants_file_delivery = false;
    route_result.output_contract = crate::IntentOutputContract::default();
    route_result.set_ask_mode(crate::AskMode::state_patch_ack());
    if !route_result.has_route_reason_machine_marker("alias_state_patch_ack") {
        if !route_result.route_reason.trim().is_empty() {
            route_result.route_reason.push_str("; ");
        }
        route_result.route_reason.push_str("alias_state_patch_ack");
    }
}

pub(super) fn alias_state_patch_ack_reply(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    boundary_envelope: Option<&crate::intent_router::BoundaryEnvelope>,
) -> Option<crate::AskReply> {
    let bindings = alias_only_state_patch_bindings(turn_analysis);
    if bindings.is_empty() {
        return None;
    }
    if !route_allows_alias_state_patch_ack(route_result, boundary_envelope, true) {
        return None;
    }
    let prior_aliases = crate::conversation_state::load_active_session_snapshot(state, task)
        .conversation_state
        .map(|state| state.alias_bindings)
        .unwrap_or_default();
    let is_update = bindings.iter().any(|binding| {
        prior_aliases
            .iter()
            .any(|prior| prior.alias.eq_ignore_ascii_case(&binding.alias))
    });
    let (message_key, reason_code) = if is_update {
        ("clawd.msg.memory.alias_updated", "memory_alias_updated")
    } else {
        (
            "clawd.msg.memory.alias_remembered",
            "memory_alias_remembered",
        )
    };
    let payload = alias_state_patch_ack_payload(message_key, reason_code, &bindings);
    let machine_default = payload.to_string();
    let language_hint = alias_state_patch_ack_language_hint(state, task, prompt, boundary_envelope);
    let text = crate::i18n_t_for_language_hint_with_default_vars(
        state,
        &language_hint,
        message_key,
        &machine_default,
        &[],
    );
    Some(crate::AskReply::non_llm(text))
}

pub(super) fn session_binding_value_reply(
    route_result: &crate::RouteResult,
    boundary_envelope: Option<&crate::intent_router::BoundaryEnvelope>,
) -> Option<crate::AskReply> {
    if !route_allows_alias_state_patch_ack(route_result, boundary_envelope, false) {
        return None;
    }
    let value = boundary_envelope
        .and_then(|envelope| envelope.session_binding.as_deref())
        .map(str::trim)
        .filter(|value| session_binding_direct_value_is_displayable(value))?;
    Some(crate::AskReply::non_llm(value.to_string()))
}

fn session_binding_direct_value_is_displayable(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    !normalized.is_empty()
        && !matches!(
            normalized.as_str(),
            "resume"
                | "resume_execute"
                | "resume_discuss"
                | "defer"
                | "reuse_active"
                | "replace_active"
                | "pause_and_queue"
                | "standalone"
                | "continuing"
                | "continue"
                | "carryover"
                | "carryover_unresolved_target"
        )
}

fn alias_state_patch_ack_language_hint(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    boundary_envelope: Option<&crate::intent_router::BoundaryEnvelope>,
) -> String {
    boundary_envelope
        .and_then(|envelope| envelope.language_hint.as_deref())
        .map(str::trim)
        .filter(|hint| !hint.is_empty() && *hint != "config_default")
        .map(str::to_string)
        .unwrap_or_else(|| crate::language_policy::task_response_language_hint(state, task, prompt))
}

fn alias_state_patch_ack_payload(
    message_key: &str,
    reason_code: &str,
    bindings: &[crate::conversation_state::SessionAliasBinding],
) -> Value {
    json!({
        "schema_version": 1,
        "status": "ok",
        "message_key": message_key,
        "reason_code": reason_code,
        "binding_count": bindings.len(),
        "bindings": bindings
            .iter()
            .map(|binding| json!({
                "alias": binding.alias.as_str(),
                "target": binding.target.as_str(),
            }))
            .collect::<Vec<_>>(),
    })
}

fn route_allows_alias_state_patch_ack(
    route_result: &crate::RouteResult,
    _boundary_envelope: Option<&crate::intent_router::BoundaryEnvelope>,
    has_alias_only_state_patch: bool,
) -> bool {
    let explicit_alias_ack_marker = has_alias_only_state_patch
        && route_result.has_route_reason_machine_marker("alias_state_patch_ack");
    route_result.schedule_kind == crate::ScheduleKind::None
        && !route_result.needs_clarify
        && !route_result.wants_file_delivery
        && (explicit_alias_ack_marker
            || !route_result
                .has_route_reason_machine_marker("executable_contract_preserved_for_agent_loop"))
        && !route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && route_result.output_contract.delivery_intent == crate::OutputDeliveryIntent::None
}

#[cfg(test)]
#[path = "ask_pipeline_state_patch_ack_tests.rs"]
mod tests;
