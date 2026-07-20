use tracing::info;

use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

use super::{log_deterministic_delivery_record, looks_like_structured_machine_output};

pub(super) fn latest_plan_requested_synthesis(loop_state: &LoopState) -> bool {
    loop_state.round_traces.iter().rev().any(|round| {
        round
            .plan_result
            .as_ref()
            .is_some_and(|plan| plan.raw_plan_text.contains("\"synthesize_answer\""))
    })
}

pub(super) fn latest_path_batch_facts_has_implicit_metadata_fields(loop_state: &LoopState) -> bool {
    let Some(observed) =
        crate::agent_engine::observed_output::extract_latest_generic_successful_output(loop_state)
    else {
        return false;
    };
    if !matches!(observed.skill.as_str(), "fs_basic" | "system_basic") {
        return false;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&observed.body) else {
        return false;
    };
    if value.get("action").and_then(|value| value.as_str()) != Some("path_batch_facts")
        || value.get("fields").is_some()
    {
        return false;
    }
    value
        .get("facts")
        .and_then(|value| value.as_array())
        .is_some_and(|facts| {
            facts.iter().any(|entry| {
                entry
                    .get("fact")
                    .and_then(|value| value.as_object())
                    .is_some_and(|fact| {
                        fact.get("size_bytes").is_some() || fact.get("modified_ts").is_some()
                    })
            })
        })
}

pub(super) fn route_allows_latest_tail_read_range_delivery(
    route: &crate::IntentOutputContract,
) -> bool {
    let contract = route.clone();
    if matches!(
        contract.response_shape,
        crate::OutputResponseShape::FileToken | crate::OutputResponseShape::Scalar
    ) {
        return false;
    }
    !contract.delivery_required
        && contract.requires_content_evidence
        && route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
}

pub(super) fn latest_tail_read_range_observed_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.output_contract())?;
    if !route_allows_latest_tail_read_range_delivery(route) {
        return None;
    }
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let prefer_english =
        crate::fallback::fallback_prefers_english_for_language_hint(state, &language_hint);
    let answer = latest_bounded_read_range_answer_from_loop(loop_state, prefer_english)?;
    if answer.trim().is_empty() {
        return None;
    }
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            contract_ok: true,
            used_evidence_ids_count: 1,
            ..Default::default()
        },
    ))
}

fn latest_bounded_read_range_failure_recovery_answer(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !finalizer_summary_rejects_current_delivery(finalizer_summary) {
        return None;
    }
    let route = agent_run_context.and_then(|ctx| ctx.output_contract())?;
    let contract = route.clone();
    if contract.delivery_required
        || matches!(
            contract.response_shape,
            crate::OutputResponseShape::FileToken | crate::OutputResponseShape::Scalar
        )
    {
        return None;
    }
    let answer = latest_bounded_read_range_answer_from_loop(loop_state, false)?;
    if answer.trim().is_empty() {
        return None;
    }
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            contract_ok: true,
            completion_ok: Some(true),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: loop_state.executed_step_results.len().max(1),
            ..Default::default()
        },
    ))
}

fn latest_tail_read_directory_inventory_answer(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_allows_tail_read_directory_inventory_projection(agent_run_context) {
        return None;
    }
    let answer = latest_tail_read_directory_inventory_projection(loop_state)?;
    if answer.trim().is_empty() {
        return None;
    }
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            contract_ok: true,
            completion_ok: Some(true),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: loop_state.executed_step_results.len().max(1),
            ..Default::default()
        },
    ))
}

pub(super) fn tail_read_directory_inventory_projection_available(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    latest_tail_read_directory_inventory_answer(loop_state, agent_run_context).is_some()
}

fn route_allows_tail_read_directory_inventory_projection(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return true;
    };
    let contract = route.clone();
    !contract.delivery_required
        && !matches!(
            contract.response_shape,
            crate::OutputResponseShape::FileToken | crate::OutputResponseShape::Scalar
        )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlannedTailReadRequest {
    path: String,
    requested_n: usize,
}

fn latest_tail_read_directory_inventory_projection(loop_state: &LoopState) -> Option<String> {
    if !loop_has_directory_read_error(loop_state) {
        return None;
    }
    let request = latest_planned_tail_read_request(loop_state)?;
    let value = latest_inventory_dir_value_for_path(loop_state, &request.path)?;
    let names = inventory_dir_ordered_entry_names(&value)?;
    let selected = tail_entries(&names, request.requested_n);
    if selected.is_empty() {
        return None;
    }
    let mut lines = vec![
        format!("entries.count={}", selected.len()),
        "entries:".to_string(),
    ];
    lines.extend(selected.into_iter().map(|name| format!("- {name}")));
    Some(lines.join("\n"))
}

