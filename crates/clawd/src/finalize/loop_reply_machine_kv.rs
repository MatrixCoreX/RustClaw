use serde_json::Value;

use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::ClaimedTask;

use super::{
    final_answer_text_from_delivery, log_deterministic_delivery_record,
    raw_command_machine_field_delivery_satisfies_request,
};

pub(super) fn replace_delivery_with_requested_machine_kv_summary(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    delivery_messages: &mut Vec<String>,
) -> bool {
    if current_delivery_contains_full_structured_contract(loop_state, delivery_messages) {
        return false;
    }
    let current = normalize_markdown_format_table_delivery(loop_state, delivery_messages);
    if agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            route.output_contract_marker_is(crate::OutputSemanticKind::RawCommandOutput)
                && raw_command_machine_field_delivery_satisfies_request(route, &current)
        })
    {
        loop_state.last_user_visible_respond = Some(current);
        return false;
    }
    let mut observed_texts = Vec::new();
    for step in &loop_state.executed_step_results {
        if !step.is_ok() {
            continue;
        }
        let Some(output) = step.output.as_deref() else {
            continue;
        };
        crate::machine_kv_projection::collect_machine_text_fragments_from_output(
            output,
            &mut observed_texts,
        );
    }
    for message in delivery_messages.iter() {
        crate::machine_kv_projection::collect_machine_text_fragments_from_output(
            message,
            &mut observed_texts,
        );
    }
    for message in &loop_state.delivery_messages {
        crate::machine_kv_projection::collect_machine_text_fragments_from_output(
            message,
            &mut observed_texts,
        );
    }
    for message in [
        loop_state.last_user_visible_respond.as_deref(),
        loop_state.last_publishable_synthesis_output.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        crate::machine_kv_projection::collect_machine_text_fragments_from_output(
            message,
            &mut observed_texts,
        );
    }
    observed_texts.sort();
    observed_texts.dedup();
    let request_surfaces = requested_machine_kv_request_surfaces(user_text, agent_run_context);
    let Some(answer) =
        crate::machine_kv_projection::requested_machine_kv_summary_from_observation_inputs(
            request_surfaces.iter().map(String::as_str),
            &observed_texts,
        )
    else {
        return false;
    };
    let answer_is_service_status_selector =
        service_status_selector_only_summary(agent_run_context, &answer);
    let current_is_service_status_selector =
        service_status_selector_only_summary(agent_run_context, &current);
    if answer_is_service_status_selector || current_is_service_status_selector {
        if let Some(restored) =
            latest_publishable_service_status_terminal_delivery(loop_state, agent_run_context)
        {
            delivery_messages.clear();
            delivery_messages.push(restored.clone());
            loop_state.delivery_messages.clear();
            append_delivery_message(
                &task.task_id,
                &mut loop_state.delivery_messages,
                restored.clone(),
            );
            loop_state.last_user_visible_respond = Some(restored);
            *finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                parsed: true,
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: loop_state.executed_step_results.len(),
                ..Default::default()
            });
            log_deterministic_delivery_record(
                &task.task_id,
                if answer_is_service_status_selector {
                    "requested_machine_kv_summary_service_status_terminal_delivery"
                } else {
                    "requested_machine_kv_summary_service_status_current_selector_terminal_delivery"
                },
                "restored",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
            return true;
        }
        if answer_is_service_status_selector {
            return false;
        }
    }
    if should_restore_config_guard_payload(agent_run_context, &answer) {
        if let Some(payload) = latest_config_guard_machine_payload(loop_state, delivery_messages) {
            delivery_messages.clear();
            delivery_messages.push(payload.clone());
            loop_state.delivery_messages.clear();
            append_delivery_message(
                &task.task_id,
                &mut loop_state.delivery_messages,
                payload.clone(),
            );
            loop_state.last_user_visible_respond = Some(payload);
            *finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                parsed: true,
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: loop_state.executed_step_results.len(),
                ..Default::default()
            });
            log_deterministic_delivery_record(
                &task.task_id,
                "requested_machine_kv_summary_config_guard_payload",
                "restored",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
            return true;
        }
    }
    if service_status_one_sentence_delivery_should_be_preserved(agent_run_context, &current) {
        loop_state.last_user_visible_respond = Some(current);
        return false;
    }
    if delivery_contains_agent_loop_control_envelope(loop_state, delivery_messages) {
        loop_state.last_user_visible_respond = Some(current);
        return false;
    }
    if current_delivery_is_terminal_scalar_answer(agent_run_context, &current) {
        loop_state.last_user_visible_respond = Some(current);
        return false;
    }
    if current_delivery_satisfies_service_status_selector(agent_run_context, &current)
        || current_delivery_is_execution_recipe_closeout(&current)
        || current_delivery_is_structured_json_answer(&current)
    {
        loop_state.last_user_visible_respond = Some(current);
        return false;
    }
    if let Some(answer) =
        latest_terminal_scalar_respond_replacement(agent_run_context, loop_state, &current, &answer)
    {
        delivery_messages.clear();
        delivery_messages.push(answer.clone());
        loop_state.delivery_messages.clear();
        append_delivery_message(
            &task.task_id,
            &mut loop_state.delivery_messages,
            answer.clone(),
        );
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            parsed: true,
            contract_ok: true,
            completion_ok: Some(true),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: loop_state.executed_step_results.len(),
            ..Default::default()
        });
        log_deterministic_delivery_record(
            &task.task_id,
            "requested_machine_kv_summary_terminal_scalar_respond",
            "replaced",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return true;
    }
    if current.trim() == answer {
        loop_state.last_user_visible_respond = Some(answer);
        return true;
    }
    if current_delivery_preserves_web_search_listing(agent_run_context, loop_state, &current) {
        loop_state.last_user_visible_respond = Some(current);
        return false;
    }
    if current_delivery_is_richer_than_requested_machine_summary(
        agent_run_context,
        &current,
        &answer,
    ) {
        loop_state.last_user_visible_respond = Some(current);
        return false;
    }
    delivery_messages.clear();
    delivery_messages.push(answer.clone());
    loop_state.delivery_messages.clear();
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.clone(),
    );
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: loop_state.executed_step_results.len(),
        ..Default::default()
    });
    log_deterministic_delivery_record(
        &task.task_id,
        "requested_machine_kv_summary",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn normalize_markdown_format_table_delivery(
    loop_state: &mut LoopState,
    delivery_messages: &mut Vec<String>,
) -> String {
    let current = final_answer_text_from_delivery(delivery_messages);
    let Some(normalized) = strip_markdown_format_label_table(&current) else {
        return current;
    };
    delivery_messages.clear();
    delivery_messages.push(normalized.clone());
    loop_state.delivery_messages.clear();
    loop_state.delivery_messages.push(normalized.clone());
    loop_state.last_user_visible_respond = Some(normalized.clone());
    normalized
}

fn strip_markdown_format_label_table(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let rest = trimmed
        .strip_prefix("markdown")
        .or_else(|| trimmed.strip_prefix("md"))?;
    if !rest.starts_with('\n') && !rest.starts_with("\r\n") {
        return None;
    }
    let table = rest.trim_start();
    let mut nonempty = table.lines().map(str::trim).filter(|line| !line.is_empty());
    let header = nonempty.next()?;
    let separator = nonempty.next()?;
    if looks_like_markdown_table_row(header) && looks_like_markdown_table_separator(separator) {
        Some(table.trim().to_string())
    } else {
        None
    }
}

fn looks_like_markdown_table_row(line: &str) -> bool {
    let line = line.trim();
    line.starts_with('|') && line.ends_with('|') && line.matches('|').count() >= 3
}

fn looks_like_markdown_table_separator(line: &str) -> bool {
    let line = line.trim();
    if !looks_like_markdown_table_row(line) {
        return false;
    }
    line.trim_matches('|')
        .split('|')
        .all(|cell| cell.trim().chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
}

fn service_status_one_sentence_delivery_should_be_preserved(
    agent_run_context: Option<&AgentRunContext>,
    current: &str,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route_is_service_status_contract(route)
        || route.output_contract.response_shape != crate::OutputResponseShape::OneSentence
    {
        return false;
    }
    let current = current.trim();
    !current.is_empty()
        && !service_status_selector_only_summary(agent_run_context, current)
        && !current.starts_with('{')
        && !current.starts_with('[')
        && !current.contains('\n')
        && !current.contains('=')
}

fn current_delivery_satisfies_service_status_selector(
    agent_run_context: Option<&AgentRunContext>,
    current: &str,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route_is_service_status_contract(route) {
        return false;
    }
    let Some(selector) = route
        .output_contract
        .self_extension
        .structured_field_selector
        .as_deref()
        .map(str::trim)
        .filter(|selector| !selector.is_empty())
    else {
        return false;
    };
    let current = current.trim();
    if current.is_empty() || current == selector {
        return false;
    }
    selector
        .split(',')
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .all(|field| current_delivery_has_selector_field(current, field))
}

fn current_delivery_has_selector_field(current: &str, field: &str) -> bool {
    if let Some(prefix) = field.strip_suffix(".*") {
        let prefix = format!("{prefix}.");
        return current
            .lines()
            .map(str::trim)
            .any(|line| line.starts_with(&prefix) && line.contains('='));
    }
    current.lines().map(str::trim).any(|line| {
        line.strip_prefix(field)
            .is_some_and(|rest| rest.starts_with('=') || rest.starts_with(':'))
    })
}

fn current_delivery_is_execution_recipe_closeout(current: &str) -> bool {
    current
        .lines()
        .map(str::trim)
        .any(|line| line.starts_with("message_key=clawd.msg.execution_recipe_closeout_"))
}

fn current_delivery_is_structured_json_answer(current: &str) -> bool {
    let current = current.trim();
    let Ok(value) = serde_json::from_str::<Value>(current) else {
        return false;
    };
    match value {
        Value::Array(items) => !items.is_empty(),
        Value::Object(object) => {
            object
                .get("message_key")
                .and_then(Value::as_str)
                .is_some_and(|message_key| message_key.starts_with("clawd.msg."))
                || object
                    .values()
                    .filter(|value| value.is_object() || value.is_array())
                    .take(2)
                    .count()
                    >= 2
        }
        _ => false,
    }
}

fn delivery_contains_agent_loop_control_envelope(
    loop_state: &LoopState,
    delivery_messages: &[String],
) -> bool {
    delivery_messages
        .iter()
        .chain(loop_state.delivery_messages.iter())
        .chain(loop_state.last_user_visible_respond.iter())
        .any(|message| {
            serde_json::from_str::<Value>(message.trim())
                .ok()
                .is_some_and(|value| {
                    value.get("owner_layer").and_then(Value::as_str) == Some("agent_loop_control")
                })
        })
}

fn current_delivery_is_terminal_scalar_answer(
    agent_run_context: Option<&AgentRunContext>,
    current: &str,
) -> bool {
    let route = match agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) {
        Some(route)
            if route.output_contract.response_shape == crate::OutputResponseShape::Scalar =>
        {
            route
        }
        _ => return false,
    };
    terminal_scalar_respond_matches_route(route, current)
}

fn latest_terminal_scalar_respond_replacement(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &LoopState,
    current: &str,
    requested_summary: &str,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || route.output_contract.delivery_required
        || !machine_field_placeholder_summary(current)
        || !machine_field_placeholder_summary(requested_summary)
    {
        return None;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && step.skill == "respond")
        .filter_map(|step| step.output.as_deref())
        .map(str::trim)
        .find(|candidate| terminal_scalar_respond_matches_route(route, candidate))
        .map(ToOwned::to_owned)
}

fn machine_field_placeholder_summary(value: &str) -> bool {
    matches!(
        value.trim(),
        "field_value" | "command_output" | "value" | "count"
    )
}

fn terminal_scalar_respond_matches_route(route: &crate::RouteResult, candidate: &str) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || candidate
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
            != 1
        || candidate.starts_with('{')
        || candidate.starts_with('[')
        || candidate.contains('=')
        || crate::finalize::parse_delivery_token(candidate).is_some()
        || crate::finalize::looks_like_planner_artifact(candidate)
        || crate::finalize::looks_like_internal_trace_artifact(candidate)
        || crate::finalize::is_execution_summary_message(candidate)
    {
        return false;
    }
    if route.output_contract_marker_is(crate::OutputSemanticKind::ScalarPathOnly) {
        return candidate.starts_with('/')
            || candidate.starts_with("./")
            || candidate.starts_with("../")
            || candidate.contains('/');
    }
    true
}

