use serde_json::Value;

use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::ClaimedTask;

#[path = "loop_reply_machine_kv/machine_unit_delivery.rs"]
mod machine_unit_delivery;
#[path = "loop_reply_machine_kv/path_fact_delivery.rs"]
mod path_fact_delivery;
#[path = "loop_reply_machine_kv/request_surfaces.rs"]
mod request_surfaces;
#[path = "loop_reply_machine_kv/structured_contract_delivery.rs"]
mod structured_contract_delivery;
#[path = "loop_reply_machine_kv/structured_scalar_delivery.rs"]
mod structured_scalar_delivery;

use machine_unit_delivery::{
    bare_machine_markers, current_delivery_contains_all_requested_machine_units,
    current_delivery_has_conflicting_values_for_requested_keys,
    current_delivery_has_values_for_requested_marker_summary, current_delivery_is_machine_kv_only,
    current_delivery_is_publishable_evidence_summary,
    latest_publishable_delivery_with_requested_machine_units, machine_kv_units,
    machine_kv_units_strictly_extend, normalized_state_patch_key,
    patch_current_delivery_empty_requested_machine_fields, requested_machine_summary_pairs,
    route_required_machine_evidence_is_present_in_current_delivery,
    strict_machine_field_contract_requested, valid_machine_unit_key,
};
use request_surfaces::requested_machine_kv_request_surfaces;
use structured_contract_delivery::{
    current_delivery_contains_full_structured_contract, latest_config_guard_machine_payload,
    should_restore_config_guard_payload,
};

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
    if current_delivery_contains_agent_hook_policy_surface(&current) {
        loop_state.last_user_visible_respond = Some(current);
        return false;
    }
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
    if let Some(restored) =
        path_fact_delivery::latest_path_batch_fact_delivery_for_requested_summary(
            loop_state,
            agent_run_context,
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
            "requested_machine_kv_summary_path_fact_delivery",
            "restored",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return true;
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
    if let Some(restored) = latest_web_search_candidate_listing_delivery(loop_state, &current) {
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
            "requested_machine_kv_summary_web_search_candidate_listing",
            "restored",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return true;
    }
    if current_delivery_is_terminal_scalar_answer(agent_run_context, &current)
        && !requested_machine_summary_should_override_scalar(&current, &answer)
    {
        loop_state.last_user_visible_respond = Some(current);
        return false;
    }
    if current_delivery_satisfies_service_status_selector(agent_run_context, &current)
        || current_delivery_is_execution_recipe_closeout(&current)
        || current_delivery_is_structured_json_answer(&current)
        || current_delivery_is_generated_file_report_machine_projection(&current)
        || current_delivery_is_async_adapter_machine_projection(&current)
    {
        loop_state.last_user_visible_respond = Some(current);
        return false;
    }
    if current_delivery_is_service_control_observed_field_projection(&current) {
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

fn current_delivery_contains_agent_hook_policy_surface(current: &str) -> bool {
    let current = current.trim();
    !current.is_empty()
        && current.contains("stage=pre_tool_use")
        && [
            "agent.hooks.blocked_action_refs",
            "agent.hooks.blocked_tools",
            "agent.hooks.require_confirmation_action_refs",
            "agent.hooks.background_wait_action_refs",
        ]
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
    if crate::finalize::route_matches_single_path_output_contract(route) {
        return candidate.starts_with('/')
            || candidate.starts_with("./")
            || candidate.starts_with("../")
            || candidate.contains('/');
    }
    true
}

fn requested_machine_summary_should_override_scalar(
    current: &str,
    requested_summary: &str,
) -> bool {
    let current = current.trim();
    let requested = requested_summary.trim();
    if requested.is_empty() || !requested.contains('=') || current.contains(requested) {
        return false;
    }
    !requested_machine_summary_value_matches_scalar(current, requested)
}

fn requested_machine_summary_value_matches_scalar(current: &str, requested_summary: &str) -> bool {
    let mut lines = requested_summary
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let Some(line) = lines.next() else {
        return false;
    };
    if lines.next().is_some() {
        return false;
    }
    let Some((_key, value)) = line.split_once('=') else {
        return false;
    };
    let value = value.trim();
    !current.is_empty() && value == current
}

fn current_delivery_preserves_web_search_listing(
    _agent_run_context: Option<&AgentRunContext>,
    loop_state: &LoopState,
    current: &str,
) -> bool {
    let current = current.trim();
    if current.is_empty() {
        return false;
    }
    let pairs = web_search_candidate_title_sources_from_loop_state(loop_state);
    web_search_candidate_titles_are_covered(&pairs, current)
}

fn latest_web_search_candidate_listing_delivery(
    loop_state: &LoopState,
    current: &str,
) -> Option<String> {
    if !current_delivery_is_machine_kv_only(current) {
        return None;
    }
    let pairs = web_search_candidate_title_sources_from_loop_state(loop_state);
    web_search_candidate_listing_from_pairs(pairs)
}

fn web_search_candidate_title_sources_from_loop_state(
    loop_state: &LoopState,
) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for step in &loop_state.executed_step_results {
        if !step.is_ok() || !matches!(step.skill.as_str(), "web_search_extract" | "browser_web") {
            continue;
        }
        if let Some(output) = step.output.as_deref() {
            for pair in web_search_candidate_title_sources_from_output(output) {
                if !pairs.iter().any(|existing| existing == &pair) {
                    pairs.push(pair);
                }
            }
        }
    }
    pairs
}

fn web_search_candidate_titles_are_covered(pairs: &[(String, String)], visible: &str) -> bool {
    let mut titles: Vec<&str> = Vec::new();
    for (title, _source) in pairs {
        let title = title.as_str();
        if !titles.contains(&title) {
            titles.push(title);
        }
    }
    !titles.is_empty() && titles.into_iter().all(|title| visible.contains(title))
}

fn web_search_candidate_listing_from_pairs(pairs: Vec<(String, String)>) -> Option<String> {
    if pairs.is_empty() {
        return None;
    }
    Some(
        pairs
            .into_iter()
            .map(|(title, source)| format!("{title} - {source}"))
            .collect::<Vec<_>>()
            .join("\n"),
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
    if current_delivery_contains_all_requested_machine_units(current, requested_summary) {
        return true;
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

fn current_delivery_is_service_control_observed_field_projection(current: &str) -> bool {
    let units = machine_kv_units(current);
    if units.is_empty()
        || !units
            .iter()
            .any(|unit| unit.as_str() == "source=service_control")
    {
        return false;
    }
    let has_key = |key: &str| {
        units.iter().any(|unit| {
            unit.split_once('=')
                .is_some_and(|(unit_key, _)| unit_key == key)
        })
    };
    has_key("target")
        && has_key("service_name")
        && has_key("status")
        && has_key("verified")
        && (has_key("post_state") || has_key("pre_state"))
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
