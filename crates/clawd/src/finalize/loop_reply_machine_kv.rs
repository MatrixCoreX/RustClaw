use serde_json::Value;

use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::ClaimedTask;

#[path = "loop_reply_machine_kv/machine_unit_delivery.rs"]
mod machine_unit_delivery;
#[path = "loop_reply_machine_kv/request_surfaces.rs"]
mod request_surfaces;
#[path = "loop_reply_machine_kv/structured_contract_delivery.rs"]
mod structured_contract_delivery;
#[path = "loop_reply_machine_kv/structured_scalar_delivery.rs"]
mod structured_scalar_delivery;

use machine_unit_delivery::{
    contains_labeled_machine_scalar, contains_requested_machine_field_label,
    current_delivery_contains_all_requested_machine_units,
    current_delivery_has_conflicting_values_for_requested_keys,
    current_delivery_has_values_for_requested_marker_summary, current_delivery_is_machine_kv_only,
    current_delivery_is_publishable_evidence_summary,
    latest_publishable_delivery_with_requested_machine_units, machine_kv_units,
    machine_kv_units_strictly_extend, normalized_state_patch_key,
    patch_current_delivery_conflicting_requested_machine_fields,
    patch_current_delivery_empty_requested_machine_fields,
    route_required_machine_evidence_is_present_in_current_delivery,
    strict_machine_field_contract_requested, valid_machine_unit_key,
};
use request_surfaces::requested_machine_kv_request_surfaces;
use structured_contract_delivery::current_delivery_contains_full_structured_contract;

use super::{
    current_delivery_is_latest_publishable_synthesis, current_delivery_is_latest_terminal_respond,
    exact_observation_machine_field_delivery_satisfies_request, final_answer_text_from_delivery,
    log_deterministic_delivery_record,
};