fn latest_planned_tail_read_request(loop_state: &LoopState) -> Option<PlannedTailReadRequest> {
    loop_state.round_traces.iter().rev().find_map(|round| {
        let plan = round.plan_result.as_ref()?;
        plan.steps
            .iter()
            .rev()
            .find_map(planned_step_tail_read_request)
            .or_else(|| raw_plan_tail_read_request(&plan.raw_plan_text))
    })
}

fn planned_step_tail_read_request(step: &crate::PlanStep) -> Option<PlannedTailReadRequest> {
    planned_tail_read_request_from_args(&step.skill, &step.args)
}

fn raw_plan_tail_read_request(raw_plan_text: &str) -> Option<PlannedTailReadRequest> {
    let value = serde_json::from_str::<serde_json::Value>(raw_plan_text.trim()).ok()?;
    let steps = value.get("steps").and_then(|value| value.as_array())?;
    steps.iter().rev().find_map(|step| {
        let args = step.get("args")?;
        let skill = step
            .get("capability")
            .or_else(|| step.get("tool"))
            .or_else(|| step.get("skill"))
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        planned_tail_read_request_from_args(skill, args)
    })
}

fn planned_tail_read_request_from_args(
    skill: &str,
    args: &serde_json::Value,
) -> Option<PlannedTailReadRequest> {
    if !planned_args_are_read_range(skill, args) {
        return None;
    }
    if args.get("mode").and_then(|value| value.as_str()) != Some("tail") {
        return None;
    }
    let requested_n = tail_read_requested_n(args)?;
    if requested_n == 0 || requested_n > 100 {
        return None;
    }
    let path = args
        .get("path")
        .or_else(|| args.get("resolved_path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(PlannedTailReadRequest {
        path: path.to_string(),
        requested_n: requested_n as usize,
    })
}

fn planned_args_are_read_range(skill: &str, args: &serde_json::Value) -> bool {
    let skill = skill.trim().to_ascii_lowercase();
    let action = args
        .get("action")
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_ascii_lowercase());
    matches!(action.as_deref(), Some("read_range" | "read_text_range"))
        || matches!(
            skill.as_str(),
            "filesystem.read_range"
                | "filesystem.read_text_range"
                | "fs_basic.read_range"
                | "fs_basic.read_text_range"
                | "system_basic.read_range"
                | "system_basic.read_text_range"
        )
        || (matches!(skill.as_str(), "fs_basic" | "system_basic")
            && args.get("path").is_some()
            && args.get("mode").is_some()
            && tail_read_requested_n(args).is_some())
}

fn loop_has_directory_read_error(loop_state: &LoopState) -> bool {
    loop_state.executed_step_results.iter().any(|step| {
        matches!(step.skill.as_str(), "fs_basic" | "system_basic")
            && !step.is_ok()
            && step_error_kind(step).as_deref() == Some("is_directory")
    })
}

fn step_error_kind(step: &crate::executor::StepExecutionResult) -> Option<String> {
    let raw = step.error.as_deref()?.trim();
    let payload = raw.strip_prefix("__RC_SKILL_ERROR__:").unwrap_or(raw);
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()?
        .get("error_kind")
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

fn latest_inventory_dir_value_for_path(
    loop_state: &LoopState,
    request_path: &str,
) -> Option<serde_json::Value> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "fs_basic" | "system_basic"))
        .find_map(|step| {
            let output = step.output.as_deref()?.trim();
            let body =
                crate::agent_engine::observed_output::normalized_success_body_for_observed_output(
                    output,
                );
            let value = serde_json::from_str::<serde_json::Value>(body.trim()).ok()?;
            let value = inventory_dir_payload(value);
            (inventory_dir_path_matches(&value, request_path)).then_some(value)
        })
}

fn inventory_dir_payload(value: serde_json::Value) -> serde_json::Value {
    value
        .get("extra")
        .filter(|extra| {
            extra
                .get("action")
                .and_then(|value| value.as_str())
                .is_some()
        })
        .cloned()
        .unwrap_or(value)
}

fn inventory_dir_path_matches(value: &serde_json::Value, request_path: &str) -> bool {
    if value.get("action").and_then(|value| value.as_str()) != Some("inventory_dir") {
        return false;
    }
    let request_path = normalize_inventory_path_token(request_path);
    if request_path.is_empty() {
        return false;
    }
    ["resolved_path", "path"].into_iter().any(|key| {
        value
            .get(key)
            .and_then(|value| value.as_str())
            .map(normalize_inventory_path_token)
            .is_some_and(|path| path == request_path)
    })
}

