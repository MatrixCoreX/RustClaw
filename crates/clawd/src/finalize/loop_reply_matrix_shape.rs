#[path = "loop_reply_matrix_shape_list_projection.rs"]
mod list_projection;
#[cfg(test)]
pub(super) use list_projection::matrix_strict_list_observed_answer;
use list_projection::{
    archive_member_list_prefers_observed_projection, docker_text_list_candidate_is_observed,
    file_name_list_prefers_observed_projection, matrix_observed_answer_candidate_for_shape,
    route_requests_docker_text_list_projection, stale_file_token_delivery_bounded_read_answer,
    stale_file_token_delivery_listing_answer,
};
pub(super) use list_projection::{
    generic_observed_machine_projection_answer, matrix_grouped_name_list_observed_answer,
};

use tracing::info;

use crate::agent_engine::{AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

use super::{
    build_loop_journal, direct_created_archive_path_from_observed_archive_pack,
    direct_file_token_from_observed_auto_locator_filename,
    direct_file_token_from_observed_find_entries, direct_file_token_from_observed_inventory,
    direct_generated_file_path_report_from_dry_run_payload,
    direct_generated_file_path_report_from_written_path, direct_path_from_active_bound_inventory,
    direct_scalar_observed_answer, direct_scalar_path_candidate_list_from_observed_outputs,
    direct_structured_observed_answer_allowing_implicit_metadata_path_facts,
    directory_entry_groups_prefers_observed_groups, final_answer_text_from_delivery,
    inventory_ranked_size_list_answer, latest_bounded_read_range_answer_from_loop,
    latest_grounded_synthesis_for_mixed_listing_contract, latest_plan_requested_synthesis,
    log_deterministic_delivery_record, looks_like_structured_machine_output,
    successful_content_observation_should_precede_status_summary,
};

fn evidence_policy_final_answer_shape_class(
    route: &crate::IntentOutputContract,
) -> Option<crate::evidence_policy::FinalAnswerShapeClass> {
    if route_requests_docker_text_list_projection(route) {
        return Some(crate::evidence_policy::FinalAnswerShapeClass::StrictList);
    }
    crate::evidence_policy::final_answer_shape_for_output_contract(route).map(|shape| shape.class())
}

pub(super) fn route_requires_evidence_policy_deterministic_final_answer(
    route: &crate::IntentOutputContract,
) -> bool {
    evidence_policy_final_answer_shape_class(route)
        .is_some_and(|class| !class.allows_model_language())
}

pub(super) fn agent_context_allows_observed_output_language_fallback(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.output_contract())
        .is_none_or(|route| !route_requires_evidence_policy_deterministic_final_answer(route))
}

pub(super) fn should_try_observed_output_language_fallback(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.output_contract())
        .is_some_and(crate::agent_engine::observed_output::route_requires_synthesized_delivery)
        || agent_context_allows_observed_output_language_fallback(agent_run_context)
        || latest_plan_requested_synthesis(loop_state)
        || successful_content_observation_should_precede_status_summary(
            agent_run_context,
            loop_state,
        )
}

#[cfg(test)]
pub(super) fn route_has_evidence_policy_final_shape(route: &crate::IntentOutputContract) -> bool {
    evidence_policy_final_answer_shape_class(route).is_some()
}

pub(super) fn route_requires_observed_output_projection(
    route: &crate::IntentOutputContract,
) -> bool {
    if crate::finalize::route_matches_service_status_output_contract(route) {
        return true;
    }
    if matches!(
        evidence_policy_final_answer_shape_class(route),
        Some(
            crate::evidence_policy::FinalAnswerShapeClass::DeliveryArtifact
                | crate::evidence_policy::FinalAnswerShapeClass::SinglePath
                | crate::evidence_policy::FinalAnswerShapeClass::StrictList
                | crate::evidence_policy::FinalAnswerShapeClass::Table
        )
    ) {
        return true;
    }
    matches!(
        route.semantic_kind,
        crate::OutputSemanticKind::DirectoryNames | crate::OutputSemanticKind::QuantityComparison
    )
}

pub(super) fn evidence_policy_candidate_satisfies_final_shape(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary>,
    route: &crate::IntentOutputContract,
    candidate: &str,
) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return false;
    }
    if route_requests_docker_text_list_projection(route)
        && docker_text_list_candidate_is_observed(route, loop_state, candidate)
    {
        return true;
    }
    let delivery_messages = vec![candidate.to_string()];
    let journal = build_loop_journal(
        task,
        user_text,
        loop_state,
        agent_run_context,
        finalizer_summary,
        crate::task_journal::delivery_payload_consistent(candidate, &delivery_messages),
        candidate,
        crate::task_journal::TaskJournalFinalStatus::Success,
    );
    let answer_contract = crate::answer_verifier::AnswerContract::new("", route.clone());
    crate::answer_verifier::structurally_satisfies_answer_contract(
        &answer_contract,
        &journal,
        candidate,
    )
}