fn current_delivery_preserves_web_search_listing(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &LoopState,
    current: &str,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route_is_web_search_listing(route) {
        return false;
    }
    let current = current.trim();
    if current.is_empty() {
        return false;
    }
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            step.is_ok() && matches!(step.skill.as_str(), "web_search_extract" | "browser_web")
        })
        .filter_map(|step| step.output.as_deref())
        .flat_map(web_search_candidate_title_sources_from_output)
        .any(|(title, _source)| current.contains(&title))
}

fn route_is_web_search_listing(route: &crate::RouteResult) -> bool {
    crate::machine_capability_ref::route_has_capability_action(
        route,
        &["web", "browser"],
        &["search", "results"],
    )
}

fn web_search_candidate_title_sources_from_output(output: &str) -> Vec<(String, String)> {
    let Ok(value) = serde_json::from_str::<Value>(output.trim()) else {
        return Vec::new();
    };
    let mut pairs = Vec::new();
    collect_web_search_candidate_title_sources_from_json(&value, &mut pairs);
    pairs
}

fn collect_web_search_candidate_title_sources_from_json(
    value: &Value,
    pairs: &mut Vec<(String, String)>,
) {
    for pointer in [
        "/extra/candidates",
        "/extra/items",
        "/candidates",
        "/items",
        "/results",
    ] {
        if let Some(items) = value.pointer(pointer).and_then(Value::as_array) {
            collect_web_search_candidate_array_title_sources(items, pairs);
        }
    }
}