fn normalize_inventory_path_token(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed == "/" {
        return trimmed.to_string();
    }
    trimmed.trim_end_matches('/').to_string()
}

fn inventory_dir_ordered_entry_names(value: &serde_json::Value) -> Option<Vec<String>> {
    if let Some(names) = string_array_field(value, "names").filter(|names| !names.is_empty()) {
        return Some(names);
    }
    if let Some(entries) = value.get("entries").and_then(|value| value.as_array()) {
        let names = entries
            .iter()
            .filter_map(|entry| entry.get("name").and_then(|value| value.as_str()))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if !names.is_empty() {
            return Some(names);
        }
    }
    let mut names = ["dirs", "files", "other"]
        .into_iter()
        .flat_map(|kind| {
            value
                .get("names_by_kind")
                .and_then(|names_by_kind| names_by_kind.get(kind))
                .and_then(|items| items.as_array())
                .into_iter()
                .flatten()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    if names.is_empty() {
        return None;
    }
    names.sort();
    names.dedup();
    Some(names)
}

fn string_array_field(value: &serde_json::Value, key: &str) -> Option<Vec<String>> {
    value
        .get(key)
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
}

fn tail_entries(names: &[String], requested_n: usize) -> Vec<String> {
    if requested_n == 0 {
        return Vec::new();
    }
    let start = names.len().saturating_sub(requested_n);
    names[start..].to_vec()
}

pub(super) fn latest_tail_read_range_answer_from_loop(
    loop_state: &LoopState,
    prefer_english: bool,
) -> Option<String> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find_map(|step| {
            if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
                return None;
            }
            let output = step.output.as_deref()?.trim();
            let value = serde_json::from_str::<serde_json::Value>(output).ok()?;
            tail_read_range_answer_from_value(&value, prefer_english)
        })
}

pub(super) fn latest_bounded_read_range_answer_from_loop(
    loop_state: &LoopState,
    prefer_english: bool,
) -> Option<String> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find_map(|step| {
            if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
                return None;
            }
            let output = step.output.as_deref()?.trim();
            let value = serde_json::from_str::<serde_json::Value>(output).ok()?;
            bounded_read_range_answer_from_value(&value, prefer_english)
        })
}

fn step_output_is_bounded_read_range(step: &crate::executor::StepExecutionResult) -> bool {
    if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
        return false;
    }
    let Some(output) = step
        .output
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
        return false;
    };
    value_is_bounded_read_range(&value)
}

fn value_is_bounded_read_range(value: &serde_json::Value) -> bool {
    if flat_value_is_bounded_read_range(value) {
        return true;
    }
    value.get("extra").is_some_and(value_is_bounded_read_range)
}

fn flat_value_is_tail_read_range(value: &serde_json::Value) -> bool {
    matches!(
        value.get("action").and_then(|value| value.as_str()),
        Some("read_range" | "read_text_range")
    ) && value.get("mode").and_then(|value| value.as_str()) == Some("tail")
        && tail_read_requested_n(value)
            .is_some_and(|requested_n| requested_n > 0 && requested_n <= 50)
        && value
            .get("excerpt")
            .and_then(|value| value.as_str())
            .is_some_and(|excerpt| !excerpt.trim().is_empty())
}

fn flat_value_is_bounded_read_range(value: &serde_json::Value) -> bool {
    matches!(
        value.get("action").and_then(|value| value.as_str()),
        Some("read_range" | "read_text_range")
    ) && matches!(
        value.get("mode").and_then(|value| value.as_str()),
        Some("head" | "tail" | "range")
    ) && bounded_read_requested_lines(value)
        .is_some_and(|requested_n| requested_n > 0 && requested_n <= 100)
        && value
            .get("excerpt")
            .and_then(|value| value.as_str())
            .is_some_and(|excerpt| !excerpt.trim().is_empty())
}

fn tail_read_requested_n(value: &serde_json::Value) -> Option<u64> {
    value
        .get("requested_n")
        .or_else(|| value.get("n"))
        .or_else(|| value.get("count"))
        .and_then(|value| value.as_u64())
}

fn bounded_read_requested_lines(value: &serde_json::Value) -> Option<u64> {
    tail_read_requested_n(value).or_else(|| {
        let start = value.get("start_line")?.as_u64()?;
        let end = value.get("end_line")?.as_u64()?;
        (end >= start).then_some(end - start + 1)
    })
}