pub(super) fn synthetic_task_for_evidence_policy_shape_check(task_id: &str) -> ClaimedTask {
    ClaimedTask {
        task_id: task_id.to_string(),
        user_id: 0,
        chat_id: 0,
        user_key: None,
        channel: "finalize".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

pub(super) fn current_synthesis_satisfies_evidence_policy_shape(
    task_id: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary>,
    route: &crate::IntentOutputContract,
    delivery_messages: &[String],
) -> bool {
    if !route_requires_evidence_policy_deterministic_final_answer(route) {
        return true;
    }
    let Some(message) = delivery_messages.last() else {
        return false;
    };
    if directory_entry_groups_prefers_observed_groups(route, loop_state) {
        return false;
    }
    if archive_member_list_prefers_observed_projection(route) {
        return false;
    }
    if file_name_list_prefers_observed_projection(route, loop_state) {
        return false;
    }
    let task = synthetic_task_for_evidence_policy_shape_check(task_id);
    evidence_policy_candidate_satisfies_final_shape(
        &task,
        "",
        loop_state,
        agent_run_context,
        finalizer_summary,
        route,
        message,
    )
}

fn matrix_table_observed_answer(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_requests_table_listing(route) {
        return None;
    }
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok()
            || matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
        {
            continue;
        }
        let Some(output) = step
            .output
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
            continue;
        };
        if let Some(answer) = matrix_markdown_table_from_json(&value) {
            return Some((answer, matrix_observed_shape_summary(loop_state)));
        }
    }
    None
}

fn route_requests_table_listing(route: &crate::IntentOutputContract) -> bool {
    crate::evidence_policy::final_answer_shape_for_output_contract(route)
        == Some(crate::evidence_policy::FinalAnswerShape::TableListing)
        || route.semantic_kind_is(crate::OutputSemanticKind::SqliteTableListing)
}

fn matrix_markdown_table_from_json(value: &serde_json::Value) -> Option<String> {
    let rows = value
        .get("rows")
        .or_else(|| value.pointer("/result/rows"))?
        .as_array()?;
    if rows.is_empty() {
        return None;
    }
    let columns = matrix_table_columns(value, rows)?;
    let mut table = String::new();
    table.push('|');
    for column in &columns {
        table.push(' ');
        table.push_str(column);
        table.push_str(" |");
    }
    table.push('\n');
    table.push('|');
    for _ in &columns {
        table.push_str(" --- |");
    }
    for row in rows {
        let cells = matrix_table_row_cells(row, &columns)?;
        table.push('\n');
        table.push('|');
        for cell in cells {
            table.push(' ');
            table.push_str(&cell);
            table.push_str(" |");
        }
    }
    Some(table)
}

fn matrix_table_columns(
    value: &serde_json::Value,
    rows: &[serde_json::Value],
) -> Option<Vec<String>> {
    let mut columns = value
        .get("columns")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for row in rows {
        if let Some(map) = row.as_object() {
            for key in map.keys() {
                if !columns.iter().any(|column| column == key) {
                    columns.push(key.clone());
                }
            }
        }
    }
    (!columns.is_empty()).then_some(columns)
}

fn matrix_table_row_cells(row: &serde_json::Value, columns: &[String]) -> Option<Vec<String>> {
    match row {
        serde_json::Value::Object(map) => {
            let mut cells = Vec::new();
            for column in columns {
                let cell = map
                    .get(column)
                    .and_then(matrix_table_cell_text)
                    .unwrap_or_default();
                if cell.contains(['\n', '|']) {
                    return None;
                }
                cells.push(cell);
            }
            Some(cells)
        }
        serde_json::Value::Array(values) => values
            .iter()
            .map(matrix_table_cell_text)
            .collect::<Option<Vec<_>>>(),
        value => matrix_table_cell_text(value).map(|cell| vec![cell]),
    }
}

fn matrix_table_cell_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.trim().to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Null => Some(String::new()),
        _ => None,
    }
}

pub(super) fn matrix_observed_shape_summary(
    loop_state: &LoopState,
) -> crate::task_journal::TaskJournalFinalizerSummary {
    crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: loop_state.executed_step_results.len(),
        ..Default::default()
    }
}

