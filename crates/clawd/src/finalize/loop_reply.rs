use std::path::{Path, PathBuf};

use tracing::info;

use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, AskReply, ClaimedTask};

fn contractual_last_respond_delivery_value(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    let contract = &route.output_contract;
    let answer = loop_state
        .last_user_visible_respond
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    let exact_single_line_observation =
        last_respond_matches_single_line_observation(loop_state, answer);
    if crate::agent_engine::observed_output::route_requires_synthesized_delivery(route)
        && !exact_single_line_observation
    {
        return None;
    }
    let has_explicit_answer_contract = contract.delivery_required
        || !matches!(contract.semantic_kind, crate::OutputSemanticKind::None)
        || matches!(
            contract.response_shape,
            crate::OutputResponseShape::Scalar
                | crate::OutputResponseShape::FileToken
                | crate::OutputResponseShape::OneSentence
                | crate::OutputResponseShape::Strict
        );
    if (!has_explicit_answer_contract && !exact_single_line_observation)
        || !loop_state.has_tool_or_skill_output
    {
        return None;
    }
    if crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
        || crate::finalize::is_execution_summary_message(answer)
        || looks_like_structured_machine_output(answer)
        || looks_like_raw_command_snapshot(answer)
    {
        return None;
    }
    match crate::output_contract_verifier::verify_output_contract(
        contract,
        answer,
        &route.resolved_intent,
    ) {
        crate::output_contract_verifier::OutputContractVerdict::Pass => Some(answer.to_string()),
        crate::output_contract_verifier::OutputContractVerdict::Reshape { reshaped, .. } => {
            Some(reshaped)
        }
        crate::output_contract_verifier::OutputContractVerdict::Reject { .. } => None,
    }
}

fn last_respond_matches_single_line_observation(loop_state: &LoopState, answer: &str) -> bool {
    let Some(body) = latest_successful_observation_body(loop_state) else {
        return false;
    };
    let mut lines = body.lines().map(str::trim).filter(|line| !line.is_empty());
    let Some(line) = lines.next() else {
        return false;
    };
    if lines.next().is_some() || answer.trim() != line {
        return false;
    }
    !looks_like_structured_machine_output(line)
        && !looks_like_raw_command_snapshot(line)
        && !crate::finalize::looks_like_planner_artifact(line)
        && !crate::finalize::looks_like_internal_trace_artifact(line)
}

fn backfill_delivery_from_last_outputs(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) {
    if loop_state.delivery_messages.is_empty() {
        if let Some(answer) = contractual_last_respond_delivery_value(loop_state, agent_run_context)
        {
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            info!(
                "final_result_use_contractual_last_respond task_id={} (delivery was empty)",
                task.task_id
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some(ref last_synthesis_output) = loop_state.last_publishable_synthesis_output {
            if !last_synthesis_output.trim().is_empty() {
                append_delivery_message(
                    &task.task_id,
                    &mut loop_state.delivery_messages,
                    last_synthesis_output.clone(),
                );
                info!(
                    "final_result_use_synthesis_output task_id={} (delivery was empty)",
                    task.task_id
                );
            }
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some(ref last_respond) = loop_state.last_user_visible_respond {
            if !last_respond.trim().is_empty() {
                append_delivery_message(
                    &task.task_id,
                    &mut loop_state.delivery_messages,
                    last_respond.clone(),
                );
                info!(
                    "final_result_use_last_respond task_id={} (delivery was empty)",
                    task.task_id
                );
            }
        }
    }
}

fn is_bare_template_placeholder(text: &str) -> bool {
    let trimmed = text.trim();
    if !trimmed.starts_with("{{") || !trimmed.ends_with("}}") {
        return false;
    }
    let inner = trimmed[2..trimmed.len() - 2].trim();
    !inner.is_empty() && !inner.contains("{{") && !inner.contains("}}")
}

fn replace_placeholder_delivery_with_synthesis(task: &ClaimedTask, loop_state: &mut LoopState) {
    let Some(synthesis) = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return;
    };
    let Some(last_delivery) = loop_state.delivery_messages.last().map(String::as_str) else {
        return;
    };
    if !is_bare_template_placeholder(last_delivery) {
        return;
    }
    info!(
        "final_result_replace_placeholder_delivery_with_synthesis task_id={} placeholder={}",
        task.task_id,
        crate::truncate_for_log(last_delivery)
    );
    loop_state.delivery_messages.pop();
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        synthesis.to_string(),
    );
    loop_state.last_user_visible_respond = Some(synthesis.to_string());
}

fn route_requires_content_evidence(agent_run_context: Option<&AgentRunContext>) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.output_contract.requires_content_evidence)
        .unwrap_or(false)
}

fn preferred_route_clarify_question(agent_run_context: Option<&AgentRunContext>) -> Option<&str> {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .filter(|route| route.needs_clarify)
        .map(|route| route.clarify_question.trim())
        .filter(|question| !question.is_empty())
}

fn route_requires_file_token(agent_run_context: Option<&AgentRunContext>) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| {
            route.output_contract.delivery_required
                || matches!(
                    route.output_contract.response_shape,
                    crate::OutputResponseShape::FileToken
                )
        })
        .unwrap_or(false)
}

pub(crate) fn output_excerpt_has_missing_file_evidence(output: &str) -> bool {
    if output.trim().eq_ignore_ascii_case("NOT_FOUND") {
        return true;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
        return false;
    };
    let locator_found_nothing = value
        .get("action")
        .and_then(|v| v.as_str())
        .is_some_and(|action| matches!(action, "find_name" | "find_path"))
        && value.get("count").and_then(|v| v.as_i64()) == Some(0)
        && ["results", "matches"].iter().any(|field| {
            value
                .get(field)
                .and_then(|v| v.as_array())
                .is_some_and(|items| items.is_empty())
        });
    if locator_found_nothing {
        return true;
    }

    let path_facts = value.get("facts").and_then(|v| v.as_array());
    let has_path_batch_shape = value.get("action").and_then(|v| v.as_str())
        == Some("path_batch_facts")
        || path_facts.is_some();
    has_path_batch_shape
        && path_facts.is_some_and(|facts| {
            facts.iter().any(|fact| {
                fact.get("exists").and_then(|v| v.as_bool()) == Some(false)
                    && fact
                        .get("path")
                        .and_then(|v| v.as_str())
                        .is_some_and(|path| !path.trim().is_empty())
            })
        })
}

fn has_missing_file_search_evidence(loop_state: &LoopState) -> bool {
    loop_state.executed_step_results.iter().rev().any(|step| {
        if step
            .output
            .as_deref()
            .is_some_and(output_excerpt_has_missing_file_evidence)
        {
            return true;
        }
        step_error_has_missing_file_evidence(step)
    })
}

fn step_error_has_missing_file_evidence(step: &crate::executor::StepExecutionResult) -> bool {
    let Some(error) = step
        .error
        .as_deref()
        .map(str::trim)
        .filter(|error| !error.is_empty())
    else {
        return false;
    };
    if error.starts_with("__RC_READ_FILE_NOT_FOUND__:") {
        return true;
    }
    crate::skills::parse_structured_skill_error(error).is_some_and(|structured| {
        structured.error_kind == "not_found"
            && matches!(
                structured.skill.as_str(),
                "read_file" | "system_basic" | "fs_search"
            )
    })
}

fn output_excerpt_has_existing_file_evidence(output: &str) -> bool {
    if crate::finalize::parse_delivery_file_token(output.trim()).is_some() {
        return true;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
        return false;
    };
    if value.get("action").and_then(|v| v.as_str()) == Some("find_name") {
        return value.get("count").and_then(|v| v.as_i64()).unwrap_or(0) > 0
            && value
                .get("results")
                .and_then(|v| v.as_array())
                .is_some_and(|results| !results.is_empty());
    }
    let path_facts = value.get("facts").and_then(|v| v.as_array());
    let has_path_batch_shape = value.get("action").and_then(|v| v.as_str())
        == Some("path_batch_facts")
        || path_facts.is_some();
    has_path_batch_shape
        && path_facts.is_some_and(|facts| {
            facts.iter().any(|fact| {
                fact.get("exists").and_then(|v| v.as_bool()) == Some(true)
                    && missing_path_from_path_fact(fact).is_some()
            })
        })
}

fn latest_file_delivery_observation_is_missing(loop_state: &LoopState) -> bool {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer"
            )
        })
        .find_map(|step| {
            if step_error_has_missing_file_evidence(step) {
                Some(true)
            } else if step
                .output
                .as_deref()
                .is_some_and(output_excerpt_has_missing_file_evidence)
            {
                Some(true)
            } else if step
                .output
                .as_deref()
                .is_some_and(output_excerpt_has_existing_file_evidence)
            {
                Some(false)
            } else {
                None
            }
        })
        .unwrap_or(false)
}

fn loop_has_existing_delivery_file_token(loop_state: &LoopState) -> bool {
    loop_state
        .last_user_visible_respond
        .as_deref()
        .into_iter()
        .chain(loop_state.delivery_messages.iter().map(String::as_str))
        .any(|message| {
            crate::finalize::parse_delivery_file_token(message.trim())
                .map(|(_, path)| Path::new(path.trim()).exists())
                .unwrap_or(false)
        })
}

fn should_return_missing_file_delivery_reply(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    route_requires_file_token(agent_run_context)
        && latest_file_delivery_observation_is_missing(loop_state)
        && !loop_has_existing_delivery_file_token(loop_state)
}

fn route_locator_hint(agent_run_context: Option<&AgentRunContext>) -> Option<&str> {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.output_contract.locator_hint.trim())
        .filter(|value| !value.is_empty())
}

fn missing_path_from_path_fact(fact: &serde_json::Value) -> Option<String> {
    let fact_obj = fact.get("fact").and_then(|value| value.as_object());
    fact_obj
        .and_then(|item| item.get("resolved_path"))
        .or_else(|| fact_obj.and_then(|item| item.get("path")))
        .or_else(|| fact.get("resolved_path"))
        .or_else(|| fact.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
}

fn missing_file_path_from_output(output: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(output).ok()?;
    let facts = value.get("facts").and_then(|value| value.as_array())?;
    facts.iter().find_map(|fact| {
        (fact.get("exists").and_then(|value| value.as_bool()) == Some(false))
            .then(|| missing_path_from_path_fact(fact))
            .flatten()
    })
}

fn missing_file_path_from_loop(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find_map(|step| {
            step.output
                .as_deref()
                .and_then(missing_file_path_from_output)
                .or_else(|| missing_file_path_from_step_error(step))
        })
        .or_else(|| route_locator_hint(agent_run_context).map(ToString::to_string))
}

fn missing_file_path_from_step_error(
    step: &crate::executor::StepExecutionResult,
) -> Option<String> {
    let error = step.error.as_deref()?.trim();
    if let Some(path) = error
        .strip_prefix("__RC_READ_FILE_NOT_FOUND__:")
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        return Some(path.to_string());
    }
    let structured = crate::skills::parse_structured_skill_error(error)?;
    if structured.error_kind != "not_found" {
        return None;
    }
    structured
        .extra
        .as_ref()
        .and_then(|extra| extra.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
}

fn resolve_file_token_from_auto_locator_answer(
    answer: &str,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let trimmed = answer.trim();
    if trimmed.is_empty()
        || trimmed.contains('\n')
        || crate::finalize::parse_delivery_file_token(trimmed).is_some()
    {
        return None;
    }
    let auto_locator_path = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    let auto_path = Path::new(auto_locator_path);

    let resolved = if auto_path.is_file() {
        let file_name = auto_path.file_name().and_then(|v| v.to_str())?;
        if trimmed != file_name {
            return None;
        }
        auto_path
            .canonicalize()
            .unwrap_or_else(|_| auto_path.to_path_buf())
    } else if auto_path.is_dir() {
        let candidate = auto_path.join(trimmed);
        if !candidate.is_file() {
            return None;
        }
        candidate
            .canonicalize()
            .unwrap_or_else(|_| candidate.to_path_buf())
    } else {
        return None;
    };

    Some(format!("FILE:{}", resolved.display()))
}

fn normalize_file_token_delivery_from_auto_locator(
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) {
    if !route_requires_file_token(agent_run_context) {
        return;
    }
    let auto_locator_path = agent_run_context.and_then(|ctx| ctx.auto_locator_path.as_deref());

    if let Some(token) = loop_state
        .last_user_visible_respond
        .as_deref()
        .and_then(|answer| resolve_file_token_from_auto_locator_answer(answer, auto_locator_path))
    {
        loop_state.last_user_visible_respond = Some(token);
    }

    for message in &mut loop_state.delivery_messages {
        if let Some(token) = resolve_file_token_from_auto_locator_answer(message, auto_locator_path)
        {
            *message = token;
        }
    }
}

fn direct_file_token_from_observed_auto_locator_filename(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_requires_file_token(agent_run_context) {
        return None;
    }
    let auto_locator_path = agent_run_context.and_then(|ctx| ctx.auto_locator_path.as_deref())?;
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok()
            || matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer"
            )
        {
            continue;
        }
        let Some(output) = step
            .output
            .as_deref()
            .map(str::trim)
            .filter(|output| !output.is_empty())
        else {
            continue;
        };
        let Some(token) =
            resolve_file_token_from_auto_locator_answer(output, Some(auto_locator_path))
        else {
            continue;
        };
        return Some((
            token,
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                ..Default::default()
            },
        ));
    }
    None
}

fn bare_delivery_filename(answer: &str) -> Option<&str> {
    let trimmed = answer.trim();
    if trimmed.is_empty() || trimmed.contains('\n') {
        return None;
    }
    let payload = crate::finalize::parse_delivery_file_token(trimmed)
        .map(|(_, payload)| payload.trim())
        .unwrap_or(trimmed);
    if payload.is_empty()
        || payload.contains('/')
        || payload.contains('\\')
        || Path::new(payload).is_absolute()
    {
        return None;
    }
    Some(payload)
}

fn observed_file_path_for_payload(
    state: &AppState,
    raw_path: &str,
    payload: &str,
) -> Option<PathBuf> {
    let raw_path = raw_path.trim();
    if raw_path.is_empty() {
        return None;
    }
    let path = Path::new(raw_path);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(path)
    };
    let file_name = candidate.file_name()?.to_string_lossy();
    if file_name != payload {
        return None;
    }
    if !candidate.is_file() {
        return None;
    }
    Some(candidate.canonicalize().unwrap_or(candidate))
}

fn collect_observed_file_paths(
    state: &AppState,
    value: &serde_json::Value,
    payload: &str,
    out: &mut Vec<PathBuf>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if matches!(key.as_str(), "path" | "resolved_path") {
                    if let Some(raw_path) = child.as_str() {
                        if let Some(path) = observed_file_path_for_payload(state, raw_path, payload)
                        {
                            out.push(path);
                        }
                    }
                }
                collect_observed_file_paths(state, child, payload, out);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                collect_observed_file_paths(state, child, payload, out);
            }
        }
        _ => {}
    }
}

fn resolve_file_token_from_observed_paths(
    state: &AppState,
    answer: &str,
    loop_state: &LoopState,
) -> Option<String> {
    let payload = bare_delivery_filename(answer)?;
    let mut matches = Vec::new();
    for step in loop_state.executed_step_results.iter().rev() {
        let Some(output) = step.output.as_deref() else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
            continue;
        };
        collect_observed_file_paths(state, &value, payload, &mut matches);
    }
    matches.sort();
    matches.dedup();
    if matches.len() == 1 {
        Some(format!("FILE:{}", matches[0].display()))
    } else {
        None
    }
}

fn normalize_file_token_delivery_from_observed_paths(
    state: &AppState,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) {
    if !route_requires_file_token(agent_run_context) {
        return;
    }

    if let Some(token) = loop_state
        .last_user_visible_respond
        .as_deref()
        .and_then(|answer| resolve_file_token_from_observed_paths(state, answer, loop_state))
    {
        loop_state.last_user_visible_respond = Some(token);
    }

    let replacements = loop_state
        .delivery_messages
        .iter()
        .map(|message| {
            resolve_file_token_from_observed_paths(state, message, loop_state)
                .unwrap_or_else(|| message.clone())
        })
        .collect::<Vec<_>>();
    loop_state.delivery_messages = replacements;
}

fn planned_file_delivery_used_unresolved_runtime_placeholder(loop_state: &LoopState) -> bool {
    loop_state
        .round_traces
        .iter()
        .rev()
        .filter_map(|round| round.plan_result.as_ref())
        .any(|plan| {
            plan.steps.iter().any(|step| {
                if step.action_type != "respond" {
                    return false;
                }
                let content = step
                    .args
                    .get("content")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .unwrap_or_default();
                crate::finalize::parse_delivery_token(content).is_some()
                    && content.contains("{{")
                    && content.contains("}}")
            }) || {
                let raw = plan.raw_plan_text.as_str();
                raw.contains("FILE:") && raw.contains("{{") && raw.contains("}}")
            }
        })
}

fn inventory_root_path(value: &serde_json::Value) -> Option<PathBuf> {
    value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

fn inventory_ranked_for_single_file_selection(value: &serde_json::Value) -> bool {
    value
        .get("sort_by")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .is_some_and(|sort_by| {
            matches!(
                sort_by,
                "mtime_desc" | "mtime_asc" | "size_desc" | "size_asc"
            )
        })
}

fn inventory_has_deterministic_order(value: &serde_json::Value) -> bool {
    value
        .get("sort_by")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .is_some_and(|sort_by| {
            matches!(
                sort_by,
                "name" | "mtime_desc" | "mtime_asc" | "size_desc" | "size_asc"
            )
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlannedInventorySelection {
    First,
    Last,
}

fn planned_inventory_selection_from_template_text(text: &str) -> Option<PlannedInventorySelection> {
    let mut rest = text;
    let mut selection = None;
    while let Some(start) = rest.find("{{") {
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("}}") else {
            break;
        };
        let expression = after_start[..end]
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        rest = &after_start[end + 2..];

        if !expression.contains("last_output") {
            continue;
        }
        let next = if expression.contains(".last(")
            || expression.contains("[-1]")
            || expression.contains(".rev().next(")
        {
            PlannedInventorySelection::Last
        } else if expression.contains(".first(")
            || expression.contains("[0]")
            || expression.contains(".next(")
        {
            PlannedInventorySelection::First
        } else {
            continue;
        };
        if selection.is_some_and(|existing| existing != next) {
            return None;
        }
        selection = Some(next);
    }
    selection
}

fn planned_file_delivery_inventory_selection(
    loop_state: &LoopState,
) -> Option<PlannedInventorySelection> {
    for plan in loop_state
        .round_traces
        .iter()
        .rev()
        .filter_map(|round| round.plan_result.as_ref())
    {
        for step in &plan.steps {
            if step.action_type != "respond" {
                continue;
            }
            let Some(content) = step
                .args
                .get("content")
                .and_then(|value| value.as_str())
                .map(str::trim)
            else {
                continue;
            };
            if crate::finalize::parse_delivery_token(content).is_some()
                || (content.contains("FILE:") && content.contains("{{"))
            {
                if let Some(selection) = planned_inventory_selection_from_template_text(content) {
                    return Some(selection);
                }
            }
        }
        let raw = plan.raw_plan_text.as_str();
        if raw.contains("FILE:") && raw.contains("{{") && raw.contains("}}") {
            if let Some(selection) = planned_inventory_selection_from_template_text(raw) {
                return Some(selection);
            }
        }
    }
    None
}

fn inventory_candidate_names(value: &serde_json::Value) -> Vec<String> {
    if let Some(names) = value.get("names").and_then(|value| value.as_array()) {
        return names
            .iter()
            .filter_map(|value| value.as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(ToString::to_string)
            .collect();
    }
    value
        .get("entries")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry
                .get("kind")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .map(|kind| kind == "file")
                .unwrap_or(true)
        })
        .filter_map(|entry| entry.get("name").and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn observed_inventory_file_candidates(value: &serde_json::Value) -> Option<Vec<PathBuf>> {
    if value.get("action").and_then(|value| value.as_str()) != Some("inventory_dir") {
        return None;
    }
    let root = inventory_root_path(value)?;
    let mut candidates = Vec::new();
    for name in inventory_candidate_names(value) {
        let name_path = Path::new(&name);
        let candidate = if name_path.is_absolute() {
            name_path.to_path_buf()
        } else {
            root.join(name_path)
        };
        if candidate.is_file() {
            candidates.push(candidate.canonicalize().unwrap_or(candidate));
        }
    }
    (!candidates.is_empty()).then_some(candidates)
}

fn direct_file_token_from_observed_inventory(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_requires_file_token(agent_run_context) {
        return None;
    }
    let malformed_placeholder_delivery =
        planned_file_delivery_used_unresolved_runtime_placeholder(loop_state);
    let planned_inventory_selection = planned_file_delivery_inventory_selection(loop_state);
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok()
            || !matches!(
                step.skill.as_str(),
                "fs_basic" | "system_basic" | "list_dir"
            )
        {
            continue;
        }
        let Some(output) = step
            .output
            .as_deref()
            .map(str::trim)
            .filter(|output| !output.is_empty())
        else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
            continue;
        };
        let Some(candidates) = observed_inventory_file_candidates(&value) else {
            continue;
        };
        let selected = if candidates.len() == 1 {
            candidates.first()
        } else if planned_inventory_selection.is_some() && inventory_has_deterministic_order(&value)
        {
            match planned_inventory_selection? {
                PlannedInventorySelection::First => candidates.first(),
                PlannedInventorySelection::Last => candidates.last(),
            }
        } else if malformed_placeholder_delivery
            && inventory_ranked_for_single_file_selection(&value)
        {
            candidates.first()
        } else {
            None
        }?;
        return Some((
            format!("FILE:{}", selected.display()),
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                ..Default::default()
            },
        ));
    }
    None
}

async fn enforce_delivery_output_contract(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return;
    };
    if loop_state.delivery_messages.is_empty()
        && loop_state
            .last_user_visible_respond
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
    {
        return;
    }
    let publishable_synthesis = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let seed_text = if publishable_synthesis.is_some() {
        loop_state
            .delivery_messages
            .last()
            .cloned()
            .or_else(|| loop_state.last_publishable_synthesis_output.clone())
            .or_else(|| loop_state.last_user_visible_respond.clone())
            .unwrap_or_default()
    } else {
        loop_state
            .last_user_visible_respond
            .clone()
            .or_else(|| loop_state.delivery_messages.last().cloned())
            .unwrap_or_default()
    };
    let (mut normalized_text, mut normalized_messages) =
        crate::intercept_response_payload_for_delivery(
            state,
            user_text,
            route.wants_file_delivery,
            &route.output_contract,
            seed_text,
            loop_state.delivery_messages.clone(),
        );

    // §7.1 output_contract verifier hook：在 enforce_output_contract 的"shape 整形"
    // 之后再做一层最小结构合规性判定。不要在这里用自然语言词表判断 yes/no、
    // same/different、语气或意图；这些交给 LLM composer/prompt。
    // 三态结果：
    // - Pass：已合规，原文直出。
    // - Reshape：候选基本合规但可结构化抽取严格值（如 scalar path/count），verifier
    //   给出已修复文本，直接覆盖 normalized_text。
    // - Reject：候选明显违反结构 contract（如 strict scalar 缺路径/整数），走 §7.2
    //   ClarifyFallbackSource::VerifyRejected fallback，丢弃 candidate。
    // 三种情况都打 tracing event verify_contract_emitted，便于 inspect_task.sh 关联。
    if !normalized_text.trim().is_empty() {
        let verdict = crate::output_contract_verifier::verify_output_contract(
            &route.output_contract,
            &normalized_text,
            user_text,
        );
        match &verdict {
            crate::output_contract_verifier::OutputContractVerdict::Pass => {
                info!(
                    "verify_contract_emitted task_id={} verdict=pass response_shape={:?} semantic_kind={:?}",
                    task.task_id,
                    route.output_contract.response_shape,
                    route.output_contract.semantic_kind,
                );
            }
            crate::output_contract_verifier::OutputContractVerdict::Reshape {
                reason,
                reshaped,
            } => {
                info!(
                    "verify_contract_emitted task_id={} verdict=reshape response_shape={:?} semantic_kind={:?} reason={} from={} to={}",
                    task.task_id,
                    route.output_contract.response_shape,
                    route.output_contract.semantic_kind,
                    reason,
                    crate::truncate_for_log(&normalized_text),
                    crate::truncate_for_log(reshaped),
                );
                normalized_text = reshaped.clone();
                if let Some(last) = normalized_messages.last_mut() {
                    *last = reshaped.clone();
                } else {
                    normalized_messages.push(reshaped.clone());
                }
            }
            crate::output_contract_verifier::OutputContractVerdict::Reject { reason } => {
                info!(
                    "verify_contract_emitted task_id={} verdict=reject response_shape={:?} semantic_kind={:?} reason={} dropped_candidate={}",
                    task.task_id,
                    route.output_contract.response_shape,
                    route.output_contract.semantic_kind,
                    reason,
                    crate::truncate_for_log(&normalized_text),
                );
                let language_hint =
                    crate::language_policy::task_response_language_hint(state, task, user_text);
                let contract = crate::fallback::UserResponseContract::verify_rejected(
                    user_text,
                    &route.resolved_intent,
                    &format!("{:?}", route.output_contract.response_shape),
                    &format!("{:?}", route.output_contract.semantic_kind),
                    reason,
                    &language_hint,
                );
                let fallback_text = crate::fallback::compose_user_response_from_contract(
                    state,
                    task,
                    &contract,
                    crate::fallback::ClarifyFallbackSource::VerifyRejected,
                );
                let fallback_text = fallback_text.await;
                normalized_text = fallback_text.clone();
                normalized_messages = vec![fallback_text];
            }
        }
    }

    loop_state.last_user_visible_respond =
        (!normalized_text.trim().is_empty()).then_some(normalized_text);
    loop_state.delivery_messages = normalized_messages;
}

async fn discard_meta_respond_placeholder_for_content_evidence(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    requires_content_evidence: bool,
    agent_run_context: Option<&AgentRunContext>,
) {
    let Some(last_respond) = loop_state.last_user_visible_respond.as_deref() else {
        return;
    };
    let respond = last_respond.trim();
    let Some(raw_passthrough) = should_drop_passthrough_delivery_for_content_evidence(
        loop_state,
        requires_content_evidence,
        agent_run_context,
        respond,
    ) else {
        return;
    };
    if !raw_passthrough
        && content_evidence_terminal_respond_is_contractual_answer(
            loop_state,
            agent_run_context,
            respond,
        )
    {
        info!(
            "content_evidence_keep_contractual_terminal_respond task_id={} text={}",
            task.task_id,
            crate::truncate_for_log(respond)
        );
        return;
    }
    // §3.4 finalize-tier: drop_passthrough_delivery 是 finalize 决策层。
    let meta_placeholder =
        crate::semantic_judge::is_meta_respond_instruction(state, task, respond).await;
    if !raw_passthrough && !meta_placeholder {
        return;
    }
    info!(
        "content_evidence_drop_passthrough_respond task_id={} raw_passthrough={} meta_placeholder={} text={}",
        task.task_id,
        raw_passthrough,
        meta_placeholder,
        crate::truncate_for_log(respond)
    );
    loop_state.delivery_messages.clear();
    loop_state.last_user_visible_respond = None;
}

fn should_drop_passthrough_delivery_for_content_evidence(
    loop_state: &LoopState,
    requires_content_evidence: bool,
    agent_run_context: Option<&AgentRunContext>,
    respond: &str,
) -> Option<bool> {
    if loop_state.pending_user_input_required {
        return None;
    }
    if !requires_content_evidence {
        return None;
    }
    if !loop_state.has_tool_or_skill_output {
        return None;
    }
    if loop_state.delivery_messages.len() != 1 {
        return None;
    }
    let delivery = loop_state.delivery_messages[0].trim();
    let respond = respond.trim();
    if delivery.is_empty() || respond.is_empty() || delivery != respond {
        return None;
    }

    let route_has_semantic_answer_contract = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            route.output_contract.semantic_kind != crate::OutputSemanticKind::None
        });
    let direct_structured_answer = route_has_semantic_answer_contract
        .then(|| direct_structured_observed_answer(None, loop_state, agent_run_context))
        .flatten()
        .map(|(answer, _)| answer);
    let direct_observed_answer_matches =
        direct_scalar_observed_answer(None, loop_state, agent_run_context)
            .map(|(answer, _)| answer)
            .into_iter()
            .chain(direct_structured_answer)
            .any(|answer| answer.trim() == respond);
    if direct_observed_answer_matches {
        return Some(false);
    }
    if last_respond_matches_single_line_observation(loop_state, respond) {
        return Some(false);
    }

    let raw_passthrough = loop_state
        .executed_step_results
        .iter()
        .rfind(|step| {
            step.is_ok() && !matches!(step.skill.as_str(), "respond" | "synthesize_answer")
        })
        .and_then(|step| {
            let body = step.output.as_deref()?.trim();
            if body.is_empty() {
                return None;
            }
            if respond == body {
                return Some(true);
            }
            (step.skill == "list_dir"
                && crate::agent_engine::observed_output::normalized_observed_listing(body)
                    .is_some_and(|listing| {
                        listing.trim() == respond
                            || listing
                                .lines()
                                .map(str::trim)
                                .any(|entry| !entry.is_empty() && entry == respond)
                    }))
            .then_some(true)
        })
        .unwrap_or(false);
    Some(raw_passthrough)
}

fn content_evidence_terminal_respond_is_contractual_answer(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    respond: &str,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route.output_contract.requires_content_evidence {
        return false;
    }
    if !matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free
            | crate::OutputResponseShape::OneSentence
            | crate::OutputResponseShape::Strict
    ) {
        return false;
    }
    if matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
    ) {
        return false;
    }
    let has_answer_semantic = !matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    let has_constrained_answer_shape = matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Strict
    );
    if !has_answer_semantic && !has_constrained_answer_shape {
        return false;
    }
    let answer = respond.trim();
    if answer.is_empty()
        || answer.chars().count() > 800
        || crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
        || crate::finalize::is_execution_summary_message(answer)
        || looks_like_structured_machine_output(answer)
        || looks_like_raw_command_snapshot(answer)
    {
        return false;
    }
    if crate::finalize::parse_delivery_token(answer).is_some() {
        return true;
    }
    let has_successful_observation = loop_state.executed_step_results.iter().any(|step| {
        step.is_ok()
            && !matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer"
            )
            && step
                .output
                .as_deref()
                .map(str::trim)
                .is_some_and(|output| !output.is_empty())
    });
    if !has_successful_observation {
        return false;
    }
    !matches!(
        crate::output_contract_verifier::verify_output_contract(
            &route.output_contract,
            answer,
            &route.resolved_intent,
        ),
        crate::output_contract_verifier::OutputContractVerdict::Reject { .. }
    )
}

fn discard_raw_passthrough_delivery_when_structured_answer_available(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) {
    if loop_state.pending_user_input_required {
        return;
    }
    if loop_state.delivery_messages.len() != 1 {
        return;
    }
    let Some(current_delivery) = loop_state.delivery_messages.last().map(|v| v.trim()) else {
        return;
    };
    if current_delivery.is_empty() {
        return;
    }
    let raw_passthrough = loop_state
        .executed_step_results
        .iter()
        .rfind(|step| {
            step.is_ok() && !matches!(step.skill.as_str(), "respond" | "synthesize_answer")
        })
        .and_then(|step| {
            let body = step.output.as_deref()?.trim();
            if body.is_empty() {
                return None;
            }
            if current_delivery == body {
                return Some(true);
            }
            let first_line = body.lines().map(str::trim).find(|line| !line.is_empty())?;
            (current_delivery == first_line).then_some(true)
        })
        .unwrap_or(false);
    if !raw_passthrough {
        return;
    }
    if last_respond_matches_single_line_observation(loop_state, current_delivery) {
        return;
    }

    let structured_answer = direct_structured_observed_answer(None, loop_state, agent_run_context)
        .map(|(answer, _)| answer.trim().to_string())
        .filter(|answer| !answer.is_empty() && answer != current_delivery);

    let exact_delivery_requested = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(output_contract_requests_exact_delivery)
        .unwrap_or(false);
    if structured_answer.is_none()
        && (exact_delivery_requested
            || !crate::agent_engine::observed_output::has_observed_answer_candidates(loop_state))
    {
        return;
    }

    info!(
        "drop_raw_passthrough_delivery_for_structured_answer task_id={} raw={} structured={}",
        task.task_id,
        crate::truncate_for_log(current_delivery),
        crate::truncate_for_log(structured_answer.as_deref().unwrap_or("<synthesis>"))
    );
    loop_state.delivery_messages.clear();
    loop_state.last_user_visible_respond = None;
}