fn tail_read_range_answer_from_value(
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if let Some(answer) = flat_tail_read_range_answer_from_value(value, prefer_english) {
        return Some(answer);
    }
    value
        .get("extra")
        .and_then(|extra| tail_read_range_answer_from_value(extra, prefer_english))
}

fn bounded_read_range_answer_from_value(
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if let Some(answer) = flat_bounded_read_range_answer_from_value(value, prefer_english) {
        return Some(answer);
    }
    value
        .get("extra")
        .and_then(|extra| bounded_read_range_answer_from_value(extra, prefer_english))
}

fn flat_tail_read_range_answer_from_value(
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if !flat_value_is_tail_read_range(value) {
        return None;
    }
    let mut candidate = value.clone();
    let obj = candidate.as_object_mut()?;
    obj.insert(
        "action".to_string(),
        serde_json::Value::String("read_range".to_string()),
    );
    if !obj.contains_key("requested_n") {
        obj.insert(
            "requested_n".to_string(),
            serde_json::Value::Number(tail_read_requested_n(value)?.into()),
        );
    }
    crate::agent_engine::observed_output::tail_read_range_direct_answer_candidate(
        &candidate.to_string(),
        prefer_english,
    )
}

fn flat_bounded_read_range_answer_from_value(
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if flat_value_is_tail_read_range(value) {
        return flat_tail_read_range_answer_from_value(value, prefer_english);
    }
    if !flat_value_is_bounded_read_range(value) {
        return None;
    }
    value
        .get("excerpt")
        .and_then(|value| value.as_str())
        .and_then(crate::agent_engine::observed_output::normalize_read_range_excerpt)
        .map(|answer| answer.trim().to_string())
        .filter(|answer| !answer.is_empty())
}

pub(super) fn current_user_visible_delivery_text(loop_state: &LoopState) -> Option<&str> {
    loop_state
        .delivery_messages
        .iter()
        .rev()
        .find_map(|message| {
            let text = message.trim();
            (!text.is_empty() && !crate::finalize::is_execution_summary_message(text))
                .then_some(text)
        })
        .or_else(|| {
            loop_state
                .last_user_visible_respond
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
        })
}

fn latest_publishable_terminal_summary_output(loop_state: &LoopState) -> Option<&str> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| {
            step.is_ok() && matches!(step.skill.as_str(), "respond" | "synthesize_answer")
        })
        .filter_map(|step| step.output.as_deref())
        .map(str::trim)
        .find(|output| {
            super::planned_delivery_is_publishable_model_language_answer(output)
                && !crate::finalize::is_execution_summary_message(output)
        })
}