fn collect_web_search_candidate_array_title_sources(
    items: &[Value],
    pairs: &mut Vec<(String, String)>,
) {
    for item in items {
        let Some(object) = item.as_object() else {
            continue;
        };
        let title = object
            .get("title")
            .or_else(|| object.get("name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let source = object
            .get("source")
            .or_else(|| object.get("domain"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let (Some(title), Some(source)) = (title, source) {
            pairs.push((title.to_string(), source.to_string()));
        }
    }
}

fn service_status_selector_only_summary(
    agent_run_context: Option<&AgentRunContext>,
    current: &str,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route_is_service_status_contract(route) {
        return false;
    }
    let Some(selector) = route
        .output_contract
        .self_extension
        .structured_field_selector
        .as_deref()
        .map(str::trim)
        .filter(|selector| !selector.is_empty())
    else {
        return false;
    };
    current.trim() == selector
}

fn latest_publishable_service_status_terminal_delivery(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if !route_is_service_status_contract(route) {
        return None;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| {
            step.is_ok() && matches!(step.skill.as_str(), "respond" | "synthesize_answer")
        })
        .filter_map(|step| step.output.as_deref())
        .find_map(|candidate| publishable_service_status_terminal_delivery(route, candidate))
        .or_else(|| {
            loop_state
                .last_user_visible_respond
                .as_deref()
                .and_then(|candidate| {
                    publishable_service_status_terminal_delivery(route, candidate)
                })
        })
        .or_else(|| {
            loop_state
                .last_publishable_synthesis_output
                .as_deref()
                .and_then(|candidate| {
                    publishable_service_status_terminal_delivery(route, candidate)
                })
        })
}

fn route_is_service_status_contract(route: &crate::RouteResult) -> bool {
    crate::finalize::route_matches_service_status_output_contract(route)
}

fn publishable_service_status_terminal_delivery(
    route: &crate::RouteResult,
    candidate: &str,
) -> Option<String> {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || candidate.starts_with('{')
        || candidate.starts_with('[')
        || route
            .output_contract
            .self_extension
            .structured_field_selector
            .as_deref()
            .map(str::trim)
            .filter(|selector| !selector.is_empty())
            .is_some_and(|selector| candidate == selector)
    {
        return None;
    }
    Some(candidate.to_string())
}

fn current_delivery_is_richer_than_requested_machine_summary(
    agent_run_context: Option<&AgentRunContext>,
    current: &str,
    requested_summary: &str,
) -> bool {
    if current_delivery_has_conflicting_values_for_requested_keys(current, requested_summary) {
        return false;
    }
    if strict_machine_field_contract_requested(agent_run_context)
        && !current_delivery_is_machine_kv_only(current)
    {
        return false;
    }
    if current_delivery_has_values_for_requested_marker_summary(current, requested_summary) {
        return true;
    }
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if route.output_contract.delivery_required {
        return false;
    }
    if current_delivery_is_publishable_evidence_summary(route, current, requested_summary) {
        return true;
    }
    let preserves_richer_delivery = route
        .output_contract_marker_is(crate::OutputSemanticKind::RecentScalarEqualityCheck)
        || route_required_machine_evidence_is_present_in_current_delivery(route, current);
    if !preserves_richer_delivery {
        return false;
    }
    machine_kv_units_strictly_extend(current, requested_summary)
}

fn current_delivery_has_conflicting_values_for_requested_keys(
    current: &str,
    requested_summary: &str,
) -> bool {
    requested_machine_keys(requested_summary)
        .into_iter()
        .any(|key| machine_kv_values_for_key(current, &key).len() > 1)
}

fn requested_machine_keys(requested_summary: &str) -> Vec<String> {
    let mut keys = machine_kv_unit_keys(requested_summary);
    for marker in bare_machine_markers(requested_summary) {
        if !keys.iter().any(|key| key == &marker) {
            keys.push(marker);
        }
    }
    keys
}

fn machine_kv_values_for_key(text: &str, requested_key: &str) -> Vec<String> {
    let mut values = Vec::new();
    for unit in machine_kv_units(text) {
        let Some((key, value)) = unit.split_once('=') else {
            continue;
        };
        if key != requested_key || values.iter().any(|existing| existing == value) {
            continue;
        }
        values.push(value.to_string());
    }
    values
}

fn strict_machine_field_contract_requested(agent_run_context: Option<&AgentRunContext>) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.turn_analysis.as_ref())
        .and_then(|analysis| analysis.state_patch.as_ref())
        .is_some_and(state_patch_has_required_machine_field_contract)
}

fn state_patch_has_required_machine_field_contract(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, child)| {
            let key = normalized_state_patch_key(key);
            matches!(
                key.as_str(),
                "required_field" | "required_machine_field" | "required_machine_fields"
            ) || state_patch_has_required_machine_field_contract(child)
        }),
        Value::Array(items) => items
            .iter()
            .any(state_patch_has_required_machine_field_contract),
        Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => false,
    }
}