fn direct_scalar_observed_answer(
    state: Option<&AppState>,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if !route_allows_direct_scalar_observed_answer(route) {
        return None;
    }
    if let Some(answer) = state.and_then(|state| {
        let user_text = route.resolved_intent.trim();
        deterministic_missing_observed_target_answer(
            state,
            user_text,
            loop_state,
            agent_run_context,
        )
    }) {
        return Some((
            answer,
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                ..Default::default()
            },
        ));
    }
    let answer =
        if crate::agent_engine::observed_output::scalar_route_prefers_structured_observed_answer(
            route, loop_state,
        ) {
            state
            .and_then(|state| {
                crate::agent_engine::observed_output::extract_direct_answer_from_generic_output_i18n(
                    loop_state,
                    state,
                    agent_run_context,
                )
            })
            .or_else(|| {
                crate::agent_engine::observed_output::extract_direct_answer_from_generic_output(
                    loop_state,
                    agent_run_context,
                )
            })
            .or_else(|| {
                state.and_then(|state| {
                    crate::agent_engine::observed_output::extract_direct_scalar_from_generic_output_i18n(
                        loop_state,
                        state,
                        agent_run_context,
                    )
                })
            })
            .or_else(|| {
                crate::agent_engine::observed_output::extract_direct_scalar_from_generic_output(
                    loop_state,
                    agent_run_context,
                )
            })?
        } else {
            state
            .and_then(|state| {
                crate::agent_engine::observed_output::extract_direct_scalar_from_generic_output_i18n(
                    loop_state,
                    state,
                    agent_run_context,
                )
            })
            .or_else(|| {
                crate::agent_engine::observed_output::extract_direct_scalar_from_generic_output(
                    loop_state,
                    agent_run_context,
                )
            })?
        };
    if crate::finalize::looks_like_planner_artifact(&answer)
        || crate::finalize::looks_like_internal_trace_artifact(&answer)
    {
        return None;
    }
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            contract_ok: true,
            ..Default::default()
        },
    ))
}

fn latest_scalar_observed_answer_from_loop_contract(
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let contract = loop_state.output_contract.as_ref()?;
    if contract.response_shape != crate::OutputResponseShape::Scalar {
        return None;
    }
    let body = latest_successful_observation_body(loop_state)?;
    let mut lines = body.lines().map(str::trim).filter(|line| !line.is_empty());
    let answer = lines.next()?;
    if lines.next().is_some() {
        return None;
    }
    if answer.is_empty()
        || crate::finalize::parse_delivery_token(answer).is_some()
        || crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
        || looks_like_structured_machine_output(answer)
        || crate::finalize::is_execution_summary_message(answer)
    {
        return None;
    }
    Some((
        answer.to_string(),
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            contract_ok: true,
            used_evidence_ids_count: loop_state.executed_step_results.len(),
            ..Default::default()
        },
    ))
}

fn latest_successful_observation_body(loop_state: &LoopState) -> Option<&str> {
    loop_state
        .executed_step_results
        .iter()
        .rfind(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
        })
        .filter(|step| step.is_ok())
        .and_then(|step| step.output.as_deref())
}

fn latest_path_observed_answer_from_loop_contract(
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let contract = loop_state.output_contract.as_ref()?;
    if !matches!(
        contract.semantic_kind,
        crate::OutputSemanticKind::FilePaths
            | crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::DirectoryEntryGroups
    ) {
        return None;
    }
    let body = latest_successful_observation_body(loop_state)?.trim();
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let results = value.get("results").and_then(serde_json::Value::as_array)?;
    if results.len() != 1 {
        return None;
    }
    let answer = results
        .first()
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    if crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
    {
        return None;
    }
    Some((
        answer.to_string(),
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            contract_ok: true,
            used_evidence_ids_count: loop_state.executed_step_results.len(),
            ..Default::default()
        },
    ))
}

#[derive(Clone, Copy)]
enum LoopContractObservedAnswerKind {
    Scalar,
    PathList,
}

fn loop_contract_observed_answer_satisfies_required_evidence(
    loop_state: &LoopState,
    answer_kind: LoopContractObservedAnswerKind,
) -> bool {
    let Some(output_contract) = loop_state.output_contract.as_ref() else {
        return false;
    };
    let required_fields =
        crate::task_contract::required_evidence_fields_for_output_contract(output_contract);
    if required_fields.is_empty() {
        return true;
    }
    required_fields.iter().all(|field| match field.as_str() {
        "field_value" | "count" | "command_output" => {
            matches!(answer_kind, LoopContractObservedAnswerKind::Scalar)
        }
        "candidates" | "path" => matches!(answer_kind, LoopContractObservedAnswerKind::PathList),
        _ => false,
    })
}

fn replace_delivery_with_loop_contract_observed_answer(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    if loop_state
        .delivery_messages
        .last()
        .is_some_and(|message| delivery_message_is_json_object(message))
    {
        return false;
    }
    let Some((answer, summary, answer_kind)) =
        latest_scalar_observed_answer_from_loop_contract(loop_state)
            .map(|(answer, summary)| (answer, summary, LoopContractObservedAnswerKind::Scalar))
            .or_else(|| {
                latest_path_observed_answer_from_loop_contract(loop_state).map(
                    |(answer, summary)| (answer, summary, LoopContractObservedAnswerKind::PathList),
                )
            })
    else {
        return false;
    };
    if !loop_contract_observed_answer_satisfies_required_evidence(loop_state, answer_kind) {
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
    loop_state
        .delivery_messages
        .retain(|message| crate::finalize::is_execution_summary_message(message));
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.clone(),
    );
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(summary);
    info!(
        "delivery replace_with_loop_contract_observed task_id={}",
        task.task_id
    );
    true
}

fn delivery_message_is_json_object(message: &str) -> bool {
    matches!(
        serde_json::from_str::<serde_json::Value>(message.trim()),
        Ok(serde_json::Value::Object(_))
    )
}

fn prefer_english_for_user_text(state: &AppState, user_text: &str) -> bool {
    match crate::language_policy::request_language_hint(user_text) {
        "zh-CN" => false,
        "en" => true,
        _ => state
            .policy
            .command_intent
            .default_locale
            .to_ascii_lowercase()
            .starts_with("en"),
    }
}

fn route_resolved_intent(agent_run_context: Option<&AgentRunContext>) -> String {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.resolved_intent.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
}

fn execution_recipe_budget_exhausted_default_message(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
) -> String {
    let prefer_english = prefer_english_for_user_text(state, user_text);
    let repair_count = loop_state.execution_recipe.repair_count.to_string();
    let max_repairs = loop_state.execution_recipe.max_repairs.to_string();
    crate::bilingual_t_with_default_vars(
        state,
        "clawd.msg.execution_recipe_repair_budget_exhausted",
        "我已经按闭环流程继续检查、应用和验证，但修复次数已达到上限（{repair_count}/{max_repairs}），当前还没有验证通过。",
        "I kept iterating through inspect, apply, and validation, but the repair budget is exhausted ({repair_count}/{max_repairs}) and the result is still not validated.",
        prefer_english,
        &[("repair_count", &repair_count), ("max_repairs", &max_repairs)],
    )
}

async fn execution_recipe_budget_exhausted_message(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> String {
    let default_text =
        execution_recipe_budget_exhausted_default_message(state, user_text, loop_state);
    let repair_count = loop_state.execution_recipe.repair_count.to_string();
    let max_repairs = loop_state.execution_recipe.max_repairs.to_string();
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let contract = crate::fallback::UserResponseContract::tool_failure(
        "execution_recipe_repair_budget_exhausted",
        user_text,
        &route_resolved_intent(agent_run_context),
        vec![
            "closed_loop_stage: inspect/apply/validate".to_string(),
            format!("repair_count: {repair_count}"),
            format!("max_repairs: {max_repairs}"),
            "result_validated: false".to_string(),
        ],
        vec![
            "Do not mark the run as successful.".to_string(),
            "Do not claim validation passed.".to_string(),
            "Explain the blocker and ask for permission to continue with a different approach or more context."
                .to_string(),
        ],
        "brief_failure_with_next_step",
        &language_hint,
    );
    crate::fallback::compose_user_response_from_contract_with_default(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::ExecutionFailedPartial,
        &default_text,
    )
    .await
}

fn execution_recipe_missing_success_marker_default_message(
    state: &AppState,
    user_text: &str,
    marker: &str,
) -> String {
    let prefer_english = prefer_english_for_user_text(state, user_text);
    crate::bilingual_t_with_default_vars(
        state,
        "clawd.msg.execution_recipe_missing_success_marker",
        "这次闭环执行还没有拿到你要求的验证标记 {marker}，所以我先不把结果标记为成功。",
        "This closed-loop run did not produce the required verification marker {marker}, so I am not marking it as successful yet.",
        prefer_english,
        &[("marker", marker)],
    )
}

async fn execution_recipe_missing_success_marker_message(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    marker: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> String {
    let default_text =
        execution_recipe_missing_success_marker_default_message(state, user_text, marker);
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let contract = crate::fallback::UserResponseContract::tool_failure(
        "execution_recipe_missing_success_marker",
        user_text,
        &route_resolved_intent(agent_run_context),
        vec![
            format!("required_success_marker: {marker}"),
            "marker_observed: false".to_string(),
            "result_marked_success: false".to_string(),
        ],
        vec![
            "Do not mark the run as successful.".to_string(),
            "Do not invent the required verification marker.".to_string(),
            "Explain that the required verification signal is missing and offer to continue verification."
                .to_string(),
        ],
        "brief_failure_with_next_step",
        &language_hint,
    );
    crate::fallback::compose_user_response_from_contract_with_default(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::ExecutionFailedPartial,
        &default_text,
    )
    .await
}

fn execution_recipe_profile_closeout_label(
    profile: crate::execution_recipe::ExecutionRecipeProfile,
    prefer_english: bool,
) -> &'static str {
    match (profile, prefer_english) {
        (crate::execution_recipe::ExecutionRecipeProfile::ConfigChange, false) => "配置变更",
        (crate::execution_recipe::ExecutionRecipeProfile::ConfigChange, true) => {
            "configuration change"
        }
        (crate::execution_recipe::ExecutionRecipeProfile::CodeChange, false) => "代码修改",
        (crate::execution_recipe::ExecutionRecipeProfile::CodeChange, true) => "code changes",
        (crate::execution_recipe::ExecutionRecipeProfile::SkillAuthoring, false) => "技能开发",
        (crate::execution_recipe::ExecutionRecipeProfile::SkillAuthoring, true) => {
            "skill authoring"
        }
        (crate::execution_recipe::ExecutionRecipeProfile::OpsService, false) => "运维处理",
        (crate::execution_recipe::ExecutionRecipeProfile::OpsService, true) => "ops work",
        (crate::execution_recipe::ExecutionRecipeProfile::None, false) => "处理",
        (crate::execution_recipe::ExecutionRecipeProfile::None, true) => "work",
    }
}

fn prefer_english_for_user_text_without_state(user_text: &str) -> bool {
    crate::language_policy::request_language_hint(user_text) == "en"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionSummaryLanguage {
    Zh,
    En,
    Ja,
    Ko,
}

fn execution_summary_language_from_hint(hint: &str) -> ExecutionSummaryLanguage {
    let normalized = hint.trim().to_ascii_lowercase();
    if normalized.starts_with("ja") {
        ExecutionSummaryLanguage::Ja
    } else if normalized.starts_with("ko") {
        ExecutionSummaryLanguage::Ko
    } else if normalized.starts_with("zh") || normalized == "mixed" {
        ExecutionSummaryLanguage::Zh
    } else if normalized == "config_default" || normalized.is_empty() {
        ExecutionSummaryLanguage::Zh
    } else {
        ExecutionSummaryLanguage::En
    }
}

fn execution_summary_language(
    agent_run_context: Option<&AgentRunContext>,
    user_text: Option<&str>,
) -> ExecutionSummaryLanguage {
    if let Some(original) = agent_run_context
        .and_then(|ctx| ctx.original_user_request.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        let hint = crate::language_policy::request_language_hint(original);
        if hint != "config_default" {
            return execution_summary_language_from_hint(hint);
        }
    }
    user_text
        .map(crate::language_policy::request_language_hint)
        .map(execution_summary_language_from_hint)
        .unwrap_or(ExecutionSummaryLanguage::Zh)
}

fn execution_summary_prefix(language: ExecutionSummaryLanguage) -> &'static str {
    match language {
        ExecutionSummaryLanguage::Zh => crate::finalize::EXECUTION_SUMMARY_MESSAGE_PREFIX,
        ExecutionSummaryLanguage::En => crate::finalize::EXECUTION_SUMMARY_MESSAGE_PREFIX_EN,
        ExecutionSummaryLanguage::Ja => crate::finalize::EXECUTION_SUMMARY_MESSAGE_PREFIX_JA,
        ExecutionSummaryLanguage::Ko => crate::finalize::EXECUTION_SUMMARY_MESSAGE_PREFIX_KO,
    }
}

fn execution_summary_status_label(language: ExecutionSummaryLanguage, ok: bool) -> &'static str {
    match (language, ok) {
        (ExecutionSummaryLanguage::Zh, true) => "输出",
        (ExecutionSummaryLanguage::Zh, false) => "错误",
        (ExecutionSummaryLanguage::En, true) => "Output",
        (ExecutionSummaryLanguage::En, false) => "Error",
        (ExecutionSummaryLanguage::Ja, true) => "出力",
        (ExecutionSummaryLanguage::Ja, false) => "エラー",
        (ExecutionSummaryLanguage::Ko, true) => "출력",
        (ExecutionSummaryLanguage::Ko, false) => "오류",
    }
}

fn execution_recipe_closeout_note(
    state: Option<&AppState>,
    user_text: &str,
    loop_state: &LoopState,
) -> Option<String> {
    let recipe = loop_state.execution_recipe;
    if !recipe.is_active() || !recipe.saw_validation {
        return None;
    }

    let prefer_english = state
        .map(|state| prefer_english_for_user_text(state, user_text))
        .unwrap_or_else(|| prefer_english_for_user_text_without_state(user_text));
    let profile = execution_recipe_profile_closeout_label(recipe.profile, prefer_english);
    match recipe.target_scope {
        crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace
            if recipe.saw_external_target =>
        {
            Some(match state {
                Some(state) => crate::bilingual_t_with_default_vars(
                    state,
                    "clawd.msg.execution_recipe_closeout_external_workspace",
                    "已在外部工作区完成{profile}，并已通过验证。",
                    "Completed {profile} in the external workspace and validated it.",
                    prefer_english,
                    &[("profile", profile)],
                ),
                None if prefer_english => {
                    format!("Completed {profile} in the external workspace and validated it.")
                }
                None => format!("已在外部工作区完成{profile}，并已通过验证。"),
            })
        }
        crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo => Some(match state {
            Some(state) => crate::bilingual_t_with_default_vars(
                state,
                "clawd.msg.execution_recipe_closeout_current_repo",
                "已在当前仓库完成{profile}，并已通过验证。",
                "Completed {profile} in the current repository and validated it.",
                prefer_english,
                &[("profile", profile)],
            ),
            None if prefer_english => {
                format!("Completed {profile} in the current repository and validated it.")
            }
            None => format!("已在当前仓库完成{profile}，并已通过验证。"),
        }),
        crate::execution_recipe::ExecutionRecipeTargetScope::System => Some(match state {
            Some(state) => crate::bilingual_t_with_default_vars(
                state,
                "clawd.msg.execution_recipe_closeout_system",
                "已在系统范围完成{profile}，并已通过验证。",
                "Completed {profile} at the system scope and validated it.",
                prefer_english,
                &[("profile", profile)],
            ),
            None if prefer_english => {
                format!("Completed {profile} at the system scope and validated it.")
            }
            None => format!("已在系统范围完成{profile}，并已通过验证。"),
        }),
        crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield
            if recipe.saw_greenfield_creation =>
        {
            Some(match state {
                Some(state) => crate::bilingual_t_with_default_vars(
                    state,
                    "clawd.msg.execution_recipe_closeout_greenfield",
                    "已完成新产物创建，并已完成{profile}验证。",
                    "Created the new artifact and completed {profile} validation.",
                    prefer_english,
                    &[("profile", profile)],
                ),
                None if prefer_english => {
                    format!("Created the new artifact and completed {profile} validation.")
                }
                None => format!("已完成新产物创建，并已完成{profile}验证。"),
            })
        }
        _ => None,
    }
}

fn can_attach_execution_recipe_closeout(
    final_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let trimmed = final_text.trim();
    if trimmed.is_empty()
        || crate::finalize::parse_delivery_token(trimmed).is_some()
        || looks_like_structured_machine_output(trimmed)
        || looks_like_raw_command_snapshot(trimmed)
    {
        return false;
    }
    let is_scalar = matches!(
        agent_run_context
            .and_then(|ctx| ctx.route_result.as_ref())
            .map(|route| route.output_contract.response_shape),
        Some(crate::OutputResponseShape::Scalar)
    );
    !is_scalar
        || crate::agent_engine::loop_control::requested_success_marker(agent_run_context).is_some()
}

fn attach_execution_recipe_closeout_to_delivery(
    state: Option<&AppState>,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut [String],
) {
    let Some(last) = delivery_messages.last_mut() else {
        return;
    };
    if !can_attach_execution_recipe_closeout(last, agent_run_context) {
        return;
    }
    let Some(mut note) = execution_recipe_closeout_note(state, user_text, loop_state) else {
        return;
    };
    if let Some(marker) =
        crate::agent_engine::loop_control::requested_success_marker(agent_run_context)
    {
        if !note.contains(marker) {
            note = format!("{note}\n\n{marker}");
        }
    }
    *last = format!("{note}\n\n{}", last.trim());
}

fn ensure_requested_success_marker_visible(
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
) {
    let Some(marker) =
        crate::agent_engine::loop_control::requested_success_marker(agent_run_context)
    else {
        return;
    };
    if delivery_messages.iter().any(|item| item.contains(marker)) {
        return;
    }

    if let Some(last) = delivery_messages.last_mut() {
        let trimmed = last.trim();
        if !trimmed.is_empty() && crate::finalize::parse_delivery_token(trimmed).is_none() {
            *last = format!("{trimmed}\n\n{marker}");
            return;
        }
    }
    delivery_messages.push(marker.to_string());
}

fn missing_requested_success_marker<'a>(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &crate::agent_engine::LoopState,
    delivery_messages: &'a [String],
) -> Option<&'static str> {
    let marker = crate::agent_engine::loop_control::requested_success_marker(agent_run_context)?;
    let has_marker = delivery_messages.iter().any(|item| item.contains(marker));
    if loop_state.execution_recipe.is_active() && !has_marker {
        Some(marker)
    } else {
        None
    }
}

fn auto_requested_success_marker<'a>(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &crate::agent_engine::LoopState,
    delivery_messages: &'a [String],
) -> Option<&'static str> {
    let marker = crate::agent_engine::loop_control::requested_success_marker(agent_run_context)?;
    let has_marker = delivery_messages.iter().any(|item| item.contains(marker));
    if loop_state.execution_recipe.is_active()
        && matches!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Done
        )
        && loop_state.execution_recipe.saw_validation
        && !has_marker
    {
        Some(marker)
    } else {
        None
    }
}

fn direct_structured_observed_answer(
    state: Option<&AppState>,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken | crate::OutputResponseShape::Scalar
    ) {
        return None;
    }
    if crate::agent_engine::observed_output::recent_structured_scalar_observation_count(loop_state)
        > 1
    {
        return None;
    }
    let successful_observation_count = loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think"
                )
                && step
                    .output
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|output| !output.is_empty())
        })
        .count();
    if route.output_contract.requires_content_evidence
        && successful_observation_count > 1
        && !route_prefers_observed_answer(route)
    {
        return None;
    }
    let answer = state
        .and_then(|state| {
            crate::agent_engine::observed_output::extract_direct_answer_from_generic_output_i18n(
                loop_state,
                state,
                agent_run_context,
            )
        })
        .or_else(|| {
            crate::agent_engine::observed_output::extract_direct_answer_from_generic_output(
                loop_state,
                agent_run_context,
            )
        })?;
    if answer.trim().is_empty() {
        return None;
    }
    if crate::agent_engine::observed_output::route_requires_synthesized_delivery(route) {
        let latest_raw_observation = loop_state
            .executed_step_results
            .iter()
            .rfind(|step| {
                step.is_ok()
                    && !matches!(
                        step.skill.as_str(),
                        "respond" | "synthesize_answer" | "think"
                    )
            })
            .and_then(|step| step.output.as_deref())
            .map(str::trim)
            .unwrap_or_default();
        if successful_observation_count != 1 || latest_raw_observation == answer.trim() {
            return None;
        }
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

fn route_allows_latest_tail_read_range_delivery(route: &crate::RouteResult) -> bool {
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
            | crate::OutputResponseShape::Scalar
            | crate::OutputResponseShape::OneSentence
    ) {
        return false;
    }
    !route.output_contract.delivery_required
        && route.output_contract.requires_content_evidence
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::RawCommandOutput
                | crate::OutputSemanticKind::None
        )
}

fn latest_tail_read_range_observed_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if !route_allows_latest_tail_read_range_delivery(route) {
        return None;
    }
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let prefer_english =
        crate::fallback::fallback_prefers_english_for_language_hint(state, &language_hint);
    let answer = loop_state
        .executed_step_results
        .iter()
        .rev()
        .find_map(|step| {
            if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
                return None;
            }
            let output = step.output.as_deref()?.trim();
            crate::agent_engine::observed_output::tail_read_range_direct_answer_candidate(
                output,
                prefer_english,
            )
        })?;
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

fn replace_delivery_with_latest_tail_read_range_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some((answer, summary)) = latest_tail_read_range_observed_answer(
        state,
        task,
        user_text,
        loop_state,
        agent_run_context,
    ) else {
        return false;
    };
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
    loop_state
        .delivery_messages
        .retain(|message| crate::finalize::is_execution_summary_message(message));
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.clone(),
    );
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(summary);
    info!(
        "delivery replace_with_latest_tail_read_range task_id={}",
        task.task_id
    );
    true
}

fn prefer_observed_answer_for_exact_contract(
    state: &AppState,
    task_id: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return;
    };
    if !route_prefers_observed_answer(route) || route_requires_file_token(agent_run_context) {
        return;
    }
    if delivery_messages.is_empty() {
        return;
    }
    if delivery_messages
        .last()
        .is_some_and(|message| delivery_message_is_json_object(message))
    {
        info!(
            "delivery exact_contract_keep_planned_json task_id={}",
            task_id
        );
        return;
    }
    let has_prior_step_error = loop_state
        .executed_step_results
        .iter()
        .any(|step| matches!(step.status, crate::executor::StepExecutionStatus::Error));
    let allow_prior_step_error_replacement =
        route_allows_prior_step_error_observed_replacement(route);
    if has_prior_step_error && !allow_prior_step_error_replacement {
        return;
    }
    if let Some(synthesis) = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        if delivery_messages
            .last()
            .map(|message| message.trim() == synthesis)
            .unwrap_or(false)
            && !(has_prior_step_error && allow_prior_step_error_replacement)
        {
            info!(
                "delivery exact_contract_keep_synthesis task_id={} answer={}",
                task_id,
                crate::truncate_for_log(synthesis)
            );
            return;
        }
    }
    let current_delivery_is_publishable_synthesis =
        delivery_messages.last().is_some_and(|message| {
            loop_state
                .last_publishable_synthesis_output
                .as_deref()
                .map(str::trim)
                .is_some_and(|synthesis| synthesis == message.trim())
        });
    if !current_delivery_is_publishable_synthesis
        && delivery_messages
            .last()
            .is_some_and(|message| planned_delivery_is_explicit_contractual_answer(route, message))
    {
        info!(
            "delivery exact_contract_keep_planned_contractual_answer task_id={} answer={}",
            task_id,
            crate::truncate_for_log(
                delivery_messages
                    .last()
                    .map(String::as_str)
                    .unwrap_or_default()
            )
        );
        return;
    }
    let Some((answer, summary)) =
        direct_scalar_observed_answer(Some(state), loop_state, agent_run_context).or_else(|| {
            direct_structured_observed_answer(Some(state), loop_state, agent_run_context)
        })
    else {
        return;
    };
    let answer = answer.trim();
    if answer.is_empty()
        || crate::finalize::parse_delivery_token(answer).is_some()
        || crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
    {
        return;
    }
    if delivery_messages
        .last()
        .map(|message| message.trim() == answer)
        .unwrap_or(false)
    {
        loop_state.last_user_visible_respond = Some(answer.to_string());
        *finalizer_summary = Some(summary);
        return;
    }
    if delivery_messages.last().is_some_and(|message| {
        should_keep_planned_delivery_over_observed_answer(route, message, answer)
    }) {
        info!(
            "delivery exact_contract_keep_planned_delivery task_id={} observed={}",
            task_id,
            crate::truncate_for_log(answer)
        );
        return;
    }

    info!(
        "delivery exact_contract_from_observed task_id={} previous={} observed={}",
        task_id,
        crate::truncate_for_log(
            delivery_messages
                .last()
                .map(String::as_str)
                .unwrap_or_default()
        ),
        crate::truncate_for_log(answer)
    );
    delivery_messages.clear();
    delivery_messages.push(answer.to_string());
    loop_state.last_user_visible_respond = Some(answer.to_string());
    *finalizer_summary = Some(summary);
}

fn planned_delivery_is_explicit_contractual_answer(
    route: &crate::RouteResult,
    delivery: &str,
) -> bool {
    let delivery = delivery.trim();
    if delivery.is_empty()
        || crate::finalize::is_execution_summary_message(delivery)
        || crate::finalize::parse_delivery_token(delivery).is_some()
        || crate::finalize::looks_like_planner_artifact(delivery)
        || crate::finalize::looks_like_internal_trace_artifact(delivery)
    {
        return false;
    }
    matches!(
        crate::output_contract_verifier::verify_output_contract(
            &route.output_contract,
            delivery,
            ""
        ),
        crate::output_contract_verifier::OutputContractVerdict::Pass
    ) && list_contract_candidate_is_line_list(route, delivery)
}

fn list_contract_candidate_is_line_list(route: &crate::RouteResult, delivery: &str) -> bool {
    if !matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::DirectoryEntryGroups
            | crate::OutputSemanticKind::FilePaths
            | crate::OutputSemanticKind::StructuredKeys
    ) {
        return true;
    }
    let lines = delivery
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() > 1 {
        return true;
    }
    lines
        .first()
        .is_some_and(|line| !line.chars().any(char::is_whitespace))
}

fn direct_non_builtin_skill_raw_answer(
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .is_some_and(|text| !text.is_empty())
    {
        return None;
    }
    let last_skill_name = loop_state
        .output_vars
        .get("last_skill_name")
        .map(String::as_str)?;
    if state.is_builtin_skill(last_skill_name) {
        return None;
    }
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let answer = loop_state
        .executed_step_results
        .iter()
        .rfind(|step| step.is_ok() && step.skill == last_skill_name)
        .and_then(|step| step.output.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty())?
        .to_string();
    if direct_structured_observed_answer(None, loop_state, agent_run_context)
        .is_some_and(|(structured_answer, _)| structured_answer.trim() != answer.trim())
    {
        return None;
    }
    if matches!(
        route.map(|route| route.output_contract.response_shape),
        Some(crate::OutputResponseShape::Scalar)
    ) && !matches!(
        route.map(|route| route.output_contract.semantic_kind),
        Some(crate::OutputSemanticKind::RawCommandOutput)
    ) {
        return None;
    }
    if matches!(
        route.map(|route| route.output_contract.response_shape),
        Some(crate::OutputResponseShape::OneSentence)
    ) && !matches!(
        route.map(|route| route.output_contract.semantic_kind),
        Some(crate::OutputSemanticKind::RawCommandOutput)
    ) {
        return None;
    }
    if crate::finalize::looks_like_planner_artifact(&answer)
        || crate::finalize::looks_like_internal_trace_artifact(&answer)
        || (looks_like_structured_machine_output(&answer)
            && !matches!(
                route.map(|route| route.output_contract.semantic_kind),
                Some(crate::OutputSemanticKind::RawCommandOutput)
            ))
    {
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

fn route_allows_direct_scalar_observed_answer(route: &crate::RouteResult) -> bool {
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarCount {
        return true;
    }
    if route.output_contract.response_shape == crate::OutputResponseShape::Scalar {
        return true;
    }
    route.output_contract.response_shape == crate::OutputResponseShape::Strict
        && route.output_contract.exact_sentence_count == Some(1)
        && !route.output_contract.delivery_required
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::None
}

async fn direct_publishable_observed_answer(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return None;
    };
    if route.output_contract.requires_content_evidence
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
    {
        return None;
    }
    let observed =
        crate::agent_engine::observed_output::extract_latest_generic_successful_output(loop_state)?;
    let answer = observed.body.trim().to_string();
    if answer.is_empty()
        || crate::finalize::looks_like_planner_artifact(&answer)
        || crate::finalize::looks_like_internal_trace_artifact(&answer)
        || looks_like_structured_machine_output(&answer)
    {
        return None;
    }
    if observed.skill == "run_cmd" && !route_explicitly_requests_command_result(route) {
        return None;
    }
    if looks_like_raw_command_snapshot(&answer)
        && !(observed.skill == "run_cmd" && route_explicitly_requests_command_result(route))
    {
        return None;
    }
    let raw_command_passthrough =
        observed.skill == "run_cmd" && route_explicitly_requests_command_result(route);
    // §3.4 finalize-tier: observed_generic_finalize 是 finalize 决策层。
    if !raw_command_passthrough
        && !crate::semantic_judge::is_publishable_raw(state, task, &answer).await
    {
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

fn looks_like_structured_machine_output(answer: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(answer)
        .map(|value| value.is_object() || value.is_array())
        .unwrap_or(false)
}

fn looks_like_raw_command_snapshot(answer: &str) -> bool {
    let trimmed = answer.trim();
    trimmed.starts_with("exit=")
        && trimmed.contains('\n')
        && (trimmed.contains("\nCOMMAND ")
            || trimmed.contains("(LISTEN)")
            || trimmed.contains("\nLISTEN ")
            || trimmed.contains("State  Recv-Q")
            || trimmed.contains("%CPU")
            || trimmed.contains("PID PPID"))
}

fn route_explicitly_requests_command_result(route: &crate::RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
        && route.output_contract.response_shape != crate::OutputResponseShape::Strict
}

fn output_contract_requests_exact_delivery(route: &crate::RouteResult) -> bool {
    matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    ) || matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
            | crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::DirectoryEntryGroups
            | crate::OutputSemanticKind::FilePaths
            | crate::OutputSemanticKind::GitCommitSubject
            | crate::OutputSemanticKind::StructuredKeys
    )
}

fn route_prefers_observed_answer(route: &crate::RouteResult) -> bool {
    if output_contract_requests_exact_delivery(route) {
        return true;
    }
    if route_path_locator_plain_act_allows_observed_listing(route) {
        return true;
    }
    let contract = crate::TaskContract::from_route_result(route);
    if contract
        .required_evidence_fields
        .iter()
        .any(|field| field == "content_excerpt")
    {
        return false;
    }
    if contract.required_evidence_fields.is_empty() {
        return false;
    }
    match contract.delivery_shape {
        crate::task_contract::TaskDeliveryShape::Raw
        | crate::task_contract::TaskDeliveryShape::List
        | crate::task_contract::TaskDeliveryShape::File => true,
        crate::task_contract::TaskDeliveryShape::OneSentence
        | crate::task_contract::TaskDeliveryShape::Summary
        | crate::task_contract::TaskDeliveryShape::Table => matches!(
            contract.operation,
            crate::task_contract::TaskOperation::Inspect
                | crate::task_contract::TaskOperation::List
                | crate::task_contract::TaskOperation::Count
                | crate::task_contract::TaskOperation::Run
        ),
    }
}

fn route_path_locator_plain_act_allows_observed_listing(route: &crate::RouteResult) -> bool {
    !route.output_contract.delivery_required
        && route.output_contract.locator_kind == crate::OutputLocatorKind::Path
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None | crate::OutputSemanticKind::ExistenceWithPath
        )
        && route.ask_mode.is_plain_act()
}

fn route_allows_prior_step_error_observed_replacement(route: &crate::RouteResult) -> bool {
    if route_path_locator_plain_act_allows_observed_listing(route) {
        return true;
    }
    if route.output_contract.response_shape == crate::OutputResponseShape::Scalar {
        return true;
    }
    matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::DirectoryEntryGroups
            | crate::OutputSemanticKind::FilePaths
            | crate::OutputSemanticKind::ScalarPathOnly
            | crate::OutputSemanticKind::ExistenceWithPath
    )
}

fn delivery_has_planned_content_beyond_observed_answer(delivery: &str, observed: &str) -> bool {
    let delivery = delivery.trim();
    let observed = observed.trim();
    if delivery.is_empty() || observed.is_empty() || delivery == observed {
        return false;
    }
    if !delivery.contains(observed) {
        return false;
    }
    delivery
        .replacen(observed, "", 1)
        .chars()
        .any(|ch| !ch.is_whitespace())
}

fn should_keep_planned_delivery_over_observed_answer(
    route: &crate::RouteResult,
    delivery: &str,
    observed: &str,
) -> bool {
    let planned_delivery_contains_more_than_observed =
        delivery_has_planned_content_beyond_observed_answer(delivery, observed);
    if !planned_delivery_contains_more_than_observed {
        return false;
    }
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    ) {
        return false;
    }
    if !output_contract_requests_exact_delivery(route) {
        return true;
    }
    route.ask_mode.finalize_chat_wrapped()
        && !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::RawCommandOutput
        )
}