pub(super) fn replace_delivery_with_requested_machine_kv_summary(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    delivery_messages: &mut Vec<String>,
) -> bool {
    if agent_run_context
        .and_then(|ctx| ctx.output_contract())
        .is_some_and(crate::IntentOutputContract::requests_path_inspection)
    {
        return false;
    }
    if current_delivery_contains_full_structured_contract(loop_state, delivery_messages) {
        return false;
    }
    let current = normalize_markdown_format_table_delivery(loop_state, delivery_messages);
    if current_delivery_contains_agent_hook_runtime_surface(&current) {
        loop_state.last_user_visible_respond = Some(current);
        return false;
    }
    if agent_run_context
        .and_then(|ctx| ctx.output_contract())
        .is_some_and(|route| {
            route.requests_exact_command_output()
                && exact_observation_machine_field_delivery_satisfies_request(route, &current)
        })
    {
        loop_state.last_user_visible_respond = Some(current);
        return false;
    }
    if structured_scalar_delivery::replace_current_field_selector_with_value(
        &task.task_id,
        loop_state,
        delivery_messages,
        finalizer_summary,
        &current,
    ) {
        return true;
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
    if current_json_delivery_satisfies_required_machine_fields(agent_run_context, &current)
        || current_json_delivery_satisfies_requested_machine_tokens(&current, &request_surfaces)
    {
        loop_state.last_user_visible_respond = Some(current);
        return false;
    }
    if let Some(restored) = latest_json_delivery_satisfying_requested_machine_tokens(
        loop_state,
        delivery_messages,
        &request_surfaces,
    ) {
        if restored.trim() == current.trim() {
            loop_state.last_user_visible_respond = Some(current);
            return false;
        }
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
            "requested_machine_kv_summary_latest_requested_json",
            "restored",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return true;
    }
    let Some(answer) =
        crate::machine_kv_projection::requested_machine_kv_summary_from_observation_inputs(
            request_surfaces.iter().map(String::as_str),
            &observed_texts,
        )
    else {
        return false;
    };
    if !strict_machine_field_contract_requested(agent_run_context)
        && (current_delivery_is_latest_publishable_synthesis(loop_state, &current)
            || current_delivery_is_latest_terminal_respond(loop_state, &current))
        && !contains_requested_machine_field_label(&current, &answer)
        && !contains_labeled_machine_scalar(&current)
    {
        loop_state.last_user_visible_respond = Some(current);
        return false;
    }
    if marker_only_requested_summary(&answer)
        && !strict_machine_field_contract_requested(agent_run_context)
    {
        if let Some(restored) = latest_publishable_delivery_for_marker_only_summary(
            loop_state,
            delivery_messages,
            &answer,
        ) {
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
                "requested_machine_kv_summary_marker_only_rich_delivery",
                "restored",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
            return true;
        }
        loop_state.last_user_visible_respond = Some(current);
        return false;
    }
    if let Some(restored) =
        crate::finalize::search_path_projection::path_listing_from_marker_summary_outputs(
            loop_state
                .executed_step_results
                .iter()
                .filter(|step| step.is_ok())
                .filter_map(|step| step.output.as_deref()),
            &answer,
        )
    {
        if restored.trim() == current.trim() {
            loop_state.last_user_visible_respond = Some(current);
            return false;
        }
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
            "requested_machine_kv_summary_search_path_listing",
            "restored",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return true;
    }
    if delivery_contains_agent_loop_control_envelope(loop_state, delivery_messages) {
        loop_state.last_user_visible_respond = Some(current);
        return false;
    }
    if current_delivery_is_terminal_scalar_answer(agent_run_context, &current)
        && !requested_machine_summary_should_override_scalar(agent_run_context, &current, &answer)
    {
        loop_state.last_user_visible_respond = Some(current);
        return false;
    }
    if current_delivery_satisfies_explicit_selector(agent_run_context, &current)
        || current_delivery_is_execution_recipe_closeout(&current)
        || current_delivery_is_structured_json_answer(&current)
        || current_delivery_is_generated_file_report_machine_projection(&current)
        || current_delivery_is_async_adapter_machine_projection(&current)
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
    if let Some(restored) = latest_publishable_delivery_with_requested_machine_units(
        loop_state,
        delivery_messages,
        &answer,
    ) {
        if restored.trim() == current.trim() {
            loop_state.last_user_visible_respond = Some(current);
            return false;
        }
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
            "requested_machine_kv_summary_latest_rich_delivery",
            "restored",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return true;
    }
    if let Some(patched) = patch_current_delivery_empty_requested_machine_fields(&current, &answer)
    {
        delivery_messages.clear();
        delivery_messages.push(patched.clone());
        loop_state.delivery_messages.clear();
        append_delivery_message(
            &task.task_id,
            &mut loop_state.delivery_messages,
            patched.clone(),
        );
        loop_state.last_user_visible_respond = Some(patched);
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
            "requested_machine_kv_summary_patch_empty_field",
            "patched",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return true;
    }
    if let Some(patched) =
        patch_current_delivery_conflicting_requested_machine_fields(&current, &answer)
    {
        delivery_messages.clear();
        delivery_messages.push(patched.clone());
        loop_state.delivery_messages.clear();
        append_delivery_message(
            &task.task_id,
            &mut loop_state.delivery_messages,
            patched.clone(),
        );
        loop_state.last_user_visible_respond = Some(patched);
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
            "requested_machine_kv_summary_patch_conflicting_field",
            "patched",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return true;
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

fn current_delivery_contains_agent_hook_runtime_surface(current: &str) -> bool {
    let current = current.trim();
    !current.is_empty()
        && ["agent.hooks.handlers", "hook_stages", "hook_decisions"]
            .iter()
            .all(|field| current.contains(field))
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

fn current_delivery_satisfies_explicit_selector(
    agent_run_context: Option<&AgentRunContext>,
    current: &str,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    let Some(selector) = route
        .selection
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

fn current_json_delivery_satisfies_required_machine_fields(
    agent_run_context: Option<&AgentRunContext>,
    current: &str,
) -> bool {
    let required = required_machine_field_keys_from_state_patch(agent_run_context);
    if required.is_empty() {
        return false;
    }
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(current.trim()) else {
        return false;
    };
    object.len() == required.len()
        && required
            .iter()
            .all(|key| object.get(key).is_some_and(json_value_has_payload))
}

fn current_json_delivery_satisfies_requested_machine_tokens(
    current: &str,
    request_surfaces: &[String],
) -> bool {
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(current.trim()) else {
        return false;
    };
    !object.is_empty()
        && object.len() <= 16
        && object.iter().all(|(key, value)| {
            valid_machine_unit_key(key)
                && json_value_has_payload(value)
                && request_surfaces
                    .iter()
                    .any(|surface| surface_contains_machine_token(surface, key))
        })
}

fn latest_json_delivery_satisfying_requested_machine_tokens(
    loop_state: &LoopState,
    delivery_messages: &[String],
    request_surfaces: &[String],
) -> Option<String> {
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok() || !matches!(step.skill.as_str(), "respond" | "synthesize_answer") {
            continue;
        }
        if let Some(candidate) = step.output.as_deref().and_then(|candidate| {
            json_delivery_satisfying_requested_machine_tokens(candidate, request_surfaces)
        }) {
            return Some(candidate);
        }
    }
    for candidate in [
        loop_state.last_user_visible_respond.as_deref(),
        loop_state.last_publishable_synthesis_output.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(candidate) =
            json_delivery_satisfying_requested_machine_tokens(candidate, request_surfaces)
        {
            return Some(candidate);
        }
    }
    for candidate in loop_state
        .delivery_messages
        .iter()
        .rev()
        .chain(delivery_messages.iter().rev())
    {
        if let Some(candidate) =
            json_delivery_satisfying_requested_machine_tokens(candidate, request_surfaces)
        {
            return Some(candidate);
        }
    }
    None
}

fn json_delivery_satisfying_requested_machine_tokens(
    candidate: &str,
    request_surfaces: &[String],
) -> Option<String> {
    let candidate = candidate.trim();
    if current_json_delivery_satisfies_requested_machine_tokens(candidate, request_surfaces) {
        return Some(candidate.to_string());
    }
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(candidate) else {
        return None;
    };
    let answer = object.get("answer").and_then(Value::as_str)?.trim();
    current_json_delivery_satisfies_requested_machine_tokens(answer, request_surfaces)
        .then(|| answer.to_string())
}

fn surface_contains_machine_token(surface: &str, token: &str) -> bool {
    !token.is_empty()
        && surface
            .split(|ch| !machine_token_char(ch))
            .any(|segment| segment == token)
}

fn machine_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')
}

fn required_machine_field_keys_from_state_patch(
    agent_run_context: Option<&AgentRunContext>,
) -> Vec<String> {
    let Some(state_patch) = agent_run_context
        .and_then(|ctx| ctx.turn_analysis.as_ref())
        .and_then(|analysis| analysis.state_patch.as_ref())
    else {
        return Vec::new();
    };
    let mut keys = Vec::new();
    collect_required_machine_field_keys(state_patch, &mut keys);
    keys.sort();
    keys.dedup();
    keys
}

fn collect_required_machine_field_keys(value: &Value, keys: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let key = normalized_state_patch_key(key);
                if matches!(
                    key.as_str(),
                    "required_field" | "required_machine_field" | "required_machine_fields"
                ) {
                    collect_machine_field_key_values(child, keys);
                } else {
                    collect_required_machine_field_keys(child, keys);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_required_machine_field_keys(item, keys);
            }
        }
        Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => {}
    }
}

fn collect_machine_field_key_values(value: &Value, keys: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            let key = text.trim();
            if valid_machine_unit_key(key) {
                keys.push(key.to_string());
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_machine_field_key_values(item, keys);
            }
        }
        Value::Object(object) => {
            for child in object.values() {
                collect_machine_field_key_values(child, keys);
            }
        }
        Value::Number(_) | Value::Bool(_) | Value::Null => {}
    }
}