fn normalized_state_patch_key(key: &str) -> String {
    key.trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>()
        .to_ascii_lowercase()
}

fn current_delivery_is_publishable_evidence_summary(
    route: &crate::RouteResult,
    current: &str,
    requested_summary: &str,
) -> bool {
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    ) || !route_allows_model_language_delivery(route)
        || (machine_kv_units(requested_summary).is_empty()
            && bare_machine_markers(requested_summary).is_empty())
    {
        return false;
    }
    let current = current.trim();
    if current.is_empty()
        || current.starts_with('{')
        || current.starts_with('[')
        || crate::finalize::parse_delivery_token(current).is_some()
        || crate::finalize::looks_like_planner_artifact(current)
        || crate::finalize::looks_like_internal_trace_artifact(current)
        || crate::finalize::is_execution_summary_message(current)
        || super::looks_like_raw_command_snapshot(current)
        || super::looks_like_structured_machine_output(current)
        || current_delivery_is_machine_kv_only(current)
    {
        return false;
    }
    let current_chars = current.chars().count();
    let summary_chars = requested_summary.trim().chars().count();
    let nonempty_lines = current
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .count();
    let token_count = current.split_whitespace().count();
    current_chars > summary_chars.saturating_add(16)
        && (nonempty_lines > 1 || token_count >= 6 || current_chars >= 48)
}