const EXECUTION_SUMMARY_MAX_STEPS: usize = 4;
const EXECUTION_SUMMARY_ARGS_MAX_CHARS: usize = 180;
const EXECUTION_SUMMARY_OUTPUT_MAX_CHARS: usize = 420;

fn should_attach_execution_summary(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    _user_text: Option<&str>,
) -> bool {
    let _ = loop_state;
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some()
}

fn truncate_with_ellipsis(text: &str, max_chars: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 3 {
        return "...".to_string();
    }
    let mut truncated = text
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    truncated.push_str("...");
    truncated
}

fn execution_summary_value_to_string(value: &serde_json::Value) -> String {
    let raw = match value {
        serde_json::Value::String(value) => value.trim().to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Null => String::new(),
        _ => value.to_string(),
    };
    crate::visible_text::sanitize_user_visible_text(&raw)
}

fn execution_summary_arg_is_sensitive(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "secret", "token", "key", "password", "passwd", "cookie", "auth",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

fn safe_execution_args_summary(args: &serde_json::Value, max_chars: usize) -> String {
    let Some(object) = args.as_object() else {
        return String::new();
    };
    let mut parts = Vec::new();
    for key in [
        "action",
        "command",
        "cmd",
        "path",
        "file_path",
        "target",
        "target_path",
        "dir",
        "directory",
        "field",
        "field_path",
        "query",
        "pattern",
        "url",
        "limit",
        "name",
    ] {
        if execution_summary_arg_is_sensitive(key) {
            continue;
        }
        let Some(value) = object.get(key) else {
            continue;
        };
        let value = execution_summary_value_to_string(value);
        if value.is_empty() {
            continue;
        }
        parts.push(format!("{key}={}", truncate_with_ellipsis(&value, 56)));
    }
    truncate_with_ellipsis(&parts.join(", "), max_chars)
}

fn plan_step_matches_execution(
    plan_step: &crate::PlanStep,
    step: &crate::executor::StepExecutionResult,
) -> bool {
    let plan_skill = plan_step.skill.trim();
    plan_skill.eq_ignore_ascii_case(step.skill.trim())
        || (step.skill == "run_cmd" && plan_skill.eq_ignore_ascii_case("run_cmd"))
}

fn plan_step_for_execution<'a>(
    loop_state: &'a LoopState,
    step: &crate::executor::StepExecutionResult,
) -> Option<&'a crate::PlanStep> {
    loop_state
        .round_traces
        .iter()
        .filter_map(|trace| trace.plan_result.as_ref())
        .flat_map(|plan| plan.steps.iter())
        .find(|plan_step| {
            plan_step.step_id == step.step_id && plan_step_matches_execution(plan_step, step)
        })
}

fn command_arg_from_plan_step(plan_step: Option<&crate::PlanStep>) -> Option<String> {
    let args = &plan_step?.args;
    args.get("command")
        .or_else(|| args.get("cmd"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| truncate_with_ellipsis(value, 140))
}

fn raw_command_arg_from_plan_step(plan_step: Option<&crate::PlanStep>) -> Option<&str> {
    let args = &plan_step?.args;
    args.get("command")
        .or_else(|| args.get("cmd"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn execution_summary_invocation_label(
    step: &crate::executor::StepExecutionResult,
    plan_step: Option<&crate::PlanStep>,
    language: ExecutionSummaryLanguage,
) -> String {
    if let Some(command) = command_arg_from_plan_step(plan_step) {
        return match language {
            ExecutionSummaryLanguage::Zh => format!("命令 `{command}`"),
            ExecutionSummaryLanguage::En => format!("command `{command}`"),
            ExecutionSummaryLanguage::Ja => format!("コマンド `{command}`"),
            ExecutionSummaryLanguage::Ko => format!("명령 `{command}`"),
        };
    }

    let action_type = plan_step
        .map(|step| step.action_type.as_str())
        .unwrap_or("call_skill");
    let skill = plan_step
        .map(|step| step.skill.as_str())
        .unwrap_or(step.skill.as_str());
    let is_tool =
        action_type == "call_tool" || crate::virtual_tools::is_planner_facing_virtual_tool(skill);
    let kind = match (language, is_tool) {
        (ExecutionSummaryLanguage::Zh, true) => "工具",
        (ExecutionSummaryLanguage::Zh, false) => "技能",
        (ExecutionSummaryLanguage::En, true) => "tool",
        (ExecutionSummaryLanguage::En, false) => "skill",
        (ExecutionSummaryLanguage::Ja, true) => "ツール",
        (ExecutionSummaryLanguage::Ja, false) => "スキル",
        (ExecutionSummaryLanguage::Ko, true) => "도구",
        (ExecutionSummaryLanguage::Ko, false) => "스킬",
    };
    let args = plan_step
        .map(|step| safe_execution_args_summary(&step.args, EXECUTION_SUMMARY_ARGS_MAX_CHARS))
        .unwrap_or_default();
    if args.is_empty() {
        format!("{kind} `{skill}`")
    } else {
        match language {
            ExecutionSummaryLanguage::Zh | ExecutionSummaryLanguage::Ja => {
                format!("{kind} `{skill}`（{args}）")
            }
            ExecutionSummaryLanguage::En | ExecutionSummaryLanguage::Ko => {
                format!("{kind} `{skill}` ({args})")
            }
        }
    }
}

fn output_text_from_execution_result(
    step: &crate::executor::StepExecutionResult,
) -> Option<String> {
    let raw = if step.is_ok() {
        step.output.as_deref()
    } else {
        step.error.as_deref().or(step.output.as_deref())
    }?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.eq_ignore_ascii_case("NOT_FOUND") {
        return Some("file not found".to_string());
    }
    if let Some(path) = trimmed.strip_prefix("__RC_READ_FILE_NOT_FOUND__:") {
        return Some(crate::visible_text::sanitize_user_visible_text(&format!(
            "file not found: {}",
            path.trim()
        )));
    }
    if crate::skills::parse_structured_skill_error(trimmed).is_some() {
        return Some(crate::visible_text::sanitize_user_visible_text(
            &crate::skills::normalize_skill_error_for_user(&step.skill, trimmed),
        ));
    }
    if !step.is_ok() && crate::skills::is_recoverable_skill_error(&step.skill, trimmed) {
        return Some(crate::visible_text::sanitize_user_visible_text(
            &crate::skills::normalize_skill_error_for_user(&step.skill, trimmed),
        ));
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(text) = value
            .get("text")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(crate::visible_text::sanitize_user_visible_text(text));
        }
        if let Some(text) = value
            .get("stdout")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(crate::visible_text::sanitize_user_visible_text(text));
        }
        if let Some(text) = value
            .get("error_text")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(crate::visible_text::sanitize_user_visible_text(text));
        }
    }
    Some(crate::visible_text::sanitize_user_visible_text(trimmed))
}

fn structured_observation_suppresses_execution_summary(
    _step: &crate::executor::StepExecutionResult,
    _route: Option<&crate::RouteResult>,
) -> bool {
    false
}

fn build_execution_summary_messages(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    user_text: Option<&str>,
) -> Vec<String> {
    if !should_attach_execution_summary(loop_state, agent_run_context, user_text) {
        return Vec::new();
    }
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let steps = loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer"
            ) && !structured_observation_suppresses_execution_summary(step, route)
                && output_text_from_execution_result(step).is_some()
        })
        .collect::<Vec<_>>();
    if steps.is_empty() {
        return Vec::new();
    }

    let language = execution_summary_language(agent_run_context, user_text);
    let prefix = execution_summary_prefix(language).to_string();
    let omitted = steps.len().saturating_sub(EXECUTION_SUMMARY_MAX_STEPS);
    steps
        .iter()
        .take(EXECUTION_SUMMARY_MAX_STEPS)
        .enumerate()
        .filter_map(|(index, step)| {
            let plan_step = plan_step_for_execution(loop_state, step);
            let output = output_text_from_execution_result(step)?.replace("```", "'''");
            let output = truncate_with_ellipsis(&output, EXECUTION_SUMMARY_OUTPUT_MAX_CHARS);
            let status_label = execution_summary_status_label(language, step.is_ok());
            let invocation = execution_summary_invocation_label(step, plan_step, language);
            let line = match language {
                ExecutionSummaryLanguage::Zh => format!("{}. 调用{}", index + 1, invocation),
                ExecutionSummaryLanguage::En => format!("{}. Called {}", index + 1, invocation),
                ExecutionSummaryLanguage::Ja => {
                    format!("{}. {}を呼び出しました", index + 1, invocation)
                }
                ExecutionSummaryLanguage::Ko => format!("{}. {} 호출", index + 1, invocation),
            };
            let mut lines = vec![prefix.clone(), line];
            let status_separator = if matches!(language, ExecutionSummaryLanguage::En) {
                ":"
            } else {
                "："
            };
            lines.push(format!("   {status_label}{status_separator}"));
            lines.push("```text".to_string());
            lines.push(output);
            lines.push("```".to_string());
            if omitted > 0 && index + 1 == EXECUTION_SUMMARY_MAX_STEPS {
                match language {
                    ExecutionSummaryLanguage::Zh => {
                        lines.push(format!("...（还有 {omitted} 个执行步骤已省略）"));
                    }
                    ExecutionSummaryLanguage::En => {
                        let suffix = if omitted == 1 { "step" } else { "steps" };
                        lines.push(format!("... ({omitted} more execution {suffix} omitted)"));
                    }
                    ExecutionSummaryLanguage::Ja => {
                        lines.push(format!("...（他 {omitted} 件の実行手順を省略）"));
                    }
                    ExecutionSummaryLanguage::Ko => {
                        lines.push(format!("... (추가 실행 단계 {omitted}개 생략)"));
                    }
                }
            }
            Some(lines.join("\n"))
        })
        .collect()
}

#[cfg(test)]
fn build_execution_summary_message(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    user_text: Option<&str>,
) -> Option<String> {
    let messages = build_execution_summary_messages(loop_state, agent_run_context, user_text);
    if messages.is_empty() {
        None
    } else {
        Some(messages.join("\n\n"))
    }
}

fn attach_execution_summary_to_delivery(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    user_text: Option<&str>,
    delivery_messages: &mut Vec<String>,
) {
    let summaries = build_execution_summary_messages(loop_state, agent_run_context, user_text);
    if summaries.is_empty() {
        return;
    };
    for summary in summaries.into_iter().rev() {
        if delivery_messages.iter().any(|message| message == &summary) {
            continue;
        }
        delivery_messages.insert(0, summary);
    }
}

fn final_answer_text_from_delivery(delivery_messages: &[String]) -> String {
    let publishable_messages = delivery_messages
        .iter()
        .map(|message| message.trim())
        .filter(|message| !message.is_empty())
        .filter(|message| !crate::finalize::is_execution_summary_message(message))
        .collect::<Vec<_>>();
    if !publishable_messages.is_empty() {
        return publishable_messages.join("\n\n");
    }
    delivery_messages
        .iter()
        .rev()
        .find_map(|message| {
            let trimmed = message.trim();
            (!trimmed.is_empty()).then_some(trimmed.to_string())
        })
        .unwrap_or_default()
}

fn error_looks_like_os_permission_denied(error: &str) -> bool {
    crate::skills::error_looks_like_os_permission_denied(error)
}

fn error_looks_like_missing_file_or_directory(error: &str) -> bool {
    if let Some(structured) = crate::skills::parse_structured_skill_error(error) {
        return structured.error_kind == "not_found";
    }
    error.trim().starts_with("__RC_READ_FILE_NOT_FOUND__:")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceStatusFailureObservation {
    UnitNotFound,
    Inactive,
    Failed,
}

fn route_is_service_status(agent_run_context: Option<&AgentRunContext>) -> bool {
    matches!(
        agent_run_context
            .and_then(|ctx| ctx.route_result.as_ref())
            .map(|route| route.output_contract.semantic_kind),
        Some(crate::OutputSemanticKind::ServiceStatus)
    )
}

fn service_status_observation_from_error(error: &str) -> Option<ServiceStatusFailureObservation> {
    if let Some(structured) = crate::skills::parse_structured_skill_error(error) {
        return match structured.error_kind.as_str() {
            "not_found" => Some(ServiceStatusFailureObservation::UnitNotFound),
            "service_inactive" => Some(ServiceStatusFailureObservation::Inactive),
            "service_failed" | "service_control_failed" => {
                Some(ServiceStatusFailureObservation::Failed)
            }
            _ => None,
        };
    }
    None
}

fn extract_systemd_unit_from_error(error: &str) -> Option<String> {
    let _ = error;
    None
}

fn service_status_target_label(error: &str, agent_run_context: Option<&AgentRunContext>) -> String {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.output_contract.locator_hint.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            crate::skills::parse_structured_skill_error(error)
                .and_then(|structured| structured.service_name)
        })
        .or_else(|| extract_systemd_unit_from_error(error))
        .unwrap_or_else(|| "requested service".to_string())
}

fn service_status_failure_answer(
    state: &AppState,
    user_text: &str,
    error: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    if !route_is_service_status(agent_run_context) {
        return None;
    }
    let observation = service_status_observation_from_error(error)?;
    let target = service_status_target_label(error, agent_run_context);
    let prefer_english = prefer_english_for_user_text(state, user_text);
    Some(match (prefer_english, observation) {
        (true, ServiceStatusFailureObservation::UnitNotFound) => {
            format!("`{target}` is not active: systemd has no service unit with that name.")
        }
        (true, ServiceStatusFailureObservation::Inactive) => {
            format!("`{target}` is not active; systemd reports it as inactive.")
        }
        (true, ServiceStatusFailureObservation::Failed) => {
            format!("`{target}` is not active; systemd reports it as failed.")
        }
        (false, ServiceStatusFailureObservation::UnitNotFound) => {
            format!("`{target}` 现在不是 active：systemd 没有找到这个服务单元。")
        }
        (false, ServiceStatusFailureObservation::Inactive) => {
            format!("`{target}` 现在不是 active：systemd 显示它处于 inactive 状态。")
        }
        (false, ServiceStatusFailureObservation::Failed) => {
            format!("`{target}` 现在不是 active：systemd 显示它处于 failed 状态。")
        }
    })
}

fn content_evidence_step_failure_default_answer(
    state: &AppState,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    failed_step: &crate::executor::StepExecutionResult,
    error: &str,
    permission_denied: bool,
) -> String {
    let locator = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.output_contract.locator_hint.trim())
        .filter(|locator| !locator.is_empty());
    let prefer_english = prefer_english_for_user_text(state, user_text);
    let answer = match (prefer_english, locator) {
        (true, Some(locator)) => {
            format!("Tried to access `{locator}`, but execution failed: {error}.")
        }
        (true, None) => format!("The `{}` step failed: {error}.", failed_step.skill.trim()),
        (false, Some(locator)) => {
            format!("已尝试访问 `{locator}`，但执行失败：{error}。")
        }
        (false, None) => format!("`{}` 步骤执行失败：{error}。", failed_step.skill.trim()),
    };
    if permission_denied {
        if prefer_english {
            format!("{answer} The `clawd` process does not have sudo/root permission to access it.")
        } else {
            format!("{answer}`clawd` 进程当前没有 sudo/root 权限，所以无法访问。")
        }
    } else {
        answer
    }
}

fn structured_extra_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| crate::truncate_for_agent_trace(&compact_observed_stream(value)))
}

fn structured_extra_i64(value: &serde_json::Value, key: &str) -> Option<i64> {
    value.get(key).and_then(|value| value.as_i64())
}

fn structured_extra_bool(value: &serde_json::Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn compact_observed_stream(text: &str) -> String {
    let compact = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    if compact.is_empty() {
        text.trim().to_string()
    } else {
        compact
    }
}

fn run_cmd_failure_direct_answer(
    state: &AppState,
    user_text: &str,
    skill_name: &str,
    raw_error: &str,
    normalized_error: &str,
) -> Option<String> {
    let structured = crate::skills::parse_structured_skill_error(raw_error)?;
    let effective_skill = if structured.skill.trim().is_empty() {
        skill_name
    } else {
        structured.skill.as_str()
    };
    if !effective_skill.eq_ignore_ascii_case("run_cmd") {
        return None;
    }
    let extra = structured.extra.as_ref()?;
    let exit_code = structured_extra_i64(extra, "exit_code");
    let stderr = structured_extra_string(extra, "stderr");
    let stdout = structured_extra_string(extra, "stdout");
    let output_truncated = structured_extra_bool(extra, "output_truncated");
    let prefer_english = prefer_english_for_user_text(state, user_text);

    if prefer_english {
        let mut sentence = if let Some(exit_code) = exit_code {
            format!("The command failed with exit code {exit_code}")
        } else {
            format!("The command failed: {normalized_error}")
        };
        if let Some(stderr) = stderr.as_deref() {
            sentence.push_str(&format!(". Stderr: {stderr}"));
        } else if let Some(stdout) = stdout.as_deref() {
            sentence.push_str(&format!(". Stdout: {stdout}"));
        }
        if output_truncated {
            sentence.push_str(". Output was truncated");
        }
        sentence.push('.');
        return Some(sentence);
    }

    let mut sentence = if let Some(exit_code) = exit_code {
        format!("命令执行失败，退出码为 {exit_code}")
    } else {
        format!("命令执行失败：{normalized_error}")
    };
    if let Some(stderr) = stderr.as_deref() {
        sentence.push_str(&format!("，错误输出为：{stderr}"));
    } else if let Some(stdout) = stdout.as_deref() {
        sentence.push_str(&format!("，标准输出为：{stdout}"));
    }
    if output_truncated {
        sentence.push_str("，输出已截断");
    }
    sentence.push('。');
    Some(sentence)
}

fn missing_content_target_label(
    agent_run_context: Option<&AgentRunContext>,
    error: &str,
) -> String {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.output_contract.locator_hint.trim())
        .filter(|locator| !locator.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            error
                .trim()
                .strip_prefix("__RC_READ_FILE_NOT_FOUND__:")
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| "requested target".to_string())
}

fn content_evidence_missing_target_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    error: &str,
) -> String {
    let target = missing_content_target_label(agent_run_context, error);
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    if crate::fallback::fallback_prefers_english_for_language_hint(state, &language_hint) {
        format!(
            "I couldn't find `{target}`, so I didn't read any content. Please confirm the path or filename and send it again."
        )
    } else {
        format!("未找到 `{target}`，所以没有读取内容。请确认路径或文件名后再发一次。")
    }
}

async fn content_evidence_step_failure_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_requires_content_evidence(agent_run_context) {
        return None;
    }
    if loop_state.executed_step_results.iter().any(|step| {
        step.is_ok()
            && !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
            && step
                .output
                .as_deref()
                .map(str::trim)
                .is_some_and(|output| !output.is_empty())
    }) {
        return None;
    }

    let failed_step = loop_state.executed_step_results.iter().rev().find(|step| {
        !step.is_ok()
            && !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
    })?;
    let raw_error = failed_step.error.as_deref().map(str::trim)?;
    if raw_error.is_empty() {
        return None;
    }
    let recoverable_skill_error =
        crate::skills::is_recoverable_skill_error(&failed_step.skill, raw_error);
    let observable_run_cmd_error =
        crate::skills::is_observable_run_cmd_error(&failed_step.skill, raw_error);
    let user_visible_error = if recoverable_skill_error || observable_run_cmd_error {
        crate::skills::normalize_skill_error_for_user(&failed_step.skill, raw_error)
    } else {
        raw_error.to_string()
    };
    let error = user_visible_error.as_str();

    if let Some(answer) =
        service_status_failure_answer(state, user_text, raw_error, agent_run_context)
    {
        return Some((
            answer,
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                ..Default::default()
            },
        ));
    }

    let missing_target = error_looks_like_missing_file_or_directory(raw_error);
    if missing_target {
        let answer = content_evidence_missing_target_answer(
            state,
            task,
            user_text,
            agent_run_context,
            raw_error,
        );
        return Some((
            answer,
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                ..Default::default()
            },
        ));
    }

    let permission_denied = error_looks_like_os_permission_denied(raw_error);
    let default_answer = if observable_run_cmd_error {
        run_cmd_failure_direct_answer(state, user_text, &failed_step.skill, raw_error, error)
            .unwrap_or_else(|| {
                content_evidence_step_failure_default_answer(
                    state,
                    user_text,
                    agent_run_context,
                    failed_step,
                    error,
                    permission_denied,
                )
            })
    } else {
        content_evidence_step_failure_default_answer(
            state,
            user_text,
            agent_run_context,
            failed_step,
            error,
            permission_denied,
        )
    };
    if permission_denied || recoverable_skill_error || observable_run_cmd_error {
        return Some((
            default_answer,
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                ..Default::default()
            },
        ));
    }
    let locator = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.output_contract.locator_hint.trim())
        .filter(|locator| !locator.is_empty());
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let mut observed_facts = vec![
        format!("failed_skill: {}", failed_step.skill.trim()),
        format!("error_summary: {}", crate::truncate_for_agent_trace(error)),
        "content_evidence_observed: false".to_string(),
    ];
    if let Some(locator) = locator {
        observed_facts.push(format!("locator: {locator}"));
    }
    if permission_denied {
        observed_facts.push("os_permission_denied: true".to_string());
        observed_facts.push("clawd_process_lacks_sudo_or_root_permission: true".to_string());
    }
    let mut policy_boundary = vec![
        "Do not claim the content was read or summarized.".to_string(),
        "Do not expose prompt names, schema names, stack traces, or internal route details."
            .to_string(),
        "Explain only the observed execution failure and the immediate recovery path.".to_string(),
    ];
    if permission_denied {
        policy_boundary.push(
            "Mention that the clawd process itself lacks sudo/root permission for this OS-level access."
                .to_string(),
        );
    }
    let contract = crate::fallback::UserResponseContract::tool_failure(
        if permission_denied {
            "content_evidence_step_permission_denied"
        } else {
            "content_evidence_step_failed"
        },
        user_text,
        &route_resolved_intent(agent_run_context),
        observed_facts,
        policy_boundary,
        "brief_failure_with_next_step",
        &language_hint,
    );
    let answer = crate::fallback::compose_user_response_from_contract_with_default(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::ExecutionFailedPartial,
        &default_answer,
    )
    .await;
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::AllowFallback),
            contract_ok: true,
            completion_ok: Some(false),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: 1,
            ..Default::default()
        },
    ))
}

async fn content_evidence_step_failure_reply_from_loop(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<AskReply> {
    if latest_tail_read_range_observed_answer(state, task, user_text, loop_state, agent_run_context)
        .is_some()
    {
        return None;
    }
    let (error_answer, summary) =
        content_evidence_step_failure_answer(state, task, user_text, loop_state, agent_run_context)
            .await?;
    let mut delivery_messages =
        build_execution_summary_messages(loop_state, agent_run_context, Some(user_text));
    delivery_messages.push(error_answer.clone());
    let delivery_consistent =
        crate::task_journal::delivery_payload_consistent(&error_answer, &delivery_messages);
    let should_fail = !matches!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    ) || summary.completion_ok == Some(false);
    let final_status = if should_fail {
        crate::task_journal::TaskJournalFinalStatus::Failure
    } else {
        crate::task_journal::TaskJournalFinalStatus::Success
    };
    let journal = build_loop_journal(
        task,
        user_text,
        loop_state,
        agent_run_context,
        Some(summary),
        delivery_consistent,
        &error_answer,
        final_status,
    );
    let reply = AskReply::non_llm(error_answer.clone())
        .with_messages(delivery_messages)
        .with_task_journal(journal);
    Some(if should_fail {
        reply.with_failure(error_answer)
    } else {
        reply
    })
}

async fn pending_confirmation_resume_payload(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
) -> Option<(String, serde_json::Value)> {
    let round = loop_state.round_traces.last()?;
    let verify = round.verify_result.as_ref()?;
    if !verify_summary_requires_resume_confirmation(verify) {
        return None;
    }
    let plan = round.plan_result.as_ref()?;
    let detail = verify
        .issues
        .iter()
        .find(|issue| issue.kind == crate::verifier::VerifyIssueKind::ConfirmationRequired)
        .map(|issue| issue.detail.as_str())
        .unwrap_or("current plan requires explicit confirmation");
    Some(
        crate::agent_engine::build_confirmation_required_resume_context(
            state,
            task,
            &plan.steps,
            user_text,
            &round.goal,
            &loop_state.subtask_results,
            &loop_state.delivery_messages,
            detail,
        )
        .await,
    )
}

fn verify_summary_requires_resume_confirmation(
    verify: &crate::task_journal::TaskJournalVerifySummary,
) -> bool {
    verify.mode == crate::verifier::VerifyMode::Enforce
        && verify.approved
        && verify.needs_confirmation
}

fn finalizer_requires_clarify(
    summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
    requires_content_evidence: bool,
    has_authoritative_delivery: bool,
) -> bool {
    if requires_content_evidence {
        if has_authoritative_delivery {
            return false;
        }
        return !matches!(
            summary.and_then(|summary| summary.disposition),
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }
    false
}

fn build_finalizer_clarify_reason(
    summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
) -> String {
    let Some(summary) = summary else {
        return "finalizer could not confirm a reliable final answer from the observed execution result"
            .to_string();
    };
    let mut parts = Vec::new();
    if let Some(stage) = summary
        .stage
        .map(crate::task_journal::TaskJournalFinalizerStage::as_str)
    {
        parts.push(format!("stage={stage}"));
    }
    if let Some(disposition) = summary
        .disposition
        .map(crate::finalize::FinalizerDisposition::as_str)
        .filter(|v| !v.trim().is_empty())
    {
        parts.push(format!("disposition={disposition}"));
    }
    if let Some(fallback) = summary
        .fallback
        .map(crate::task_journal::TaskJournalFinalizerFallback::as_str)
    {
        parts.push(format!("fallback={fallback}"));
    }
    if let Some(value) = summary.completion_ok {
        parts.push(format!("completion_ok={value}"));
    }
    if let Some(value) = summary.grounded_ok {
        parts.push(format!("grounded_ok={value}"));
    }
    if let Some(value) = summary.format_ok {
        parts.push(format!("format_ok={value}"));
    }
    if let Some(value) = summary.needs_clarify {
        parts.push(format!("needs_clarify={value}"));
    }
    if parts.is_empty() {
        "finalizer could not confirm a reliable final answer from the observed execution result"
            .to_string()
    } else {
        format!(
            "finalizer could not confirm a reliable final answer from the observed execution result; {}",
            parts.join(", ")
        )
    }
}

fn build_missing_delivery_clarify_reason(
    summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
) -> String {
    match summary {
        Some(summary) => format!(
            "no publishable final answer was produced; {}",
            build_finalizer_clarify_reason(Some(summary))
        ),
        None => "no publishable final answer was produced from the execution result".to_string(),
    }
}

fn observed_execution_facts_for_missing_delivery(
    loop_state: &crate::agent_engine::LoopState,
    clarify_reason: &str,
) -> Vec<String> {
    let mut facts = vec![format!(
        "finalizer_reason: {}",
        crate::truncate_for_agent_trace(clarify_reason)
    )];
    let mut steps = loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer"
            ) && output_text_from_execution_result(step).is_some()
        })
        .rev()
        .take(4)
        .collect::<Vec<_>>();
    steps.reverse();
    for step in steps {
        let mut parts = vec![
            format!("skill={}", step.skill.trim()),
            format!("status={}", step.status.as_str()),
        ];
        if let Some(output) = output_text_from_execution_result(step) {
            parts.push(format!(
                "observed_output={}",
                crate::truncate_for_agent_trace(&output)
            ));
        }
        facts.push(format!("observed_step: {}", parts.join(", ")));
    }
    facts
}

fn missing_delivery_after_observation_default_message(state: &AppState, user_text: &str) -> String {
    if prefer_english_for_user_text(state, user_text) {
        "I have execution results, but I could not turn them into a reliable final answer. Ask me to retry from the raw results.".to_string()
    } else {
        "已有执行结果，但我没能整理成可靠结论。你可以让我基于原始结果重新整理一次。".to_string()
    }
}

fn observed_execution_status_steps<'a>(
    loop_state: &'a crate::agent_engine::LoopState,
) -> Vec<&'a crate::executor::StepExecutionResult> {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer"
            ) && output_text_from_execution_result(step).is_some()
        })
        .collect::<Vec<_>>()
}

fn deterministic_observed_execution_status_answer(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
) -> Option<String> {
    let prefer_english = prefer_english_for_user_text(state, user_text);
    let steps = observed_execution_status_steps(loop_state);
    if steps.len() < 2 || !steps.iter().any(|step| !step.is_ok()) {
        return None;
    }
    if steps.last().is_some_and(|step| step.is_ok()) {
        return None;
    }

    let lines = steps
        .iter()
        .take(6)
        .enumerate()
        .map(|(idx, step)| {
            let skill = step.skill.trim();
            if step.is_ok() {
                if prefer_english {
                    format!("Step {} `{skill}` succeeded.", idx + 1)
                } else {
                    format!("第 {} 步 `{skill}` 成功。", idx + 1)
                }
            } else {
                let error = output_text_from_execution_result(step)
                    .unwrap_or_else(|| "execution failed".to_string());
                let error = truncate_with_ellipsis(&error.replace('\n', " "), 220);
                if prefer_english {
                    format!("Step {} `{skill}` failed: {error}.", idx + 1)
                } else {
                    format!("第 {} 步 `{skill}` 失败：{error}。", idx + 1)
                }
            }
        })
        .collect::<Vec<_>>();
    Some(lines.join(if prefer_english { " " } else { "" }))
}

fn deterministic_missing_observed_target_answer(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let latest_missing_idx = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .rev()
        .find_map(|(idx, step)| {
            (step
                .output
                .as_deref()
                .is_some_and(output_excerpt_has_missing_file_evidence)
                || step_error_has_missing_file_evidence(step))
            .then_some(idx)
        })?;
    let has_later_successful_observation = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .skip(latest_missing_idx + 1)
        .any(|(_, step)| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "think" | "synthesize_answer"
                )
                && step.output.as_deref().map(str::trim).is_some_and(|output| {
                    !output.is_empty() && !output_excerpt_has_missing_file_evidence(output)
                })
        });
    if has_later_successful_observation {
        return None;
    }
    let path = missing_file_path_from_loop(loop_state, agent_run_context)?;
    let prefer_english = prefer_english_for_user_text(state, user_text);
    let scalar_count = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            route.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarCount
        });
    if prefer_english {
        if scalar_count {
            Some(format!(
                "`{path}` does not exist, so the matching item count cannot be computed."
            ))
        } else {
            Some(format!(
                "I could not find `{path}`, so this request cannot be completed until the path is corrected."
            ))
        }
    } else if scalar_count {
        Some(format!("`{path}` 不存在，无法统计匹配项数量。"))
    } else {
        Some(format!("未找到 `{path}`，请确认路径后再继续。"))
    }
}

fn route_requests_execution_failed_step_answer(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            route.output_contract.semantic_kind == crate::OutputSemanticKind::ExecutionFailedStep
        })
}

fn failed_execution_step_item(
    loop_state: &crate::agent_engine::LoopState,
    step_index: usize,
    step: &crate::executor::StepExecutionResult,
    prefer_english: bool,
) -> String {
    let command = plan_step_for_execution(loop_state, step)
        .and_then(|plan_step| raw_command_arg_from_plan_step(Some(plan_step)))
        .map(|value| truncate_with_ellipsis(&value.replace('`', "'"), 180));
    match command {
        Some(command) if prefer_english => format!("Step {} failed: `{command}`.", step_index + 1),
        Some(command) => format!("第 {} 步失败：`{command}`。", step_index + 1),
        None if prefer_english => format!("Step {} failed.", step_index + 1),
        None => format!("第 {} 步失败。", step_index + 1),
    }
}

fn deterministic_execution_failed_step_answer(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    if !route_requests_execution_failed_step_answer(agent_run_context) {
        return None;
    }
    let steps = observed_execution_status_steps(loop_state);
    if steps.len() < 2 {
        return None;
    }
    let prefer_english = prefer_english_for_user_text(state, user_text);
    let failed = steps
        .iter()
        .enumerate()
        .filter(|(_, step)| !step.is_ok())
        .map(|(idx, step)| failed_execution_step_item(loop_state, idx, step, prefer_english))
        .collect::<Vec<_>>();
    if failed.is_empty() {
        return Some(if prefer_english {
            "No step failed.".to_string()
        } else {
            "没有步骤失败。".to_string()
        });
    }
    Some(failed.join(if prefer_english { " " } else { "" }))
}

fn deterministic_observed_execution_status_summary(
    loop_state: &crate::agent_engine::LoopState,
) -> crate::task_journal::TaskJournalFinalizerSummary {
    crate::task_journal::TaskJournalFinalizerSummary {
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
    }
}