pub(super) fn replace_delivery_with_matrix_observed_shape_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    if loop_state.pending_user_input_required {
        return false;
    }
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    if !route_requires_evidence_policy_deterministic_final_answer(route) {
        return false;
    }
    if let Some((candidate, summary)) =
        direct_path_from_active_bound_inventory(loop_state, agent_run_context)
    {
        let answer = candidate.trim().to_string();
        if answer.is_empty() {
            return false;
        }
        if final_answer_text_from_delivery(delivery_messages).trim() == answer {
            *finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer);
            return true;
        }
        delivery_messages.clear();
        delivery_messages.push(answer.clone());
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        log_deterministic_delivery_record(
            &task.task_id,
            "matrix_replace_active_bound_inventory_path",
            "replaced",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return true;
    }
    let Some(shape_class) = evidence_policy_final_answer_shape_class(route) else {
        return false;
    };
    let current_answer = final_answer_text_from_delivery(delivery_messages);
    if let Some((candidate, summary)) =
        stale_file_token_delivery_listing_answer(route, loop_state, delivery_messages)
    {
        let answer = candidate.trim().to_string();
        if answer.is_empty() {
            return false;
        }
        delivery_messages.clear();
        delivery_messages.push(answer.clone());
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        log_deterministic_delivery_record(
            &task.task_id,
            "matrix_replace_stale_file_token_with_listing",
            "replaced",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return true;
    }
    if let Some((candidate, summary)) =
        stale_file_token_delivery_bounded_read_answer(route, loop_state, delivery_messages)
    {
        let answer = candidate.trim().to_string();
        if answer.is_empty() {
            return false;
        }
        delivery_messages.clear();
        delivery_messages.push(answer.clone());
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        log_deterministic_delivery_record(
            &task.task_id,
            "matrix_replace_stale_file_token_with_bounded_read",
            "replaced",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return true;
    }
    if !current_answer.trim().is_empty()
        && !directory_entry_groups_prefers_observed_groups(route, loop_state)
        && !archive_member_list_prefers_observed_projection(route)
        && !file_name_list_prefers_observed_projection(route, loop_state)
        && evidence_policy_candidate_satisfies_final_shape(
            task,
            user_text,
            loop_state,
            agent_run_context,
            finalizer_summary.clone(),
            route,
            &current_answer,
        )
    {
        return false;
    }
    if let Some((answer, summary)) =
        latest_grounded_synthesis_for_mixed_listing_contract(route, loop_state)
    {
        let answer = answer.trim().to_string();
        if !answer.is_empty() && current_answer.trim() == answer {
            loop_state.last_user_visible_respond = Some(answer);
            *finalizer_summary = Some(summary);
            return true;
        }
    }

    let Some((candidate, summary)) = matrix_observed_answer_candidate_for_shape(
        state,
        loop_state,
        agent_run_context,
        shape_class,
    ) else {
        return false;
    };
    if !archive_member_list_prefers_observed_projection(route)
        && !file_name_list_prefers_observed_projection(route, loop_state)
        && !evidence_policy_candidate_satisfies_final_shape(
            task,
            user_text,
            loop_state,
            agent_run_context,
            Some(summary.clone()),
            route,
            &candidate,
        )
    {
        return false;
    }

    let answer = candidate.trim().to_string();
    delivery_messages.clear();
    delivery_messages.push(answer.clone());
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(summary);
    info!(
        "delivery matrix_shape_from_observed task_id={} shape_class={} answer={}",
        task.task_id,
        shape_class.as_str(),
        crate::truncate_for_log(&candidate)
    );
    log_deterministic_delivery_record(
        &task.task_id,
        "matrix_shape_from_observed",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

pub(super) fn finalizer_summary_requires_matrix_observed_replacement(
    summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(summary) = summary else {
        return false;
    };
    summary.needs_clarify == Some(true)
        || !summary.contract_ok
        || summary.format_ok == Some(false)
        || summary.grounded_ok == Some(false)
}

pub(crate) fn deterministic_matrix_observed_shape_answer(
    state: &AppState,
    _task: &ClaimedTask,
    _user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.output_contract())?;
    if !route_requires_evidence_policy_deterministic_final_answer(route) {
        return None;
    }
    let shape_class = evidence_policy_final_answer_shape_class(route)?;
    let (candidate, summary) = matrix_observed_answer_candidate_for_shape(
        state,
        loop_state,
        agent_run_context,
        shape_class,
    )?;
    let candidate = candidate.trim().to_string();
    if candidate.is_empty() {
        return None;
    }
    Some((candidate, summary))
}