fn route_allows_model_language_delivery(route: &crate::RouteResult) -> bool {
    crate::evidence_policy::final_answer_shape_for_route(route)
        .is_some_and(|shape| shape.allows_model_language())
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        )
}

fn current_delivery_is_machine_kv_only(current: &str) -> bool {
    let mut saw_line = false;
    for line in current
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        saw_line = true;
        let units = machine_kv_units(line);
        if units.is_empty() {
            return false;
        }
        let unit_text = units.join(" ");
        if unit_text != line {
            return false;
        }
    }
    saw_line
}

fn current_delivery_has_values_for_requested_marker_summary(
    current: &str,
    requested_summary: &str,
) -> bool {
    let requested_markers = bare_machine_markers(requested_summary);
    !requested_markers.is_empty()
        && requested_markers
            .iter()
            .all(|marker| current_delivery_has_value_for_marker(current, marker))
}

fn bare_machine_markers(text: &str) -> Vec<String> {
    let tokens = text
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|ch: char| {
                matches!(
                    ch,
                    ',' | '.' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            })
        })
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() || tokens.iter().any(|token| token.contains('=')) {
        return Vec::new();
    }
    tokens
        .into_iter()
        .filter(|token| valid_machine_unit_key(token))
        .map(str::to_string)
        .collect()
}