fn path_display_label(value: &serde_json::Value, fallback: &str) -> String {
    let raw = value
        .get("path")
        .or_else(|| value.get("resolved_path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback);
    Path::new(raw)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(raw)
        .to_string()
}

fn compare_paths_size_ratio_answer(body: &str, prefer_english: bool) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(|value| value.as_str()) != Some("compare_paths") {
        return None;
    }
    let left = value.get("left")?;
    let right = value.get("right")?;
    let left_size = left.get("size_bytes").and_then(|value| value.as_u64())?;
    let right_size = right.get("size_bytes").and_then(|value| value.as_u64())?;
    let left_label = path_display_label(left, "left");
    let right_label = path_display_label(right, "right");
    if right_size == 0 {
        return Some(if prefer_english {
            format!(
                "`{right_label}` is 0 bytes, so a size ratio cannot be computed; `{left_label}` is {left_size} bytes."
            )
        } else {
            format!(
                "`{right_label}` 为 0 字节，无法计算相对倍数；`{left_label}` 为 {left_size} 字节。"
            )
        });
    }
    let ratio = left_size as f64 / right_size as f64;
    Some(if prefer_english {
        format!(
            "`{left_label}` is about {ratio:.2}x `{right_label}` ({left_label}={left_size} bytes, {right_label}={right_size} bytes)."
        )
    } else {
        format!(
            "`{left_label}` 大约是 `{right_label}` 的 {ratio:.2} 倍（{left_label}={left_size} 字节，{right_label}={right_size} 字节）。"
        )
    })
}

#[derive(Debug, Clone)]
struct PathSizeFact {
    label: String,
    size_bytes: u64,
}

fn path_batch_size_facts(value: &serde_json::Value) -> Option<Vec<PathSizeFact>> {
    if value.get("action").and_then(|value| value.as_str()) != Some("path_batch_facts") {
        return None;
    }
    let facts = value.get("facts")?.as_array()?;
    let mut out = Vec::new();
    for entry in facts {
        if entry.get("exists").and_then(|value| value.as_bool()) != Some(true) {
            continue;
        }
        let fact = entry.get("fact").unwrap_or(entry);
        let size_bytes = fact
            .get("size_bytes")
            .and_then(|value| value.as_u64())
            .or_else(|| entry.get("size_bytes").and_then(|value| value.as_u64()))?;
        let label = path_display_label(fact, "path");
        out.push(PathSizeFact { label, size_bytes });
    }
    (out.len() >= 2).then_some(out)
}

fn path_batch_size_comparison_answer(body: &str, prefer_english: bool) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let mut facts = path_batch_size_facts(&value)?;
    facts.sort_by(|a, b| {
        b.size_bytes
            .cmp(&a.size_bytes)
            .then_with(|| a.label.cmp(&b.label))
    });
    let largest = facts.first()?;
    let runner_up = facts.get(1)?;
    if largest.size_bytes == runner_up.size_bytes {
        let tied = facts
            .iter()
            .filter(|fact| fact.size_bytes == largest.size_bytes)
            .map(|fact| fact.label.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Some(if prefer_english {
            format!(
                "They are the same size: {tied} are all {} bytes.",
                largest.size_bytes
            )
        } else {
            format!("它们一样大：{tied} 都是 {} 字节。", largest.size_bytes)
        });
    }
    let ratio = if runner_up.size_bytes == 0 {
        None
    } else {
        Some(largest.size_bytes as f64 / runner_up.size_bytes as f64)
    };
    Some(match (prefer_english, ratio) {
        (true, Some(ratio)) => format!(
            "`{}` is larger: {} bytes, about {:.2}x `{}` ({} bytes).",
            largest.label, largest.size_bytes, ratio, runner_up.label, runner_up.size_bytes
        ),
        (true, None) => format!(
            "`{}` is larger: {} bytes; `{}` is 0 bytes.",
            largest.label, largest.size_bytes, runner_up.label
        ),
        (false, Some(ratio)) => format!(
            "`{}` 更大：{} 字节，大约是 `{}`（{} 字节）的 {:.2} 倍。",
            largest.label, largest.size_bytes, runner_up.label, runner_up.size_bytes, ratio
        ),
        (false, None) => format!(
            "`{}` 更大：{} 字节；`{}` 为 0 字节。",
            largest.label, largest.size_bytes, runner_up.label
        ),
    })
}

fn direct_quantity_comparison_from_compare_paths(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::QuantityComparison
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
    {
        return None;
    }
    let prefer_english = prefer_english_for_user_text(state, user_text);
    let answer = loop_state
        .executed_step_results
        .iter()
        .rev()
        .find_map(|step| {
            if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
                return None;
            }
            let output = step.output.as_deref()?;
            compare_paths_size_ratio_answer(output, prefer_english)
                .or_else(|| path_batch_size_comparison_answer(output, prefer_english))
        })?;
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            parsed: true,
            contract_ok: true,
            completion_ok: Some(true),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: 1,
            ..Default::default()
        },
    ))
}

fn has_publishable_synthesis_other_than_status(
    loop_state: &crate::agent_engine::LoopState,
    status_answer: &str,
) -> bool {
    loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .is_some_and(|text| text != status_answer.trim())
}

fn attach_deterministic_observed_execution_status_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut crate::agent_engine::LoopState,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(answer) = deterministic_observed_execution_status_answer(state, user_text, loop_state)
    else {
        return false;
    };
    if has_publishable_synthesis_other_than_status(loop_state, &answer) {
        return false;
    }
    *finalizer_summary = Some(deterministic_observed_execution_status_summary(loop_state));
    loop_state.last_user_visible_respond = Some(answer.clone());
    append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
    info!(
        "delivery fallback_from_deterministic_observed_status task_id={}",
        task.task_id
    );
    true
}

fn attach_deterministic_execution_failed_step_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(answer) =
        deterministic_execution_failed_step_answer(state, user_text, loop_state, agent_run_context)
    else {
        return false;
    };
    *finalizer_summary = Some(deterministic_observed_execution_status_summary(loop_state));
    loop_state.last_user_visible_respond = Some(answer.clone());
    append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
    info!(
        "delivery fallback_from_deterministic_execution_failed_step task_id={}",
        task.task_id
    );
    true
}

fn replace_delivery_with_deterministic_observed_execution_status_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut crate::agent_engine::LoopState,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(answer) = deterministic_observed_execution_status_answer(state, user_text, loop_state)
    else {
        return false;
    };
    if has_publishable_synthesis_other_than_status(loop_state, &answer) {
        return false;
    }
    if loop_state.delivery_messages.last().is_some_and(|message| {
        planned_delivery_identifies_failed_observed_step(message, loop_state)
    }) {
        *finalizer_summary = Some(deterministic_observed_execution_status_summary(loop_state));
        return false;
    }
    let unchanged = loop_state
        .delivery_messages
        .last()
        .map(|message| message.trim() == answer.trim())
        .unwrap_or(false);
    *finalizer_summary = Some(deterministic_observed_execution_status_summary(loop_state));
    loop_state.last_user_visible_respond = Some(answer.clone());
    loop_state
        .delivery_messages
        .retain(|message| crate::finalize::is_execution_summary_message(message));
    if !unchanged {
        append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
        info!(
            "delivery replace_with_deterministic_observed_status task_id={}",
            task.task_id
        );
    }
    true
}

fn replace_delivery_with_deterministic_execution_failed_step_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(answer) =
        deterministic_execution_failed_step_answer(state, user_text, loop_state, agent_run_context)
    else {
        return false;
    };
    let unchanged = loop_state
        .delivery_messages
        .last()
        .map(|message| message.trim() == answer.trim())
        .unwrap_or(false);
    *finalizer_summary = Some(deterministic_observed_execution_status_summary(loop_state));
    loop_state.last_user_visible_respond = Some(answer.clone());
    loop_state
        .delivery_messages
        .retain(|message| crate::finalize::is_execution_summary_message(message));
    if !unchanged {
        append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
        info!(
            "delivery replace_with_deterministic_execution_failed_step task_id={}",
            task.task_id
        );
    }
    true
}

fn planned_delivery_identifies_failed_observed_step(
    delivery: &str,
    loop_state: &crate::agent_engine::LoopState,
) -> bool {
    let delivery = delivery.trim();
    if delivery.is_empty() {
        return false;
    }
    loop_state.executed_step_results.iter().any(|step| {
        !step.is_ok()
            && !matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer"
            )
            && plan_step_for_execution(loop_state, step)
                .and_then(|plan_step| raw_command_arg_from_plan_step(Some(plan_step)))
                .is_some_and(|command| delivery.contains(command))
    })
}

async fn missing_delivery_after_observation_message(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
    clarify_reason: &str,
) -> String {
    if let Some(answer) =
        deterministic_execution_failed_step_answer(state, user_text, loop_state, agent_run_context)
    {
        return answer;
    }
    if let Some(answer) =
        deterministic_observed_execution_status_answer(state, user_text, loop_state)
    {
        return answer;
    }
    let default_text = missing_delivery_after_observation_default_message(state, user_text);
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let contract = crate::fallback::UserResponseContract::tool_failure(
        "final_answer_missing_after_observed_execution",
        user_text,
        &route_resolved_intent(agent_run_context),
        observed_execution_facts_for_missing_delivery(loop_state, clarify_reason),
        vec![
            "Do not claim the task succeeded.".to_string(),
            "Do not ask which item the user wants if execution outputs are already attached."
                .to_string(),
            "Use observed execution facts to explain the blocker or incomplete result."
                .to_string(),
            "Offer one concrete next step only when the observed facts do not already answer the user's request."
                .to_string(),
        ],
        "brief_failure_with_next_step",
        &language_hint,
    );
    crate::fallback::compose_user_response_from_contract_with_default(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::SynthesisEmpty,
        &default_text,
    )
    .await
}

async fn observed_execution_without_publishable_delivery_reply(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary>,
    clarify_reason: &str,
) -> Option<AskReply> {
    let execution_summaries =
        build_execution_summary_messages(loop_state, agent_run_context, Some(user_text));
    let deterministic_answer =
        deterministic_execution_failed_step_answer(state, user_text, loop_state, agent_run_context)
            .or_else(|| {
                deterministic_observed_execution_status_answer(state, user_text, loop_state)
            })
            .or_else(|| {
                deterministic_missing_observed_target_answer(
                    state,
                    user_text,
                    loop_state,
                    agent_run_context,
                )
            });
    let message = missing_delivery_after_observation_message(
        state,
        task,
        user_text,
        loop_state,
        agent_run_context,
        clarify_reason,
    )
    .await;
    let mut delivery_messages = Vec::new();
    delivery_messages.extend(execution_summaries);
    delivery_messages.push(message.clone());
    let delivery_consistent =
        crate::task_journal::delivery_payload_consistent(&message, &delivery_messages);
    let has_deterministic_answer = deterministic_answer.is_some();
    let (final_status, should_fail_task) = observed_execution_without_publishable_delivery_outcome(
        has_deterministic_answer,
        finalizer_summary.as_ref(),
    );
    let journal = build_loop_journal(
        task,
        user_text,
        loop_state,
        agent_run_context,
        finalizer_summary,
        delivery_consistent,
        &message,
        final_status,
    );
    let reply = AskReply::non_llm(message.clone())
        .with_messages(delivery_messages)
        .with_task_journal(journal);
    Some(if should_fail_task {
        reply.with_failure(message)
    } else {
        reply
    })
}

fn observed_synthesis_unavailable_reply(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
    err: &str,
) -> AskReply {
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let context_hint = format!(
        "observed_finalizer={}",
        crate::truncate_for_agent_trace(err)
    );
    let message = crate::fallback::render_clarify_fallback_with_language_hint(
        state,
        &task.task_id,
        crate::fallback::ClarifyFallbackSource::LlmUnavailable,
        Some(&context_hint),
        &language_hint,
    );
    let mut delivery_messages =
        build_execution_summary_messages(loop_state, agent_run_context, Some(user_text));
    delivery_messages.push(message.clone());
    let delivery_consistent =
        crate::task_journal::delivery_payload_consistent(&message, &delivery_messages);
    let finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::AllowFallback),
        parsed: false,
        contract_ok: false,
        completion_ok: Some(false),
        grounded_ok: None,
        format_ok: None,
        needs_clarify: Some(false),
        used_evidence_ids_count: loop_state.executed_step_results.len(),
        ..Default::default()
    });
    let journal = build_loop_journal(
        task,
        user_text,
        loop_state,
        agent_run_context,
        finalizer_summary,
        delivery_consistent,
        &message,
        crate::task_journal::TaskJournalFinalStatus::Failure,
    );
    AskReply::non_llm(message.clone())
        .with_messages(delivery_messages)
        .with_task_journal(journal)
        .with_failure(message)
}

fn observed_execution_without_publishable_delivery_outcome(
    has_deterministic_answer: bool,
    finalizer_summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
) -> (crate::task_journal::TaskJournalFinalStatus, bool) {
    if has_deterministic_answer {
        return (crate::task_journal::TaskJournalFinalStatus::Success, false);
    }
    if finalizer_summary
        .and_then(|summary| summary.needs_clarify)
        .unwrap_or(false)
    {
        return (crate::task_journal::TaskJournalFinalStatus::Clarify, false);
    }
    (crate::task_journal::TaskJournalFinalStatus::Failure, true)
}

fn successful_delivery_final_status(
    loop_state: &crate::agent_engine::LoopState,
    finalizer_summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
) -> crate::task_journal::TaskJournalFinalStatus {
    if loop_state.pending_user_input_required
        || finalizer_summary
            .and_then(|summary| summary.needs_clarify)
            .unwrap_or(false)
    {
        crate::task_journal::TaskJournalFinalStatus::Clarify
    } else {
        crate::task_journal::TaskJournalFinalStatus::Success
    }
}