fn json_value_has_payload(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(object) => !object.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    }
}

fn current_delivery_is_generated_file_report_machine_projection(current: &str) -> bool {
    let current = current.trim();
    if current.is_empty() {
        return false;
    }
    let units = machine_kv_units(current);
    if units.is_empty() {
        return false;
    }
    units.iter().any(|unit| {
        unit.strip_prefix("output_path=")
            .is_some_and(|value| !value.is_empty())
    }) && units.iter().any(|unit| {
        unit.strip_prefix("planned_outputs=")
            .is_some_and(|value| value.starts_with('[') && !value.is_empty())
    })
}

fn current_delivery_is_async_adapter_machine_projection(current: &str) -> bool {
    let current = current.trim();
    if current.is_empty() {
        return false;
    }
    let units = machine_kv_units(current);
    if units.is_empty() {
        return false;
    }
    let has_adapter_result = units.iter().any(|unit| {
        unit.strip_prefix("async_poll_adapter_result=")
            .or_else(|| unit.strip_prefix("async_cancel_adapter_result="))
            .is_some_and(|value| value.starts_with('{') && !value.is_empty())
    });
    let has_task = units.iter().any(|unit| {
        unit.strip_prefix("task_id=")
            .is_some_and(|value| !value.is_empty())
    });
    let has_job = units.iter().any(|unit| {
        unit.strip_prefix("job_id=")
            .is_some_and(|value| !value.is_empty())
    });
    let has_status = units.iter().any(|unit| {
        unit.strip_prefix("status=")
            .is_some_and(|value| !value.is_empty())
    });
    has_adapter_result && has_task && has_job && has_status
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
    let route = match agent_run_context.and_then(|ctx| ctx.output_contract()) {
        Some(route) if route.response_shape == crate::OutputResponseShape::Scalar => route,
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
    let route = agent_run_context.and_then(|ctx| ctx.output_contract())?;
    if route.response_shape != crate::OutputResponseShape::Scalar
        || route.delivery_required
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

fn terminal_scalar_respond_matches_route(
    route: &crate::IntentOutputContract,
    candidate: &str,
) -> bool {
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
    if crate::finalize::route_matches_single_path_output_contract(route) {
        return candidate.starts_with('/')
            || candidate.starts_with("./")
            || candidate.starts_with("../")
            || candidate.contains('/');
    }
    true
}

fn requested_machine_summary_should_override_scalar(
    agent_run_context: Option<&AgentRunContext>,
    current: &str,
    requested_summary: &str,
) -> bool {
    let current = current.trim();
    let requested = requested_summary.trim();
    if requested.is_empty() || !requested.contains('=') || current.contains(requested) {
        return false;
    }
    let route = agent_run_context
        .and_then(|ctx| ctx.output_contract())
        .filter(|route| route.response_shape == crate::OutputResponseShape::Scalar);
    !crate::machine_kv_projection::requested_machine_summary_matches_scalar(
        route, current, requested,
    )
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
    if current_delivery_contains_all_requested_machine_units(current, requested_summary) {
        return true;
    }
    if current_delivery_has_values_for_requested_marker_summary(current, requested_summary) {
        return true;
    }
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    if route.delivery_required {
        return false;
    }
    if current_delivery_is_publishable_evidence_summary(route, current, requested_summary) {
        return true;
    }
    let preserves_richer_delivery =
        route_required_machine_evidence_is_present_in_current_delivery(route, current);
    if !preserves_richer_delivery {
        return false;
    }
    machine_kv_units_strictly_extend(current, requested_summary)
}

fn marker_only_requested_summary(summary: &str) -> bool {
    let summary = summary.trim();
    !summary.is_empty()
        && !summary.contains('=')
        && summary
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
            == 1
        && valid_machine_unit_key(summary)
}

fn latest_publishable_delivery_for_marker_only_summary(
    loop_state: &LoopState,
    delivery_messages: &[String],
    marker: &str,
) -> Option<String> {
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok() || !matches!(step.skill.as_str(), "respond" | "synthesize_answer") {
            continue;
        }
        if let Some(candidate) = step
            .output
            .as_deref()
            .and_then(|candidate| publishable_delivery_for_marker_only_summary(candidate, marker))
        {
            return Some(candidate);
        }
    }
    for candidate in [
        loop_state.last_user_visible_respond.as_deref(),
        loop_state.last_publishable_synthesis_output.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(candidate) = publishable_delivery_for_marker_only_summary(candidate, marker) {
            return Some(candidate);
        }
    }
    for candidate in loop_state
        .delivery_messages
        .iter()
        .rev()
        .chain(delivery_messages.iter().rev())
    {
        if let Some(candidate) = publishable_delivery_for_marker_only_summary(candidate, marker) {
            return Some(candidate);
        }
    }
    None
}

fn publishable_delivery_for_marker_only_summary(candidate: &str, marker: &str) -> Option<String> {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || candidate == marker.trim()
        || marker_only_requested_summary(candidate)
        || crate::finalize::is_execution_summary_message(candidate)
    {
        return None;
    }
    if let Ok(Value::Object(object)) = serde_json::from_str::<Value>(candidate) {
        if object.contains_key("steps") {
            return None;
        }
        if let Some(answer) = object.get("answer").and_then(Value::as_str) {
            return publishable_delivery_for_marker_only_summary(answer, marker);
        }
        if structured_marker_evidence_payload(&object, marker) {
            return Some(candidate.to_string());
        }
    }
    if crate::finalize::looks_like_planner_artifact(candidate)
        || crate::finalize::looks_like_internal_trace_artifact(candidate)
    {
        return None;
    }
    candidate
        .contains(marker.trim())
        .then(|| candidate.to_string())
}

fn structured_marker_evidence_payload(
    object: &serde_json::Map<String, Value>,
    marker: &str,
) -> bool {
    let marker = marker.trim();
    if marker.is_empty()
        || !(object.contains_key("message_key") || object.contains_key("reason_code"))
        || !object
            .values()
            .any(|value| value_contains_text(value, marker))
    {
        return false;
    }
    [
        "current_value",
        "value",
        "value_text",
        "field_path",
        "path",
        "risk_count",
        "count",
        "candidates",
        "risks",
        "applied",
        "would_write",
    ]
    .iter()
    .any(|key| object.contains_key(*key))
}

fn value_contains_text(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(text) => text.contains(needle),
        Value::Array(items) => items.iter().any(|item| value_contains_text(item, needle)),
        Value::Object(object) => object
            .values()
            .any(|item| value_contains_text(item, needle)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}