fn route_prefers_publishable_summary_over_tail_read(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> Option<String> {
    let contract = route.clone();
    if !contract.requires_content_evidence
        || contract.delivery_required
        || matches!(
            contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
        || route_requires_raw_tail_read_passthrough(Some(route))
        || !matches!(
            crate::evidence_policy::final_answer_shape_for_output_contract(route),
            Some(crate::evidence_policy::FinalAnswerShape::SummaryWithEvidence)
        )
    {
        return None;
    }
    latest_publishable_terminal_summary_output(loop_state).map(ToString::to_string)
}

fn latest_tail_replacement_can_recover_stale_synthesis(
    loop_state: &LoopState,
    current_delivery: &str,
) -> bool {
    let current_delivery = current_delivery.trim();
    let Some(synthesis_idx) = loop_state.executed_step_results.iter().rposition(|step| {
        step.skill == "synthesize_answer"
            && step
                .output
                .as_deref()
                .map(str::trim)
                .is_some_and(|output| output == current_delivery)
    }) else {
        return false;
    };
    loop_state
        .executed_step_results
        .iter()
        .rposition(step_output_is_bounded_read_range)
        .is_some_and(|tail_idx| tail_idx > synthesis_idx)
}

fn current_delivery_is_latest_registered_output(
    loop_state: &LoopState,
    current_delivery: &str,
) -> bool {
    loop_state
        .last_output
        .as_deref()
        .map(str::trim)
        .is_some_and(|last_output| last_output == current_delivery.trim())
}

fn latest_tail_replacement_was_synthesized_after_tail(
    loop_state: &LoopState,
    replacement_answer: &str,
) -> bool {
    let Some(tail_idx) = loop_state
        .executed_step_results
        .iter()
        .rposition(step_output_is_bounded_read_range)
    else {
        return false;
    };
    loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .skip(tail_idx + 1)
        .any(|(_, step)| {
            step.skill == "synthesize_answer"
                && step
                    .output
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|output| output == replacement_answer.trim())
        })
}

fn latest_tail_read_range_should_preserve_current_delivery(
    route: Option<&crate::IntentOutputContract>,
    loop_state: &LoopState,
    replacement_answer: &str,
    finalizer_summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(current_delivery) = current_user_visible_delivery_text(loop_state) else {
        return false;
    };
    if current_delivery.trim() == replacement_answer.trim() {
        return false;
    }
    if finalizer_summary_rejects_current_delivery(finalizer_summary) {
        return false;
    }
    if latest_tail_replacement_can_recover_stale_synthesis(loop_state, current_delivery) {
        return false;
    }
    if route_requires_raw_tail_read_passthrough(route) {
        return false;
    }
    if latest_tail_replacement_was_synthesized_after_tail(loop_state, current_delivery) {
        return true;
    }
    if latest_tail_replacement_was_synthesized_after_tail(loop_state, replacement_answer) {
        return false;
    }
    if looks_like_structured_machine_output(current_delivery) {
        return false;
    }
    if current_delivery_is_latest_registered_output(loop_state, current_delivery) {
        return true;
    }
    false
}

fn finalizer_summary_rejects_current_delivery(
    summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(summary) = summary else {
        return false;
    };
    summary.disposition.is_some_and(|disposition| {
        disposition != crate::finalize::FinalizerDisposition::QualifiedCompletion
    }) || !summary.contract_ok
        || summary.completion_ok == Some(false)
        || summary.grounded_ok == Some(false)
        || summary.format_ok == Some(false)
}

pub(super) fn route_requires_raw_tail_read_passthrough(
    route: Option<&crate::IntentOutputContract>,
) -> bool {
    route
        .map(|route| {
            let contract = route.clone();
            route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
                && contract.response_shape == crate::OutputResponseShape::Strict
                && contract.requires_content_evidence
                && !contract.delivery_required
        })
        .unwrap_or(false)
}

pub(super) fn replace_delivery_with_latest_tail_read_range_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    if let Some((answer, summary)) =
        latest_tail_read_directory_inventory_answer(loop_state, agent_run_context)
    {
        loop_state.delivery_messages.clear();
        append_delivery_message(
            &task.task_id,
            &mut loop_state.delivery_messages,
            answer.clone(),
        );
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        log_deterministic_delivery_record(
            &task.task_id,
            "replace_with_tail_read_directory_inventory",
            "replaced",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return true;
    }
    let Some((answer, summary)) = latest_tail_read_range_observed_answer(
        state,
        task,
        user_text,
        loop_state,
        agent_run_context,
    )
    .or_else(|| {
        latest_bounded_read_range_failure_recovery_answer(
            loop_state,
            agent_run_context,
            finalizer_summary.as_ref(),
        )
    }) else {
        return false;
    };
    let route = agent_run_context.and_then(|ctx| ctx.output_contract());
    if let Some(summary) = route
        .and_then(|route| route_prefers_publishable_summary_over_tail_read(route, loop_state))
        .filter(|summary| summary.trim() != answer.trim())
    {
        loop_state.delivery_messages.clear();
        append_delivery_message(
            &task.task_id,
            &mut loop_state.delivery_messages,
            summary.clone(),
        );
        loop_state.last_user_visible_respond = Some(summary);
        *finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            parsed: true,
            contract_ok: true,
            completion_ok: Some(true),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: loop_state.executed_step_results.len().max(1),
            ..Default::default()
        });
        log_deterministic_delivery_record(
            &task.task_id,
            "preserve_publishable_summary_over_tail_read",
            "preserved",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return true;
    }
    if latest_tail_read_range_should_preserve_current_delivery(
        route,
        loop_state,
        &answer,
        finalizer_summary.as_ref(),
    ) {
        info!(
            "delivery keep_current_summary_over_tail_read_range task_id={}",
            task.task_id
        );
        log_deterministic_delivery_record(
            &task.task_id,
            "keep_current_summary_over_tail_read_range",
            "preserved",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return false;
    }
    if loop_state
        .delivery_messages
        .last()
        .map(|message| message.trim() == answer.trim())
        .unwrap_or(false)
    {
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        return true;
    }
    loop_state.delivery_messages.clear();
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.clone(),
    );
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(summary);
    log_deterministic_delivery_record(
        &task.task_id,
        "replace_with_latest_tail_read_range",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}