async fn missing_file_delivery_reply_from_loop(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> Option<AskReply> {
    if !route_requires_file_token(agent_run_context)
        || !has_missing_file_search_evidence(loop_state)
    {
        return None;
    }

    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let missing_path = missing_file_path_from_loop(loop_state, agent_run_context);
    let message = crate::fallback::missing_file_delivery_response_text_for_language(
        state,
        &language_hint,
        missing_path.as_deref(),
    );
    let mut delivery_messages =
        build_execution_summary_messages(loop_state, agent_run_context, Some(user_text));
    delivery_messages.push(message.clone());
    let delivery_consistent =
        crate::task_journal::delivery_payload_consistent(&message, &delivery_messages);
    let journal = build_loop_journal(
        task,
        user_text,
        loop_state,
        agent_run_context,
        finalizer_summary,
        delivery_consistent,
        &message,
        crate::task_journal::TaskJournalFinalStatus::Success,
    );
    Some(
        AskReply::non_llm(message.clone())
            .with_messages(delivery_messages)
            .with_task_journal(journal),
    )
}

// Stage 3.1：build_loop_journal 已搬移到 `crate::finalize::build_from_loop_state`，
// 行为零变化。本文件保留 thin alias 以最小化 diff。
use crate::finalize::build_from_loop_state as build_loop_journal;

pub(crate) async fn finalize_loop_reply(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    mut loop_state: LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<AskReply, String> {
    // §3.3 Stage 3.2 invariant：进入 LOOP REPLY finalize 子层时，
    // ask_state 必须处于 Executing 或 Finalizing 之一。Executing 表示
    // agent loop 刚跑完一轮、本函数即将做最后归约；Finalizing 表示
    // 主路径已经在 ResumeExecuting 分支预先标记过 finalize 阶段。
    // 注：测试环境与未启用 §3.1 注册（registry 未 set）时返回 None，
    // 此时不触发 panic（相当于运行期 noop），release build 完全无开销。
    debug_assert!(
        matches!(
            state.current_ask_state(&task.task_id),
            None | Some(crate::AskState::Executing) | Some(crate::AskState::Finalizing)
        ),
        "finalize_loop_reply invariant: ask_state must be Executing|Finalizing, got {:?} (task_id={})",
        state.current_ask_state(&task.task_id),
        task.task_id,
    );

    backfill_delivery_from_last_outputs(task, &mut loop_state, agent_run_context);

    if let Some((user_error, resume_context)) =
        pending_confirmation_resume_payload(state, task, user_text, &loop_state).await
    {
        let delivery_messages = vec![user_error.clone()];
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&user_error, &delivery_messages);
        let journal = build_loop_journal(
            task,
            user_text,
            &loop_state,
            agent_run_context,
            None,
            delivery_consistent,
            &user_error,
            crate::task_journal::TaskJournalFinalStatus::ResumeFailure,
        );
        return Ok(AskReply::non_llm(user_error.clone())
            .with_messages(delivery_messages)
            .with_task_journal(journal)
            .with_failure(user_error)
            .with_resume_context(resume_context));
    }

    if loop_state.last_stop_signal.as_deref() == Some("recipe_repair_budget_exhausted") {
        let message = execution_recipe_budget_exhausted_message(
            state,
            task,
            user_text,
            &loop_state,
            agent_run_context,
        )
        .await;
        let delivery_messages = vec![message.clone()];
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&message, &delivery_messages);
        let journal = build_loop_journal(
            task,
            user_text,
            &loop_state,
            agent_run_context,
            None,
            delivery_consistent,
            &message,
            crate::task_journal::TaskJournalFinalStatus::Failure,
        );
        return Ok(AskReply::non_llm(message.clone())
            .with_messages(delivery_messages)
            .with_task_journal(journal)
            .with_failure(message));
    }

    let requires_content_evidence = route_requires_content_evidence(agent_run_context);
    discard_meta_respond_placeholder_for_content_evidence(
        state,
        task,
        &mut loop_state,
        requires_content_evidence,
        agent_run_context,
    )
    .await;
    discard_raw_passthrough_delivery_when_structured_answer_available(
        task,
        &mut loop_state,
        agent_run_context,
    );
    backfill_delivery_from_last_outputs(task, &mut loop_state, agent_run_context);
    let mut finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary> = None;
    if should_return_missing_file_delivery_reply(&loop_state, agent_run_context) {
        if let Some(reply) = missing_file_delivery_reply_from_loop(
            state,
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary.clone(),
        )
        .await
        {
            return Ok(reply);
        }
    }
    let should_try_observed_scalar_fallback = crate::finalize::should_attempt_observed_fallback(
        loop_state.has_tool_or_skill_output,
        loop_state.has_recoverable_failure_context,
    ) && loop_state.delivery_messages.is_empty();
    if should_try_observed_scalar_fallback {
        if let Some((answer, summary)) =
            direct_scalar_observed_answer(Some(state), &loop_state, agent_run_context)
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            info!(
                "delivery fallback_from_observed_scalar task_id={}",
                task.task_id
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            direct_non_builtin_skill_raw_answer(state, &loop_state, agent_run_context)
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            info!(
                "delivery fallback_from_non_builtin_skill_raw task_id={}",
                task.task_id
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            direct_structured_observed_answer(Some(state), &loop_state, agent_run_context)
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            info!(
                "delivery fallback_from_observed_structured task_id={}",
                task.task_id
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) = direct_quantity_comparison_from_compare_paths(
            state,
            user_text,
            &loop_state,
            agent_run_context,
        ) {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            info!(
                "delivery fallback_from_compare_paths_quantity task_id={}",
                task.task_id
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some(reply) = missing_file_delivery_reply_from_loop(
            state,
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary.clone(),
        )
        .await
        {
            return Ok(reply);
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            direct_file_token_from_observed_auto_locator_filename(&loop_state, agent_run_context)
                .or_else(|| {
                    direct_file_token_from_observed_inventory(&loop_state, agent_run_context)
                })
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            info!(
                "delivery fallback_from_observed_file_token task_id={}",
                task.task_id
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        attach_deterministic_execution_failed_step_answer(
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        );
    }

    if loop_state.delivery_messages.is_empty() {
        attach_deterministic_observed_execution_status_answer(
            state,
            task,
            user_text,
            &mut loop_state,
            &mut finalizer_summary,
        );
    }

    if loop_state.delivery_messages.is_empty() {
        match crate::agent_engine::observed_output::try_synthesize_answer_from_observed_output(
            state,
            task,
            user_text,
            &loop_state,
            agent_run_context,
        )
        .await
        {
            Ok(Some((answer, summary))) => {
                if matches!(
                    summary.disposition,
                    Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
                ) && !answer.trim().is_empty()
                {
                    finalizer_summary = Some(summary);
                    loop_state.last_user_visible_respond = Some(answer.clone());
                    append_delivery_message(
                        &task.task_id,
                        &mut loop_state.delivery_messages,
                        answer,
                    );
                    info!(
                        "delivery fallback_from_observed_answer task_id={}",
                        task.task_id
                    );
                } else if finalizer_summary.is_none() {
                    finalizer_summary = Some(summary);
                }
            }
            Ok(None) => {}
            Err(err) => {
                return Ok(observed_synthesis_unavailable_reply(
                    state,
                    task,
                    user_text,
                    &loop_state,
                    agent_run_context,
                    &err,
                ));
            }
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            direct_publishable_observed_answer(state, task, &loop_state, agent_run_context).await
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            info!(
                "delivery fallback_from_observed_raw task_id={}",
                task.task_id
            );
        }
    }

    if let Some(marker) = auto_requested_success_marker(
        agent_run_context,
        &loop_state,
        &loop_state.delivery_messages,
    ) {
        let marker_text = marker.to_string();
        loop_state.last_user_visible_respond = Some(marker_text.clone());
        append_delivery_message(
            &task.task_id,
            &mut loop_state.delivery_messages,
            marker_text,
        );
        info!(
            "delivery auto_requested_success_marker task_id={} marker={}",
            task.task_id, marker
        );
    }

    if let Some(reply) = content_evidence_step_failure_reply_from_loop(
        state,
        task,
        user_text,
        &loop_state,
        agent_run_context,
    )
    .await
    {
        return Ok(reply);
    }

    normalize_file_token_delivery_from_auto_locator(&mut loop_state, agent_run_context);
    normalize_file_token_delivery_from_observed_paths(state, &mut loop_state, agent_run_context);
    enforce_delivery_output_contract(state, task, user_text, &mut loop_state, agent_run_context)
        .await;
    replace_placeholder_delivery_with_synthesis(task, &mut loop_state);
    let replaced_contract_answer = replace_delivery_with_loop_contract_observed_answer(
        task,
        &mut loop_state,
        &mut finalizer_summary,
    );
    let replaced_failed_step = if !replaced_contract_answer {
        replace_delivery_with_deterministic_execution_failed_step_answer(
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        )
    } else {
        false
    };
    if !replaced_contract_answer && !replaced_failed_step {
        replace_delivery_with_deterministic_observed_execution_status_answer(
            state,
            task,
            user_text,
            &mut loop_state,
            &mut finalizer_summary,
        );
    }
    replace_delivery_with_latest_tail_read_range_answer(
        state,
        task,
        user_text,
        &mut loop_state,
        agent_run_context,
        &mut finalizer_summary,
    );

    if let Some(reply) = content_evidence_step_failure_reply_from_loop(
        state,
        task,
        user_text,
        &loop_state,
        agent_run_context,
    )
    .await
    {
        return Ok(reply);
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some(reply) = missing_file_delivery_reply_from_loop(
            state,
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary.clone(),
        )
        .await
        {
            return Ok(reply);
        }
    }

    let has_authoritative_delivery = !loop_state.delivery_messages.is_empty();
    if finalizer_requires_clarify(
        finalizer_summary.as_ref(),
        requires_content_evidence,
        has_authoritative_delivery,
    ) {
        let clarify_reason = build_finalizer_clarify_reason(finalizer_summary.as_ref());
        if let Some(reply) = observed_execution_without_publishable_delivery_reply(
            state,
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary.clone(),
            &clarify_reason,
        )
        .await
        {
            return Ok(reply);
        }
        let clarify = crate::intent_router::generate_or_reuse_clarify_question(
            state,
            task,
            user_text,
            &clarify_reason,
            None,
            preferred_route_clarify_question(agent_run_context),
            crate::intent_router::ClarifyQuestionPolicy::SafeFallback,
            // §7.2: finalize 触发 requires_clarify（无 evidence 可合成）→ SynthesisEmpty。
            crate::fallback::ClarifyFallbackSource::SynthesisEmpty,
        )
        .await;
        let delivery_messages = vec![clarify.clone()];
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&clarify, &delivery_messages);
        let journal = build_loop_journal(
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary,
            delivery_consistent,
            &clarify,
            crate::task_journal::TaskJournalFinalStatus::Clarify,
        );
        return Ok(AskReply::non_llm(clarify.clone())
            .with_messages(delivery_messages)
            .with_task_journal(journal));
    }

    let synthesis_is_publishable = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .is_some_and(|text| !text.is_empty());
    let priority_last_respond = if synthesis_is_publishable {
        None
    } else {
        loop_state.last_user_visible_respond.as_ref()
    };
    let (mut delivery_deduped, _, used_last_respond) =
        crate::finalize::build_final_delivery_with_priority(
            &loop_state.delivery_messages,
            priority_last_respond,
        );

    if delivery_deduped.is_empty() {
        if let Some(reply) = missing_file_delivery_reply_from_loop(
            state,
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary.clone(),
        )
        .await
        {
            return Ok(reply);
        }
    }

    if delivery_deduped.is_empty() {
        let clarify_reason = build_missing_delivery_clarify_reason(finalizer_summary.as_ref());
        if let Some(reply) = observed_execution_without_publishable_delivery_reply(
            state,
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary.clone(),
            &clarify_reason,
        )
        .await
        {
            return Ok(reply);
        }
        let clarify = crate::intent_router::generate_or_reuse_clarify_question(
            state,
            task,
            user_text,
            &clarify_reason,
            None,
            preferred_route_clarify_question(agent_run_context),
            crate::intent_router::ClarifyQuestionPolicy::SafeFallback,
            // §7.2: 执行结束但 delivery 全空（最常见的"我需要确认一下..."触发点之一）→ SynthesisEmpty。
            crate::fallback::ClarifyFallbackSource::SynthesisEmpty,
        )
        .await;
        let delivery_messages = vec![clarify.clone()];
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&clarify, &delivery_messages);
        let journal = build_loop_journal(
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary,
            delivery_consistent,
            &clarify,
            crate::task_journal::TaskJournalFinalStatus::Clarify,
        );
        return Ok(AskReply::non_llm(clarify.clone())
            .with_messages(delivery_messages)
            .with_task_journal(journal));
    }

    if let Some(marker) =
        missing_requested_success_marker(agent_run_context, &loop_state, &delivery_deduped)
    {
        let message = execution_recipe_missing_success_marker_message(
            state,
            task,
            user_text,
            marker,
            agent_run_context,
        )
        .await;
        let delivery_messages = vec![message.clone()];
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&message, &delivery_messages);
        let journal = build_loop_journal(
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary,
            delivery_consistent,
            &message,
            crate::task_journal::TaskJournalFinalStatus::Failure,
        );
        return Ok(AskReply::non_llm(message.clone())
            .with_messages(delivery_messages)
            .with_task_journal(journal)
            .with_failure(message));
    }

    prefer_observed_answer_for_exact_contract(
        state,
        &task.task_id,
        &mut loop_state,
        agent_run_context,
        &mut delivery_deduped,
        &mut finalizer_summary,
    );
    let exact_delivery_requested = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(output_contract_requests_exact_delivery)
        .unwrap_or(false);
    if !exact_delivery_requested {
        attach_execution_recipe_closeout_to_delivery(
            Some(state),
            user_text,
            &loop_state,
            agent_run_context,
            &mut delivery_deduped,
        );
        ensure_requested_success_marker_visible(agent_run_context, &mut delivery_deduped);
    }
    attach_execution_summary_to_delivery(
        &loop_state,
        agent_run_context,
        Some(user_text),
        &mut delivery_deduped,
    );

    let final_text = final_answer_text_from_delivery(&delivery_deduped);

    if used_last_respond {
        info!(
            "final_result_source=last_respond task_id={} len={}",
            task.task_id,
            delivery_deduped.len()
        );
    } else if !delivery_deduped.is_empty() {
        info!(
            "final_result_source=delivery_messages task_id={} len={}",
            task.task_id,
            delivery_deduped.len()
        );
    }
    let delivery_consistent =
        crate::task_journal::delivery_payload_consistent(&final_text, &delivery_deduped);

    let mut journal = build_loop_journal(
        task,
        user_text,
        &loop_state,
        agent_run_context,
        finalizer_summary.clone(),
        delivery_consistent,
        &final_text,
        successful_delivery_final_status(&loop_state, finalizer_summary.as_ref()),
    );
    if let Some(route_result) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) {
        if let Some(answer_verifier) = crate::answer_verifier::verify_answer_observe_only(
            state,
            task,
            user_text,
            route_result,
            &journal,
            &final_text,
        )
        .await
        {
            journal.record_answer_verifier_summary(answer_verifier);
        }
    }

    crate::append_act_plan_log(
        state,
        task,
        "loop_done",
        loop_state.total_steps_executed,
        loop_state.subtask_results.len(),
        loop_state.tool_calls_total,
        &format!(
            "rounds={} messages={} no_progress_count={}",
            loop_state.round_no,
            loop_state.delivery_messages.len(),
            loop_state.consecutive_no_progress
        ),
    );
    Ok(AskReply::non_llm(final_text)
        .with_messages(delivery_deduped)
        .with_task_journal(journal))
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, RwLock};

    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        attach_deterministic_observed_execution_status_answer,
        attach_execution_recipe_closeout_to_delivery, attach_execution_summary_to_delivery,
        auto_requested_success_marker, backfill_delivery_from_last_outputs,
        build_execution_summary_message, build_execution_summary_messages,
        compare_paths_size_ratio_answer, content_evidence_step_failure_answer,
        content_evidence_terminal_respond_is_contractual_answer,
        deterministic_missing_observed_target_answer,
        deterministic_observed_execution_status_answer,
        direct_file_token_from_observed_auto_locator_filename,
        direct_file_token_from_observed_inventory, direct_non_builtin_skill_raw_answer,
        direct_publishable_observed_answer, direct_quantity_comparison_from_compare_paths,
        direct_scalar_observed_answer, direct_structured_observed_answer,
        discard_raw_passthrough_delivery_when_structured_answer_available,
        ensure_requested_success_marker_visible, execution_recipe_closeout_note,
        final_answer_text_from_delivery, finalize_loop_reply, finalizer_requires_clarify,
        has_missing_file_search_evidence, latest_file_delivery_observation_is_missing,
        looks_like_raw_command_snapshot, looks_like_structured_machine_output,
        missing_requested_success_marker, normalize_file_token_delivery_from_auto_locator,
        normalize_file_token_delivery_from_observed_paths,
        observed_execution_without_publishable_delivery_outcome,
        observed_execution_without_publishable_delivery_reply,
        observed_synthesis_unavailable_reply, path_batch_size_comparison_answer,
        prefer_observed_answer_for_exact_contract,
        replace_delivery_with_deterministic_execution_failed_step_answer,
        replace_delivery_with_deterministic_observed_execution_status_answer,
        replace_delivery_with_latest_tail_read_range_answer,
        resolve_file_token_from_auto_locator_answer,
        should_drop_passthrough_delivery_for_content_evidence,
        should_return_missing_file_delivery_reply, successful_delivery_final_status,
        verify_summary_requires_resume_confirmation,
    };
    use crate::executor::{StepExecutionResult, StepExecutionStatus};
    use crate::{
        AgentRuntimeConfig, AppState, ClaimedTask, IntentOutputContract, OutputLocatorKind,
        OutputResponseShape, ResumeBehavior, RiskCeiling, RouteResult, ScheduleKind,
        SkillViewsSnapshot, ToolsPolicy, DEFAULT_AGENT_ID,
    };
    use claw_core::config::{AgentConfig, ToolsConfig};

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            let mut path = std::env::temp_dir();
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time before unix epoch")
                .as_nanos();
            path.push(format!(
                "clawd_loop_finalize_{prefix}_{}_{}",
                std::process::id(),
                nanos
            ));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    fn claimed_task(task_id: &str) -> ClaimedTask {
        ClaimedTask {
            task_id: task_id.to_string(),
            user_id: 1,
            chat_id: 1,
            user_key: None,
            channel: "test".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        }
    }

    fn test_state() -> AppState {
        let agents_by_id = HashMap::from([(
            DEFAULT_AGENT_ID.to_string(),
            AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
        )]);
        AppState {
            core: crate::CoreServices {
                agents_by_id: Arc::new(agents_by_id),
                skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                    registry: None,
                    skills_list: Arc::new(
                        ["crypto".to_string(), "stock".to_string()]
                            .into_iter()
                            .collect::<HashSet<_>>(),
                    ),
                }))),
                ..crate::CoreServices::test_default()
            },
            skill_rt: crate::SkillRuntime {
                locator_scan_max_files: 200,
                tools_policy: Arc::new(
                    ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
                ),
                ..crate::SkillRuntime::test_default()
            },
            policy: crate::PolicyConfig::test_default(),
            worker: crate::WorkerConfig::test_default(),
            metrics: crate::TaskMetricsRegistry::default(),
            channels: crate::ChannelConfig::default(),
            reload_ctx: crate::ReloadContext::default(),
            ask_states: crate::AskStateRegistry::default(),
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn verify_summary(
        mode: crate::verifier::VerifyMode,
    ) -> crate::task_journal::TaskJournalVerifySummary {
        crate::task_journal::TaskJournalVerifySummary {
            mode,
            approved: true,
            needs_confirmation: true,
            ..Default::default()
        }
    }

    fn finalizer_summary(
        disposition: crate::finalize::FinalizerDisposition,
    ) -> crate::task_journal::TaskJournalFinalizerSummary {
        crate::task_journal::TaskJournalFinalizerSummary {
            disposition: Some(disposition),
            ..Default::default()
        }
    }

    fn scalar_route_result() -> RouteResult {
        RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "extract scalar".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Filename,
                delivery_intent: Default::default(),
                semantic_kind: Default::default(),
                locator_hint: "package.json".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        }
    }

    fn free_route_result() -> RouteResult {
        let mut route = scalar_route_result();
        route.output_contract.response_shape = OutputResponseShape::Free;
        route.output_contract.requires_content_evidence = false;
        route
    }

    #[test]
    fn compare_paths_size_ratio_answer_computes_ratio_from_structured_output() {
        let answer = compare_paths_size_ratio_answer(
            r#"{"action":"compare_paths","left":{"path":"Cargo.lock","size_bytes":121647},"right":{"path":"Cargo.toml","size_bytes":2606},"comparison":{"same_size":false}}"#,
            false,
        )
        .expect("ratio answer");

        assert!(answer.contains("Cargo.lock"));
        assert!(answer.contains("Cargo.toml"));
        assert!(answer.contains("46.68"));
    }

    #[test]
    fn path_batch_size_comparison_answer_picks_largest_structured_size() {
        let answer = path_batch_size_comparison_answer(
            r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"Cargo.toml","size_bytes":2606},"path":"Cargo.toml"},{"exists":true,"fact":{"kind":"file","path":"Cargo.lock","size_bytes":121647},"path":"Cargo.lock"}]}"#,
            false,
        )
        .expect("size comparison answer");

        assert!(answer.contains("Cargo.lock"));
        assert!(answer.contains("更大"));
        assert!(answer.contains("46.68"));
    }

    #[test]
    fn direct_quantity_comparison_from_compare_paths_recovers_after_synthesis_failure() {
        let state = test_state();
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.has_recoverable_failure_context = true;
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "fs_basic",
            r#"{"action":"compare_paths","left":{"path":"Cargo.lock","resolved_path":"/tmp/Cargo.lock","kind":"file","size_bytes":121647},"right":{"path":"Cargo.toml","resolved_path":"/tmp/Cargo.toml","kind":"file","size_bytes":2606},"comparison":{"same_kind":true,"same_name":false,"same_size":false,"size_delta_bytes":119041,"left_newer":false,"same_content":false}}"#,
        ));
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_2".to_string(),
            skill: "synthesize_answer".to_string(),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some("synthesis failed".to_string()),
            started_at: 0,
            finished_at: 0,
        });
        let mut route = free_route_result();
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "Cargo.lock|Cargo.toml".to_string();
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        let (answer, summary) = direct_quantity_comparison_from_compare_paths(
            &state,
            "比较 Cargo.lock 和 Cargo.toml 的大小，告诉我 lock 大概是 toml 的几倍",
            &loop_state,
            Some(&ctx),
        )
        .expect("structured ratio fallback");

        assert!(answer.contains("46.68"));
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn direct_quantity_comparison_from_path_batch_facts_recovers_after_synthesis_failure() {
        let state = test_state();
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.has_recoverable_failure_context = true;
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "fs_basic",
            r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"Cargo.toml","resolved_path":"/tmp/Cargo.toml","size_bytes":2606},"path":"Cargo.toml"},{"exists":true,"fact":{"kind":"file","path":"Cargo.lock","resolved_path":"/tmp/Cargo.lock","size_bytes":121647},"path":"Cargo.lock"}],"include_missing":true}"#,
        ));
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_2".to_string(),
            skill: "synthesize_answer".to_string(),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some("synthesis failed".to_string()),
            started_at: 0,
            finished_at: 0,
        });
        let mut route = free_route_result();
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "Cargo.toml|Cargo.lock".to_string();
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        let (answer, summary) = direct_quantity_comparison_from_compare_paths(
            &state,
            "比较 Cargo.toml 和 Cargo.lock 哪个更大，顺手用一句通俗话解释原因",
            &loop_state,
            Some(&ctx),
        )
        .expect("structured path facts size fallback");

        assert!(answer.contains("Cargo.lock"));
        assert!(answer.contains("46.68"));
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    fn plan_result_with_steps(steps: Vec<crate::PlanStep>) -> crate::PlanResult {
        crate::PlanResult {
            goal: "test goal".to_string(),
            missing_slots: Vec::new(),
            needs_confirmation: false,
            steps,
            planner_notes: String::new(),
            plan_kind: crate::PlanKind::Single,
            raw_plan_text: String::new(),
        }
    }

    fn ok_step_result(step_id: &str, skill: &str, output: &str) -> StepExecutionResult {
        StepExecutionResult {
            step_id: step_id.to_string(),
            skill: skill.to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(output.to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        }
    }

    fn err_step_result(step_id: &str, skill: &str, error: &str) -> StepExecutionResult {
        StepExecutionResult {
            step_id: step_id.to_string(),
            skill: skill.to_string(),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some(error.to_string()),
            started_at: 1,
            finished_at: 2,
        }
    }

    #[test]
    fn execution_summary_attaches_before_final_delivery_without_changing_final_text() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "list recent logs".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "run_cmd".to_string(),
                    args: serde_json::json!({"command": "ls -t logs | head -2"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }])),
                verify_result: None,
            });
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "run_cmd",
            "model_io.log\nact_plan.log\n",
        ));
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };
        let mut delivery = vec!["这更像运行日志。".to_string()];

        attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

        assert_eq!(delivery.len(), 2);
        assert!(delivery[0].contains("**执行过程**"));
        assert!(delivery[0].contains("命令 `ls -t logs | head -2`"));
        assert!(delivery[0].contains("model_io.log"));
        assert!(delivery[0].contains("act_plan.log"));
        assert_eq!(
            delivery.last().map(String::as_str),
            Some("这更像运行日志。")
        );
        assert!(crate::task_journal::delivery_payload_consistent(
            "这更像运行日志。",
            &delivery
        ));
        assert_eq!(
            final_answer_text_from_delivery(&delivery),
            "这更像运行日志。"
        );
    }

    #[test]
    fn final_answer_text_from_delivery_joins_publishable_chunks() {
        let delivery = vec![
            "**执行过程**\n1. 调用技能 `read_file`".to_string(),
            "第一部分内容。".to_string(),
            "第二部分内容。".to_string(),
        ];

        assert_eq!(
            final_answer_text_from_delivery(&delivery),
            "第一部分内容。\n\n第二部分内容。"
        );
    }

    #[test]
    fn execution_summary_uses_japanese_labels_for_japanese_request() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "logs ディレクトリのファイル名を3つだけ一覧して。".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "system_basic".to_string(),
                    args: serde_json::json!({
                        "action": "inventory_dir",
                        "path": "/tmp/logs",
                        "names_only": true,
                        "max_entries": 3
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }])),
                verify_result: None,
            });
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "system_basic",
            "act_plan.log\nclawd.log\nclawd.run.log\n",
        ));
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            original_user_request: Some(
                "logs ディレクトリのファイル名を3つだけ一覧して。".to_string(),
            ),
            ..Default::default()
        };
        let mut delivery = vec!["act_plan.log\nclawd.log\nclawd.run.log".to_string()];

        attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

        assert_eq!(delivery.len(), 2);
        assert!(delivery[0].contains("**実行過程**"));
        assert!(delivery[0].contains("スキル `system_basic`"));
        assert!(delivery[0].contains("出力："));
        assert!(crate::finalize::is_execution_summary_message(&delivery[0]));
    }

    #[test]
    fn execution_summary_attaches_for_scalar_value_contract() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "extract package name".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "system_basic".to_string(),
                    args: serde_json::json!({
                        "action": "extract_field",
                        "path": "/tmp/package.json",
                        "field_path": "name"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }])),
                verify_result: None,
            });
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","field_path":"name","value_text":"rustclaw-nl-fixture"}"#,
        ));
        let mut route = scalar_route_result();
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut delivery = vec!["rustclaw-nl-fixture".to_string()];

        attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

        assert_eq!(delivery.len(), 2);
        assert!(delivery[0].contains("**执行过程**"));
        assert!(delivery[0].contains("system_basic"));
        assert!(delivery[0].contains("rustclaw-nl-fixture"));
        assert_eq!(
            delivery.last().map(String::as_str),
            Some("rustclaw-nl-fixture")
        );
    }

    #[test]
    fn execution_summary_attaches_each_execution_step_as_separate_delivery() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "tell joke and print pwd".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![
                    crate::PlanStep {
                        step_id: "step_1".to_string(),
                        action_type: "call_tool".to_string(),
                        skill: "run_cmd".to_string(),
                        args: serde_json::json!({"command": "pwd"}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "step_2".to_string(),
                        action_type: "call_tool".to_string(),
                        skill: "run_cmd".to_string(),
                        args: serde_json::json!({"command": "date"}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                ])),
                verify_result: None,
            });
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "run_cmd",
            "/home/guagua/rustclaw\n",
        ));
        loop_state
            .executed_step_results
            .push(ok_step_result("step_2", "run_cmd", "Sun May 3\n"));
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };
        let mut delivery = vec!["为什么程序员喜欢黑夜？因为 bug 比较容易显现。".to_string()];

        attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

        assert_eq!(delivery.len(), 3);
        assert!(delivery[0].contains("命令 `pwd`"));
        assert!(delivery[0].contains("/home/guagua/rustclaw"));
        assert!(delivery[1].contains("命令 `date`"));
        assert!(delivery[1].contains("Sun May 3"));
        assert_eq!(
            delivery.last().map(String::as_str),
            Some("为什么程序员喜欢黑夜？因为 bug 比较容易显现。")
        );
    }

    #[test]
    fn execution_summary_uses_english_labels_for_english_requests() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "list recent logs".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "run_cmd".to_string(),
                    args: serde_json::json!({"command": "ls -t logs | head -2"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }])),
                verify_result: None,
            });
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "run_cmd",
            "model_io.log\nact_plan.log\n",
        ));
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };

        let summary = build_execution_summary_message(
            &loop_state,
            Some(&ctx),
            Some("List the two most recently modified files in logs, then tell me what they are."),
        )
        .expect("execution summary");

        assert!(summary.starts_with("**Execution**"));
        assert!(summary.contains("1. Called command `ls -t logs | head -2`"));
        assert!(summary.contains("   Output:"));
        assert!(crate::finalize::is_execution_summary_message(&summary));
    }

    #[test]
    fn execution_summary_does_not_reuse_same_step_id_from_wrong_round() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "pack archive".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![
                    crate::PlanStep {
                        step_id: "step_1".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "archive_basic".to_string(),
                        args: serde_json::json!({"action": "pack"}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "step_2".to_string(),
                        action_type: "respond".to_string(),
                        skill: "respond".to_string(),
                        args: serde_json::json!({}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                ])),
                verify_result: None,
            });
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 2,
                goal: "verify archive".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![
                    crate::PlanStep {
                        step_id: "step_1".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "system_basic".to_string(),
                        args: serde_json::json!({"action": "path_batch_facts"}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "step_2".to_string(),
                        action_type: "respond".to_string(),
                        skill: "respond".to_string(),
                        args: serde_json::json!({}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                ])),
                verify_result: None,
            });
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "archive_basic",
            "exit=0\n",
        ));
        loop_state.executed_step_results.push(ok_step_result(
            "step_2",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1}"#,
        ));
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };

        let summary = build_execution_summary_message(
            &loop_state,
            Some(&ctx),
            Some("Zip scripts/skill_calls into tmp/nl_archive_case_en.zip, then tell me briefly whether it succeeded."),
        )
        .expect("execution summary");

        assert!(summary.contains("Called skill `archive_basic`"));
        assert!(summary.contains("Called skill `system_basic`"));
        assert!(!summary.contains("Called skill `respond`"));
    }

    #[test]
    fn virtual_tool_execution_summary_uses_tool_label_without_plan_step() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "fs_basic",
            r#"{"action":"inventory_dir","count":5}"#,
        ));
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };

        let summary = build_execution_summary_message(
            &loop_state,
            Some(&ctx),
            Some("列出当前目录最近修改的文件"),
        )
        .expect("execution summary");

        assert!(summary.contains("调用工具 `fs_basic`"));
        assert!(!summary.contains("调用技能 `fs_basic`"));
    }

    #[test]
    fn virtual_tool_execution_summary_uses_tool_label_even_when_plan_used_call_skill() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "compare file sizes".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "fs_basic".to_string(),
                    args: serde_json::json!({"action": "stat_paths"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }])),
                verify_result: None,
            });
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "fs_basic",
            r#"{"action":"path_batch_facts","count":2}"#,
        ));
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };

        let summary =
            build_execution_summary_message(&loop_state, Some(&ctx), Some("Compare file sizes."))
                .expect("execution summary");

        assert!(summary.contains("Called tool `fs_basic`"));
        assert!(!summary.contains("Called skill `fs_basic`"));
    }

    #[tokio::test]
    async fn observed_execution_without_delivery_reply_attaches_raw_summary() {
        let state = test_state();
        let task = claimed_task("task-missing-delivery-observed");
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "list recent logs".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "run_cmd".to_string(),
                    args: serde_json::json!({"command": "ls -t logs | head -2"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }])),
                verify_result: None,
            });
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "run_cmd",
            "model_io.log\nact_plan.log\n",
        ));
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };

        let reply = observed_execution_without_publishable_delivery_reply(
            &state,
            &task,
            "列出 logs 最近两个文件，再判断类型",
            &loop_state,
            Some(&ctx),
            None,
            "no publishable final answer was produced",
        )
        .await
        .expect("observed execution reply");

        assert!(reply.should_fail_task);
        assert_eq!(reply.messages.len(), 2);
        assert!(reply.messages[0].contains("**执行过程**"));
        assert!(reply.messages[0].contains("命令 `ls -t logs | head -2`"));
        assert!(reply.messages[0].contains("model_io.log"));
        assert!(reply.messages[0].contains("act_plan.log"));
        assert!(!reply.text.contains("你最想看的是哪一项"));
    }

    #[test]
    fn observed_synthesis_unavailable_fails_loud_and_keeps_execution_summary() {
        let state = test_state();
        let task = claimed_task("task-observed-llm-unavailable");
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "run_cmd",
            "Cargo.toml\nREADME.md\n",
        ));
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };

        let reply = observed_synthesis_unavailable_reply(
            &state,
            &task,
            "列一下当前目录，然后总结一下",
            &loop_state,
            Some(&ctx),
            "No available LLM provider configured",
        );

        assert!(reply.should_fail_task);
        assert!(reply.text.contains("模型暂时不可用"));
        assert_eq!(reply.messages.last(), Some(&reply.text));
        assert!(reply.messages[0].contains("**执行过程**"));
        assert!(reply.messages[0].contains("Cargo.toml"));
        assert_eq!(
            reply
                .task_journal
                .as_ref()
                .and_then(|journal| journal.final_status),
            Some(crate::task_journal::TaskJournalFinalStatus::Failure)
        );
    }

    #[tokio::test]
    async fn observed_execution_without_delivery_skips_summary_for_extract_field_result() {
        let state = test_state();
        let task = claimed_task("task-missing-field-observed");
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "read package name".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "system_basic".to_string(),
                    args: serde_json::json!({
                        "action": "extract_field",
                        "path": "package.json",
                        "field_path": "name"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }])),
                verify_result: None,
            });
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":false,"field_path":"name","format":"json","path":"package.json","resolved_path":"/tmp/package.json","value":null,"value_text":"","value_type":"null"}"#,
        ));
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };

        let reply = observed_execution_without_publishable_delivery_reply(
            &state,
            &task,
            "读取 package.json 里的 name 字段，只输出值",
            &loop_state,
            Some(&ctx),
            None,
            "no publishable final answer was produced",
        )
        .await
        .expect("observed execution reply");

        assert_eq!(reply.messages.len(), 2);
        assert!(reply.messages[0].contains("**执行过程**"));
        assert!(reply.messages[0].contains("system_basic"));
    }

    #[test]
    fn execution_summary_attaches_for_exact_observed_passthrough_delivery() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "print pwd".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "run_cmd".to_string(),
                    args: serde_json::json!({"command": "pwd"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }])),
                verify_result: None,
            });
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "run_cmd",
            "/home/guagua/rustclaw\n",
        ));
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };
        let mut delivery = vec!["/home/guagua/rustclaw".to_string()];

        attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

        assert_eq!(delivery.len(), 2);
        assert!(delivery[0].contains("**执行过程**"));
        assert!(delivery[0].contains("命令 `pwd`"));
        assert_eq!(
            delivery.last().map(String::as_str),
            Some("/home/guagua/rustclaw")
        );
    }

    #[test]
    fn execution_summary_attaches_for_raw_command_output_route() {
        let mut route = free_route_result();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "run_cmd",
            "/home/guagua/rustclaw\n",
        ));

        let summary = build_execution_summary_message(&loop_state, Some(&ctx), None)
            .expect("raw command output should expose execution process");
        assert!(summary.contains("**执行过程**"));
        assert!(summary.contains("/home/guagua/rustclaw"));
    }

    #[test]
    fn execution_summary_attaches_for_strict_content_excerpt_contract() {
        let mut route = free_route_result();
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "read tail".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "system_basic".to_string(),
                    args: serde_json::json!({
                        "action": "read_range",
                        "path": "/tmp/model_io.log",
                        "mode": "tail",
                        "n": 10
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }])),
                verify_result: None,
            });
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","excerpt":"1|alpha\n2|beta","path":"/tmp/model_io.log"}"#,
        ));

        let summary = build_execution_summary_message(&loop_state, Some(&ctx), None)
            .expect("strict content excerpt should expose execution process");

        assert!(summary.contains("**执行过程**"));
        assert!(summary.contains("system_basic"));
        assert!(summary.contains("read_range"));
        assert!(summary.contains("alpha"));
    }

    #[test]
    fn execution_summary_sanitizes_log_excerpt_secrets_and_ansi() {
        let mut route = free_route_result();
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","excerpt":"1|\u001b[32mconnected\u001b[0m to wss://host/ws?device_id=123&access_key=abc123&service_id=7&ticket=deadbeef","path":"/tmp/feishud.log"}"#,
        ));

        let summary = build_execution_summary_message(&loop_state, Some(&ctx), None)
            .expect("strict content excerpt should expose sanitized execution process");

        assert!(summary.contains("**执行过程**"));
        assert!(summary.contains("access_key=[REDACTED]"));
        assert!(summary.contains("ticket=[REDACTED]"));
        assert!(!summary.contains("\\u001b"));
        assert!(!summary.contains("abc123"));
        assert!(!summary.contains("deadbeef"));
    }

    #[test]
    fn execution_summary_attaches_for_exact_file_names_contract() {
        let mut route = free_route_result();
        route.output_contract.locator_hint = "document".to_string();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "list_dir",
            "alpha.md\nbeta.md\n",
        ));
        let mut delivery = vec!["alpha.md\nbeta.md".to_string()];

        attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

        assert_eq!(delivery.len(), 2);
        assert!(delivery[0].contains("**执行过程**"));
        assert!(delivery[0].contains("list_dir"));
        assert_eq!(delivery[1], "alpha.md\nbeta.md");
        assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_some());
    }

    #[test]
    fn execution_summary_language_uses_original_user_request_before_resolved_text() {
        let mut route = free_route_result();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            original_user_request: Some("先列出 logs 目录下前 5 个文件名".to_string()),
            user_request: Some("List the first five filenames under logs.".to_string()),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "list_dir",
            "act_plan.log\nclawd.log\n",
        ));

        let summary = build_execution_summary_message(
            &loop_state,
            Some(&ctx),
            Some("List the first five filenames under logs."),
        )
        .expect("execution summary should be attached");

        assert!(summary.starts_with("**执行过程**"));
        assert!(summary.contains("调用"));
        assert!(!summary.starts_with("**Execution**"));
    }

    #[test]
    fn execution_summary_attaches_for_failed_file_token_delivery() {
        let mut route = free_route_result();
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "send file".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: serde_json::json!({"path": "/tmp/missing.txt"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }])),
                verify_result: None,
            });
        loop_state.executed_step_results.push(err_step_result(
            "step_1",
            "read_file",
            "__RC_READ_FILE_NOT_FOUND__:/tmp/missing.txt",
        ));
        let mut delivery = vec!["File not found at the provided path.".to_string()];

        attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

        assert_eq!(delivery.len(), 2);
        assert!(delivery[0].contains("**执行过程**"));
        assert!(delivery[0].contains("read_file"));
        assert!(delivery[0].contains("file not found"));
        assert_eq!(
            delivery.last().map(String::as_str),
            Some("File not found at the provided path.")
        );
    }

    #[test]
    fn execution_summary_attaches_for_existence_with_path_contract() {
        let mut route = free_route_result();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
        route.output_contract.locator_hint = "rustclaw.service".to_string();
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","count":1,"results":["rustclaw.service"]}"#,
        ));
        let mut delivery = vec!["有，路径：rustclaw.service".to_string()];

        attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

        assert_eq!(delivery.len(), 2);
        assert!(delivery[0].contains("**执行过程**"));
        assert!(delivery[0].contains("fs_search"));
        assert_eq!(delivery[1], "有，路径：rustclaw.service");
        assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_some());
    }

    #[test]
    fn execution_summary_attaches_for_sqlite_table_names_contract() {
        let mut route = free_route_result();
        route.output_contract.response_shape = OutputResponseShape::Strict;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::SqliteTableNamesOnly;
        route.output_contract.locator_hint = "/tmp/test.sqlite".to_string();
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "list sqlite tables".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: serde_json::json!({
                        "command": "sqlite3 /tmp/test.sqlite \"SELECT name FROM sqlite_master WHERE type='table' ORDER BY name;\""
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }])),
                verify_result: None,
            });
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "run_cmd",
            "orders\nusers\n",
        ));
        let mut delivery = vec!["这个 SQLite 数据库里有表：orders、users。".to_string()];

        attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

        assert_eq!(delivery.len(), 2);
        assert!(delivery[0].contains("**执行过程**"));
        assert!(delivery[0].contains("sqlite3 /tmp/test.sqlite"));
        assert!(delivery[0].contains("orders"));
        assert!(delivery[0].contains("users"));
        assert_eq!(
            delivery.last().map(String::as_str),
            Some("这个 SQLite 数据库里有表：orders、users。")
        );
    }

    #[test]
    fn execution_summary_includes_direct_fs_search_structured_observation() {
        let route = free_route_result();
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","count":1,"results":["rustclaw.service"],"root":""}"#,
        ));
        let mut delivery = vec!["有，路径：rustclaw.service".to_string()];

        attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

        assert_eq!(delivery.len(), 2);
        assert!(crate::finalize::is_execution_summary_message(&delivery[0]));
        assert!(delivery[0].contains("fs_search"));
        assert!(delivery[0].contains("rustclaw.service"));
        assert_eq!(
            delivery.last().map(String::as_str),
            Some("有，路径：rustclaw.service")
        );
    }

    #[test]
    fn execution_summary_includes_scalar_contract_without_reading_user_text() {
        let mut route = free_route_result();
        route.output_contract.response_shape = OutputResponseShape::Scalar;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::HiddenEntriesCheck;
        route.output_contract.locator_hint = ".".to_string();
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "list_dir",
            ".git\n.gitignore\n",
        ));
        let mut delivery = vec!["有。示例：.git, .gitignore".to_string()];

        attach_execution_summary_to_delivery(
            &loop_state,
            Some(&ctx),
            Some("plain runtime text that is intentionally ignored"),
            &mut delivery,
        );

        assert_eq!(delivery.len(), 2);
        assert!(crate::finalize::is_execution_summary_message(&delivery[0]));
        assert!(delivery[0].contains("list_dir"));
        assert!(delivery[0].contains(".git"));
        assert_eq!(
            delivery.last().map(String::as_str),
            Some("有。示例：.git, .gitignore")
        );
    }

    #[test]
    fn exact_file_names_contract_prefers_observed_list_over_synthesized_sentence() {
        let state = test_state();
        let mut route = free_route_result();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_hint = "document".to_string();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "list_dir",
            "alpha.md\nbeta.md\n",
        ));
        loop_state.executed_step_results.push(ok_step_result(
            "step_2",
            "synthesize_answer",
            "document 目录下有 alpha.md 和 beta.md。",
        ));
        let mut delivery = vec!["document 目录下有 alpha.md 和 beta.md。".to_string()];
        let mut finalizer_summary = None;

        prefer_observed_answer_for_exact_contract(
            &state,
            "task_test",
            &mut loop_state,
            Some(&ctx),
            &mut delivery,
            &mut finalizer_summary,
        );

        assert_eq!(delivery, vec!["alpha.md\nbeta.md"]);
        assert!(finalizer_summary.is_some());
    }

    #[test]
    fn execution_summary_truncates_long_outputs_with_ascii_ellipsis() {
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        let long_output = format!("{}END", "x".repeat(1000));
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "system_basic",
            &long_output,
        ));

        let summary = build_execution_summary_message(&loop_state, Some(&ctx), None)
            .expect("execution summary");

        assert!(summary.contains("..."));
        assert!(!summary.contains("END"));
        assert!(
            summary.len() < 700,
            "summary should stay compact, got {} chars",
            summary.len()
        );
    }

    #[test]
    fn preferred_route_clarify_question_only_uses_explicit_route_clarify() {
        let mut route = scalar_route_result();
        route.needs_clarify = true;
        route.clarify_question = "请确认要读取哪个文件？".to_string();
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        assert_eq!(
            super::preferred_route_clarify_question(Some(&ctx)),
            Some("请确认要读取哪个文件？")
        );

        let mut route = scalar_route_result();
        route.clarify_question = "不会被复用".to_string();
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        assert_eq!(super::preferred_route_clarify_question(Some(&ctx)), None);
    }

    #[test]
    fn confirmation_resume_requires_enforce_mode() {
        let mut verify = verify_summary(crate::verifier::VerifyMode::ObserveOnly);
        assert!(!verify_summary_requires_resume_confirmation(&verify));

        verify.mode = crate::verifier::VerifyMode::Enforce;
        assert!(verify_summary_requires_resume_confirmation(&verify));

        verify.approved = false;
        assert!(!verify_summary_requires_resume_confirmation(&verify));
    }

    #[test]
    fn content_evidence_routes_require_clarify_without_qualified_completion() {
        assert!(finalizer_requires_clarify(None, true, false));
        assert!(!finalizer_requires_clarify(None, true, true));

        let allow_fallback =
            finalizer_summary(crate::finalize::FinalizerDisposition::AllowFallback);
        assert!(finalizer_requires_clarify(
            Some(&allow_fallback),
            true,
            false
        ));
        assert!(!finalizer_requires_clarify(
            Some(&allow_fallback),
            true,
            true
        ));

        let qualified =
            finalizer_summary(crate::finalize::FinalizerDisposition::QualifiedCompletion);
        assert!(!finalizer_requires_clarify(Some(&qualified), true, false));
        assert!(!finalizer_requires_clarify(None, false, false));
    }

    #[test]
    fn missing_publishable_delivery_can_finish_as_clarify() {
        let summary = crate::task_journal::TaskJournalFinalizerSummary {
            needs_clarify: Some(true),
            ..Default::default()
        };

        let (status, should_fail) =
            observed_execution_without_publishable_delivery_outcome(false, Some(&summary));
        assert_eq!(status, crate::task_journal::TaskJournalFinalStatus::Clarify);
        assert!(!should_fail);

        let (status, should_fail) =
            observed_execution_without_publishable_delivery_outcome(true, Some(&summary));
        assert_eq!(status, crate::task_journal::TaskJournalFinalStatus::Success);
        assert!(!should_fail);

        let (status, should_fail) =
            observed_execution_without_publishable_delivery_outcome(false, None);
        assert_eq!(status, crate::task_journal::TaskJournalFinalStatus::Failure);
        assert!(should_fail);
    }

    #[test]
    fn successful_delivery_can_preserve_structured_user_input_clarify() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        assert_eq!(
            successful_delivery_final_status(&loop_state, None),
            crate::task_journal::TaskJournalFinalStatus::Success
        );

        loop_state.pending_user_input_required = true;
        assert_eq!(
            successful_delivery_final_status(&loop_state, None),
            crate::task_journal::TaskJournalFinalStatus::Clarify
        );
    }

    #[tokio::test]
    async fn content_evidence_step_failure_answer_reports_real_error() {
        let state = test_state();
        let task = claimed_task("task-content-error-direct");
        let mut route = free_route_result();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_hint = "/etc/shadow".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some(format!(
                "__RC_SKILL_ERROR__:{}",
                serde_json::json!({
                    "skill": "system_basic",
                    "error_kind": "permission_denied",
                    "error_text": "read_range failed for /etc/shadow",
                    "platform": "linux",
                    "extra": {
                        "operation": "metadata",
                        "path": "/etc/shadow"
                    }
                })
            )),
            started_at: 0,
            finished_at: 0,
        });

        let (answer, summary) = content_evidence_step_failure_answer(
            &state,
            &task,
            "读 /etc/shadow 第一行",
            &loop_state,
            Some(&agent_run_context),
        )
        .await
        .expect("content evidence failure should be publishable");

        assert!(answer.contains("`/etc/shadow`"));
        assert!(answer.to_ascii_lowercase().contains("permission denied"));
        assert!(answer.contains("`clawd` 进程当前没有 sudo/root 权限"));
        assert_eq!(summary.grounded_ok, Some(true));
        assert_eq!(summary.completion_ok, Some(true));
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[tokio::test]
    async fn content_evidence_recoverable_crypto_account_error_is_completion() {
        let state = test_state();
        let task = claimed_task("task-crypto-account-error");
        let mut route = free_route_result();
        route.output_contract.requires_content_evidence = true;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let err = r#"__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__:{"exchange":"binance","detail":"binance error status=401: {\"code\":-2015,\"msg\":\"Invalid API-key, IP, or permissions for action.\"}"}"#;
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state
            .executed_step_results
            .push(err_step_result("step_1", "crypto", err));

        let (answer, summary) = content_evidence_step_failure_answer(
            &state,
            &task,
            "查一下我现在的持仓。",
            &loop_state,
            Some(&agent_run_context),
        )
        .await
        .expect("recoverable crypto account error should be publishable");

        assert!(answer.contains("crypto account access failed on binance"));
        assert!(!answer.contains("__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__"));
        assert_eq!(summary.completion_ok, Some(true));
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn execution_summary_normalizes_recoverable_crypto_account_error() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_recoverable_failure_context = true;
        let err = r#"__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__:{"exchange":"binance","detail":"binance error status=401: {\"code\":-2015,\"msg\":\"Invalid API-key, IP, or permissions for action.\"}"}"#;
        loop_state
            .executed_step_results
            .push(err_step_result("step_1", "crypto", err));

        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };
        let summaries = build_execution_summary_messages(
            &loop_state,
            Some(&agent_run_context),
            Some("查一下持仓"),
        );

        assert_eq!(summaries.len(), 1);
        assert!(summaries[0].contains("crypto account access failed on binance"));
        assert!(!summaries[0].contains("__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__"));
    }

    #[test]
    fn deterministic_observed_execution_status_answer_reports_mixed_results() {
        let state = test_state();
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "health_check",
            r#"{"ok":true}"#,
        ));
        loop_state.executed_step_results.push(err_step_result(
            "step_2",
            "run_cmd",
            "Command failed with exit code 127\nstderr:\nmissing command",
        ));

        let answer = deterministic_observed_execution_status_answer(
            &state,
            "先检查健康，再执行缺失命令，然后总结哪一步成功了、哪一步失败了。",
            &loop_state,
        )
        .expect("mixed observed results should produce deterministic answer");

        assert!(answer.contains("第 1 步 `health_check` 成功"));
        assert!(answer.contains("第 2 步 `run_cmd` 失败"));
        assert!(answer.contains("exit code 127"));
    }

    #[test]
    fn deterministic_missing_observed_target_answer_reports_missing_scalar_count_path() {
        let state = test_state();
        let mut route = free_route_result();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.response_shape = OutputResponseShape::Scalar;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "configs/config_copy".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"configs/config_copy"}],"include_missing":true}"#,
        ));

        let answer = deterministic_missing_observed_target_answer(
            &state,
            "查一下 configs/config_copy 下面有几个 toml 文件",
            &loop_state,
            Some(&agent_run_context),
        )
        .expect("missing path observation should produce a handled user answer");

        assert!(answer.contains("configs/config_copy"));
        assert!(answer.contains("不存在"));
        assert!(answer.contains("无法统计"));
    }

    #[test]
    fn deterministic_missing_observed_target_answer_skips_after_later_fallback_success() {
        let state = test_state();
        let mut route = free_route_result();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.response_shape = OutputResponseShape::Scalar;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "plan/missing.md".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"plan/missing.md"}]}"#,
        ));
        loop_state.executed_step_results.push(ok_step_result(
            "step_2",
            "fs_search",
            r#"{"action":"find_name","count":1,"patterns":["agent_intelligence"],"results":["plan/agent_intelligence_architecture_plan_20260511.md"],"root":"plan"}"#,
        ));

        assert!(deterministic_missing_observed_target_answer(
            &state,
            "读取缺失文件；如果不存在，就搜索 fallback 文件。",
            &loop_state,
            Some(&agent_run_context),
        )
        .is_none());

        let (answer, _) =
            direct_scalar_observed_answer(Some(&state), &loop_state, Some(&agent_run_context))
                .expect("fallback success should become scalar answer");
        assert_eq!(
            answer,
            "plan/agent_intelligence_architecture_plan_20260511.md"
        );
    }

    #[test]
    fn direct_structured_observed_answer_prefers_latest_path_result_for_exact_contract() {
        let state = test_state();
        let mut route = free_route_result();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.response_shape = OutputResponseShape::Strict;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "plan".to_string();
        route.resolved_intent =
            "If the first plan path is missing, find execution_intent markdown files under plan"
                .to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"plan/missing.md"}]}"#,
        ));
        loop_state.executed_step_results.push(err_step_result(
            "step_2",
            "read_file",
            "file not found: /home/guagua/rustclaw/plan/missing.md",
        ));
        loop_state.executed_step_results.push(ok_step_result(
            "step_3",
            "fs_search",
            r#"{"action":"find_name","count":2,"patterns":["execution_intent"],"results":["plan/execution_intent_route_trace_cases.txt","plan/execution_intent_routing_repair_plan_20260509.md"],"root":"plan"}"#,
        ));

        let (answer, summary) =
            direct_structured_observed_answer(Some(&state), &loop_state, Some(&agent_run_context))
                .expect("latest structured path result should answer exact path contract");

        assert!(answer.contains("plan/execution_intent_route_trace_cases.txt"));
        assert!(answer.contains("plan/execution_intent_routing_repair_plan_20260509.md"));
        assert!(!answer.contains("第 1 步"), "answer: {answer}");
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn exact_path_observed_answer_replaces_step_status_after_fallback_success() {
        let state = test_state();
        let mut route = free_route_result();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.response_shape = OutputResponseShape::Strict;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "plan".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state.executed_step_results.push(err_step_result(
            "step_1",
            "read_file",
            "file not found: /home/guagua/rustclaw/plan/missing.md",
        ));
        loop_state.executed_step_results.push(ok_step_result(
            "step_2",
            "fs_search",
            r#"{"action":"find_ext","count":1,"ext":"md","patterns":["execution_intent.md"],"results":["plan/execution_intent_routing_repair_plan_20260509.md"],"root":"plan"}"#,
        ));
        let status_summary = "第 1 步 read_file 失败。第 2 步 fs_search 成功。".to_string();
        loop_state.last_publishable_synthesis_output = Some(status_summary.clone());
        let mut delivery_messages = vec![status_summary];
        let mut finalizer_summary = None;

        prefer_observed_answer_for_exact_contract(
            &state,
            "task-exact-path-fallback",
            &mut loop_state,
            Some(&agent_run_context),
            &mut delivery_messages,
            &mut finalizer_summary,
        );

        assert_eq!(
            delivery_messages,
            vec!["plan/execution_intent_routing_repair_plan_20260509.md".to_string()]
        );
        assert_eq!(
            loop_state.last_user_visible_respond.as_deref(),
            Some("plan/execution_intent_routing_repair_plan_20260509.md")
        );
        assert!(
            !delivery_messages[0].contains("第 1 步"),
            "answer: {}",
            delivery_messages[0]
        );
    }

    #[test]
    fn path_locator_observed_answer_replaces_step_status_after_fallback_success() {
        let state = test_state();
        let mut route = free_route_result();
        route.ask_mode = crate::AskMode::planner_execute_plain();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.response_shape = OutputResponseShape::Free;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "plan/extra_missing_repair_probe.md".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state.executed_step_results.push(err_step_result(
            "step_1",
            "read_file",
            "file not found: /home/guagua/rustclaw/plan/extra_missing_repair_probe.md",
        ));
        loop_state.executed_step_results.push(ok_step_result(
            "step_2",
            "fs_search",
            r#"{"action":"find_name","count":2,"patterns":["execution_intent"],"results":["plan/execution_intent_route_trace_cases.txt","plan/execution_intent_routing_repair_plan_20260509.md"],"root":"plan"}"#,
        ));
        let status_summary = "第 1 步 `read_file` 失败。第 2 步 `fs_search` 成功。".to_string();
        loop_state.last_publishable_synthesis_output = Some(status_summary.clone());
        let mut delivery_messages = vec![status_summary];
        let mut finalizer_summary = None;

        prefer_observed_answer_for_exact_contract(
            &state,
            "task-path-locator-fallback",
            &mut loop_state,
            Some(&agent_run_context),
            &mut delivery_messages,
            &mut finalizer_summary,
        );

        assert_eq!(
            delivery_messages,
            vec![
                "plan/execution_intent_route_trace_cases.txt\nplan/execution_intent_routing_repair_plan_20260509.md"
                    .to_string()
            ]
        );
        assert!(
            !delivery_messages[0].contains("第 1 步"),
            "answer: {}",
            delivery_messages[0]
        );
    }

    #[test]
    fn strict_existence_path_observed_answer_replaces_step_status_after_fallback_success() {
        let state = test_state();
        let mut route = free_route_result();
        route.ask_mode = crate::AskMode::planner_execute_plain();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.response_shape = OutputResponseShape::Strict;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "plan/extra_missing_repair_probe.md".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state.executed_step_results.push(err_step_result(
            "step_1",
            "read_file",
            "file not found: /home/guagua/rustclaw/plan/extra_missing_repair_probe.md",
        ));
        loop_state.executed_step_results.push(ok_step_result(
            "step_2",
            "fs_search",
            r#"{"action":"find_name","count":1,"patterns":["execution_intent.md"],"results":["plan/execution_intent_routing_repair_plan_20260509.md"],"root":"plan"}"#,
        ));
        let status_summary = "第 1 步 `read_file` 失败。第 2 步 `fs_search` 成功。".to_string();
        loop_state.last_publishable_synthesis_output = Some(status_summary.clone());
        let mut delivery_messages = vec![status_summary];
        let mut finalizer_summary = None;

        prefer_observed_answer_for_exact_contract(
            &state,
            "task-strict-existence-path-fallback",
            &mut loop_state,
            Some(&agent_run_context),
            &mut delivery_messages,
            &mut finalizer_summary,
        );

        assert_eq!(
            delivery_messages,
            vec!["plan/execution_intent_routing_repair_plan_20260509.md".to_string()]
        );
        assert!(
            !delivery_messages[0].contains("第 1 步"),
            "answer: {}",
            delivery_messages[0]
        );
    }

    #[test]
    fn scalar_path_observed_answer_replaces_step_status_after_broad_fallback_search() {
        let state = test_state();
        let mut route = free_route_result();
        route.ask_mode = crate::AskMode::planner_execute_plain();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.response_shape = OutputResponseShape::Scalar;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
        route.output_contract.locator_kind = OutputLocatorKind::Filename;
        route.output_contract.locator_hint = "plan/extra_missing_repair_probe.md".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state.executed_step_results.push(err_step_result(
            "step_1",
            "read_file",
            "file not found: /home/guagua/rustclaw/plan/extra_missing_repair_probe.md",
        ));
        loop_state.executed_step_results.push(ok_step_result(
            "step_2",
            "fs_search",
            r#"{"action":"find_name","count":2,"patterns":["execution_intent"],"results":["plan/execution_intent_route_trace_cases.txt","plan/execution_intent_routing_repair_plan_20260509.md"],"root":"plan"}"#,
        ));
        let status_summary = "第 1 步 `read_file` 失败。第 2 步 `fs_search` 成功。".to_string();
        loop_state.last_publishable_synthesis_output = Some(status_summary.clone());
        let mut delivery_messages = vec![status_summary];
        let mut finalizer_summary = None;

        prefer_observed_answer_for_exact_contract(
            &state,
            "task-scalar-path-fallback",
            &mut loop_state,
            Some(&agent_run_context),
            &mut delivery_messages,
            &mut finalizer_summary,
        );

        assert!(
            delivery_messages[0].ends_with("plan/execution_intent_routing_repair_plan_20260509.md"),
            "answer: {}",
            delivery_messages[0]
        );
        assert!(
            !delivery_messages[0].contains("第 1 步"),
            "answer: {}",
            delivery_messages[0]
        );
    }

    #[test]
    fn scalar_observed_answer_replaces_run_cmd_step_status_after_fallback_success() {
        let state = test_state();
        let mut route = free_route_result();
        route.ask_mode = crate::AskMode::planner_execute_plain();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.response_shape = OutputResponseShape::Scalar;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        let err = format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "run_cmd",
                "error_kind": "nonzero_exit",
                "error_text": "Command failed with exit code 127",
                "platform": "linux",
                "extra": {
                    "exit_code": 127,
                    "exit_category": "command_not_found",
                    "stderr": "missing command",
                    "output_truncated": false
                }
            })
        );
        loop_state
            .executed_step_results
            .push(err_step_result("step_1", "run_cmd", &err));
        loop_state.executed_step_results.push(ok_step_result(
            "step_2",
            "run_cmd",
            "/usr/bin/bash\n",
        ));
        let status_summary = "第 1 步 `run_cmd` 失败。第 2 步 `run_cmd` 成功。".to_string();
        loop_state.last_publishable_synthesis_output = Some(status_summary.clone());
        let mut delivery_messages = vec![status_summary];
        let mut finalizer_summary = None;

        prefer_observed_answer_for_exact_contract(
            &state,
            "task-scalar-run-cmd-fallback",
            &mut loop_state,
            Some(&agent_run_context),
            &mut delivery_messages,
            &mut finalizer_summary,
        );

        assert_eq!(delivery_messages, vec!["/usr/bin/bash".to_string()]);
        assert_eq!(
            loop_state.last_user_visible_respond.as_deref(),
            Some("/usr/bin/bash")
        );
    }

    #[test]
    fn loop_contract_scalar_observed_answer_replaces_status_but_keeps_progress() {
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        let mut contract = scalar_route_result().output_contract;
        contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
        loop_state.output_contract = Some(contract);
        loop_state.executed_step_results.push(err_step_result(
            "step_1",
            "run_cmd",
            "command failed",
        ));
        loop_state.executed_step_results.push(ok_step_result(
            "step_2",
            "run_cmd",
            "/usr/bin/bash\n",
        ));
        loop_state.delivery_messages.push(
            "**执行过程**\n1. 调用命令 `missing`\n   错误：\n```text\ncommand failed\n```"
                .to_string(),
        );
        loop_state
            .delivery_messages
            .push("第 1 步 `run_cmd` 失败。第 2 步 `run_cmd` 成功。".to_string());
        let task = claimed_task("task-loop-contract-scalar");
        let mut finalizer_summary = None;

        assert!(super::replace_delivery_with_loop_contract_observed_answer(
            &task,
            &mut loop_state,
            &mut finalizer_summary,
        ));

        assert_eq!(loop_state.delivery_messages.len(), 2);
        assert!(loop_state.delivery_messages[0].contains("执行过程"));
        assert_eq!(loop_state.delivery_messages[1], "/usr/bin/bash");
        assert_eq!(
            loop_state.last_user_visible_respond.as_deref(),
            Some("/usr/bin/bash")
        );
    }

    #[test]
    fn loop_contract_path_observed_answer_replaces_status_but_keeps_progress() {
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        let mut contract = scalar_route_result().output_contract;
        contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
        loop_state.output_contract = Some(contract);
        loop_state.executed_step_results.push(err_step_result(
            "step_1",
            "read_file",
            "file not found: plan/missing.md",
        ));
        loop_state.executed_step_results.push(ok_step_result(
            "step_2",
            "fs_search",
            r#"{"action":"find_ext","count":1,"results":["plan/execution_intent_routing_repair_plan_20260509.md"]}"#,
        ));
        loop_state.delivery_messages.push(
            "**执行过程**\n1. 调用技能 `read_file`\n   错误：\n```text\nfile not found\n```"
                .to_string(),
        );
        loop_state
            .delivery_messages
            .push("Step 1 `read_file` failed. Step 2 `fs_search` succeeded.".to_string());
        let task = claimed_task("task-loop-contract-path");
        let mut finalizer_summary = None;

        assert!(super::replace_delivery_with_loop_contract_observed_answer(
            &task,
            &mut loop_state,
            &mut finalizer_summary,
        ));

        assert_eq!(loop_state.delivery_messages.len(), 2);
        assert!(loop_state.delivery_messages[0].contains("执行过程"));
        assert_eq!(
            loop_state.delivery_messages[1],
            "plan/execution_intent_routing_repair_plan_20260509.md"
        );
        assert_eq!(
            loop_state.last_user_visible_respond.as_deref(),
            Some("plan/execution_intent_routing_repair_plan_20260509.md")
        );
    }

    #[test]
    fn loop_contract_observed_answer_preserves_explicit_json_delivery() {
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        let mut contract = scalar_route_result().output_contract;
        contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
        loop_state.output_contract = Some(contract);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "system_basic",
            r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#,
        ));
        loop_state
            .delivery_messages
            .push("**执行过程**\n1. 调用技能 `system_basic`".to_string());
        loop_state
            .delivery_messages
            .push(r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#.to_string());
        let task = claimed_task("task-loop-contract-json");
        let mut finalizer_summary = None;

        assert!(!super::replace_delivery_with_loop_contract_observed_answer(
            &task,
            &mut loop_state,
            &mut finalizer_summary,
        ));

        assert_eq!(
            loop_state.delivery_messages.last().map(String::as_str),
            Some(r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#)
        );
        assert!(finalizer_summary.is_none());
    }

    #[test]
    fn loop_contract_observed_answer_requires_contract_evidence_completeness() {
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        let mut contract = scalar_route_result().output_contract;
        contract.response_shape = crate::OutputResponseShape::Scalar;
        contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
        loop_state.output_contract = Some(contract);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "run_cmd",
            "a short answer\n",
        ));
        loop_state
            .delivery_messages
            .push("Step 1 `run_cmd` succeeded.".to_string());
        let task = claimed_task("task-loop-contract-incomplete-evidence");
        let mut finalizer_summary = None;

        assert!(!super::replace_delivery_with_loop_contract_observed_answer(
            &task,
            &mut loop_state,
            &mut finalizer_summary,
        ));

        assert_eq!(
            loop_state.delivery_messages.last().map(String::as_str),
            Some("Step 1 `run_cmd` succeeded.")
        );
        assert!(finalizer_summary.is_none());
    }

    #[test]
    fn loop_contract_observed_answer_does_not_hide_later_failure() {
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        let mut contract = scalar_route_result().output_contract;
        contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
        loop_state.output_contract = Some(contract);
        loop_state
            .executed_step_results
            .push(ok_step_result("step_1", "run_cmd", "/tmp/value\n"));
        loop_state.executed_step_results.push(err_step_result(
            "step_2",
            "run_cmd",
            "command failed",
        ));
        loop_state
            .delivery_messages
            .push("Step 2 `run_cmd` failed.".to_string());
        let task = claimed_task("task-loop-contract-later-failure");
        let mut finalizer_summary = None;

        assert!(!super::replace_delivery_with_loop_contract_observed_answer(
            &task,
            &mut loop_state,
            &mut finalizer_summary,
        ));
        assert_eq!(loop_state.last_user_visible_respond, None);
    }

    #[test]
    fn deterministic_observed_execution_status_answer_uses_structured_run_cmd_stderr() {
        let state = test_state();
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        let err = format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "run_cmd",
                "error_kind": "nonzero_exit",
                "error_text": "Command failed with exit code 7",
                "platform": "linux",
                "extra": {
                    "exit_code": 7,
                    "stderr": "problem",
                    "output_truncated": false
                }
            })
        );
        loop_state
            .executed_step_results
            .push(ok_step_result("step_1", "run_cmd", "READY\n"));
        loop_state
            .executed_step_results
            .push(err_step_result("step_2", "run_cmd", &err));

        let answer = deterministic_observed_execution_status_answer(
            &state,
            "执行两个命令，告诉我退出码和错误输出。",
            &loop_state,
        )
        .expect("mixed observed results should produce deterministic answer");

        assert!(answer.contains("exit code 7"), "answer: {answer}");
        assert!(answer.contains("stderr: problem"), "answer: {answer}");
    }

    #[test]
    fn deterministic_observed_execution_status_answer_attaches_before_llm_fallback() {
        let state = test_state();
        let task = claimed_task("task-deterministic-observed-status");
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "health_check",
            r#"{"ok":true}"#,
        ));
        loop_state.executed_step_results.push(err_step_result(
            "step_2",
            "run_cmd",
            "Command failed with exit code 127\nstderr:\nmissing command",
        ));
        let mut finalizer_summary = None;

        assert!(attach_deterministic_observed_execution_status_answer(
            &state,
            &task,
            "先检查健康，再执行缺失命令，然后总结哪一步成功了、哪一步失败了。",
            &mut loop_state,
            &mut finalizer_summary,
        ));

        assert_eq!(loop_state.delivery_messages.len(), 1);
        assert!(loop_state.delivery_messages[0].contains("第 1 步 `health_check` 成功"));
        assert!(loop_state.delivery_messages[0].contains("第 2 步 `run_cmd` 失败"));
        let summary = finalizer_summary.expect("summary");
        assert_eq!(summary.completion_ok, Some(true));
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn deterministic_observed_execution_status_answer_replaces_bad_synthesis() {
        let state = test_state();
        let task = claimed_task("task-deterministic-observed-status-replace");
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state
            .delivery_messages
            .push("步骤2未观察到执行结果，因此无法确认成功或失败。".to_string());
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "health_check",
            r#"{"ok":true}"#,
        ));
        loop_state.executed_step_results.push(err_step_result(
            "step_2",
            "run_cmd",
            "Command failed with exit code 127\nstderr:\nmissing command",
        ));
        let mut finalizer_summary = None;

        assert!(
            replace_delivery_with_deterministic_observed_execution_status_answer(
                &state,
                &task,
                "先检查健康，再执行缺失命令，然后总结哪一步成功了、哪一步失败了。",
                &mut loop_state,
                &mut finalizer_summary,
            )
        );

        assert_eq!(loop_state.delivery_messages.len(), 1);
        assert!(loop_state.delivery_messages[0].contains("第 2 步 `run_cmd` 失败"));
        assert!(!loop_state.delivery_messages[0].contains("无法确认成功或失败"));
        assert_eq!(
            finalizer_summary.and_then(|summary| summary.completion_ok),
            Some(true)
        );
    }

    #[test]
    fn deterministic_observed_execution_status_keeps_recovered_content_answer() {
        let state = test_state();
        let task = claimed_task("task-deterministic-observed-status-recovered");
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        let answer =
            "目标文件不存在；候选路径：plan/llm_first_agent_convergence_plan_20260511_已完成.md"
                .to_string();
        loop_state.delivery_messages.push(answer.clone());
        loop_state.last_user_visible_respond = Some(answer.clone());
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "fs_basic",
            r#"{"exists":false}"#,
        ));
        loop_state.executed_step_results.push(err_step_result(
            "step_2",
            "read_file",
            "file not found: /home/guagua/rustclaw/plan/missing.md",
        ));
        loop_state.executed_step_results.push(ok_step_result(
            "step_3",
            "fs_basic",
            r#"{"results":["plan/llm_first_agent_convergence_plan_20260511_已完成.md"]}"#,
        ));
        let mut finalizer_summary = None;

        assert!(
            !replace_delivery_with_deterministic_observed_execution_status_answer(
                &state,
                &task,
                "读取缺失文件；如果不存在就返回候选路径",
                &mut loop_state,
                &mut finalizer_summary,
            )
        );
        assert_eq!(loop_state.delivery_messages, vec![answer]);
        assert!(finalizer_summary.is_none());
    }

    #[test]
    fn deterministic_observed_execution_status_keeps_planned_failed_step_answer() {
        let state = test_state();
        let task = claimed_task("task-deterministic-observed-status-keep-planned");
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "run two commands and report failed step".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![
                    crate::PlanStep {
                        step_id: "step_1".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "run_cmd".to_string(),
                        args: serde_json::json!({"command": "echo BEFORE_BREAK"}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "step_2".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "run_cmd".to_string(),
                        args: serde_json::json!({
                            "command": "definitely_missing_command_rustclaw_user_ops_13579"
                        }),
                        depends_on: vec!["step_1".to_string()],
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "step_3".to_string(),
                        action_type: "respond".to_string(),
                        skill: "respond".to_string(),
                        args: serde_json::json!({
                            "content": "第二步挂了，`definitely_missing_command_rustclaw_user_ops_13579` 命令不存在。"
                        }),
                        depends_on: vec!["step_2".to_string()],
                        why: String::new(),
                    },
                ])),
                verify_result: None,
            });
        let planned =
            "第二步挂了，`definitely_missing_command_rustclaw_user_ops_13579` 命令不存在。"
                .to_string();
        loop_state.delivery_messages.push(planned.clone());
        loop_state.last_user_visible_respond = Some(planned.clone());
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "run_cmd",
            "BEFORE_BREAK\n",
        ));
        loop_state.executed_step_results.push(err_step_result(
            "step_2",
            "run_cmd",
            "Command failed with exit code 127\nstderr:\nmissing command",
        ));
        let mut finalizer_summary = None;

        assert!(!replace_delivery_with_deterministic_observed_execution_status_answer(
            &state,
            &task,
            "先执行 echo BEFORE_BREAK，再执行 definitely_missing_command_rustclaw_user_ops_13579，只告诉我哪一步挂了",
            &mut loop_state,
            &mut finalizer_summary,
        ));

        assert_eq!(loop_state.delivery_messages, vec![planned.clone()]);
        assert_eq!(
            loop_state.last_user_visible_respond.as_deref(),
            Some(planned.as_str())
        );
        assert_eq!(
            finalizer_summary.and_then(|summary| summary.completion_ok),
            Some(true)
        );
    }

    #[test]
    fn deterministic_execution_failed_step_contract_replaces_verbose_status() {
        let state = test_state();
        let task = claimed_task("task-deterministic-failed-step-only");
        let mut route = free_route_result();
        route.output_contract.response_shape = OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ExecutionFailedStep;
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "run two commands and identify only failed step".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![
                    crate::PlanStep {
                        step_id: "step_1".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "run_cmd".to_string(),
                        args: serde_json::json!({"command": "echo BEFORE_BREAK"}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "step_2".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "run_cmd".to_string(),
                        args: serde_json::json!({
                            "command": "definitely_missing_command_rustclaw_user_ops_13579"
                        }),
                        depends_on: vec!["step_1".to_string()],
                        why: String::new(),
                    },
                ])),
                verify_result: None,
            });
        loop_state.delivery_messages.push(
            "第 1 步 `run_cmd` 成功。第 2 步 `run_cmd` 失败：Command failed with exit code 127。"
                .to_string(),
        );
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "run_cmd",
            "BEFORE_BREAK\n",
        ));
        loop_state.executed_step_results.push(err_step_result(
            "step_2",
            "run_cmd",
            "Command failed with exit code 127\nstderr:\nmissing command",
        ));
        let mut finalizer_summary = None;

        assert!(replace_delivery_with_deterministic_execution_failed_step_answer(
            &state,
            &task,
            "先执行 echo BEFORE_BREAK，再执行 definitely_missing_command_rustclaw_user_ops_13579，只告诉我哪一步挂了",
            &mut loop_state,
            Some(&ctx),
            &mut finalizer_summary,
        ));

        assert_eq!(loop_state.delivery_messages.len(), 1);
        let answer = &loop_state.delivery_messages[0];
        assert!(answer.contains("第 2 步失败"));
        assert!(answer.contains("definitely_missing_command_rustclaw_user_ops_13579"));
        assert!(!answer.contains("第 1 步"));
        assert!(!answer.contains("exit code 127"));
        assert_eq!(
            finalizer_summary.and_then(|summary| summary.completion_ok),
            Some(true)
        );
    }

    #[test]
    fn deterministic_observed_execution_status_replaces_raw_success_output() {
        let state = test_state();
        let task = claimed_task("task-deterministic-observed-status-replace-raw");
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state
            .delivery_messages
            .push("THINK_BREAK_CN".to_string());
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "run_cmd",
            "THINK_BREAK_CN\n",
        ));
        loop_state.executed_step_results.push(err_step_result(
            "step_2",
            "run_cmd",
            "Command failed with exit code 127\nstderr:\nbash: definitely_missing_command: command not found",
        ));
        let mut finalizer_summary = None;

        assert!(
            replace_delivery_with_deterministic_observed_execution_status_answer(
                &state,
                &task,
                "先执行第一个命令，再执行第二个命令，然后总结成功和失败分别是什么。",
                &mut loop_state,
                &mut finalizer_summary,
            )
        );

        assert_eq!(loop_state.delivery_messages.len(), 1);
        assert!(loop_state.delivery_messages[0].contains("第 1 步 `run_cmd` 成功"));
        assert!(loop_state.delivery_messages[0].contains("第 2 步 `run_cmd` 失败"));
        assert!(!loop_state.delivery_messages[0].trim().eq("THINK_BREAK_CN"));
        assert_eq!(
            finalizer_summary.and_then(|summary| summary.completion_ok),
            Some(true)
        );
    }

    #[test]
    fn exact_observed_answer_does_not_replace_mixed_failure_summary() {
        let state = test_state();
        let mut route = free_route_result();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
        route.output_contract.requires_content_evidence = true;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state
            .executed_step_results
            .push(ok_step_result("step_1", "run_cmd", "BREAK_A\n"));
        loop_state.executed_step_results.push(err_step_result(
            "step_2",
            "run_cmd",
            "Command failed with exit code 127\nstderr:\nmissing command",
        ));
        let summary =
            "第 1 步 `run_cmd` 成功。第 2 步 `run_cmd` 失败：Command failed with exit code 127。"
                .to_string();
        let mut delivery_messages = vec![summary.clone()];
        let mut finalizer_summary = Some(super::deterministic_observed_execution_status_summary(
            &loop_state,
        ));

        prefer_observed_answer_for_exact_contract(
            &state,
            "task-exact-observed-mixed-failure",
            &mut loop_state,
            Some(&agent_run_context),
            &mut delivery_messages,
            &mut finalizer_summary,
        );

        assert_eq!(delivery_messages, vec![summary]);
        assert_ne!(
            loop_state.last_user_visible_respond.as_deref(),
            Some("BREAK_A")
        );
    }

    #[test]
    fn raw_command_chatact_keeps_planned_delivery_with_extra_content() {
        let state = test_state();
        let mut route = free_route_result();
        route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
        route.output_contract.requires_content_evidence = true;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "run_cmd",
            "/workspace/project\n",
        ));
        let planned = "/workspace/project\n\nworkspace ready".to_string();
        loop_state.last_user_visible_respond = Some(planned.clone());
        let mut delivery_messages = vec![planned.clone()];
        let mut finalizer_summary = None;

        prefer_observed_answer_for_exact_contract(
            &state,
            "task-raw-command-chatact-planned",
            &mut loop_state,
            Some(&agent_run_context),
            &mut delivery_messages,
            &mut finalizer_summary,
        );

        assert_eq!(delivery_messages, vec![planned]);
        assert!(finalizer_summary.is_none());
    }

    #[tokio::test]
    async fn finalize_loop_reply_returns_graceful_result_for_permission_denied_content_evidence() {
        let state = test_state();
        let task = claimed_task("task-content-error-finalize");
        let mut route = free_route_result();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_hint = "/etc/shadow".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.last_user_visible_respond =
            Some("我还没能根据现有证据生成可靠最终答案。".to_string());
        loop_state
            .delivery_messages
            .push("我还没能根据现有证据生成可靠最终答案。".to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some(format!(
                "__RC_SKILL_ERROR__:{}",
                serde_json::json!({
                    "skill": "system_basic",
                    "error_kind": "permission_denied",
                    "error_text": "read_range failed for /etc/shadow",
                    "platform": "linux",
                    "extra": {
                        "operation": "metadata",
                        "path": "/etc/shadow"
                    }
                })
            )),
            started_at: 0,
            finished_at: 0,
        });

        let reply = finalize_loop_reply(
            &state,
            &task,
            "读 /etc/shadow 第一行",
            loop_state,
            Some(&agent_run_context),
        )
        .await
        .expect("finalize should return a user-visible failure");

        assert!(reply.text.contains("`/etc/shadow`"));
        assert!(reply.text.contains("permission denied"));
        assert!(reply.text.contains("`clawd` 进程当前没有 sudo/root 权限"));
        assert!(!reply.should_fail_task);
        assert_eq!(reply.messages.len(), 2);
        assert!(crate::finalize::is_execution_summary_message(
            &reply.messages[0]
        ));
        assert_eq!(reply.messages.last(), Some(&reply.text));
    }

    #[tokio::test]
    async fn finalize_loop_reply_does_not_infer_service_status_from_raw_systemd_text() {
        let state = test_state();
        let task = claimed_task("task-service-status-raw-systemd-text");
        let mut route = free_route_result();
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
        route.output_contract.locator_hint.clear();
        route.output_contract.locator_hint = "telegramd.service".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some(
                "Command failed with exit code 4\nstderr:\nUnit telegramd.service could not be found."
                    .to_string(),
            ),
            started_at: 0,
            finished_at: 0,
        });

        let reply = finalize_loop_reply(
            &state,
            &task,
            "check whether telegramd is running right now and briefly explain the status",
            loop_state,
            Some(&agent_run_context),
        )
        .await
        .expect("finalize should return a user-visible command result");

        assert!(
            reply.should_fail_task,
            "raw systemd prose should not be promoted to a qualified service-status answer"
        );
        assert!(
            !reply.text.contains("no service unit"),
            "raw text should not trigger local service-status phrase inference: {}",
            reply.text
        );
    }

    #[tokio::test]
    async fn finalize_loop_reply_uses_structured_service_error_kind() {
        let state = test_state();
        let task = claimed_task("task-service-status-structured-missing");
        let mut route = free_route_result();
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
        route.output_contract.locator_hint.clear();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let structured_error = serde_json::json!({
            "skill": "service_control",
            "error_kind": "not_found",
            "error_text": "no matching service found for the given target",
            "platform": "linux",
            "manager_type": "unknown",
            "service_name": "telegramd"
        });
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "service_control".to_string(),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some(format!("__RC_SKILL_ERROR__:{structured_error}")),
            started_at: 0,
            finished_at: 0,
        });

        let reply = finalize_loop_reply(
            &state,
            &task,
            "check whether telegramd is running right now and briefly explain the status",
            loop_state,
            Some(&agent_run_context),
        )
        .await
        .expect("finalize should return a service status answer");

        assert!(!reply.should_fail_task);
        assert!(reply.text.contains("telegramd"));
        assert!(reply.text.contains("not active"));
        assert!(reply.text.contains("no service unit"));
        assert!(!reply.text.contains("__RC_SKILL_ERROR__"));
    }

    #[tokio::test]
    async fn finalize_loop_reply_treats_structured_run_cmd_failure_as_user_result() {
        let state = test_state();
        let task = claimed_task("task-structured-run-cmd-nonzero");
        let mut route = free_route_result();
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let structured_error = serde_json::json!({
            "skill": "run_cmd",
            "error_kind": "nonzero_exit",
            "error_text": "Command failed with exit code 7",
            "platform": "linux",
            "extra": {
                "command": "printf problem >&2; exit 7",
                "exit_code": 7,
                "stderr": "problem",
                "output_truncated": false
            }
        });
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(err_step_result(
            "step_1",
            "run_cmd",
            &format!("__RC_SKILL_ERROR__:{structured_error}"),
        ));

        let reply = finalize_loop_reply(
            &state,
            &task,
            "执行命令 printf problem >&2; exit 7，告诉我退出码和错误输出。",
            loop_state,
            Some(&agent_run_context),
        )
        .await
        .expect("finalize should return a user-visible command failure");

        assert!(!reply.should_fail_task);
        assert!(reply.text.contains("退出码为 7"), "text: {}", reply.text);
        assert!(
            reply.text.contains("错误输出为：problem"),
            "text: {}",
            reply.text
        );
        assert!(!reply.text.contains("__RC_SKILL_ERROR__"));
        assert_eq!(reply.messages.len(), 2);
        assert!(crate::finalize::is_execution_summary_message(
            &reply.messages[0]
        ));
        assert_eq!(reply.messages.last(), Some(&reply.text));
    }

    #[tokio::test]
    async fn finalize_loop_reply_treats_missing_read_target_as_user_result() {
        let state = test_state();
        let task = claimed_task("task-missing-read-target");
        let mut route = free_route_result();
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
        route.output_contract.locator_hint = "document/missing.txt".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some(format!(
                "__RC_SKILL_ERROR__:{}",
                serde_json::json!({
                    "skill": "system_basic",
                    "error_kind": "not_found",
                    "error_text": "path was not found: document/missing.txt",
                    "platform": "linux",
                    "extra": {
                        "operation": "metadata",
                        "path": "document/missing.txt"
                    }
                })
            )),
            started_at: 0,
            finished_at: 0,
        });

        let reply = finalize_loop_reply(
            &state,
            &task,
            "读一下 document/missing.txt 开头，然后用一句话总结",
            loop_state,
            Some(&agent_run_context),
        )
        .await
        .expect("finalize should return a missing-target answer");

        assert!(!reply.should_fail_task);
        assert!(
            reply.text.contains("不存在")
                || reply.text.contains("未找到")
                || reply.text.to_ascii_lowercase().contains("not found")
                || reply.text.to_ascii_lowercase().contains("does not exist")
                || reply.text.to_ascii_lowercase().contains("no such file")
        );
        assert_eq!(reply.messages.len(), 2);
        assert!(crate::finalize::is_execution_summary_message(
            &reply.messages[0]
        ));
        assert_eq!(reply.messages.last(), Some(&reply.text));
        assert_eq!(
            reply
                .task_journal
                .as_ref()
                .and_then(|journal| journal.final_status),
            Some(crate::task_journal::TaskJournalFinalStatus::Success)
        );
    }

    #[tokio::test]
    async fn missing_read_target_reply_prefers_original_user_language() {
        let state = test_state();
        let mut task = claimed_task("task-missing-read-target-language");
        task.payload_json = serde_json::json!({
            "text": "读取 ./NO_SUCH_RUSTCLAW_TEST_987654.txt 的第一行"
        })
        .to_string();
        let mut route = free_route_result();
        route.output_contract.response_shape = OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_hint = "./NO_SUCH_RUSTCLAW_TEST_987654.txt".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some(format!(
                "__RC_SKILL_ERROR__:{}",
                serde_json::json!({
                    "skill": "system_basic",
                    "error_kind": "not_found",
                    "error_text": "path was not found: ./NO_SUCH_RUSTCLAW_TEST_987654.txt",
                    "platform": "linux",
                    "extra": {
                        "operation": "metadata",
                        "path": "./NO_SUCH_RUSTCLAW_TEST_987654.txt"
                    }
                })
            )),
            started_at: 0,
            finished_at: 0,
        });

        let reply = finalize_loop_reply(
            &state,
            &task,
            "Read the first line of the file ./NO_SUCH_RUSTCLAW_TEST_987654.txt.",
            loop_state,
            Some(&agent_run_context),
        )
        .await
        .expect("finalize should return a missing-target answer");

        assert!(reply.text.contains("未找到"), "text: {}", reply.text);
        assert!(
            !reply.text.contains("I couldn't find"),
            "text: {}",
            reply.text
        );
    }

    #[tokio::test]
    async fn missing_read_target_scalar_contract_keeps_failure_answer_not_path_only() {
        let state = test_state();
        let mut task = claimed_task("task-missing-read-target-scalar");
        task.payload_json = serde_json::json!({
            "text": "读取 ./NO_SUCH_RUSTCLAW_TEST_987654.txt 的第一行"
        })
        .to_string();
        let mut route = scalar_route_result();
        route.resolved_intent =
            "用户请求读取文件 ./NO_SUCH_RUSTCLAW_TEST_987654.txt 的第一行内容。".to_string();
        route.output_contract.response_shape = OutputResponseShape::Scalar;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "./NO_SUCH_RUSTCLAW_TEST_987654.txt".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some(format!(
                "__RC_SKILL_ERROR__:{}",
                serde_json::json!({
                    "skill": "system_basic",
                    "error_kind": "not_found",
                    "error_text": "path was not found: ./NO_SUCH_RUSTCLAW_TEST_987654.txt",
                    "platform": "linux",
                    "extra": {
                        "operation": "metadata",
                        "path": "./NO_SUCH_RUSTCLAW_TEST_987654.txt"
                    }
                })
            )),
            started_at: 0,
            finished_at: 0,
        });

        let reply = finalize_loop_reply(
            &state,
            &task,
            "Read the first line of the file ./NO_SUCH_RUSTCLAW_TEST_987654.txt.",
            loop_state,
            Some(&agent_run_context),
        )
        .await
        .expect("finalize should return a missing-target answer");

        assert!(reply.text.contains("未找到"), "text: {}", reply.text);
        assert!(
            reply.text != "./NO_SUCH_RUSTCLAW_TEST_987654.txt",
            "missing target answer must not be reshaped into path-only scalar"
        );
        assert_eq!(reply.messages.len(), 2);
        assert!(crate::finalize::is_execution_summary_message(
            &reply.messages[0]
        ));
        assert_eq!(reply.messages.last(), Some(&reply.text));
    }

    #[tokio::test]
    async fn finalize_loop_reply_treats_read_file_not_found_marker_as_user_result() {
        let state = test_state();
        let task = claimed_task("task-missing-read-target-marker");
        let mut route = free_route_result();
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
        route.output_contract.locator_hint = "/tmp/missing.txt".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some("__RC_READ_FILE_NOT_FOUND__:/tmp/missing.txt".to_string()),
            started_at: 0,
            finished_at: 0,
        });

        let reply = finalize_loop_reply(
            &state,
            &task,
            "读取 /tmp/missing.txt",
            loop_state,
            Some(&agent_run_context),
        )
        .await
        .expect("finalize should return a missing-target answer");

        assert!(!reply.should_fail_task);
        assert!(
            reply.text.contains("不存在")
                || reply.text.contains("未找到")
                || reply.text.to_ascii_lowercase().contains("not found")
                || reply.text.to_ascii_lowercase().contains("does not exist")
        );
        assert_eq!(reply.messages.len(), 2);
        assert!(crate::finalize::is_execution_summary_message(
            &reply.messages[0]
        ));
        assert_eq!(reply.messages.last(), Some(&reply.text));
        assert_eq!(
            reply
                .task_journal
                .as_ref()
                .and_then(|journal| journal.final_status),
            Some(crate::task_journal::TaskJournalFinalStatus::Success)
        );
    }

    #[test]
    fn execution_recipe_closeout_note_mentions_external_workspace_for_english_code_change() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: true,
            saw_external_target: true,
            ..Default::default()
        };

        let note = execution_recipe_closeout_note(
            None,
            "Fix the issue in /tmp/demo and verify it.",
            &loop_state,
        )
        .expect("closeout note");
        assert!(note.contains("external workspace"));
        assert!(note.contains("code changes"));
    }

    #[test]
    fn execution_recipe_closeout_prefixes_greenfield_plain_text_delivery() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: true,
            saw_greenfield_creation: true,
            ..Default::default()
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };
        let mut delivery = vec!["Validation passed.".to_string()];

        attach_execution_recipe_closeout_to_delivery(
            None,
            "Create a new script and verify it works.",
            &loop_state,
            Some(&ctx),
            &mut delivery,
        );

        assert_eq!(delivery.len(), 1);
        assert!(delivery[0].starts_with("Created the new artifact"));
        assert!(delivery[0].ends_with("Validation passed."));
    }

    #[test]
    fn execution_recipe_closeout_does_not_infer_success_marker_from_user_text() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::OpsService,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::System,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_validation: true,
            ..Default::default()
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            user_request: Some(
                "When it passes, explicitly output VALIDATION_PASSED and stop immediately."
                    .to_string(),
            ),
            ..Default::default()
        };
        let mut delivery = vec!["修复已经完成。".to_string()];

        attach_execution_recipe_closeout_to_delivery(
            None,
            "修复系统服务并在通过时明确输出 VALIDATION_PASSED。",
            &loop_state,
            Some(&ctx),
            &mut delivery,
        );

        assert_eq!(delivery.len(), 1);
        assert!(delivery[0].contains("系统范围"));
        assert!(!delivery[0].contains("VALIDATION_PASSED"));
        assert!(delivery[0].ends_with("修复已经完成。"));
    }

    #[test]
    fn execution_recipe_closeout_prefixes_current_repo_plain_text_delivery() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: true,
            ..Default::default()
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };
        let mut delivery = vec!["修复已经验证通过。".to_string()];

        attach_execution_recipe_closeout_to_delivery(
            None,
            "把当前仓库里的问题修好并验证。",
            &loop_state,
            Some(&ctx),
            &mut delivery,
        );

        assert_eq!(delivery.len(), 1);
        assert!(delivery[0].starts_with("已在当前仓库完成代码修改"));
        assert!(delivery[0].ends_with("修复已经验证通过。"));
    }

    #[test]
    fn execution_recipe_closeout_note_mentions_system_scope_for_english_ops() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::OpsService,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::System,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: true,
            ..Default::default()
        };

        let note = execution_recipe_closeout_note(
            None,
            "Repair the system service and validate it.",
            &loop_state,
        )
        .expect("closeout note");
        assert!(note.contains("system scope"));
        assert!(note.contains("ops work"));
    }

    #[test]
    fn execution_recipe_closeout_note_skips_apply_phase_without_validation() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::OpsService,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::System,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            ..Default::default()
        };

        assert!(execution_recipe_closeout_note(
            None,
            "Repair the system service and validate it.",
            &loop_state,
        )
        .is_none());
    }

    #[test]
    fn execution_recipe_closeout_skips_file_token_delivery() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::ConfigChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            validation_required: true,
            saw_validation: true,
            saw_external_target: true,
            ..Default::default()
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };
        let mut delivery = vec!["FILE:/tmp/report.txt".to_string()];

        attach_execution_recipe_closeout_to_delivery(
            None,
            "Update the config in another workspace and verify it.",
            &loop_state,
            Some(&ctx),
            &mut delivery,
        );

        assert_eq!(delivery, vec!["FILE:/tmp/report.txt".to_string()]);
    }

    #[test]
    fn execution_recipe_closeout_skips_scalar_route_delivery() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            validation_required: true,
            saw_validation: true,
            saw_external_target: true,
            ..Default::default()
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(scalar_route_result()),
            ..Default::default()
        };
        let mut delivery = vec!["42".to_string()];

        attach_execution_recipe_closeout_to_delivery(
            None,
            "Fix the value in /tmp/demo and just answer with the number.",
            &loop_state,
            Some(&ctx),
            &mut delivery,
        );

        assert_eq!(delivery, vec!["42".to_string()]);
    }

    #[test]
    fn execution_recipe_closeout_skips_scalar_route_when_marker_is_only_user_text() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::OpsService,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: true,
            ..Default::default()
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(scalar_route_result()),
            user_request: Some(
                "When it passes, explicitly output VALIDATION_PASSED and stop immediately."
                    .to_string(),
            ),
            ..Default::default()
        };
        let mut delivery = vec!["VALIDATION_PASSED".to_string()];

        attach_execution_recipe_closeout_to_delivery(
            None,
            "修复当前仓库问题，通过时明确输出 VALIDATION_PASSED。",
            &loop_state,
            Some(&ctx),
            &mut delivery,
        );

        assert_eq!(delivery, vec!["VALIDATION_PASSED".to_string()]);
    }

    #[test]
    fn ensure_requested_success_marker_visible_does_not_scan_user_text() {
        let ctx = crate::agent_engine::AgentRunContext {
            user_request: Some(
                "When it passes, explicitly output VALIDATION_PASSED and stop immediately."
                    .to_string(),
            ),
            ..Default::default()
        };
        let mut delivery =
            vec!["Completed ops work at the system scope and validated it.".to_string()];

        ensure_requested_success_marker_visible(Some(&ctx), &mut delivery);

        assert_eq!(delivery.len(), 1);
        assert!(delivery[0].contains("system scope"));
        assert!(!delivery[0].contains("VALIDATION_PASSED"));
    }

    #[test]
    fn missing_requested_success_marker_does_not_scan_user_text() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: true,
            ..Default::default()
        };
        let ctx = crate::agent_engine::AgentRunContext {
            user_request: Some(
                "When it passes, explicitly output VALIDATION_PASSED and stop immediately."
                    .to_string(),
            ),
            ..Default::default()
        };
        let delivery_messages = vec!["ops-repair-bad".to_string()];
        assert_eq!(
            missing_requested_success_marker(Some(&ctx), &loop_state, &delivery_messages),
            None
        );
    }

    #[test]
    fn requested_success_marker_allows_recipe_success_when_present() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: true,
            ..Default::default()
        };
        let ctx = crate::agent_engine::AgentRunContext {
            user_request: Some(
                "When it passes, explicitly output VALIDATION_PASSED and stop immediately."
                    .to_string(),
            ),
            ..Default::default()
        };
        let delivery_messages = vec!["VALIDATION_PASSED".to_string()];
        assert_eq!(
            missing_requested_success_marker(Some(&ctx), &loop_state, &delivery_messages),
            None
        );
    }

    #[test]
    fn auto_requested_success_marker_stays_off_without_structured_request() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: true,
            ..Default::default()
        };
        let ctx = crate::agent_engine::AgentRunContext {
            user_request: Some(
                "When it passes, explicitly output VALIDATION_PASSED and stop immediately."
                    .to_string(),
            ),
            ..Default::default()
        };
        let delivery_messages = vec!["status=200\nops-repair-ok".to_string()];
        assert_eq!(
            auto_requested_success_marker(Some(&ctx), &loop_state, &delivery_messages),
            None
        );
    }

    #[test]
    fn auto_requested_success_marker_stays_off_before_recipe_done() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: false,
            ..Default::default()
        };
        let ctx = crate::agent_engine::AgentRunContext {
            user_request: Some(
                "When it passes, explicitly output VALIDATION_PASSED and stop immediately."
                    .to_string(),
            ),
            ..Default::default()
        };
        let delivery_messages = vec!["status=200\nops-repair-ok".to_string()];
        assert_eq!(
            auto_requested_success_marker(Some(&ctx), &loop_state, &delivery_messages),
            None
        );
    }

    #[test]
    fn direct_scalar_finalize_uses_structured_extract_field_missing_message() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"action":"extract_field","exists":false,"field_path":"name","value_text":"","value":null,"value_type":"null"}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(scalar_route_result()),
            ..Default::default()
        };
        let (answer, summary) =
            direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
                .expect("scalar fallback should succeed");
        assert_eq!(answer, "未找到 name 字段");
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn direct_structured_observed_answer_skips_multi_evidence_content_routes() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"react-example","value":"react-example","value_type":"string"}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_2".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"action":"extract_field","exists":true,"field_path":"package.name","value_text":"clawd","value":"clawd","value_type":"string"}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = free_route_result();
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        assert!(
            direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context))
                .is_none()
        );
    }

    #[test]
    fn tail_read_range_observed_answer_replaces_failed_synthesis_for_content_excerpt() {
        let state = test_state();
        let task = claimed_task("task-tail");
        let mut route = free_route_result();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.response_shape = OutputResponseShape::Strict;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "logs/clawd_manual.log".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state
            .delivery_messages
            .push("**执行过程**\n1. 调用技能 `system_basic`（action=read_range）".to_string());
        loop_state
            .delivery_messages
            .push("由于日志输出被截断，无法查看最后2行内容。".to_string());
        loop_state.last_user_visible_respond =
            Some("由于日志输出被截断，无法查看最后2行内容。".to_string());
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","mode":"head","requested_n":40,"excerpt":"1|startup\n2|ready"}"#,
        ));
        loop_state.executed_step_results.push(ok_step_result(
            "step_2",
            "synthesize_answer",
            "由于日志输出被截断，无法查看最后2行内容。",
        ));
        loop_state.executed_step_results.push(ok_step_result(
            "step_3",
            "system_basic",
            r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"4318|last alpha\n4319|last beta"}"#,
        ));
        let mut finalizer_summary = None;

        assert!(replace_delivery_with_latest_tail_read_range_answer(
            &state,
            &task,
            "看最后一个最后 2 行",
            &mut loop_state,
            Some(&agent_run_context),
            &mut finalizer_summary,
        ));

        assert_eq!(
            loop_state.last_user_visible_respond.as_deref(),
            Some("last alpha\nlast beta")
        );
        assert!(loop_state
            .delivery_messages
            .iter()
            .any(|message| crate::finalize::is_execution_summary_message(message)));
        assert_eq!(
            loop_state.delivery_messages.last().map(String::as_str),
            Some("last alpha\nlast beta")
        );
        assert_eq!(
            finalizer_summary.and_then(|summary| summary.disposition),
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn tail_read_range_observed_answer_allows_malformed_none_semantic_fs_basic() {
        let state = test_state();
        let task = claimed_task("task-tail-none");
        let mut route = free_route_result();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.response_shape = OutputResponseShape::Strict;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "logs/model_io.log".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .delivery_messages
            .push("已有执行结果，但我没能整理成可靠结论。".to_string());
        loop_state.last_user_visible_respond =
            Some("已有执行结果，但我没能整理成可靠结论。".to_string());
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "fs_basic",
            r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"1548|{\"task_id\":\"task-1\",\"omitted_fields\":[\"prompt\"]}\n1549|{\"task_id\":\"task-2\",\"omitted_fields\":[\"prompt\"]}"}"#,
        ));
        let mut finalizer_summary = None;

        assert!(replace_delivery_with_latest_tail_read_range_answer(
            &state,
            &task,
            "看看最后 2 行",
            &mut loop_state,
            Some(&agent_run_context),
            &mut finalizer_summary,
        ));

        let answer = loop_state
            .last_user_visible_respond
            .as_deref()
            .unwrap_or("");
        assert!(answer.contains("task-1"));
        assert!(answer.contains("task-2"));
        assert!(!answer.contains("已有执行结果"));
    }

    #[tokio::test]
    async fn content_evidence_failure_defers_when_latest_tail_read_range_available() {
        let state = test_state();
        let task = claimed_task("task-tail-failure-defers");
        let mut route = free_route_result();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.response_shape = OutputResponseShape::Strict;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "logs/model_io.log".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(err_step_result(
            "step_1",
            "synthesize_answer",
            "synthesis failed",
        ));
        loop_state.executed_step_results.push(ok_step_result(
            "step_2",
            "fs_basic",
            r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"1|last alpha\n2|last beta"}"#,
        ));

        assert!(super::content_evidence_step_failure_reply_from_loop(
            &state,
            &task,
            "看看最后 2 行",
            &loop_state,
            Some(&agent_run_context),
        )
        .await
        .is_none());
    }

    #[test]
    fn tail_read_range_observed_answer_defers_one_sentence_summary() {
        let state = test_state();
        let task = claimed_task("task-tail-summary");
        let mut route = free_route_result();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut loop_state = crate::agent_engine::LoopState::new(1);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"1|a\n2|b"}"#,
        ));
        let mut finalizer_summary = None;

        assert!(!replace_delivery_with_latest_tail_read_range_answer(
            &state,
            &task,
            "一句话总结最后两行",
            &mut loop_state,
            Some(&agent_run_context),
            &mut finalizer_summary,
        ));
    }

    #[test]
    fn direct_structured_observed_answer_skips_ambiguous_multi_structured_scalars() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"react-example","value":"react-example","value_type":"string"}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_2".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"action":"extract_field","exists":true,"field_path":"package.name","value_text":"clawd","value":"clawd","value_type":"string"}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = free_route_result();
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = false;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        assert!(
            direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context))
                .is_none()
        );
    }

    #[test]
    fn direct_structured_observed_answer_defers_semantic_pair_answer_to_llm() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"react-example","value":"react-example","value_type":"string"}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_2".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"action":"extract_field","exists":true,"field_path":"package.name","value_text":"clawd","value":"clawd","value_type":"string"}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = free_route_result();
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        assert!(
            direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context))
                .is_none()
        );
    }

    #[test]
    fn direct_scalar_finalize_uses_hidden_entries_direct_answer() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "list_dir".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(".git\nREADME.md\n.env\nsrc\n".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = scalar_route_result();
        route.resolved_intent =
            "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子".to_string();
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint = ".".to_string();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::HiddenEntriesCheck;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let (answer, summary) =
            direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
                .expect("hidden entries scalar fallback should succeed");
        assert_eq!(answer, "2");
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn direct_scalar_finalize_defers_health_check_summary_to_synthesis() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "health_check".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = scalar_route_result();
        route.resolved_intent =
            "执行基础健康检查，仅提取并返回操作系统相关的关键字段，排除 RustClaw 自身的状态摘要"
                .to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        assert!(
            direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context)).is_none(),
            "health_check scalar summary should be synthesized from observed evidence"
        );
    }

    #[test]
    fn direct_scalar_finalize_reports_missing_path_before_extracting_path_field() {
        let state = test_state();
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"configs/config_copy"}],"include_missing":true}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = scalar_route_result();
        route.resolved_intent = "查一下 configs/config_copy 下面有几个 toml 文件".to_string();
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "configs/config_copy".to_string();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        let (answer, summary) =
            direct_scalar_observed_answer(Some(&state), &loop_state, Some(&agent_run_context))
                .expect("missing path should produce a scalar-compatible failure explanation");

        assert!(answer.contains("configs/config_copy"));
        assert!(answer.contains("不存在"));
        assert!(answer.contains("无法统计"));
        assert_ne!(answer.trim(), "configs/config_copy");
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn direct_scalar_finalize_does_not_repair_limited_listing_from_drifted_scalar_count() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"action":"inventory_dir","path":"logs","resolved_path":"/tmp/logs","names_only":true,"sort_by":"mtime_desc","names":["clawd.run.log","model_io.log","act_plan.log"],"counts":{"total":3}}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = scalar_route_result();
        route.resolved_intent = "列出 logs 目录最近修改的 2 个文件名，只输出文件名".to_string();
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "logs".to_string();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let (answer, summary) =
            direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
                .expect("scalar count fallback should follow the structured contract");
        assert_eq!(answer, "3");
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn file_delivery_fallback_uses_ranked_inventory_after_placeholder_plan() {
        let dir = TempDirGuard::new("ranked_inventory_file_delivery");
        let newest = dir.path().join("newest.txt");
        let older = dir.path().join("older.txt");
        fs::write(&newest, "new").expect("write newest");
        fs::write(&older, "old").expect("write older");

        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "deliver selected file from directory".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![
                    crate::PlanStep {
                        step_id: "step_1".to_string(),
                        action_type: "call_tool".to_string(),
                        skill: "fs_basic".to_string(),
                        args: serde_json::json!({
                            "action": "list_dir",
                            "path": dir.path().display().to_string(),
                            "names_only": true,
                            "sort_by": "mtime_desc"
                        }),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "step_2".to_string(),
                        action_type: "respond".to_string(),
                        skill: "respond".to_string(),
                        args: serde_json::json!({
                            "content": format!("FILE:{}/{{{{last_output}}}}", dir.path().display())
                        }),
                        depends_on: vec!["step_1".to_string()],
                        why: String::new(),
                    },
                ])),
                verify_result: None,
            });
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "fs_basic",
            &serde_json::json!({
                "action": "inventory_dir",
                "resolved_path": dir.path().display().to_string(),
                "names_only": true,
                "sort_by": "mtime_desc",
                "names": ["newest.txt", "older.txt"],
                "counts": {"files": 2, "dirs": 0, "total": 2}
            })
            .to_string(),
        ));
        let mut route = scalar_route_result();
        route.wants_file_delivery = true;
        route.output_contract.delivery_required = true;
        route.output_contract.response_shape = OutputResponseShape::FileToken;
        route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = dir.path().display().to_string();
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        let (token, summary) = direct_file_token_from_observed_inventory(&loop_state, Some(&ctx))
            .expect("ranked inventory should recover file token");

        assert_eq!(token, format!("FILE:{}", newest.display()));
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn file_delivery_fallback_uses_last_inventory_selection_from_placeholder_plan() {
        let dir = TempDirGuard::new("last_inventory_file_delivery");
        let first = dir.path().join("alpha.txt");
        let last = dir.path().join("zeta.txt");
        fs::write(&first, "first").expect("write first");
        fs::write(&last, "last").expect("write last");

        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "deliver selected file from directory".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![
                    crate::PlanStep {
                        step_id: "step_1".to_string(),
                        action_type: "call_tool".to_string(),
                        skill: "fs_basic".to_string(),
                        args: serde_json::json!({
                            "action": "list_dir",
                            "path": dir.path().display().to_string(),
                            "names_only": true,
                            "sort_by": "name"
                        }),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "step_2".to_string(),
                        action_type: "respond".to_string(),
                        skill: "respond".to_string(),
                        args: serde_json::json!({
                            "content": format!(
                                "FILE:{}/{{{{last_output.lines().last().unwrap()}}}}",
                                dir.path().display()
                            )
                        }),
                        depends_on: vec!["step_1".to_string()],
                        why: String::new(),
                    },
                ])),
                verify_result: None,
            });
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "fs_basic",
            &serde_json::json!({
                "action": "inventory_dir",
                "resolved_path": dir.path().display().to_string(),
                "names_only": true,
                "sort_by": "name",
                "names": ["alpha.txt", "zeta.txt"],
                "counts": {"files": 2, "dirs": 0, "total": 2}
            })
            .to_string(),
        ));
        let mut route = scalar_route_result();
        route.wants_file_delivery = true;
        route.output_contract.delivery_required = true;
        route.output_contract.response_shape = OutputResponseShape::FileToken;
        route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = dir.path().display().to_string();
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        let (token, summary) = direct_file_token_from_observed_inventory(&loop_state, Some(&ctx))
            .expect("explicit last selection over deterministic inventory should recover token");

        assert_eq!(token, format!("FILE:{}", last.display()));
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn file_delivery_fallback_defers_ambiguous_unranked_inventory() {
        let dir = TempDirGuard::new("ambiguous_inventory_file_delivery");
        fs::write(dir.path().join("a.txt"), "a").expect("write a");
        fs::write(dir.path().join("b.txt"), "b").expect("write b");

        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "fs_basic",
            &serde_json::json!({
                "action": "inventory_dir",
                "resolved_path": dir.path().display().to_string(),
                "names_only": true,
                "sort_by": "name",
                "names": ["a.txt", "b.txt"],
                "counts": {"files": 2, "dirs": 0, "total": 2}
            })
            .to_string(),
        ));
        let mut route = scalar_route_result();
        route.wants_file_delivery = true;
        route.output_contract.delivery_required = true;
        route.output_contract.response_shape = OutputResponseShape::FileToken;
        route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert!(direct_file_token_from_observed_inventory(&loop_state, Some(&ctx)).is_none());
    }

    #[test]
    fn direct_scalar_finalize_preserves_planned_count_inventory_breakdown() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .round_traces
            .push(crate::task_journal::TaskJournalRoundTrace {
                round_no: 1,
                goal: "count files and directories".to_string(),
                execution_recipe_summary: None,
                plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "system_basic".to_string(),
                    args: serde_json::json!({
                        "action": "count_inventory",
                        "path": ".",
                        "count_files": true,
                        "count_dirs": true
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }])),
                verify_result: None,
            });
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "system_basic",
            r#"{"action":"count_inventory","counts":{"total":66,"files":40,"dirs":26}}"#,
        ));
        let mut route = scalar_route_result();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            original_user_request: Some(
                "帮我检查一下当前目录底下有多少个文件和文件夹。".to_string(),
            ),
            ..Default::default()
        };

        let (answer, summary) =
            direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
                .expect("planned component counts should be preserved");

        assert!(answer.contains("40"));
        assert!(answer.contains("26"));
        assert_ne!(answer.trim(), "66");
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn direct_scalar_finalize_uses_total_count_without_component_plan() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "system_basic",
            r#"{"action":"count_inventory","counts":{"total":66,"files":40,"dirs":26}}"#,
        ));
        let mut route = scalar_route_result();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            original_user_request: Some("当前目录有多少个项目？只回复数字。".to_string()),
            ..Default::default()
        };

        let (answer, summary) =
            direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
                .expect("total count should be usable directly");

        assert_eq!(answer.trim(), "66");
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn direct_scalar_finalize_allows_scalar_count_with_one_sentence_shape() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "fs_basic",
            r#"{"action":"count_inventory","counts":{"total":34,"files":32,"dirs":2},"path":"document","recursive":false}"#,
        ));
        let mut route = scalar_route_result();
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            original_user_request: Some("再数一下 document 目录直接有多少个子项".to_string()),
            ..Default::default()
        };

        let (answer, summary) =
            direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
                .expect("scalar count should not require scalar response shape");

        assert!(answer.contains("34"));
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn direct_structured_finalize_answers_existence_with_path_from_single_observation() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"rustclaw.service","resolved_path":"/tmp/rustclaw-workspace/rustclaw.service","size_bytes":1190},"path":"/tmp/rustclaw-workspace/rustclaw.service"}],"include_missing":true}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = scalar_route_result();
        route.resolved_intent =
            "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径".to_string();
        route.output_contract.response_shape = OutputResponseShape::Free;
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
        route.output_contract.locator_hint = "rustclaw.service".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let (answer, summary) =
            super::direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context))
                .expect("single path_batch_facts observation should answer existence-with-path");
        assert_eq!(answer, "有，路径：/tmp/rustclaw-workspace/rustclaw.service");
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn direct_non_builtin_finalize_preserves_raw_skill_text() {
        let state = test_state();
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .output_vars
            .insert("last_skill_name".to_string(), "crypto".to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "crypto".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                "trade_submit order_id=123 status=FILLED binance BTCUSDT buy qty_filled=0.001 avg_price=100000 quote_spent=100 USDT"
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };

        let (answer, summary) =
            direct_non_builtin_skill_raw_answer(&state, &loop_state, Some(&agent_run_context))
                .expect("non-builtin fallback should preserve raw text");
        assert_eq!(
            answer,
            "trade_submit order_id=123 status=FILLED binance BTCUSDT buy qty_filled=0.001 avg_price=100000 quote_spent=100 USDT"
        );
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn direct_non_builtin_finalize_skips_structured_machine_output() {
        let state = test_state();
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .output_vars
            .insert("last_skill_name".to_string(), "stock".to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "stock".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(r#"{"symbol":"AAPL","price":201.32}"#.to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(free_route_result()),
            ..Default::default()
        };

        assert!(
            direct_non_builtin_skill_raw_answer(&state, &loop_state, Some(&agent_run_context))
                .is_none()
        );
    }

    #[test]
    fn backfill_delivery_prefers_contractual_last_respond_over_synthesis() {
        let task = claimed_task("task-contractual-last-respond");
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.last_user_visible_respond = Some("/home/guagua/rustclaw".to_string());
        loop_state.last_publishable_synthesis_output =
            Some("命令执行已完成，但综合答案时出错。".to_string());
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "run_cmd",
            "/home/guagua/rustclaw\n",
        ));
        let mut route = scalar_route_result();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
        route.output_contract.locator_hint.clear();
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        backfill_delivery_from_last_outputs(&task, &mut loop_state, Some(&ctx));

        assert_eq!(
            loop_state.delivery_messages,
            vec!["/home/guagua/rustclaw".to_string()]
        );
    }

    #[tokio::test]
    async fn finalize_loop_reply_keeps_exact_single_line_observed_respond() {
        let state = test_state();
        let task = claimed_task("task-single-line-observed-respond");
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.last_user_visible_respond = Some("/home/guagua/rustclaw".to_string());
        loop_state.last_publishable_synthesis_output =
            Some("执行成功了，但合成最终答案的环节遇到问题。".to_string());
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "run_cmd",
            "/home/guagua/rustclaw\n",
        ));
        loop_state.executed_step_results.push(err_step_result(
            "step_2",
            "synthesize_answer",
            "synthesis failed",
        ));
        let mut route = free_route_result();
        route.output_contract.requires_content_evidence = true;
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        let reply = finalize_loop_reply(
            &state,
            &task,
            "执行命令 pwd，直接回复执行结果，不要解释",
            loop_state,
            Some(&ctx),
        )
        .await
        .expect("finalize should succeed");

        assert_eq!(reply.text, "/home/guagua/rustclaw");
        assert!(!reply.should_fail_task);
        assert_eq!(
            reply.messages.last().map(String::as_str),
            Some("/home/guagua/rustclaw")
        );
        assert!(reply.messages[0].contains("**执行过程**"));
        assert!(reply.messages[0].contains("run_cmd"));
    }

    #[tokio::test]
    async fn finalize_loop_reply_uses_publishable_synthesis_output() {
        let state = test_state();
        let task = claimed_task("task-synth-finalize");
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some("rustclaw.service".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_2".to_string(),
            skill: "synthesize_answer".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some("有，路径：/tmp/rustclaw.service".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        loop_state.last_publishable_synthesis_output =
            Some("有，路径：/tmp/rustclaw.service".to_string());
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(scalar_route_result()),
            ..Default::default()
        };

        let reply = finalize_loop_reply(
            &state,
            &task,
            "检查 rustclaw.service 是否存在并给出路径",
            loop_state,
            Some(&agent_run_context),
        )
        .await
        .expect("finalize should succeed");

        assert_eq!(reply.text, "有，路径：/tmp/rustclaw.service");
        assert_eq!(reply.messages.len(), 2);
        assert!(reply.messages[0].contains("**执行过程**"));
        assert!(reply.messages[0].contains("run_cmd"));
        assert_eq!(reply.messages[1], "有，路径：/tmp/rustclaw.service");
        assert!(!reply.should_fail_task);
        assert!(!reply.is_llm_reply);
    }

    #[tokio::test]
    async fn finalize_loop_reply_prefers_synthesis_over_raw_last_respond() {
        let state = test_state();
        let task = claimed_task("task-synth-over-raw");
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state
            .output_vars
            .insert("last_skill_name".to_string(), "git_basic".to_string());
        let raw_git = "exit=0\nabc123 fix deployment docs\n";
        loop_state.last_user_visible_respond = Some(raw_git.to_string());
        loop_state.last_publishable_synthesis_output =
            Some("RustClaw 的部署可按项目文档和安装脚本完成。".to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "git_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(raw_git.to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_2".to_string(),
            skill: "synthesize_answer".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some("RustClaw 的部署可按项目文档和安装脚本完成。".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = free_route_result();
        route.output_contract.requires_content_evidence = true;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        let reply = finalize_loop_reply(
            &state,
            &task,
            "帮我写一段 RustClaw 部署说明",
            loop_state,
            Some(&agent_run_context),
        )
        .await
        .expect("finalize should succeed");

        assert_eq!(reply.text, "RustClaw 的部署可按项目文档和安装脚本完成。");
        assert!(reply.messages[0].contains("**执行过程**"));
        assert!(reply.messages[0].contains("git_basic"));
        assert_eq!(
            reply.messages.last().map(String::as_str),
            Some("RustClaw 的部署可按项目文档和安装脚本完成。")
        );
    }

    #[tokio::test]
    async fn finalize_loop_reply_keeps_article_synthesis_after_repair_success() {
        let state = test_state();
        let task = claimed_task("task-synth-after-repair");
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "list_dir".to_string(),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some("file operation failed: target path was not found".to_string()),
            started_at: 0,
            finished_at: 0,
        });
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_2".to_string(),
            skill: "read_file".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some("# RustClaw\n\nRustClaw is a local Rust agent runtime.".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let article = "RustClaw 是一个本地优先的 Rust 智能体运行时，围绕 clawd、技能调度和多渠道入口组织，可用于通过聊天或浏览器完成项目管理与自动化任务。".to_string();
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_3".to_string(),
            skill: "synthesize_answer".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(article.clone()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        loop_state.delivery_messages.push(
            "**执行过程**\n1. 调用技能 `list_dir`\n   错误：\n```text\nfile operation failed: target path was not found\n```"
                .to_string(),
        );
        loop_state.delivery_messages.push(article.clone());
        loop_state.last_user_visible_respond = Some(article.clone());
        loop_state.last_publishable_synthesis_output = Some(article.clone());
        let mut route = free_route_result();
        route.output_contract.requires_content_evidence = true;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        let reply = finalize_loop_reply(
            &state,
            &task,
            "帮我写一篇关于 RustClaw 的长文",
            loop_state,
            Some(&agent_run_context),
        )
        .await
        .expect("finalize should succeed");

        assert_eq!(reply.text, article);
        assert_eq!(
            reply.messages.last().map(String::as_str),
            Some(article.as_str())
        );
        assert!(
            !reply.text.contains("第 1 步"),
            "article synthesis must not be replaced by step status: {}",
            reply.text
        );
    }

    #[tokio::test]
    async fn finalize_loop_reply_replaces_template_placeholder_with_synthesis() {
        let state = test_state();
        let task = claimed_task("task-synth-placeholder");
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state
            .delivery_messages
            .push("{{synthesized}}".to_string());
        loop_state.last_user_visible_respond = Some("{{synthesized}}".to_string());
        loop_state.last_publishable_synthesis_output =
            Some("RustClaw 可以按 README 中的安装脚本路径完成部署。".to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "read_file".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some("# RustClaw\n\nUse install-rustclaw-cmd.sh".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_2".to_string(),
            skill: "synthesize_answer".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some("RustClaw 可以按 README 中的安装脚本路径完成部署。".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = free_route_result();
        route.output_contract.requires_content_evidence = true;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        let reply = finalize_loop_reply(
            &state,
            &task,
            "帮我写一段 RustClaw 部署说明",
            loop_state,
            Some(&agent_run_context),
        )
        .await
        .expect("finalize should succeed");

        assert_eq!(
            reply.text,
            "RustClaw 可以按 README 中的安装脚本路径完成部署。"
        );
        assert_eq!(
            reply.messages.last().map(String::as_str),
            Some("RustClaw 可以按 README 中的安装脚本路径完成部署。")
        );
        assert!(!reply.text.contains("{{"));
    }

    #[test]
    fn strict_scalar_count_keeps_planned_explanatory_answer() {
        let state = test_state();
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step_result("step_1", "run_cmd", "55\n"));
        loop_state.last_user_visible_respond =
            Some("55 个。当前范围内共有这么多普通文件。".to_string());
        let mut delivery_messages = vec!["55 个。当前范围内共有这么多普通文件。".to_string()];
        let mut route = scalar_route_result();
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        route.output_contract.exact_sentence_count = Some(1);
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut finalizer_summary = None;

        prefer_observed_answer_for_exact_contract(
            &state,
            "task-strict-scalar-count",
            &mut loop_state,
            Some(&agent_run_context),
            &mut delivery_messages,
            &mut finalizer_summary,
        );

        assert_eq!(
            delivery_messages,
            vec!["55 个。当前范围内共有这么多普通文件。"]
        );
        assert_eq!(
            loop_state.last_user_visible_respond.as_deref(),
            Some("55 个。当前范围内共有这么多普通文件。")
        );
        assert!(finalizer_summary.is_none());
    }

    #[test]
    fn exact_contract_keeps_publishable_synthesis_over_raw_observed_inventory() {
        let state = test_state();
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"action":"inventory_dir","counts":{"dirs":1,"files":1,"total":2},"ext_filter":["md"],"names":["regression_llm_first","垃圾代码端分析报告.md"],"names_only":true}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_2".to_string(),
            skill: "synthesize_answer".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some("垃圾代码端分析报告.md".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        loop_state.last_user_visible_respond = Some("垃圾代码端分析报告.md".to_string());
        loop_state.last_publishable_synthesis_output = Some("垃圾代码端分析报告.md".to_string());
        let mut delivery_messages = vec!["垃圾代码端分析报告.md".to_string()];
        let mut route = scalar_route_result();
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
        route.output_contract.locator_hint = "document".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut finalizer_summary = None;

        prefer_observed_answer_for_exact_contract(
            &state,
            "task-synth-file-names",
            &mut loop_state,
            Some(&agent_run_context),
            &mut delivery_messages,
            &mut finalizer_summary,
        );

        assert_eq!(delivery_messages, vec!["垃圾代码端分析报告.md"]);
        assert_eq!(
            loop_state.last_user_visible_respond.as_deref(),
            Some("垃圾代码端分析报告.md")
        );
        assert!(finalizer_summary.is_none());
    }

    #[test]
    fn exact_contract_keeps_planned_subset_over_raw_observed_file_paths() {
        let state = test_state();
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "fs_basic",
            r#"{"action":"find_ext","count":4,"ext":"toml","results":["Cargo.toml","configs/config.toml","configs/skills_registry.toml","crates/clawd/Cargo.toml"]}"#,
        ));
        let planned = "Cargo.toml\nconfigs/config.toml\nconfigs/skills_registry.toml".to_string();
        loop_state.last_user_visible_respond = Some(planned.clone());
        let mut delivery_messages = vec![planned.clone()];
        let mut route = free_route_result();
        route.ask_mode = crate::AskMode::planner_execute_plain();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut finalizer_summary = None;

        prefer_observed_answer_for_exact_contract(
            &state,
            "task-planned-subset-file-paths",
            &mut loop_state,
            Some(&agent_run_context),
            &mut delivery_messages,
            &mut finalizer_summary,
        );

        assert_eq!(delivery_messages, vec![planned]);
        assert_eq!(
            loop_state.last_user_visible_respond.as_deref(),
            Some("Cargo.toml\nconfigs/config.toml\nconfigs/skills_registry.toml")
        );
        assert!(finalizer_summary.is_none());
    }

    #[test]
    fn exact_contract_keeps_explicit_json_delivery_over_observed_phrase() {
        let state = test_state();
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"README.md","resolved_path":"/home/guagua/rustclaw/README.md","size_bytes":24929},"path":"/home/guagua/rustclaw/README.md"}],"fields":["exists","size"],"include_missing":true}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        loop_state.last_user_visible_respond =
            Some(r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#.to_string());
        let mut delivery_messages =
            vec![r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#.to_string()];
        let mut route = scalar_route_result();
        route.ask_mode = crate::AskMode::planner_execute_plain();
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
        route.output_contract.locator_hint = "README.md".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let mut finalizer_summary = None;

        prefer_observed_answer_for_exact_contract(
            &state,
            "task-strict-json-delivery",
            &mut loop_state,
            Some(&agent_run_context),
            &mut delivery_messages,
            &mut finalizer_summary,
        );

        assert_eq!(
            delivery_messages,
            vec![r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#]
        );
        assert_eq!(
            loop_state.last_user_visible_respond.as_deref(),
            Some(r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#)
        );
        assert!(finalizer_summary.is_none());
    }

    #[tokio::test]
    async fn direct_publishable_observed_answer_skips_run_cmd_without_explicit_raw_contract() {
        let state = test_state();
        let task = claimed_task("task-no-raw-run-cmd-passthrough");
        let mut loop_state = crate::agent_engine::LoopState::new(1);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some("/home/guagua/rustclaw\n".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = free_route_result();
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert!(direct_publishable_observed_answer(
            &state,
            &task,
            &loop_state,
            Some(&agent_run_context)
        )
        .await
        .is_none());
    }

    #[tokio::test]
    async fn direct_publishable_observed_answer_skips_strict_run_cmd_format_contract() {
        let state = test_state();
        let task = claimed_task("task-strict-run-cmd-format");
        let mut loop_state = crate::agent_engine::LoopState::new(1);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some("/home/guagua/rustclaw\n".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = free_route_result();
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert!(direct_publishable_observed_answer(
            &state,
            &task,
            &loop_state,
            Some(&agent_run_context)
        )
        .await
        .is_none());
    }

    #[test]
    fn direct_scalar_finalize_accepts_strict_single_line_observation() {
        let mut loop_state = crate::agent_engine::LoopState::new(1);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some("ThinkPad-X1\n".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = free_route_result();
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.exact_sentence_count = Some(1);
        route.output_contract.requires_content_evidence = true;
        route.output_contract.delivery_required = false;
        route.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        let (answer, summary) =
            direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
                .expect("direct scalar answer");
        assert_eq!(answer, "ThinkPad-X1");
        assert!(summary.contract_ok);
    }

    #[test]
    fn direct_scalar_finalize_skips_strict_raw_command_output_contract() {
        let mut loop_state = crate::agent_engine::LoopState::new(1);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some("ThinkPad-X1\n".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = free_route_result();
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.exact_sentence_count = Some(1);
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert!(
            direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context)).is_none()
        );
    }

    #[test]
    fn raw_structured_passthrough_is_dropped_for_scalar_contract() {
        let raw = r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"rustclaw","value":"rustclaw","value_type":"string"}"#;
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.last_user_visible_respond = Some(raw.to_string());
        loop_state.delivery_messages.push(raw.to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(raw.to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(scalar_route_result()),
            ..Default::default()
        };
        assert_eq!(
            should_drop_passthrough_delivery_for_content_evidence(
                &loop_state,
                true,
                Some(&agent_run_context),
                raw
            ),
            Some(true)
        );
    }

    #[test]
    fn structured_user_input_delivery_is_not_dropped_as_raw_passthrough() {
        let message = "Please provide the source directory.";
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.pending_user_input_required = true;
        loop_state.last_user_visible_respond = Some(message.to_string());
        loop_state.delivery_messages.push(message.to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "photo_organize".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(message.to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(scalar_route_result()),
            ..Default::default()
        };
        assert_eq!(
            should_drop_passthrough_delivery_for_content_evidence(
                &loop_state,
                true,
                Some(&agent_run_context),
                message
            ),
            None
        );
    }

    #[test]
    fn qualified_scalar_passthrough_is_not_dropped() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.last_user_visible_respond = Some("rustclaw".to_string());
        loop_state.delivery_messages.push("rustclaw".to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some("rustclaw\n".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(scalar_route_result()),
            ..Default::default()
        };
        assert_eq!(
            should_drop_passthrough_delivery_for_content_evidence(
                &loop_state,
                true,
                Some(&agent_run_context),
                "rustclaw"
            ),
            Some(false)
        );
    }

    #[test]
    fn scalar_path_from_write_file_is_not_dropped_as_meta_placeholder() {
        let path = "/home/guagua/rustclaw/document/pwd_line.txt";
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.last_user_visible_respond = Some(path.to_string());
        loop_state.delivery_messages.push(path.to_string());
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "run_cmd",
            "/home/guagua/rustclaw\n",
        ));
        loop_state.executed_step_results.push(ok_step_result(
            "step_2",
            "write_file",
            "written 48 bytes to /home/guagua/rustclaw/document/pwd_line.txt",
        ));
        loop_state
            .output_vars
            .insert("last_file_path".to_string(), path.to_string());
        loop_state
            .written_file_aliases
            .insert("pwd_line.txt".to_string(), path.to_string());
        let mut route = scalar_route_result();
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
        route.output_contract.locator_hint = "pwd_line.txt".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert_eq!(
            should_drop_passthrough_delivery_for_content_evidence(
                &loop_state,
                true,
                Some(&agent_run_context),
                path
            ),
            Some(false)
        );
    }

    #[test]
    fn content_evidence_contractual_terminal_answer_is_kept_before_meta_classifier() {
        let answer = "最先该做的是：验证配置能否正确加载。";
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state.has_tool_or_skill_output = true;
        loop_state.last_user_visible_respond = Some(answer.to_string());
        loop_state.delivery_messages.push(answer.to_string());
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "fs_basic",
            r#"{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.","path":"release_checklist.md"}"#,
        ));
        loop_state
            .executed_step_results
            .push(ok_step_result("step_2", "respond", answer));
        let mut route = free_route_result();
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint = "release_checklist.md".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert!(content_evidence_terminal_respond_is_contractual_answer(
            &loop_state,
            Some(&agent_run_context),
            answer,
        ));
        assert_eq!(
            should_drop_passthrough_delivery_for_content_evidence(
                &loop_state,
                true,
                Some(&agent_run_context),
                answer,
            ),
            Some(false)
        );
    }

    #[test]
    fn content_evidence_one_sentence_terminal_answer_is_kept_without_semantic_kind() {
        let answer = "最先该做的是**验证配置能正确加载**。";
        let mut loop_state = crate::agent_engine::LoopState::new(3);
        loop_state.has_tool_or_skill_output = true;
        loop_state.last_user_visible_respond = Some(answer.to_string());
        loop_state.delivery_messages.push(answer.to_string());
        loop_state.executed_step_results.push(ok_step_result(
            "step_1",
            "fs_basic",
            r#"{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.","path":"release_checklist.md"}"#,
        ));
        loop_state
            .executed_step_results
            .push(ok_step_result("step_2", "respond", answer));
        let mut route = free_route_result();
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert!(content_evidence_terminal_respond_is_contractual_answer(
            &loop_state,
            Some(&agent_run_context),
            answer,
        ));
    }

    #[test]
    fn content_evidence_contractual_terminal_answer_requires_observation() {
        let answer = "配置加载检查应先做。";
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state
            .executed_step_results
            .push(ok_step_result("step_1", "respond", answer));
        let mut route = free_route_result();
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert!(!content_evidence_terminal_respond_is_contractual_answer(
            &loop_state,
            Some(&agent_run_context),
            answer,
        ));
    }

    #[test]
    fn raw_listing_passthrough_is_dropped_for_content_evidence_free_shape() {
        let listing = "base_skill_response_contract.md\nskill_integration_guide.md";
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.last_user_visible_respond = Some(listing.to_string());
        loop_state.delivery_messages.push(listing.to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "list_dir".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(format!("{listing}\n")),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: "docs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        assert_eq!(
            should_drop_passthrough_delivery_for_content_evidence(
                &loop_state,
                true,
                Some(&agent_run_context),
                listing
            ),
            Some(true)
        );
    }

    #[test]
    fn single_listing_entry_passthrough_is_dropped_for_content_evidence() {
        let listing = "base_skill_response_contract.md\nskill_integration_guide.md";
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.last_user_visible_respond = Some("base_skill_response_contract.md".to_string());
        loop_state
            .delivery_messages
            .push("base_skill_response_contract.md".to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "list_dir".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(format!("{listing}\n")),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的"
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::DirectoryPurposeSummary,
                locator_hint: "docs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            auto_locator_path: Some("/tmp/docs".to_string()),
            ..Default::default()
        };
        assert_eq!(
            should_drop_passthrough_delivery_for_content_evidence(
                &loop_state,
                true,
                Some(&agent_run_context),
                "base_skill_response_contract.md"
            ),
            Some(true)
        );
    }

    #[test]
    fn direct_scalar_finalize_prefers_presence_plus_path_for_fs_search_presence_queries() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_search".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"action":"find_name","count":1,"results":["rustclaw.service"],"root":""}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = scalar_route_result();
        route.resolved_intent =
            "检查仓库工作区中是否存在 rustclaw.service 文件，如果存在则返回路径，如果不存在则返回不存在。回答格式只输出有或没有以及路径。"
                .to_string();
        route.output_contract.requires_content_evidence = false;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let (answer, summary) =
            direct_scalar_observed_answer(None, &loop_state, Some(&agent_run_context))
                .expect("presence+path fallback should succeed");
        assert_eq!(answer, "有，路径：rustclaw.service");
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn archive_exit_zero_passthrough_is_dropped_when_structured_answer_exists() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.last_user_visible_respond = Some("exit=0".to_string());
        loop_state.delivery_messages.push("exit=0".to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "archive_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                "exit=0\nupdating: tmp/rustclaw-workspace/scripts/skill_calls/\n".to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent:
                "把 scripts/skill_calls 打成一个 zip 到 tmp/nl_archive_case.zip，然后告诉我是否成功"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
                locator_hint: "scripts/skill_calls -> tmp/nl_archive_case.zip".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        discard_raw_passthrough_delivery_when_structured_answer_available(
            &claimed_task("task-archive"),
            &mut loop_state,
            Some(&agent_run_context),
        );

        assert!(loop_state.delivery_messages.is_empty());
        assert!(loop_state.last_user_visible_respond.is_none());
    }

    #[test]
    fn raw_publishable_guard_rejects_structured_json_payloads() {
        assert!(looks_like_structured_machine_output(
            r#"{"hostname":"rustclaw-test-host.local","cwd":"/tmp/rustclaw-workspace"}"#
        ));
        assert!(looks_like_structured_machine_output(
            r#"[{"name":"README.md"},{"name":"Cargo.toml"}]"#
        ));
        assert!(!looks_like_structured_machine_output(
            "rustclaw-test-host.local"
        ));
        assert!(!looks_like_structured_machine_output(
            "package_manager=brew"
        ));
    }

    #[test]
    fn raw_publishable_guard_rejects_multi_line_command_snapshots() {
        assert!(looks_like_raw_command_snapshot(
            "exit=0\nCOMMAND PID USER\nclawd 4498 testuser TCP *:8787 (LISTEN)\n"
        ));
        assert!(!looks_like_raw_command_snapshot("testuser"));
    }

    #[test]
    fn package_manager_summary_uses_structured_detect_answer() {
        let state = test_state();
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .output_vars
            .insert("last_skill_name".to_string(), "package_manager".to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "package_manager".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some("package_manager=brew".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });

        let mut route = free_route_result();
        route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
        route.resolved_intent =
            "check which package manager is recognized and briefly say the everyday default"
                .to_string();
        route.route_reason = "llm_contract:package_manager_detect_summary".to_string();
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        let structured_answer =
            direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context));
        assert_eq!(
            structured_answer
                .as_ref()
                .map(|(answer, _summary)| answer.as_str()),
            Some("Detected package manager: brew."),
            "package manager summary should use structured skill evidence"
        );

        assert!(
            direct_non_builtin_skill_raw_answer(&state, &loop_state, Some(&agent_run_context))
                .is_none(),
            "one-sentence summary should not raw-passthrough package_manager output"
        );
    }

    #[test]
    fn git_status_summary_defers_to_synthesis_instead_of_raw_passthrough() {
        let state = test_state();
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .output_vars
            .insert("last_skill_name".to_string(), "git_basic".to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "git_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                "exit=0\n## main...origin/main\n M Cargo.toml\n?? new_file.txt\n".to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });

        let mut route = free_route_result();
        route.ask_mode = crate::AskMode::planner_execute_plain();
        route.resolved_intent = "检查当前仓库是否有未提交改动，用一句话告诉我".to_string();
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert!(
            direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context))
                .is_none(),
            "git status summary should be synthesized from observed evidence"
        );

        assert!(
            direct_non_builtin_skill_raw_answer(&state, &loop_state, Some(&agent_run_context))
                .is_none(),
            "one-sentence summary should not raw-passthrough git status output"
        );
    }

    #[test]
    fn scalar_git_log_does_not_use_non_builtin_raw_passthrough() {
        let state = test_state();
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .output_vars
            .insert("last_skill_name".to_string(), "git_basic".to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "git_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                "exit=0\n09342a6a fix: expose nl execution and locator flows\n".to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });

        let mut route = scalar_route_result();
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert!(
            direct_non_builtin_skill_raw_answer(&state, &loop_state, Some(&agent_run_context))
                .is_none(),
            "scalar git requests should use structured extraction or synthesis, not raw passthrough"
        );
    }

    #[test]
    fn file_token_auto_locator_wraps_bare_filename_under_directory() {
        let temp = TempDirGuard::new("file_token_dir");
        let file_path = temp.path().join("report.txt");
        fs::write(&file_path, "hello").expect("write");
        let expected = format!(
            "FILE:{}",
            file_path
                .canonicalize()
                .unwrap_or(file_path.clone())
                .display()
        );
        assert_eq!(
            resolve_file_token_from_auto_locator_answer(
                "report.txt",
                Some(temp.path().to_string_lossy().as_ref())
            )
            .as_deref(),
            Some(expected.as_str())
        );
    }

    #[test]
    fn file_token_auto_locator_normalizes_delivery_messages() {
        let temp = TempDirGuard::new("file_token_messages");
        let file_path = temp.path().join("report.txt");
        fs::write(&file_path, "hello").expect("write");
        let expected = format!(
            "FILE:{}",
            file_path
                .canonicalize()
                .unwrap_or(file_path.clone())
                .display()
        );
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.last_user_visible_respond = Some("report.txt".to_string());
        loop_state.delivery_messages.push("report.txt".to_string());

        let mut route = scalar_route_result();
        route.output_contract.response_shape = OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            auto_locator_path: Some(temp.path().to_string_lossy().to_string()),
            ..Default::default()
        };

        normalize_file_token_delivery_from_auto_locator(&mut loop_state, Some(&agent_run_context));

        assert_eq!(
            loop_state.last_user_visible_respond.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(loop_state.delivery_messages, vec![expected]);
    }

    #[test]
    fn file_token_auto_locator_recovers_from_observed_bare_filename() {
        let temp = TempDirGuard::new("file_token_observed_bare_filename");
        let file_path = temp.path().join("report.txt");
        fs::write(&file_path, "hello").expect("write");
        let expected = format!(
            "FILE:{}",
            file_path
                .canonicalize()
                .unwrap_or(file_path.clone())
                .display()
        );
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step_result("step_1", "run_cmd", "report.txt\n"));

        let mut route = scalar_route_result();
        route.output_contract.response_shape = OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            auto_locator_path: Some(temp.path().to_string_lossy().to_string()),
            ..Default::default()
        };

        let (token, summary) = direct_file_token_from_observed_auto_locator_filename(
            &loop_state,
            Some(&agent_run_context),
        )
        .expect("bare filename observation under auto locator should recover file token");

        assert_eq!(token, expected);
        assert_eq!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn file_token_observed_path_normalizes_bare_filename_delivery() {
        let temp = TempDirGuard::new("file_token_observed_path");
        let file_path = temp.path().join("document/report.txt");
        fs::create_dir_all(file_path.parent().expect("parent")).expect("mkdir");
        fs::write(&file_path, "hello").expect("write");
        let expected = format!(
            "FILE:{}",
            file_path
                .canonicalize()
                .unwrap_or(file_path.clone())
                .display()
        );
        let mut state = test_state();
        state.skill_rt.workspace_root = temp.path().to_path_buf();
        state.skill_rt.default_locator_search_dir = temp.path().to_path_buf();

        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.last_user_visible_respond = Some("FILE:report.txt".to_string());
        loop_state
            .delivery_messages
            .push("FILE:report.txt".to_string());
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                serde_json::json!({
                    "entries": [
                        {"name": "report.txt", "path": "document/report.txt"}
                    ]
                })
                .to_string(),
            ),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let mut route = scalar_route_result();
        route.output_contract.response_shape = OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        normalize_file_token_delivery_from_observed_paths(
            &state,
            &mut loop_state,
            Some(&agent_run_context),
        );

        assert_eq!(
            loop_state.last_user_visible_respond.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(loop_state.delivery_messages, vec![expected]);
    }

    #[test]
    fn missing_file_search_evidence_detects_zero_match_fs_search() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_search".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                serde_json::json!({
                    "action": "find_name",
                    "count": 0,
                    "results": [],
                    "root": ""
                })
                .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });

        assert!(has_missing_file_search_evidence(&loop_state));
    }

    #[test]
    fn missing_file_search_evidence_detects_missing_path_facts() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                serde_json::json!({
                    "action": "path_batch_facts",
                    "count": 1,
                    "facts": [{
                        "exists": false,
                        "path": "/tmp/definitely-missing.txt",
                        "error": "not found"
                    }],
                    "include_missing": true
                })
                .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });

        assert!(has_missing_file_search_evidence(&loop_state));
    }

    #[test]
    fn latest_file_delivery_observation_treats_missing_path_facts_as_terminal_missing() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                serde_json::json!({
                    "action": "path_batch_facts",
                    "count": 1,
                    "facts": [{
                        "exists": false,
                        "path": "/tmp/definitely-missing.txt",
                        "error": "not found"
                    }],
                    "include_missing": true
                })
                .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        loop_state.last_publishable_synthesis_output =
            Some("文件 /tmp/definitely-missing.txt 不存在，无法发送。".to_string());
        loop_state.last_user_visible_respond = loop_state.last_publishable_synthesis_output.clone();
        loop_state.delivery_messages = vec![loop_state
            .last_publishable_synthesis_output
            .clone()
            .unwrap()];

        let mut route = scalar_route_result();
        route.wants_file_delivery = true;
        route.output_contract.response_shape = OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert!(latest_file_delivery_observation_is_missing(&loop_state));
        assert!(should_return_missing_file_delivery_reply(
            &loop_state,
            Some(&agent_run_context)
        ));
    }

    #[test]
    fn missing_file_search_evidence_detects_not_found_probe_output() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some("NOT_FOUND\n".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });

        assert!(has_missing_file_search_evidence(&loop_state));
    }

    #[test]
    fn missing_file_search_evidence_detects_system_basic_find_path_zero_matches() {
        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                serde_json::json!({
                    "action": "find_path",
                    "count": 0,
                    "matches": [],
                    "query": "missing.md",
                    "target_kind": "file"
                })
                .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });

        assert!(has_missing_file_search_evidence(&loop_state));
    }

    #[tokio::test]
    async fn finalize_loop_reply_returns_not_found_for_missing_file_delivery() {
        let state = test_state();
        let task = claimed_task("task-missing-file-delivery");
        let mut route = scalar_route_result();
        route.wants_file_delivery = true;
        route.output_contract.response_shape = OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_hint = "definitely_missing_named_file.txt".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_search".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                serde_json::json!({
                    "action": "find_name",
                    "count": 0,
                    "results": [],
                    "root": ""
                })
                .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });

        let reply = finalize_loop_reply(
            &state,
            &task,
            "把 definitely_missing_named_file.txt 发给我",
            loop_state,
            Some(&agent_run_context),
        )
        .await
        .expect("finalize should return a missing-file answer");

        assert!(!reply.should_fail_task);
        assert_eq!(reply.messages.last(), Some(&reply.text));
        assert!(reply
            .messages
            .iter()
            .any(|message| crate::finalize::is_execution_summary_message(message)));
        assert!(
            reply.text.contains("未找到")
                || reply.text.contains("没有找到")
                || reply.text.contains("not found")
        );
        assert!(reply.text.contains("definitely_missing_named_file.txt"));
        assert_eq!(
            reply
                .task_journal
                .as_ref()
                .and_then(|journal| journal.final_status),
            Some(crate::task_journal::TaskJournalFinalStatus::Success)
        );
    }

    #[tokio::test]
    async fn finalize_loop_reply_returns_not_found_for_run_cmd_not_found_delivery() {
        let state = test_state();
        let task = claimed_task("task-missing-file-delivery-run-cmd");
        let mut route = scalar_route_result();
        route.wants_file_delivery = true;
        route.output_contract.response_shape = OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_hint = "/tmp/definitely-missing.txt".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some("NOT_FOUND\n".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });

        let reply = finalize_loop_reply(
            &state,
            &task,
            "把 /tmp/definitely-missing.txt 发给我",
            loop_state,
            Some(&agent_run_context),
        )
        .await
        .expect("finalize should return a missing-file answer");

        assert!(!reply.should_fail_task);
        assert_eq!(reply.messages.last(), Some(&reply.text));
        let summary = reply
            .messages
            .iter()
            .find(|message| crate::finalize::is_execution_summary_message(message))
            .expect("missing-file reply should include execution process");
        assert!(summary.contains("file not found"));
        assert!(
            reply.text.contains("未找到")
                || reply.text.contains("没有找到")
                || reply.text.contains("not found")
        );
        assert_eq!(
            reply
                .task_journal
                .as_ref()
                .and_then(|journal| journal.final_status),
            Some(crate::task_journal::TaskJournalFinalStatus::Success)
        );
    }

    #[tokio::test]
    async fn finalize_loop_reply_returns_not_found_for_missing_path_facts_delivery() {
        let state = test_state();
        let task = claimed_task("task-missing-file-delivery-path-facts");
        let mut route = scalar_route_result();
        route.wants_file_delivery = true;
        route.output_contract.response_shape = OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_hint = "/tmp/definitely-missing.txt".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                serde_json::json!({
                    "action": "path_batch_facts",
                    "count": 1,
                    "facts": [{
                        "exists": false,
                        "path": "/tmp/definitely-missing.txt",
                        "error": "not found"
                    }],
                    "include_missing": true
                })
                .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        loop_state.last_user_visible_respond = Some("FILE:/tmp/definitely-missing.txt".to_string());
        loop_state.delivery_messages = vec!["FILE:/tmp/definitely-missing.txt".to_string()];

        let reply = finalize_loop_reply(
            &state,
            &task,
            "把 /tmp/definitely-missing.txt 发给我",
            loop_state,
            Some(&agent_run_context),
        )
        .await
        .expect("finalize should return a missing-file answer");

        assert!(!reply.should_fail_task);
        assert_eq!(reply.messages.last(), Some(&reply.text));
        assert!(reply
            .messages
            .iter()
            .any(|message| crate::finalize::is_execution_summary_message(message)));
        assert!(
            reply.text.contains("未找到")
                || reply.text.contains("没有找到")
                || reply.text.contains("not found")
        );
        assert_eq!(
            reply
                .task_journal
                .as_ref()
                .and_then(|journal| journal.final_status),
            Some(crate::task_journal::TaskJournalFinalStatus::Success)
        );
    }

    #[tokio::test]
    async fn finalize_loop_reply_keeps_missing_file_delivery_when_synthesis_is_non_token() {
        let state = test_state();
        let task = claimed_task("task-missing-file-delivery-synthesis");
        let mut route = scalar_route_result();
        route.wants_file_delivery = true;
        route.output_contract.response_shape = OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_hint = "/tmp/definitely-missing.txt".to_string();
        let agent_run_context = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        let mut loop_state = crate::agent_engine::LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                serde_json::json!({
                    "action": "path_batch_facts",
                    "count": 1,
                    "facts": [{
                        "exists": false,
                        "path": "/tmp/definitely-missing.txt",
                        "error": "not found"
                    }],
                    "include_missing": true
                })
                .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        loop_state.last_publishable_synthesis_output =
            Some("文件 /tmp/definitely-missing.txt 不存在，无法发送。".to_string());

        let reply = finalize_loop_reply(
            &state,
            &task,
            "把 /tmp/definitely-missing.txt 发给我，不要猜内容",
            loop_state,
            Some(&agent_run_context),
        )
        .await
        .expect("finalize should return a missing-file answer");

        assert!(!reply.should_fail_task);
        assert_eq!(reply.messages.last(), Some(&reply.text));
        assert!(reply.text.contains("/tmp/definitely-missing.txt"));
        assert!(
            reply.text.contains("未找到")
                || reply.text.contains("没有找到")
                || reply.text.contains("not found")
        );
        assert!(reply
            .messages
            .iter()
            .any(|message| crate::finalize::is_execution_summary_message(message)));
    }
}