fn current_delivery_has_value_for_marker(current: &str, marker: &str) -> bool {
    let marker = marker.trim();
    if marker.is_empty() {
        return false;
    }
    current.lines().any(|line| {
        let line = line.trim();
        if let Some(value) = line.strip_prefix(format!("{marker}=").as_str()) {
            return !value.trim().is_empty();
        }
        if let Some(value) = line.strip_prefix(format!("{marker}:").as_str()) {
            return !value.trim().is_empty();
        }
        false
    })
}

fn route_required_machine_evidence_is_present_in_current_delivery(
    route: &crate::RouteResult,
    current: &str,
) -> bool {
    if !route.output_contract.requires_content_evidence {
        return false;
    }
    let current_keys = machine_kv_unit_keys(current);
    if current_keys.is_empty() {
        return false;
    }
    crate::evidence_policy::required_evidence_fields_for_route(route)
        .iter()
        .any(|field| current_keys.iter().any(|key| key == field))
}

fn machine_kv_units_strictly_extend(current: &str, requested_summary: &str) -> bool {
    let current_units = machine_kv_units(current);
    let requested_units = machine_kv_units(requested_summary);
    !requested_units.is_empty()
        && current_units.len() > requested_units.len()
        && requested_units
            .iter()
            .all(|unit| current_units.iter().any(|current| current == unit))
}

fn machine_kv_units(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter_map(|token| {
            let token = token.trim_matches(|ch: char| {
                matches!(
                    ch,
                    ',' | '.' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            });
            let (key, value) = token.split_once('=')?;
            if valid_machine_unit_key(key) && !value.trim().is_empty() {
                Some(format!("{}={}", key.trim(), value.trim()))
            } else {
                None
            }
        })
        .collect()
}

fn machine_kv_unit_keys(text: &str) -> Vec<String> {
    machine_kv_units(text)
        .into_iter()
        .filter_map(|unit| unit.split_once('=').map(|(key, _)| key.to_string()))
        .collect()
}

fn valid_machine_unit_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn current_delivery_contains_full_structured_contract(
    loop_state: &LoopState,
    delivery_messages: &[String],
) -> bool {
    delivery_messages
        .iter()
        .chain(loop_state.delivery_messages.iter())
        .chain(loop_state.last_user_visible_respond.iter())
        .any(|message| structured_contract_json_should_remain_full(message))
}

fn structured_contract_json_should_remain_full(text: &str) -> bool {
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(text.trim()) else {
        return false;
    };
    if structured_config_guard_json_should_remain_full(&object) {
        return true;
    }
    object.contains_key("contract_marker")
        && (object.contains_key("async_timeout_policy")
            || object.contains_key("adapter_result")
            || object.contains_key("pending_async_job_contract")
            || object.contains_key("task_lifecycle")
            || object.contains_key("execution_policy"))
}

fn structured_config_guard_json_should_remain_full(
    object: &serde_json::Map<String, Value>,
) -> bool {
    let Some(message_key) = object.get("message_key").and_then(Value::as_str) else {
        return false;
    };
    if !matches!(
        message_key,
        "clawd.msg.config_edit.guard" | "clawd.msg.config_risk.summary"
    ) {
        return false;
    }
    object.contains_key("path")
        && (object.contains_key("risk_count")
            || object.contains_key("count")
            || object.contains_key("risks")
            || object.contains_key("candidates"))
}

fn should_restore_config_guard_payload(
    agent_run_context: Option<&AgentRunContext>,
    requested_summary: &str,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if route.output_contract.delivery_required
        || !route.output_contract_marker_is(crate::OutputSemanticKind::ContentExcerptSummary)
    {
        return false;
    }
    if config_guard_payload_from_text(requested_summary).is_some() {
        return false;
    }
    machine_kv_units(requested_summary).len() <= 2
}

fn latest_config_guard_machine_payload(
    loop_state: &LoopState,
    delivery_messages: &[String],
) -> Option<String> {
    delivery_messages
        .iter()
        .rev()
        .chain(loop_state.delivery_messages.iter().rev())
        .chain(loop_state.last_user_visible_respond.iter())
        .chain(loop_state.last_publishable_synthesis_output.iter())
        .find_map(|message| config_guard_payload_from_text(message))
        .or_else(|| {
            loop_state
                .executed_step_results
                .iter()
                .rev()
                .filter(|step| step.is_ok())
                .filter_map(|step| step.output.as_deref())
                .find_map(config_guard_payload_from_text)
        })
}

fn config_guard_payload_from_text(text: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(text.trim()).ok()?;
    config_guard_payload_from_json(&value)
}

fn config_guard_payload_from_json(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    if structured_config_guard_json_should_remain_full(object) {
        return Some(Value::Object(object.clone()).to_string());
    }
    for key in ["text", "output"] {
        if let Some(nested) = object
            .get(key)
            .and_then(Value::as_str)
            .and_then(config_guard_payload_from_text)
        {
            return Some(nested);
        }
    }
    None
}

fn requested_machine_kv_request_surfaces(
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> Vec<String> {
    let mut surfaces = vec![user_text.to_string()];
    let Some(ctx) = agent_run_context else {
        return surfaces;
    };
    for value in [
        ctx.original_user_request.as_deref(),
        ctx.user_request.as_deref(),
        ctx.route_result
            .as_ref()
            .map(|route| route.resolved_intent.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        crate::machine_kv_projection::push_unique_machine_kv_surface(&mut surfaces, value);
    }
    if let Some(state_patch) = ctx
        .turn_analysis
        .as_ref()
        .and_then(|analysis| analysis.state_patch.as_ref())
    {
        crate::machine_kv_projection::collect_requested_machine_kv_surfaces_from_state_patch(
            state_patch,
            &mut surfaces,
        );
    }
    surfaces
}
