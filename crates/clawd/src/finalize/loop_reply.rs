use std::collections::BTreeMap;
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
    if raw_command_output_needs_structural_projection(route, loop_state) {
        return None;
    }
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
        if agent_run_context
            .and_then(|ctx| ctx.route_result.as_ref())
            .is_some_and(|route| raw_command_output_needs_structural_projection(route, loop_state))
            && loop_state
                .last_user_visible_respond
                .as_deref()
                .is_some_and(|answer| {
                    crate::agent_engine::observed_output::answer_is_direct_observation_passthrough(
                        answer, loop_state,
                    )
                })
        {
            return;
        }
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

fn replace_raw_read_delivery_with_synthesis(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        || !latest_publishable_synthesis_step_matches(loop_state)
    {
        return false;
    }
    let Some(synthesis) = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return false;
    };
    if crate::finalize::looks_like_planner_artifact(synthesis)
        || crate::finalize::looks_like_internal_trace_artifact(synthesis)
        || crate::finalize::parse_delivery_token(synthesis).is_some()
    {
        return false;
    }
    let Some(current_delivery) = current_user_visible_delivery_text(loop_state) else {
        return false;
    };
    if current_delivery == synthesis
        || !delivery_is_raw_read_observation(current_delivery, loop_state)
    {
        return false;
    }

    info!(
        "final_result_replace_raw_read_delivery_with_synthesis task_id={} raw={}",
        task.task_id,
        crate::truncate_for_log(current_delivery)
    );
    loop_state.delivery_messages.clear();
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        synthesis.to_string(),
    );
    loop_state.last_user_visible_respond = Some(synthesis.to_string());
    true
}

fn delivery_is_raw_read_observation(delivery: &str, loop_state: &LoopState) -> bool {
    let delivery = delivery.trim();
    if delivery.is_empty()
        || crate::finalize::is_execution_summary_message(delivery)
        || message_is_non_answer_separator(delivery)
    {
        return false;
    }
    raw_read_range_output(delivery)
        || read_range_excerpt_like(delivery)
        || (crate::agent_engine::observed_output::answer_is_direct_observation_passthrough(
            delivery, loop_state,
        ) && loop_state
            .executed_step_results
            .iter()
            .rev()
            .any(step_output_is_read_range))
}

fn step_output_is_read_range(step: &crate::executor::StepExecutionResult) -> bool {
    if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
        return false;
    }
    step.output
        .as_deref()
        .map(str::trim)
        .is_some_and(raw_read_range_output)
}

fn raw_read_range_output(output: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(output.trim())
        .ok()
        .is_some_and(|value| {
            matches!(
                value.get("action").and_then(|value| value.as_str()),
                Some("read_range" | "read_text_range")
            ) && value
                .get("excerpt")
                .and_then(|value| value.as_str())
                .is_some_and(|excerpt| !excerpt.trim().is_empty())
        })
}

fn read_range_excerpt_like(output: &str) -> bool {
    let mut numbered_lines = 0usize;
    let mut total_lines = 0usize;
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        total_lines += 1;
        let Some((prefix, rest)) = line.split_once('|') else {
            continue;
        };
        if !rest.trim().is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit()) {
            numbered_lines += 1;
        }
    }
    total_lines >= 3 && numbered_lines >= 3
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
                "read_file" | "system_basic" | "fs_search" | "archive_basic"
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

fn active_anchor_bound_targets(agent_run_context: Option<&AgentRunContext>) -> Vec<String> {
    let Some(ctx) = agent_run_context else {
        return Vec::new();
    };
    let mut targets = Vec::new();
    let sources = [
        ctx.route_result
            .as_ref()
            .map(|route| route.resolved_intent.as_str()),
        ctx.context_bundle_summary.as_deref(),
        ctx.cross_turn_recent_execution_context.as_deref(),
        ctx.user_request.as_deref(),
    ];
    for source in sources.into_iter().flatten() {
        for line in source.lines() {
            let trimmed = line.trim_start();
            let Some(target) = ["followup_bound_target:", "observed_bound_target:"]
                .iter()
                .find_map(|prefix| trimmed.strip_prefix(prefix))
                .map(str::trim)
                .filter(|target| !target.is_empty())
            else {
                continue;
            };
            if !targets
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(target))
            {
                targets.push(target.to_string());
            }
        }
    }
    targets
}

fn path_leaf_eq(left: &str, right: &str) -> bool {
    Path::new(left)
        .file_name()
        .and_then(|value| value.to_str())
        .zip(
            Path::new(right)
                .file_name()
                .and_then(|value| value.to_str()),
        )
        .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn inventory_root_matches_bound_parent(value: &serde_json::Value, target: &str) -> bool {
    let Some(parent_name) = Path::new(target)
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    ["resolved_path", "path"]
        .iter()
        .filter_map(|field| value.get(*field).and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .any(|path| {
            Path::new(path)
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|root_name| root_name.eq_ignore_ascii_case(parent_name))
        })
}

fn inventory_entry_path_for_bound_target(
    value: &serde_json::Value,
    target: &str,
) -> Option<String> {
    if value.get("action").and_then(|value| value.as_str()) != Some("inventory_dir")
        || !inventory_root_matches_bound_parent(value, target)
    {
        return None;
    }
    let entries = value.get("entries").and_then(|value| value.as_array())?;
    for entry in entries {
        if entry
            .get("kind")
            .and_then(|value| value.as_str())
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("dir"))
        {
            continue;
        }
        let name = entry
            .get("name")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let path = entry
            .get("path")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let matches_target = name.is_some_and(|name| path_leaf_eq(name, target))
            || path.is_some_and(|path| path_leaf_eq(path, target));
        if !matches_target {
            continue;
        }
        if let Some(path) = path {
            return Some(path.to_string());
        }
        let root = value
            .get("path")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let name = name?;
        return Some(Path::new(root).join(name).display().to_string());
    }
    None
}

fn direct_path_from_active_bound_inventory(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let targets = active_anchor_bound_targets(agent_run_context);
    if targets.is_empty() {
        return None;
    }
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok() || !matches!(step.skill.as_str(), "fs_basic" | "system_basic") {
            continue;
        }
        let Some(value) = step
            .output
            .as_deref()
            .and_then(|output| serde_json::from_str::<serde_json::Value>(output).ok())
        else {
            continue;
        };
        for target in &targets {
            if let Some(path) = inventory_entry_path_for_bound_target(&value, target) {
                return Some((
                    path,
                    crate::task_journal::TaskJournalFinalizerSummary {
                        stage: Some(
                            crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric,
                        ),
                        disposition: Some(
                            crate::finalize::FinalizerDisposition::QualifiedCompletion,
                        ),
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
        }
    }
    None
}

fn path_batch_fact_file_path(entry: &serde_json::Value) -> Option<PathBuf> {
    let entry = entry.as_object()?;
    if entry.get("exists").and_then(|value| value.as_bool()) != Some(true) {
        return None;
    }
    let fact = entry.get("fact").and_then(|value| value.as_object());
    let kind = fact
        .and_then(|item| item.get("kind"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if kind.is_some_and(|kind| !kind.eq_ignore_ascii_case("file")) {
        return None;
    }
    let path = fact
        .and_then(|item| item.get("resolved_path"))
        .or_else(|| fact.and_then(|item| item.get("path")))
        .or_else(|| entry.get("resolved_path"))
        .or_else(|| entry.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let path = PathBuf::from(path);
    if kind.is_none() && !path.is_file() {
        return None;
    }
    Some(path.canonicalize().unwrap_or(path))
}

fn observed_path_batch_file_candidates(value: &serde_json::Value) -> Option<Vec<PathBuf>> {
    if value.get("action").and_then(|value| value.as_str()) != Some("path_batch_facts") {
        return None;
    }
    let facts = value.get("facts")?.as_array()?;
    let mut candidates = facts
        .iter()
        .filter_map(path_batch_fact_file_path)
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    (!candidates.is_empty()).then_some(candidates)
}

fn direct_file_token_from_observed_path_batch_facts(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_requires_file_token(agent_run_context) {
        return None;
    }
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok() || !matches!(step.skill.as_str(), "fs_basic" | "system_basic") {
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
        let candidates = observed_path_batch_file_candidates(&value)?;
        if candidates.len() != 1 {
            return None;
        }
        return Some((
            format!("FILE:{}", candidates[0].display()),
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
    if let Some(answer) =
        deterministic_scalar_markdown_heading_answer_from_loop(loop_state, agent_run_context)
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

fn deterministic_scalar_markdown_heading_answer_from_loop(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if !route_allows_direct_scalar_observed_answer(route)
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::DirectoryEntryGroups
                | crate::OutputSemanticKind::ScalarCount
                | crate::OutputSemanticKind::RawCommandOutput
                | crate::OutputSemanticKind::ScalarPathOnly
                | crate::OutputSemanticKind::ExistenceWithPath
        )
    {
        return None;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok())
        .filter_map(|step| step.output.as_deref())
        .find(|output| output.contains("\"read_range\"") || output.contains("\"read_text_range\""))
        .and_then(markdown_heading_from_read_output)
}

fn route_allows_observed_markdown_heading_scalar_delivery(route: &crate::RouteResult) -> bool {
    if route_allows_direct_scalar_observed_answer(route) {
        return true;
    }
    route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free
                | crate::OutputResponseShape::Strict
                | crate::OutputResponseShape::OneSentence
        )
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
        )
        && !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
}

fn route_allows_observed_markdown_heading_body_reduction(route: &crate::RouteResult) -> bool {
    route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
                | crate::OutputResponseShape::Strict
                | crate::OutputResponseShape::OneSentence
        )
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
        )
        && !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
}

fn observed_markdown_heading_scalar_answer_for_delivery(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery: &str,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if !route_allows_observed_markdown_heading_scalar_delivery(route)
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::DirectoryEntryGroups
                | crate::OutputSemanticKind::ScalarCount
                | crate::OutputSemanticKind::RawCommandOutput
                | crate::OutputSemanticKind::ScalarPathOnly
                | crate::OutputSemanticKind::ExistenceWithPath
                | crate::OutputSemanticKind::ExistenceWithPathSummary
        )
    {
        return None;
    }
    let trimmed_delivery = delivery.trim();
    if trimmed_delivery.is_empty() {
        return None;
    }
    let observed_output = loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok())
        .filter_map(|step| step.output.as_deref())
        .find(|output| {
            output.contains("\"read_range\"") || output.contains("\"read_text_range\"")
        })?;
    let observed_heading = markdown_heading_from_read_output(observed_output)?;
    if trimmed_delivery.contains('\n') {
        if route_allows_observed_markdown_heading_body_reduction(route)
            && markdown_read_body_matches_delivery(observed_output, trimmed_delivery)
        {
            return Some(observed_heading);
        }
        return None;
    }
    if trimmed_delivery == observed_heading.trim() {
        return Some(observed_heading);
    }
    let delivery_heading = markdown_heading_from_line(trimmed_delivery)?;
    (delivery_heading.trim() == observed_heading.trim()).then_some(observed_heading)
}

fn replace_delivery_with_observed_markdown_heading_scalar(
    task_id: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(current_delivery) = delivery_messages.last().map(String::as_str) else {
        return false;
    };
    let Some(answer) = observed_markdown_heading_scalar_answer_for_delivery(
        loop_state,
        agent_run_context,
        current_delivery,
    ) else {
        return false;
    };
    if current_delivery.trim() == answer.trim() {
        return false;
    }
    info!(
        "delivery markdown_heading_scalar_from_observed task_id={} previous={} observed={}",
        task_id,
        crate::truncate_for_log(current_delivery),
        crate::truncate_for_log(&answer)
    );
    delivery_messages.clear();
    delivery_messages.push(answer.clone());
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: 1,
        ..Default::default()
    });
    true
}

fn markdown_heading_from_read_output(output: &str) -> Option<String> {
    let text = markdown_text_from_read_output(output)?;
    standalone_markdown_heading_from_text(&text)
}

fn first_markdown_heading_from_read_output(output: &str) -> Option<String> {
    let text = markdown_text_from_read_output(output)?;
    text.lines().find_map(markdown_heading_from_line)
}

fn standalone_markdown_heading_from_text(text: &str) -> Option<String> {
    let mut heading: Option<String> = None;
    for line in text.lines() {
        let stripped = strip_markdown_read_line_prefix(line).trim();
        if stripped.is_empty() {
            continue;
        }
        if let Some(candidate) = markdown_heading_from_line(stripped) {
            if heading.is_some() {
                return None;
            }
            heading = Some(candidate);
            continue;
        }
        if markdown_line_is_non_answer_separator_heading(stripped) {
            continue;
        }
        return None;
    }
    heading
}

fn markdown_read_body_matches_delivery(output: &str, delivery: &str) -> bool {
    let Some(observed) = markdown_text_from_read_output(output) else {
        return false;
    };
    normalize_markdown_body_for_compare(&observed) == normalize_markdown_body_for_compare(delivery)
}

fn markdown_text_from_read_output(output: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    let text = value
        .get("content")
        .or_else(|| value.get("excerpt"))
        .and_then(serde_json::Value::as_str)?;
    Some(
        text.lines()
            .map(strip_markdown_read_line_prefix)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn normalize_markdown_body_for_compare(text: &str) -> String {
    text.lines()
        .map(strip_markdown_read_line_prefix)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .replace("\r\n", "\n")
}

fn strip_markdown_read_line_prefix(line: &str) -> &str {
    let trimmed = line.trim();
    if let Some((prefix, rest)) = trimmed.split_once('|') {
        if !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit()) {
            return rest.trim();
        }
    }
    line
}

fn markdown_heading_from_line(line: &str) -> Option<String> {
    let trimmed = strip_markdown_read_line_prefix(line).trim();
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = trimmed.get(hashes..)?.trim();
    if rest.is_empty() || message_is_non_answer_separator(rest) {
        return None;
    }
    Some(rest.to_string())
}

fn markdown_line_is_non_answer_separator_heading(line: &str) -> bool {
    let trimmed = strip_markdown_read_line_prefix(line).trim();
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hashes) {
        return false;
    }
    trimmed
        .get(hashes..)
        .map(str::trim)
        .is_some_and(message_is_non_answer_separator)
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
    agent_run_context: Option<&AgentRunContext>,
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
    if latest_publishable_synthesis_step_matches(loop_state)
        && current_user_visible_delivery_text(loop_state).is_some_and(|delivery| {
            let delivery = delivery.trim();
            loop_state
                .last_publishable_synthesis_output
                .as_deref()
                .map(str::trim)
                .is_some_and(|synthesis| {
                    delivery == synthesis
                        && !delivery_is_raw_read_observation(delivery, loop_state)
                        && !crate::finalize::looks_like_planner_artifact(delivery)
                        && !crate::finalize::looks_like_internal_trace_artifact(delivery)
                        && crate::finalize::parse_delivery_token(delivery).is_none()
                })
        })
    {
        return false;
    }
    if !loop_contract_observed_answer_satisfies_required_evidence(loop_state, answer_kind) {
        return false;
    }
    if let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) {
        if route_requires_matrix_deterministic_final_answer(route)
            && !matrix_candidate_satisfies_final_shape(
                task,
                &route.resolved_intent,
                loop_state,
                agent_run_context,
                Some(summary.clone()),
                route,
                &answer,
            )
        {
            return false;
        }
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

fn latest_terminal_planned_respond(loop_state: &LoopState) -> Option<&str> {
    loop_state
        .round_traces
        .iter()
        .rev()
        .filter_map(|round| round.plan_result.as_ref())
        .filter_map(|plan| plan.steps.last())
        .find_map(|step| {
            if step.action_type != "respond" && step.skill != "respond" {
                return None;
            }
            step.args
                .get("content")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|content| !content.is_empty())
        })
}

fn observed_json_scalar_matches_candidate(value: &serde_json::Value, candidate: &str) -> bool {
    match value {
        serde_json::Value::String(text) => text.trim() == candidate,
        serde_json::Value::Number(number) => number.to_string() == candidate,
        serde_json::Value::Bool(value) => value.to_string() == candidate,
        serde_json::Value::Array(items) => items
            .iter()
            .any(|value| observed_json_scalar_matches_candidate(value, candidate)),
        serde_json::Value::Object(map) => map
            .values()
            .any(|value| observed_json_scalar_matches_candidate(value, candidate)),
        serde_json::Value::Null => false,
    }
}

fn planned_terminal_respond_is_grounded_in_observation(
    loop_state: &LoopState,
    candidate: &str,
) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || crate::agent_engine::observed_output::answer_is_direct_observation_passthrough(
            candidate, loop_state,
        )
    {
        return false;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think"
                )
        })
        .filter_map(|step| step.output.as_deref())
        .any(|output| {
            let output = output.trim();
            if output == candidate && !looks_like_structured_machine_output(output) {
                return true;
            }
            serde_json::from_str::<serde_json::Value>(output)
                .ok()
                .is_some_and(|value| observed_json_scalar_matches_candidate(&value, candidate))
        })
}

fn contractual_grounded_terminal_planned_respond(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    let candidate = latest_terminal_planned_respond(loop_state)?.trim();
    if candidate.is_empty()
        || candidate.contains("{{")
        || crate::finalize::parse_delivery_token(candidate).is_some()
        || crate::finalize::looks_like_planner_artifact(candidate)
        || crate::finalize::looks_like_internal_trace_artifact(candidate)
        || crate::finalize::is_execution_summary_message(candidate)
        || looks_like_structured_machine_output(candidate)
        || looks_like_raw_command_snapshot(candidate)
        || !planned_terminal_respond_is_grounded_in_observation(loop_state, candidate)
    {
        return None;
    }
    let answer = match crate::output_contract_verifier::verify_output_contract(
        &route.output_contract,
        candidate,
        &route.resolved_intent,
    ) {
        crate::output_contract_verifier::OutputContractVerdict::Pass => candidate.to_string(),
        crate::output_contract_verifier::OutputContractVerdict::Reshape { reshaped, .. } => {
            reshaped.trim().to_string()
        }
        crate::output_contract_verifier::OutputContractVerdict::Reject { .. } => return None,
    };
    if answer.is_empty() || looks_like_structured_machine_output(&answer) {
        return None;
    }
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
            used_evidence_ids_count: loop_state.executed_step_results.len().max(1),
            ..Default::default()
        },
    ))
}

fn replace_structured_delivery_with_grounded_terminal_respond(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    if !loop_state
        .delivery_messages
        .last()
        .is_some_and(|message| delivery_message_is_json_container(message))
    {
        return false;
    }
    let Some((answer, summary)) =
        contractual_grounded_terminal_planned_respond(loop_state, agent_run_context)
    else {
        return false;
    };
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
        "delivery replace_structured_with_grounded_terminal_respond task_id={}",
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

fn delivery_message_is_json_container(message: &str) -> bool {
    matches!(
        serde_json::from_str::<serde_json::Value>(message.trim()),
        Ok(serde_json::Value::Object(_) | serde_json::Value::Array(_))
    )
}

fn prefer_english_for_user_text(state: &AppState, user_text: &str) -> bool {
    match crate::language_policy::request_language_hint(user_text) {
        "zh-CN" => false,
        "mixed" => !crate::language_policy::mixed_language_prefers_cjk_response(user_text),
        "config_default" => state
            .policy
            .command_intent
            .default_locale
            .to_ascii_lowercase()
            .starts_with("en"),
        _ => true,
    }
}

fn prefer_english_for_agent_contextual_user_text(
    state: &AppState,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    for candidate in [
        agent_run_context.and_then(|ctx| ctx.original_user_request.as_deref()),
        agent_run_context.and_then(|ctx| ctx.user_request.as_deref()),
        Some(user_text),
    ]
    .into_iter()
    .flatten()
    {
        let candidate = candidate.trim();
        if candidate.is_empty() {
            continue;
        }
        let hint = crate::language_policy::request_language_hint(candidate);
        if hint != "config_default" {
            return match hint {
                "zh-CN" => false,
                "mixed" => !crate::language_policy::mixed_language_prefers_cjk_response(candidate),
                _ => true,
            };
        }
    }
    prefer_english_for_user_text(state, user_text)
}

fn final_reply_language_hint(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> String {
    if let Some(original) = agent_run_context
        .and_then(|ctx| ctx.original_user_request.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        let hint = crate::language_policy::request_language_hint(original);
        if hint != "config_default" {
            return hint.to_string();
        }
    }
    crate::language_policy::task_response_language_hint(state, task, user_text)
}

fn prefer_english_for_final_reply(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let normalized = final_reply_language_hint(state, task, user_text, agent_run_context)
        .trim()
        .to_ascii_lowercase()
        .to_string();
    !(normalized.starts_with("zh") || normalized == "mixed")
}

fn deterministic_template_language_preference(
    state: &AppState,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<bool> {
    let hint = agent_run_context
        .and_then(|ctx| ctx.original_user_request.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(crate::language_policy::request_language_hint)
        .filter(|hint| *hint != "config_default")
        .unwrap_or_else(|| crate::language_policy::request_language_hint(user_text));
    let normalized = hint.trim().to_ascii_lowercase();
    if normalized.starts_with("zh") {
        Some(false)
    } else if normalized.starts_with("en") {
        Some(true)
    } else if normalized == "mixed" {
        Some(!crate::language_policy::mixed_language_prefers_cjk_response(user_text))
    } else if normalized == "config_default" || normalized.is_empty() {
        Some(
            state
                .policy
                .command_intent
                .default_locale
                .to_ascii_lowercase()
                .starts_with("en"),
        )
    } else {
        None
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
    !matches!(
        crate::language_policy::request_language_hint(user_text),
        "zh-CN" | "config_default"
    )
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
    direct_structured_observed_answer_impl(state, loop_state, agent_run_context, false)
}

fn direct_structured_observed_answer_allowing_implicit_metadata_path_facts(
    state: Option<&AppState>,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    direct_structured_observed_answer_impl(state, loop_state, agent_run_context, true)
}

fn direct_structured_observed_answer_impl(
    state: Option<&AppState>,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    allow_implicit_metadata_path_facts: bool,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if raw_command_output_needs_structural_projection(route, loop_state) {
        return None;
    }
    if route.ask_mode.finalize_chat_wrapped()
        && route.output_contract.requires_content_evidence
        && latest_plan_requested_synthesis(loop_state)
        && route.output_contract.semantic_kind != crate::OutputSemanticKind::GitRepositoryState
    {
        return None;
    }
    if route.ask_mode.finalize_chat_wrapped()
        && route.output_contract.requires_content_evidence
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
        && latest_path_batch_facts_has_implicit_metadata_fields(loop_state)
        && !allow_implicit_metadata_path_facts
    {
        return None;
    }
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken | crate::OutputResponseShape::Scalar
    ) {
        return None;
    }
    if let Some(answer) =
        crate::agent_engine::observed_output::structured_scalar_equality_direct_answer(
            state,
            route,
            loop_state,
            agent_run_context,
        )
    {
        return Some((
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
                used_evidence_ids_count: 2,
                ..Default::default()
            },
        ));
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

fn latest_plan_requested_synthesis(loop_state: &LoopState) -> bool {
    loop_state.round_traces.iter().rev().any(|round| {
        round
            .plan_result
            .as_ref()
            .is_some_and(|plan| plan.raw_plan_text.contains("\"synthesize_answer\""))
    })
}

fn latest_path_batch_facts_has_implicit_metadata_fields(loop_state: &LoopState) -> bool {
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

fn step_output_is_tail_read_range(step: &crate::executor::StepExecutionResult) -> bool {
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
    value.get("action").and_then(|v| v.as_str()) == Some("read_range")
        && value.get("mode").and_then(|v| v.as_str()) == Some("tail")
}

fn current_user_visible_delivery_text(loop_state: &LoopState) -> Option<&str> {
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
        .rposition(step_output_is_tail_read_range)
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
        .rposition(step_output_is_tail_read_range)
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
    route: Option<&crate::RouteResult>,
    loop_state: &LoopState,
    replacement_answer: &str,
) -> bool {
    let Some(current_delivery) = current_user_visible_delivery_text(loop_state) else {
        return false;
    };
    if current_delivery.trim() == replacement_answer.trim() {
        return false;
    }
    if latest_tail_replacement_can_recover_stale_synthesis(loop_state, current_delivery) {
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
    route
        .map(|route| {
            route.output_contract.semantic_kind == crate::OutputSemanticKind::ContentExcerptSummary
        })
        .unwrap_or(false)
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
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    if latest_tail_read_range_should_preserve_current_delivery(route, loop_state, &answer) {
        info!(
            "delivery keep_current_summary_over_tail_read_range task_id={}",
            task.task_id
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
    if let Some((answer, summary)) = direct_raw_command_output_projection(route, loop_state) {
        if delivery_messages
            .last()
            .is_some_and(|message| message.trim() == answer.trim())
        {
            loop_state.last_user_visible_respond = Some(answer);
            *finalizer_summary = Some(summary);
            return;
        }
        info!(
            "delivery exact_contract_raw_command_projection task_id={} previous={} observed={}",
            task_id,
            crate::truncate_for_log(
                delivery_messages
                    .last()
                    .map(String::as_str)
                    .unwrap_or_default()
            ),
            crate::truncate_for_log(&answer)
        );
        delivery_messages.clear();
        delivery_messages.push(answer.clone());
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        return;
    }
    if raw_command_output_needs_structural_projection(route, loop_state)
        && delivery_messages.last().is_some_and(|message| {
            let message = message.trim();
            !message.is_empty()
                && !crate::agent_engine::observed_output::answer_is_direct_observation_passthrough(
                    message, loop_state,
                )
                && matches!(
                    crate::output_contract_verifier::verify_output_contract(
                        &route.output_contract,
                        message,
                        &route.resolved_intent,
                    ),
                    crate::output_contract_verifier::OutputContractVerdict::Pass
                        | crate::output_contract_verifier::OutputContractVerdict::Reshape { .. }
                )
        })
    {
        info!(
            "delivery exact_contract_keep_structural_projection_answer task_id={} answer={}",
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
    if let Some(synthesis) = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        let scalar_value_contract =
            route.output_contract.response_shape == crate::OutputResponseShape::Scalar;
        if delivery_messages
            .last()
            .map(|message| message.trim() == synthesis)
            .unwrap_or(false)
            && !(has_prior_step_error && allow_prior_step_error_replacement)
            && !scalar_value_contract
            && route.output_contract.semantic_kind != crate::OutputSemanticKind::RawCommandOutput
            && planned_delivery_is_explicit_contractual_answer(route, synthesis)
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
    let scalar_value_contract =
        route.output_contract.response_shape == crate::OutputResponseShape::Scalar;
    if current_delivery_is_publishable_synthesis
        && latest_publishable_synthesis_step_matches(loop_state)
        && !(has_prior_step_error && allow_prior_step_error_replacement)
        && route.output_contract.semantic_kind != crate::OutputSemanticKind::RawCommandOutput
        && !route_requires_observed_semantic_projection(route)
        && current_synthesis_satisfies_matrix_shape(
            task_id,
            loop_state,
            agent_run_context,
            finalizer_summary.clone(),
            route,
            delivery_messages,
        )
        && delivery_messages.last().is_some_and(|message| {
            !delivery_is_raw_read_observation(message, loop_state)
                && !crate::finalize::looks_like_planner_artifact(message)
                && !crate::finalize::looks_like_internal_trace_artifact(message)
                && crate::finalize::parse_delivery_token(message).is_none()
        })
    {
        info!(
            "delivery exact_contract_keep_latest_synthesis task_id={} answer={}",
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
    if !current_delivery_is_publishable_synthesis
        && !scalar_value_contract
        && route.output_contract.semantic_kind != crate::OutputSemanticKind::RawCommandOutput
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
    let Some((answer, summary)) = direct_quantity_comparison_from_compare_paths(
        state,
        &route.resolved_intent,
        loop_state,
        agent_run_context,
    )
    .or_else(|| {
        direct_log_tail_status_answer(state, &route.resolved_intent, loop_state, agent_run_context)
    })
    .or_else(|| direct_scalar_observed_answer(Some(state), loop_state, agent_run_context))
    .or_else(|| latest_grounded_synthesis_for_mixed_listing_contract(route, loop_state))
    .or_else(|| direct_structured_observed_answer(Some(state), loop_state, agent_run_context))
    .or_else(|| exact_contract_fallback_observed_answer(route, loop_state)) else {
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
    let current_delivery_is_replaceable_status_synthesis = has_prior_step_error
        && allow_prior_step_error_replacement
        && current_delivery_is_publishable_synthesis;
    if !current_delivery_is_replaceable_status_synthesis
        && delivery_messages.last().is_some_and(|message| {
            should_keep_planned_delivery_over_observed_answer(route, message, answer)
        })
    {
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

fn exact_contract_fallback_observed_answer(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if raw_command_output_needs_structural_projection(route, loop_state) {
        return None;
    }
    let body = latest_successful_observation_body(loop_state)?.trim();
    if body.is_empty()
        || crate::finalize::looks_like_planner_artifact(body)
        || crate::finalize::looks_like_internal_trace_artifact(body)
        || looks_like_raw_command_snapshot(body)
    {
        return None;
    }
    let candidate = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| exact_contract_answer_from_json(route, &value))
        .or_else(|| {
            (route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput)
                .then(|| body.to_string())
        })
        .or_else(|| single_line_observation_answer(route, body))?;
    let candidate = match crate::output_contract_verifier::verify_output_contract(
        &route.output_contract,
        &candidate,
        &route.resolved_intent,
    ) {
        crate::output_contract_verifier::OutputContractVerdict::Pass => candidate,
        crate::output_contract_verifier::OutputContractVerdict::Reshape { reshaped, .. } => {
            reshaped
        }
        crate::output_contract_verifier::OutputContractVerdict::Reject { .. } => {
            if exact_fallback_candidate_is_machine_grounded(route, &candidate) {
                candidate
            } else {
                return None;
            }
        }
    };
    Some((
        candidate,
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
    ))
}

fn exact_fallback_candidate_is_machine_grounded(
    route: &crate::RouteResult,
    candidate: &str,
) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || crate::finalize::is_execution_summary_message(candidate)
        || crate::finalize::looks_like_planner_artifact(candidate)
        || crate::finalize::looks_like_internal_trace_artifact(candidate)
        || looks_like_structured_machine_output(candidate)
        || looks_like_raw_command_snapshot(candidate)
    {
        return false;
    }
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    ) {
        let mut lines = candidate
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty());
        return lines.next().is_some() && lines.next().is_none();
    }
    if route_path_locator_plain_act_allows_observed_listing(route) {
        return candidate.lines().any(|line| !line.trim().is_empty());
    }
    matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ExistenceWithPath | crate::OutputSemanticKind::FilePaths
    ) && candidate.lines().any(|line| !line.trim().is_empty())
}

fn exact_contract_answer_from_json(
    route: &crate::RouteResult,
    value: &serde_json::Value,
) -> Option<String> {
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    ) {
        return scalar_answer_from_json(value);
    }
    if matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::FilePaths | crate::OutputSemanticKind::ExistenceWithPath
    ) || matches!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    ) {
        return path_answer_from_json(value);
    }
    None
}

fn scalar_answer_from_json(value: &serde_json::Value) -> Option<String> {
    for key in ["value_text", "value", "count", "total"] {
        let Some(child) = value.get(key) else {
            continue;
        };
        if let Some(text) = child
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            return Some(text.to_string());
        }
        if child.is_number() || child.is_boolean() {
            return Some(child.to_string());
        }
    }
    None
}

fn path_answer_from_json(value: &serde_json::Value) -> Option<String> {
    for key in ["results", "paths", "names", "items"] {
        if let Some(items) = value.get(key).and_then(|child| child.as_array()) {
            let lines = items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            if !lines.is_empty() {
                return Some(lines.join("\n"));
            }
        }
    }
    for key in ["path", "resolved_path", "file_path", "output_path"] {
        if let Some(text) = value
            .get(key)
            .and_then(|child| child.as_str())
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            return Some(text.to_string());
        }
    }
    None
}

fn single_line_observation_answer(route: &crate::RouteResult, body: &str) -> Option<String> {
    let mut lines = body.lines().map(str::trim).filter(|line| !line.is_empty());
    let first = lines.next()?;
    if lines.next().is_some() {
        return None;
    }
    if !matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    ) {
        return None;
    }
    Some(first.to_string())
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
    if route.is_some_and(crate::agent_engine::observed_output::route_requires_synthesized_delivery)
    {
        return None;
    }
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
    if route_requires_matrix_deterministic_final_answer(route) {
        return None;
    }
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
    let trimmed = answer.trim();
    serde_json::from_str::<serde_json::Value>(trimmed)
        .map(|value| value.is_object() || value.is_array())
        .unwrap_or(false)
        || looks_like_contract_evidence_projection(trimmed)
        || looks_like_structured_key_path_projection(trimmed)
}

fn looks_like_contract_evidence_projection(answer: &str) -> bool {
    let mut has_path = false;
    let mut has_evidence_field = false;
    for line in answer.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if line.starts_with("path=") || line.starts_with("resolved_path=") {
            has_path = true;
            continue;
        }
        if matches!(
            line,
            "content_excerpt:" | "field_value:" | "command_output:" | "candidates:" | "results:"
        ) {
            has_evidence_field = true;
        }
    }
    has_path && has_evidence_field
}

fn looks_like_structured_key_path_projection(answer: &str) -> bool {
    let lines = answer
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return false;
    }
    let mut assignment_count = 0usize;
    let mut key_path_assignment_count = 0usize;
    for line in &lines {
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        let key = key.trim();
        if key.is_empty()
            || value.trim().is_empty()
            || !key
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '[' | ']'))
        {
            return false;
        }
        assignment_count += 1;
        if key.contains('.') || key.contains('[') || key.contains(']') {
            key_path_assignment_count += 1;
        }
    }
    assignment_count == lines.len() && key_path_assignment_count > 0
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

fn direct_raw_command_output_projection(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::RawCommandOutput
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
    {
        return None;
    }
    let outputs = loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && step.skill == "run_cmd")
        .filter_map(|step| step.output.as_deref())
        .map(str::trim)
        .filter(|output| !output.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if outputs.is_empty() {
        return None;
    }
    let projected = latest_raw_command_structural_projection(loop_state)
        .and_then(|projection| apply_raw_command_structural_projection(&outputs, &projection));
    let answer = projected.unwrap_or_else(|| outputs.join("\n"));
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
            used_evidence_ids_count: outputs.len(),
            ..Default::default()
        },
    ))
}

#[derive(Debug, Clone, Default)]
struct RawCommandStructuralProjection {
    limit: Option<usize>,
    sort_by: Option<String>,
}

fn latest_raw_command_structural_projection(
    loop_state: &LoopState,
) -> Option<RawCommandStructuralProjection> {
    loop_state.round_traces.iter().rev().find_map(|round| {
        let plan = round.plan_result.as_ref()?;
        plan.steps
            .iter()
            .find_map(|step| raw_command_projection_from_value(&step.args))
            .or_else(|| {
                serde_json::from_str::<serde_json::Value>(&plan.raw_plan_text)
                    .ok()
                    .and_then(|value| raw_command_projection_from_value(&value))
            })
    })
}

fn raw_command_projection_from_value(
    value: &serde_json::Value,
) -> Option<RawCommandStructuralProjection> {
    match value {
        serde_json::Value::Object(map) => {
            let limit = map
                .get("max_entries")
                .or_else(|| map.get("max_results"))
                .or_else(|| map.get("limit"))
                .or_else(|| map.get("n"))
                .and_then(|value| value.as_u64())
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value > 0);
            let sort_by = map
                .get("sort_by")
                .or_else(|| map.get("sort_order"))
                .or_else(|| map.get("order"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            if limit.is_some() || sort_by.is_some() {
                return Some(RawCommandStructuralProjection { limit, sort_by });
            }
            map.values().find_map(raw_command_projection_from_value)
        }
        serde_json::Value::Array(items) => items.iter().find_map(raw_command_projection_from_value),
        _ => None,
    }
}

fn apply_raw_command_structural_projection(
    outputs: &[String],
    projection: &RawCommandStructuralProjection,
) -> Option<String> {
    let mut lines = outputs
        .iter()
        .flat_map(|output| output.lines())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    if let Some(sort_by) = projection.sort_by.as_deref() {
        let normalized = sort_by.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "name" | "name_asc" | "asc" => lines.sort(),
            "name_desc" | "desc" => {
                lines.sort();
                lines.reverse();
            }
            _ => {}
        }
    }
    if let Some(limit) = projection.limit {
        lines.truncate(limit);
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn route_explicitly_requests_command_result(route: &crate::RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
        && route.output_contract.response_shape != crate::OutputResponseShape::Strict
}

fn raw_command_output_needs_structural_projection(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> bool {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::RawCommandOutput {
        return false;
    }
    let latest_is_run_cmd = loop_state
        .executed_step_results
        .iter()
        .rev()
        .find(|step| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think"
                )
        })
        .is_some_and(|step| step.skill == "run_cmd");
    latest_is_run_cmd && latest_plan_declares_structural_projection(loop_state)
}

fn latest_plan_declares_structural_projection(loop_state: &LoopState) -> bool {
    loop_state.round_traces.iter().rev().any(|round| {
        let Some(plan) = round.plan_result.as_ref() else {
            return false;
        };
        plan.steps
            .iter()
            .any(|step| value_declares_structural_projection(&step.args))
            || serde_json::from_str::<serde_json::Value>(&plan.raw_plan_text)
                .ok()
                .is_some_and(|value| value_declares_structural_projection(&value))
    })
}

fn value_declares_structural_projection(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            if map
                .get("max_entries")
                .or_else(|| map.get("max_results"))
                .or_else(|| map.get("limit"))
                .or_else(|| map.get("n"))
                .is_some_and(json_value_is_positive_number)
            {
                return true;
            }
            if map
                .get("sort_by")
                .or_else(|| map.get("sort_order"))
                .or_else(|| map.get("order"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .is_some_and(|text| !text.eq_ignore_ascii_case("name"))
            {
                return true;
            }
            if map
                .get("ext_filter")
                .or_else(|| map.get("exclude"))
                .or_else(|| map.get("exclude_names"))
                .or_else(|| map.get("exclude_patterns"))
                .is_some_and(json_value_is_non_empty)
            {
                return true;
            }
            if map
                .get("files_only")
                .or_else(|| map.get("dirs_only"))
                .is_some_and(|value| value.as_bool() == Some(true))
            {
                return true;
            }
            map.values().any(value_declares_structural_projection)
        }
        serde_json::Value::Array(items) => items.iter().any(value_declares_structural_projection),
        _ => false,
    }
}

fn json_value_is_positive_number(value: &serde_json::Value) -> bool {
    value.as_u64().is_some_and(|number| number > 0)
        || value.as_i64().is_some_and(|number| number > 0)
        || value.as_f64().is_some_and(|number| number > 0.0)
}

fn json_value_is_non_empty(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => !text.trim().is_empty(),
        serde_json::Value::Array(items) => !items.is_empty(),
        serde_json::Value::Object(map) => !map.is_empty(),
        serde_json::Value::Bool(value) => *value,
        serde_json::Value::Number(_) => true,
        serde_json::Value::Null => false,
    }
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
            | crate::OutputSemanticKind::GitRepositoryState
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
    if crate::finalize::is_execution_summary_message(delivery) {
        return false;
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput {
        return false;
    }
    let scalar_model_language_verdict = route.output_contract.response_shape
        == crate::OutputResponseShape::Scalar
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath;
    if route_allows_model_language_final_answer(route)
        && (!matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        ) || scalar_model_language_verdict)
        && planned_delivery_is_publishable_model_language_answer(delivery)
    {
        return true;
    }
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    ) {
        return false;
    }
    let planned_delivery_contains_more_than_observed =
        delivery_has_planned_content_beyond_observed_answer(delivery, observed);
    if !planned_delivery_contains_more_than_observed {
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

fn route_allows_model_language_final_answer(route: &crate::RouteResult) -> bool {
    crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract)
        .is_some_and(|shape| shape.allows_model_language())
}

fn planned_delivery_is_publishable_model_language_answer(delivery: &str) -> bool {
    let delivery = delivery.trim();
    !delivery.is_empty()
        && crate::finalize::parse_delivery_token(delivery).is_none()
        && !crate::finalize::looks_like_planner_artifact(delivery)
        && !crate::finalize::looks_like_internal_trace_artifact(delivery)
        && !looks_like_structured_machine_output(delivery)
        && !looks_like_raw_command_snapshot(delivery)
        && !message_is_non_answer_separator(delivery)
}

fn matrix_final_answer_shape_class(
    route: &crate::RouteResult,
) -> Option<crate::contract_matrix::FinalAnswerShapeClass> {
    crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract)
        .map(|shape| shape.class())
}

fn route_requires_matrix_deterministic_final_answer(route: &crate::RouteResult) -> bool {
    matrix_final_answer_shape_class(route).is_some_and(|class| !class.allows_model_language())
}

fn agent_context_allows_observed_output_language_fallback(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_none_or(|route| !route_requires_matrix_deterministic_final_answer(route))
}

fn should_try_observed_output_language_fallback(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_context_allows_observed_output_language_fallback(agent_run_context)
        || latest_plan_requested_synthesis(loop_state)
        || successful_content_observation_should_precede_status_summary(
            agent_run_context,
            loop_state,
        )
}

fn route_has_contract_matrix_final_shape(route: &crate::RouteResult) -> bool {
    if matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    ) {
        return false;
    }
    crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract).is_some()
}

fn route_requires_observed_semantic_projection(route: &crate::RouteResult) -> bool {
    matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::DirectoryNames | crate::OutputSemanticKind::QuantityComparison
    )
}

fn matrix_candidate_satisfies_final_shape(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary>,
    route: &crate::RouteResult,
    candidate: &str,
) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return false;
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
    crate::answer_verifier::structurally_satisfies_answer_contract(route, &journal, candidate)
}

fn synthetic_task_for_matrix_shape_check(task_id: &str) -> ClaimedTask {
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

fn current_synthesis_satisfies_matrix_shape(
    task_id: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary>,
    route: &crate::RouteResult,
    delivery_messages: &[String],
) -> bool {
    if !route_requires_matrix_deterministic_final_answer(route) {
        return true;
    }
    let Some(message) = delivery_messages.last() else {
        return false;
    };
    let task = synthetic_task_for_matrix_shape_check(task_id);
    matrix_candidate_satisfies_final_shape(
        &task,
        &route.resolved_intent,
        loop_state,
        agent_run_context,
        finalizer_summary,
        route,
        message,
    )
}

fn matrix_observed_answer_candidate_for_shape(
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    shape_class: crate::contract_matrix::FinalAnswerShapeClass,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    match shape_class {
        crate::contract_matrix::FinalAnswerShapeClass::DeliveryArtifact => {
            direct_file_token_from_observed_auto_locator_filename(loop_state, agent_run_context)
                .or_else(|| {
                    direct_file_token_from_observed_inventory(loop_state, agent_run_context)
                })
        }
        crate::contract_matrix::FinalAnswerShapeClass::ScalarValue
        | crate::contract_matrix::FinalAnswerShapeClass::SinglePath => {
            direct_scalar_observed_answer(Some(state), loop_state, agent_run_context)
        }
        crate::contract_matrix::FinalAnswerShapeClass::StrictList => route
            .and_then(|route| {
                matrix_grouped_name_list_observed_answer(route, loop_state)
                    .or_else(|| matrix_docker_text_list_observed_answer(route, loop_state))
                    .or_else(|| matrix_strict_list_observed_answer(route, loop_state))
            })
            .or_else(|| {
                direct_structured_observed_answer_allowing_implicit_metadata_path_facts(
                    Some(state),
                    loop_state,
                    agent_run_context,
                )
            }),
        crate::contract_matrix::FinalAnswerShapeClass::Table => route
            .and_then(|route| matrix_table_observed_answer(route, loop_state))
            .or_else(|| {
                direct_structured_observed_answer_allowing_implicit_metadata_path_facts(
                    Some(state),
                    loop_state,
                    agent_run_context,
                )
            }),
        crate::contract_matrix::FinalAnswerShapeClass::Freeform
        | crate::contract_matrix::FinalAnswerShapeClass::GroundedSummary
        | crate::contract_matrix::FinalAnswerShapeClass::Verdict => None,
    }
}

fn matrix_strict_list_observed_answer(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::HiddenEntriesCheck
            | crate::OutputSemanticKind::FilePaths
            | crate::OutputSemanticKind::StructuredKeys
            | crate::OutputSemanticKind::SqliteTableNamesOnly
    ) {
        return None;
    }
    let mut items = BTreeMap::<String, String>::new();
    for step in &loop_state.executed_step_results {
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
        if route.output_contract.semantic_kind == crate::OutputSemanticKind::HiddenEntriesCheck {
            collect_matrix_hidden_entries(&value, &mut items);
        } else {
            collect_matrix_strict_list_items(route, &value, &mut items);
        }
    }
    if items.is_empty() {
        return None;
    }
    let answer = items.into_values().collect::<Vec<_>>().join("\n");
    Some((answer, matrix_observed_shape_summary(loop_state)))
}

fn collect_matrix_hidden_entries(value: &serde_json::Value, items: &mut BTreeMap<String, String>) {
    if let Some(entries) = value.get("entries").and_then(serde_json::Value::as_array) {
        for entry in entries {
            let Some(map) = entry.as_object() else {
                continue;
            };
            let hidden = map
                .get("hidden")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if !hidden {
                continue;
            }
            for key in ["name", "path"] {
                if let Some(text) = map.get(key).and_then(serde_json::Value::as_str) {
                    push_matrix_hidden_entry_item(text, items);
                    break;
                }
            }
        }
    }
    if let Some(names) = value.get("names").and_then(serde_json::Value::as_array) {
        for name in names {
            if let Some(text) = name.as_str() {
                push_matrix_hidden_entry_item(text, items);
            }
        }
    }
    if let Some(names_by_kind) = value
        .get("names_by_kind")
        .and_then(serde_json::Value::as_object)
    {
        for child in names_by_kind.values() {
            if let Some(array) = child.as_array() {
                for name in array {
                    if let Some(text) = name.as_str() {
                        push_matrix_hidden_entry_item(text, items);
                    }
                }
            }
        }
    }
}

fn push_matrix_hidden_entry_item(raw: &str, items: &mut BTreeMap<String, String>) {
    let item = raw.trim().trim_matches('`').trim();
    if item.is_empty() || item == "." || item == ".." {
        return;
    }
    let display = std::path::Path::new(item)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(item);
    if !display.starts_with('.') || matches!(display, "." | "..") {
        return;
    }
    items
        .entry(display.to_ascii_lowercase())
        .or_insert_with(|| display.to_string());
}

fn matrix_grouped_name_list_observed_answer(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract)
        != Some(crate::contract_matrix::FinalAnswerShape::GroupedNameList)
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::DirectoryEntryGroups
    {
        return None;
    }
    let mut dirs = BTreeMap::<String, String>::new();
    let mut files = BTreeMap::<String, String>::new();
    let mut other = BTreeMap::<String, String>::new();
    for step in &loop_state.executed_step_results {
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
        collect_matrix_grouped_name_items(route, &value, &mut dirs, &mut files, &mut other);
    }
    if dirs.is_empty() && files.is_empty() && other.is_empty() {
        return None;
    }
    let mut lines = Vec::new();
    push_matrix_grouped_name_lines("dirs", dirs, &mut lines);
    push_matrix_grouped_name_lines("files", files, &mut lines);
    push_matrix_grouped_name_lines("other", other, &mut lines);
    Some((lines.join("\n"), matrix_observed_shape_summary(loop_state)))
}

fn matrix_docker_text_list_observed_answer(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::DockerImages | crate::OutputSemanticKind::DockerPs
    ) {
        return None;
    }
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok() || !matches!(step.skill.as_str(), "docker_basic" | "run_cmd") {
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
        if looks_like_structured_machine_output(output)
            || crate::finalize::looks_like_planner_artifact(output)
            || crate::finalize::looks_like_internal_trace_artifact(output)
        {
            continue;
        }
        return Some((
            output.to_string(),
            matrix_observed_shape_summary(loop_state),
        ));
    }
    None
}

fn collect_matrix_grouped_name_items(
    route: &crate::RouteResult,
    value: &serde_json::Value,
    dirs: &mut BTreeMap<String, String>,
    files: &mut BTreeMap<String, String>,
    other: &mut BTreeMap<String, String>,
) {
    let Some(names_by_kind) = value
        .get("names_by_kind")
        .and_then(serde_json::Value::as_object)
    else {
        return;
    };
    push_matrix_grouped_name_array(route, names_by_kind.get("dirs"), dirs);
    push_matrix_grouped_name_array(route, names_by_kind.get("files"), files);
    push_matrix_grouped_name_array(route, names_by_kind.get("other"), other);
}

fn push_matrix_grouped_name_array(
    route: &crate::RouteResult,
    value: Option<&serde_json::Value>,
    items: &mut BTreeMap<String, String>,
) {
    let Some(array) = value.and_then(serde_json::Value::as_array) else {
        return;
    };
    for item in array {
        if let Some(text) = item.as_str() {
            push_matrix_grouped_name_item(route, text, items);
        }
    }
}

fn push_matrix_grouped_name_item(
    route: &crate::RouteResult,
    raw: &str,
    items: &mut BTreeMap<String, String>,
) {
    let Some(display) = matrix_list_display_item(route, raw) else {
        return;
    };
    items.entry(display.to_ascii_lowercase()).or_insert(display);
}

fn push_matrix_grouped_name_lines(
    label: &str,
    items: BTreeMap<String, String>,
    lines: &mut Vec<String>,
) {
    if items.is_empty() {
        return;
    }
    lines.push(format!("{label}:"));
    lines.extend(items.into_values().map(|item| format!("- {item}")));
}

fn collect_matrix_strict_list_items(
    route: &crate::RouteResult,
    value: &serde_json::Value,
    items: &mut BTreeMap<String, String>,
) {
    push_matrix_string_arrays(
        route,
        value,
        items,
        &[
            "keys",
            "identity_values",
            "names",
            "paths",
            "files",
            "dirs",
            "directories",
            "results",
            "tables",
        ],
    );
    if let Some(names_by_kind) = value
        .get("names_by_kind")
        .and_then(serde_json::Value::as_object)
    {
        for child in names_by_kind.values() {
            push_matrix_array_items(route, child, items);
        }
    }
    for key in ["entries", "items", "facts", "rows"] {
        if let Some(rows) = value.get(key).and_then(serde_json::Value::as_array) {
            for row in rows {
                collect_matrix_list_object_fields(route, row, items);
            }
        }
    }
}

fn push_matrix_string_arrays(
    route: &crate::RouteResult,
    value: &serde_json::Value,
    items: &mut BTreeMap<String, String>,
    keys: &[&str],
) {
    for key in keys {
        if let Some(child) = value.get(*key) {
            push_matrix_array_items(route, child, items);
        }
    }
}

fn push_matrix_array_items(
    route: &crate::RouteResult,
    value: &serde_json::Value,
    items: &mut BTreeMap<String, String>,
) {
    let Some(array) = value.as_array() else {
        return;
    };
    for item in array {
        if let Some(text) = item.as_str() {
            push_matrix_list_item(route, text, items);
        } else {
            collect_matrix_list_object_fields(route, item, items);
        }
    }
}

fn collect_matrix_list_object_fields(
    route: &crate::RouteResult,
    value: &serde_json::Value,
    items: &mut BTreeMap<String, String>,
) {
    let Some(map) = value.as_object() else {
        return;
    };
    for key in [
        "name",
        "path",
        "resolved_path",
        "table",
        "table_name",
        "identity_value",
    ] {
        if let Some(text) = map.get(key).and_then(serde_json::Value::as_str) {
            push_matrix_list_item(route, text, items);
        }
    }
}

fn push_matrix_list_item(
    route: &crate::RouteResult,
    raw: &str,
    items: &mut BTreeMap<String, String>,
) {
    let Some(display) = matrix_list_display_item(route, raw) else {
        return;
    };
    items.entry(display.to_ascii_lowercase()).or_insert(display);
}

fn matrix_list_display_item(route: &crate::RouteResult, raw: &str) -> Option<String> {
    let item = raw.trim().trim_matches('`').trim();
    if item.is_empty() {
        return None;
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::FileNames {
        return std::path::Path::new(item)
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .or_else(|| Some(item.to_string()));
    }
    Some(item.to_string())
}

fn matrix_table_observed_answer(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::SqliteTableListing {
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

fn matrix_observed_shape_summary(
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

fn replace_delivery_with_matrix_observed_shape_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route_requires_matrix_deterministic_final_answer(route) {
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
        info!(
            "delivery matrix_replace_active_bound_inventory_path task_id={}",
            task.task_id
        );
        return true;
    }
    let Some(shape_class) = matrix_final_answer_shape_class(route) else {
        return false;
    };
    let current_answer = final_answer_text_from_delivery(delivery_messages);
    if !current_answer.trim().is_empty()
        && matrix_candidate_satisfies_final_shape(
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

    let Some((candidate, summary)) = matrix_observed_answer_candidate_for_shape(
        state,
        loop_state,
        agent_run_context,
        shape_class,
    ) else {
        return false;
    };
    if !matrix_candidate_satisfies_final_shape(
        task,
        user_text,
        loop_state,
        agent_run_context,
        Some(summary.clone()),
        route,
        &candidate,
    ) {
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
    true
}

fn deterministic_matrix_observed_shape_answer(
    state: &AppState,
    _task: &ClaimedTask,
    _user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if !route_requires_matrix_deterministic_final_answer(route) {
        return None;
    }
    let shape_class = matrix_final_answer_shape_class(route)?;
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

const EXECUTION_SUMMARY_MAX_STEPS: usize = 4;
const EXECUTION_SUMMARY_ARGS_MAX_CHARS: usize = 180;
const EXECUTION_SUMMARY_OUTPUT_MAX_CHARS: usize = 420;

fn should_attach_execution_summary(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    _user_text: Option<&str>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if route_has_contract_matrix_final_shape(route) {
        return false;
    }
    if route_requires_content_excerpt_evidence(route) {
        return false;
    }
    if route.output_contract.exact_sentence_count.is_some() {
        return false;
    }
    if delivery_matches_grounded_content_answer(loop_state, route, &loop_state.delivery_messages) {
        return false;
    }
    if matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ScalarCount
    ) {
        return false;
    }
    if matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
    ) {
        return false;
    }
    let has_publishable_synthesis = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .is_some_and(|text| !text.is_empty());
    let publishable_synthesis_from_step = latest_publishable_synthesis_step_matches(loop_state);
    if has_publishable_synthesis
        && !publishable_synthesis_from_step
        && route.output_contract.requires_content_evidence
        && matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        )
        && !matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::DirectoryEntryGroups
                | crate::OutputSemanticKind::ScalarPathOnly
                | crate::OutputSemanticKind::ExistenceWithPath
        )
    {
        return false;
    }
    if has_publishable_synthesis
        && crate::agent_engine::observed_output::recent_structured_scalar_observation_count(
            loop_state,
        ) > 1
    {
        return false;
    }
    if deterministic_scalar_markdown_heading_answer_from_loop(loop_state, agent_run_context)
        .is_some()
    {
        return false;
    }
    if route_allows_direct_scalar_observed_answer(route)
        && loop_has_count_inventory_observation(loop_state)
    {
        return false;
    }
    true
}

fn route_requires_content_excerpt_evidence(route: &crate::RouteResult) -> bool {
    crate::task_contract::required_evidence_fields_for_output_contract(&route.output_contract)
        .iter()
        .any(|field| field == "content_excerpt")
}

fn latest_publishable_synthesis_step_matches(loop_state: &LoopState) -> bool {
    let Some(synthesis) = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return false;
    };
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find(|step| step.skill == "synthesize_answer" && step.is_ok())
        .and_then(|step| step.output.as_deref())
        .map(str::trim)
        .is_some_and(|output| output == synthesis)
}

fn loop_has_structured_listing_observation(loop_state: &LoopState) -> bool {
    loop_state.executed_step_results.iter().any(|step| {
        if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
            return false;
        }
        let Some(output) = step.output.as_deref() else {
            return false;
        };
        serde_json::from_str::<serde_json::Value>(output.trim())
            .ok()
            .is_some_and(|value| {
                value.get("names_by_kind").is_some()
                    || value
                        .get("names")
                        .and_then(|value| value.as_array())
                        .is_some_and(|items| !items.is_empty())
                    || matches!(
                        value.get("action").and_then(|value| value.as_str()),
                        Some("inventory_dir" | "list_dir" | "tree_summary")
                    )
            })
    })
}

fn latest_grounded_synthesis_for_mixed_listing_contract(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::DirectoryEntryGroups
        || !latest_publishable_synthesis_step_matches(loop_state)
        || !loop_has_structured_listing_observation(loop_state)
        || !loop_state
            .executed_step_results
            .iter()
            .any(step_output_is_read_range)
    {
        return None;
    }
    let synthesis = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    if crate::finalize::looks_like_planner_artifact(synthesis)
        || crate::finalize::looks_like_internal_trace_artifact(synthesis)
        || crate::finalize::parse_delivery_token(synthesis).is_some()
        || looks_like_structured_machine_output(synthesis)
    {
        return None;
    }

    Some((
        synthesis.to_string(),
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            parsed: true,
            contract_ok: true,
            completion_ok: Some(true),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: 2,
            ..Default::default()
        },
    ))
}

fn loop_has_count_inventory_observation(loop_state: &LoopState) -> bool {
    loop_state.executed_step_results.iter().any(|step| {
        if !step.is_ok() {
            return false;
        }
        step.output
            .as_deref()
            .and_then(|output| serde_json::from_str::<serde_json::Value>(output.trim()).ok())
            .and_then(|value| {
                value
                    .get("action")
                    .and_then(|action| action.as_str())
                    .map(|action| action == "count_inventory")
            })
            .unwrap_or(false)
    })
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
    if !(plan_skill.eq_ignore_ascii_case(step.skill.trim())
        || (step.skill == "run_cmd" && plan_skill.eq_ignore_ascii_case("run_cmd")))
    {
        return false;
    }
    plan_step_action_matches_execution(plan_step, step)
}

fn execution_output_json(step: &crate::executor::StepExecutionResult) -> Option<serde_json::Value> {
    let raw = step.output.as_deref()?.trim();
    if raw.is_empty() {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(raw).ok()
}

fn execution_output_action(step: &crate::executor::StepExecutionResult) -> Option<String> {
    execution_output_json(step)?
        .get("action")?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn plan_step_action_arg(plan_step: &crate::PlanStep) -> Option<&str> {
    plan_step
        .args
        .get("action")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn plan_step_action_matches_execution(
    plan_step: &crate::PlanStep,
    step: &crate::executor::StepExecutionResult,
) -> bool {
    let Some(plan_action) = plan_step_action_arg(plan_step) else {
        return true;
    };
    let Some(output_action) = execution_output_action(step) else {
        return true;
    };
    plan_action.eq_ignore_ascii_case(output_action.trim())
}

fn plan_step_for_execution<'a>(
    loop_state: &'a LoopState,
    step: &crate::executor::StepExecutionResult,
) -> Option<&'a crate::PlanStep> {
    let exact = loop_state
        .round_traces
        .iter()
        .filter_map(|trace| trace.plan_result.as_ref())
        .flat_map(|plan| plan.steps.iter())
        .find(|plan_step| {
            plan_step.step_id == step.step_id && plan_step_matches_execution(plan_step, step)
        });
    if exact.is_some() {
        return exact;
    }

    let output_action = execution_output_action(step)?;
    loop_state
        .round_traces
        .iter()
        .rev()
        .filter_map(|trace| trace.plan_result.as_ref())
        .flat_map(|plan| plan.steps.iter())
        .find(|plan_step| {
            plan_step_matches_execution(plan_step, step)
                && plan_step_action_arg(plan_step)
                    .is_some_and(|action| action.eq_ignore_ascii_case(output_action.trim()))
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
    if delivery_contract_suppresses_execution_summary(
        loop_state,
        agent_run_context,
        delivery_messages,
    ) {
        delivery_messages.retain(|message| !crate::finalize::is_execution_summary_message(message));
        return;
    }
    if delivery_messages.last().is_some_and(|message| {
        observed_markdown_heading_scalar_answer_for_delivery(loop_state, agent_run_context, message)
            .is_some()
    }) {
        return;
    }
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

fn delivery_contract_suppresses_execution_summary(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &[String],
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    let has_publishable_answer = delivery_messages.iter().any(|message| {
        let trimmed = message.trim();
        !trimmed.is_empty() && !crate::finalize::is_execution_summary_message(trimmed)
    });
    if delivery_token_contract_suppresses_execution_summary(route, delivery_messages) {
        return true;
    }
    if has_publishable_answer
        && route.output_contract.requires_content_evidence
        && route.output_contract.semantic_kind != crate::OutputSemanticKind::None
    {
        return true;
    }
    if route_has_contract_matrix_final_shape(route) {
        return true;
    }
    if route_requires_content_excerpt_evidence(route) && has_publishable_answer {
        return true;
    }
    if route.output_contract.response_shape == crate::OutputResponseShape::Strict
        && delivery_messages
            .iter()
            .any(|message| delivery_message_is_json_container(message))
    {
        return true;
    }
    if delivery_matches_latest_structured_scalar_observation(loop_state, route, delivery_messages) {
        return true;
    }
    if delivery_matches_config_guard_answer(loop_state, delivery_messages) {
        return true;
    }
    if delivery_matches_latest_transform_observation(loop_state, delivery_messages) {
        return true;
    }
    if delivery_matches_observed_markdown_heading_delivery(
        loop_state,
        agent_run_context,
        delivery_messages,
    ) {
        return true;
    }
    if delivery_matches_latest_read_range_synthesis(loop_state, route, delivery_messages) {
        return true;
    }
    let has_existing_execution_summary =
        delivery_messages_have_execution_summary(delivery_messages);
    if has_existing_execution_summary
        && delivery_has_synthesized_answer_result(loop_state, route, delivery_messages)
    {
        return true;
    }
    if has_existing_execution_summary
        && delivery_matches_synthesized_content_answer(loop_state, route, delivery_messages)
    {
        return true;
    }
    if delivery_matches_grounded_content_answer(loop_state, route, delivery_messages) {
        return true;
    }
    if route.output_contract.response_shape != crate::OutputResponseShape::Scalar {
        return false;
    }
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::None {
        return false;
    }
    delivery_messages.iter().any(|message| {
        let message = message.trim();
        !message.is_empty() && !crate::finalize::is_execution_summary_message(message)
    })
}

fn delivery_token_contract_suppresses_execution_summary(
    route: &crate::RouteResult,
    delivery_messages: &[String],
) -> bool {
    let delivery_contract = route.wants_file_delivery
        || route.output_contract.delivery_required
        || !matches!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        );
    delivery_contract && delivery_messages_include_delivery_token(delivery_messages)
}

fn delivery_messages_include_delivery_token(delivery_messages: &[String]) -> bool {
    delivery_messages.iter().any(|message| {
        message
            .lines()
            .map(str::trim)
            .any(|line| crate::finalize::parse_delivery_token(line).is_some())
    })
}

fn delivery_messages_have_execution_summary(delivery_messages: &[String]) -> bool {
    delivery_messages
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message))
}

fn single_publishable_delivery_message(delivery_messages: &[String]) -> Option<&str> {
    let mut publishable = delivery_messages
        .iter()
        .map(|message| message.trim())
        .filter(|message| !message.is_empty())
        .filter(|message| !crate::finalize::is_execution_summary_message(message));
    let first = publishable.next()?;
    publishable.next().is_none().then_some(first)
}

fn delivery_matches_observed_markdown_heading_delivery(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &[String],
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route_allows_observed_markdown_heading_scalar_delivery(route)
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::DirectoryEntryGroups
                | crate::OutputSemanticKind::ScalarCount
                | crate::OutputSemanticKind::RawCommandOutput
                | crate::OutputSemanticKind::ScalarPathOnly
                | crate::OutputSemanticKind::ExistenceWithPath
                | crate::OutputSemanticKind::ExistenceWithPathSummary
        )
    {
        return false;
    }
    let Some(delivery_text) = single_publishable_delivery_message(delivery_messages) else {
        return false;
    };
    let Some(delivery_heading) = markdown_heading_from_line(delivery_text) else {
        return false;
    };
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok())
        .filter_map(|step| step.output.as_deref())
        .find(|output| output.contains("\"read_range\"") || output.contains("\"read_text_range\""))
        .and_then(first_markdown_heading_from_read_output)
        .is_some_and(|observed_heading| observed_heading.trim() == delivery_heading.trim())
}

fn delivery_matches_latest_read_range_synthesis(
    loop_state: &LoopState,
    route: &crate::RouteResult,
    delivery_messages: &[String],
) -> bool {
    if !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !latest_publishable_synthesis_step_matches(loop_state)
    {
        return false;
    }
    let Some(delivery_text) = single_publishable_delivery_message(delivery_messages) else {
        return false;
    };
    if !loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .is_some_and(|synthesis| synthesis == delivery_text.trim())
    {
        return false;
    }
    loop_state
        .executed_step_results
        .iter()
        .any(step_output_is_read_range)
}

fn delivery_matches_latest_structured_scalar_observation(
    loop_state: &LoopState,
    route: &crate::RouteResult,
    delivery_messages: &[String],
) -> bool {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::StructuredKeys {
        return false;
    }
    let Some(delivery_text) = single_publishable_delivery_message(delivery_messages) else {
        return false;
    };
    crate::agent_engine::observed_output::latest_structured_scalar_observation_text(loop_state)
        .is_some_and(|observed_text| delivery_text == observed_text.trim())
}

fn delivery_matches_synthesized_content_answer(
    loop_state: &LoopState,
    route: &crate::RouteResult,
    delivery_messages: &[String],
) -> bool {
    if !route.output_contract.requires_content_evidence || route.output_contract.delivery_required {
        return false;
    }
    if !matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
    ) {
        return false;
    }
    if !matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None | crate::OutputSemanticKind::ContentExcerptSummary
    ) {
        return false;
    }
    let Some(delivery_text) = single_publishable_delivery_message(delivery_messages) else {
        return false;
    };
    if crate::agent_engine::observed_output::answer_is_direct_observation_passthrough(
        delivery_text,
        loop_state,
    ) {
        return false;
    }
    loop_state.executed_step_results.iter().any(|step| {
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
}

fn delivery_matches_grounded_content_answer(
    loop_state: &LoopState,
    route: &crate::RouteResult,
    delivery_messages: &[String],
) -> bool {
    if !route.output_contract.requires_content_evidence || route.output_contract.delivery_required {
        return false;
    }
    if !matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
    ) {
        return false;
    }
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    ) || matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
    ) {
        return false;
    }
    if route_requires_matrix_deterministic_final_answer(route) {
        return false;
    }
    if latest_publishable_synthesis_step_matches(loop_state) {
        return false;
    }
    let Some(delivery_text) = single_publishable_delivery_message(delivery_messages) else {
        return false;
    };
    let delivery_text = delivery_text.trim();
    if delivery_text.is_empty()
        || crate::agent_engine::observed_output::answer_is_direct_observation_passthrough(
            delivery_text,
            loop_state,
        )
        || crate::finalize::looks_like_planner_artifact(delivery_text)
        || crate::finalize::looks_like_internal_trace_artifact(delivery_text)
        || looks_like_structured_machine_output(delivery_text)
        || looks_like_raw_command_snapshot(delivery_text)
        || message_is_non_answer_separator(delivery_text)
    {
        return false;
    }
    loop_state.executed_step_results.iter().any(|step| {
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
}

fn delivery_has_synthesized_answer_result(
    loop_state: &LoopState,
    route: &crate::RouteResult,
    delivery_messages: &[String],
) -> bool {
    if !route.output_contract.requires_content_evidence || route.output_contract.delivery_required {
        return false;
    }
    let Some(delivery_text) = single_publishable_delivery_message(delivery_messages) else {
        return false;
    };
    if crate::agent_engine::observed_output::answer_is_direct_observation_passthrough(
        delivery_text,
        loop_state,
    ) {
        return false;
    }
    loop_state.executed_step_results.iter().any(|step| {
        step.is_ok()
            && step.skill == "synthesize_answer"
            && step
                .output
                .as_deref()
                .map(str::trim)
                .is_some_and(|output| !output.is_empty())
    })
}

fn delivery_matches_latest_transform_observation(
    loop_state: &LoopState,
    delivery_messages: &[String],
) -> bool {
    let Some(delivery_text) = single_publishable_delivery_message(delivery_messages) else {
        return false;
    };
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && step.skill == "transform")
        .filter_map(|step| step.output.as_deref())
        .any(|output| {
            crate::agent_engine::observed_output::transform_skill_formatted_output_candidate(output)
                .is_some_and(|answer| answer.trim() == delivery_text)
        })
}

fn delivery_matches_config_guard_answer(
    loop_state: &LoopState,
    delivery_messages: &[String],
) -> bool {
    let Some(delivery_text) = single_publishable_delivery_message(delivery_messages) else {
        return false;
    };
    let outputs = config_edit_observed_outputs(loop_state);
    if outputs.is_empty() {
        return false;
    }
    [true, false].into_iter().any(|prefer_english| {
        direct_config_edit_guard_answer(&outputs, prefer_english)
            .is_some_and(|answer| answer.trim() == delivery_text)
    })
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
    let prefer_english =
        prefer_english_for_agent_contextual_user_text(state, user_text, agent_run_context);
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
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    failed_step: &crate::executor::StepExecutionResult,
    error: &str,
    permission_denied: bool,
) -> String {
    let target =
        content_evidence_failed_step_target_label(loop_state, agent_run_context, failed_step);
    let prefer_english = prefer_english_for_final_reply(state, task, user_text, agent_run_context);
    let answer = match (prefer_english, target.as_deref()) {
        (true, Some(target)) => {
            format!("Tried to access `{target}`, but execution failed: {error}.")
        }
        (true, None) => format!("The `{}` step failed: {error}.", failed_step.skill.trim()),
        (false, Some(target)) => {
            format!("已尝试访问 `{target}`，但执行失败：{error}。")
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

fn content_evidence_failed_step_target_label(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    failed_step: &crate::executor::StepExecutionResult,
) -> Option<String> {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.output_contract.locator_hint.trim())
        .filter(|locator| !locator.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            plan_step_for_execution(loop_state, failed_step)
                .and_then(|plan_step| structured_target_label_from_args(&plan_step.args))
        })
        .or_else(|| structured_target_label_from_step_error(failed_step))
}

fn structured_target_label_from_step_error(
    failed_step: &crate::executor::StepExecutionResult,
) -> Option<String> {
    let error = failed_step.error.as_deref()?.trim();
    let structured = crate::skills::parse_structured_skill_error(error)?;
    structured
        .extra
        .as_ref()
        .and_then(structured_target_label_from_args)
        .or(structured.service_name)
}

fn structured_target_label_from_args(args: &serde_json::Value) -> Option<String> {
    let object = args.as_object()?;
    for key in [
        "path",
        "resolved_path",
        "file_path",
        "target_path",
        "dir",
        "directory",
        "root",
        "service_name",
        "unit",
        "target",
        "name",
    ] {
        if execution_summary_arg_is_sensitive(key) {
            continue;
        }
        if let Some(label) = object
            .get(key)
            .and_then(structured_target_label_from_value)
            .map(|value| truncate_with_ellipsis(&value, 180))
        {
            return Some(label);
        }
    }
    None
}

fn structured_target_label_from_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        }
        serde_json::Value::Array(items) => {
            let labels = items
                .iter()
                .filter_map(structured_target_label_from_value)
                .take(3)
                .collect::<Vec<_>>();
            (!labels.is_empty()).then(|| labels.join(", "))
        }
        serde_json::Value::Object(_) => structured_target_label_from_args(value),
        _ => None,
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

fn db_basic_failure_direct_answer(
    state: &AppState,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    failed_step: &crate::executor::StepExecutionResult,
    raw_error: &str,
    normalized_error: &str,
) -> Option<String> {
    let structured = crate::skills::parse_structured_skill_error(raw_error)?;
    let effective_skill = if structured.skill.trim().is_empty() {
        failed_step.skill.as_str()
    } else {
        structured.skill.as_str()
    };
    if !effective_skill.eq_ignore_ascii_case("db_basic") {
        return None;
    }
    if !matches!(
        structured.error_kind.as_str(),
        "sqlite_open_failed"
            | "sqlite_query_failed"
            | "sqlite_execute_failed"
            | "unsafe_sql"
            | "confirmation_required"
            | "invalid_input"
            | "unsupported_action"
    ) {
        return None;
    }
    let target =
        content_evidence_failed_step_target_label(loop_state, agent_run_context, failed_step);
    let prefer_english = prefer_english_for_user_text(state, user_text);
    Some(match (prefer_english, target) {
        (true, Some(target)) => {
            format!("The database request for `{target}` failed: {normalized_error}.")
        }
        (true, None) => format!("The database request failed: {normalized_error}."),
        (false, Some(target)) => {
            format!("数据库请求 `{target}` 执行失败：{normalized_error}。")
        }
        (false, None) => format!("数据库请求执行失败：{normalized_error}。"),
    })
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
    _task: &ClaimedTask,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    error: &str,
) -> String {
    let target = missing_content_target_label(agent_run_context, error);
    if prefer_english_for_user_text(state, user_text) {
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
    let user_visible_error = if crate::skills::parse_structured_skill_error(raw_error).is_some()
        || recoverable_skill_error
        || observable_run_cmd_error
    {
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
                    task,
                    user_text,
                    loop_state,
                    agent_run_context,
                    failed_step,
                    error,
                    permission_denied,
                )
            })
    } else {
        content_evidence_step_failure_default_answer(
            state,
            task,
            user_text,
            loop_state,
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
    if let Some(answer) = db_basic_failure_direct_answer(
        state,
        user_text,
        loop_state,
        agent_run_context,
        failed_step,
        raw_error,
        error,
    ) {
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
    if agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(route_prefers_observed_answer)
        && direct_scalar_observed_answer(Some(state), loop_state, agent_run_context).is_some()
    {
        return None;
    }
    let (error_answer, summary) =
        content_evidence_step_failure_answer(state, task, user_text, loop_state, agent_run_context)
            .await?;
    let mut delivery_messages = if content_evidence_failure_suppresses_execution_summary(loop_state)
    {
        Vec::new()
    } else {
        build_execution_summary_messages(loop_state, agent_run_context, Some(user_text))
    };
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

fn content_evidence_failure_suppresses_execution_summary(loop_state: &LoopState) -> bool {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find(|step| {
            !step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think"
                )
        })
        .and_then(|step| {
            step.error
                .as_deref()
                .map(str::trim)
                .filter(|error| !error.is_empty())
                .map(|error| {
                    error_looks_like_os_permission_denied(error)
                        || error_looks_like_missing_file_or_directory(error)
                        || crate::skills::is_observable_run_cmd_error(&step.skill, error)
                })
        })
        .unwrap_or(false)
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

fn successful_content_observation_should_precede_status_summary(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &crate::agent_engine::LoopState,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route.output_contract.requires_content_evidence {
        return false;
    }
    if matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ExecutionFailedStep
            | crate::OutputSemanticKind::RawCommandOutput
            | crate::OutputSemanticKind::ServiceStatus
    ) {
        return false;
    }
    loop_state.executed_step_results.iter().any(|step| {
        step.is_ok()
            && !matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer"
            )
            && step
                .output
                .as_deref()
                .map(str::trim)
                .is_some_and(|text| !text.is_empty())
    })
}

fn delivery_is_content_answer_candidate(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &crate::agent_engine::LoopState,
    delivery_messages: &[String],
) -> bool {
    if !successful_content_observation_should_precede_status_summary(agent_run_context, loop_state)
    {
        return false;
    }
    let Some(delivery) = delivery_messages.last().map(String::as_str).map(str::trim) else {
        return false;
    };
    if delivery.is_empty()
        || crate::finalize::is_execution_summary_message(delivery)
        || crate::finalize::looks_like_planner_artifact(delivery)
        || crate::finalize::looks_like_internal_trace_artifact(delivery)
        || crate::agent_engine::observed_output::answer_is_direct_observation_passthrough(
            delivery, loop_state,
        )
    {
        return false;
    }
    planned_delivery_is_publishable_model_language_answer(delivery)
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
    let prefer_english =
        deterministic_template_language_preference(state, user_text, agent_run_context)?;
    let scalar_count = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            route.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarCount
        });
    let concise_existence = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            route.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
                && !route.output_contract.delivery_required
                && matches!(
                    route.output_contract.response_shape,
                    crate::OutputResponseShape::Scalar | crate::OutputResponseShape::OneSentence
                )
        });
    if prefer_english {
        if scalar_count {
            Some(format!(
                "`{path}` does not exist, so the matching item count cannot be computed."
            ))
        } else if concise_existence {
            Some("not found".to_string())
        } else {
            Some(format!(
                "I could not find `{path}`, so this request cannot be completed until the path is corrected."
            ))
        }
    } else if scalar_count {
        Some(format!("`{path}` 不存在，无法统计匹配项数量。"))
    } else if concise_existence {
        Some("不存在".to_string())
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

#[derive(Debug)]
struct ConfigEditObservedOutput {
    index: usize,
    value: serde_json::Value,
}

fn config_edit_output_action(value: &serde_json::Value) -> Option<&str> {
    value
        .get("action")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn config_edit_observable_action(action: &str) -> bool {
    matches!(
        action,
        "plan_config_change"
            | "apply_config_change"
            | "validate_config"
            | "guard_config"
            | "read_back"
            | "restart_if_requested"
    )
}

fn step_may_contain_config_edit_observation(step: &crate::executor::StepExecutionResult) -> bool {
    matches!(
        step.skill.as_str(),
        "config_edit" | "config_basic" | "config_guard"
    )
}

fn config_edit_observed_outputs(
    loop_state: &crate::agent_engine::LoopState,
) -> Vec<ConfigEditObservedOutput> {
    let latest_config_edit_step = loop_state
        .executed_step_results
        .iter()
        .rfind(|step| step.skill == "config_edit");
    if latest_config_edit_step.is_some_and(|step| !step.is_ok()) {
        return Vec::new();
    }
    loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .filter_map(|(index, step)| {
            if !step.is_ok() || !step_may_contain_config_edit_observation(step) {
                return None;
            }
            let value =
                serde_json::from_str::<serde_json::Value>(step.output.as_deref()?.trim()).ok()?;
            if !config_edit_output_action(&value).is_some_and(config_edit_observable_action) {
                return None;
            }
            Some(ConfigEditObservedOutput { index, value })
        })
        .collect()
}

fn config_edit_string_field<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn config_edit_path_label(value: &serde_json::Value) -> &str {
    config_edit_string_field(value, "path")
        .or_else(|| config_edit_string_field(value, "resolved_path"))
        .unwrap_or("config")
}

fn config_edit_field_label(value: &serde_json::Value) -> &str {
    config_edit_string_field(value, "field_path").unwrap_or("field")
}

fn config_edit_value_label(value: &serde_json::Value, primary_key: &str) -> Option<String> {
    config_edit_string_field(value, "value_text")
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .get(primary_key)
                .map(execution_summary_value_to_string)
                .map(|text| text.trim().to_string())
                .filter(|text| !text.is_empty())
        })
}

fn config_edit_output_matches_field(
    value: &serde_json::Value,
    field_path: &str,
    path: &str,
) -> bool {
    config_edit_string_field(value, "field_path") == Some(field_path)
        && config_edit_string_field(value, "path")
            .or_else(|| config_edit_string_field(value, "resolved_path"))
            .is_none_or(|candidate| candidate == path)
}

fn config_edit_summary() -> crate::task_journal::TaskJournalFinalizerSummary {
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
    }
}

fn direct_config_edit_apply_answer(
    outputs: &[ConfigEditObservedOutput],
    prefer_english: bool,
) -> Option<String> {
    let applied = outputs.iter().rev().find(|item| {
        config_edit_output_action(&item.value) == Some("apply_config_change")
            && item.value.get("applied").and_then(|value| value.as_bool()) == Some(true)
    })?;
    let field_path = config_edit_field_label(&applied.value);
    let path = config_edit_path_label(&applied.value);
    let read_back = outputs.iter().rev().find(|item| {
        item.index > applied.index
            && config_edit_output_action(&item.value) == Some("read_back")
            && item.value.get("exists").and_then(|value| value.as_bool()) == Some(true)
            && config_edit_output_matches_field(&item.value, field_path, path)
    });
    let value_label = read_back
        .and_then(|item| config_edit_value_label(&item.value, "value"))
        .or_else(|| config_edit_value_label(&applied.value, "new_value"));
    let validation_after_apply = outputs.iter().rev().find(|item| {
        item.index > applied.index
            && config_edit_output_action(&item.value) == Some("validate_config")
            && config_edit_path_label(&item.value) == path
    });
    let validation_passed = validation_after_apply
        .and_then(|item| item.value.get("valid").and_then(|value| value.as_bool()))
        .or_else(|| {
            applied
                .value
                .get("validated")
                .and_then(|value| value.as_bool())
        })
        .unwrap_or(false);
    let guard = outputs.iter().rev().find(|item| {
        item.index > applied.index
            && config_edit_output_action(&item.value) == Some("guard_config")
            && config_edit_path_label(&item.value) == path
    });
    let risk_count = guard.and_then(|item| item.value.get("risk_count").and_then(|v| v.as_u64()));

    let mut parts = Vec::new();
    match (prefer_english, value_label) {
        (true, Some(value)) => parts.push(format!(
            "Config updated: `{field_path}` = `{value}` in `{path}`."
        )),
        (true, None) => parts.push(format!("Config updated: `{field_path}` in `{path}`.")),
        (false, Some(value)) => parts.push(format!(
            "配置已更新：`{field_path}` = `{value}`（`{path}`）。"
        )),
        (false, None) => parts.push(format!("配置已更新：`{field_path}`（`{path}`）。")),
    }
    if validation_passed {
        parts.push(if prefer_english {
            "Validation passed.".to_string()
        } else {
            "验证通过。".to_string()
        });
    }
    if let Some(risk_count) = risk_count {
        parts.push(if prefer_english {
            if risk_count == 0 {
                "Guard check found no risks.".to_string()
            } else {
                format!("Guard check found {risk_count} risk(s).")
            }
        } else if risk_count == 0 {
            "安全检查未发现风险。".to_string()
        } else {
            format!("安全检查发现 {risk_count} 个风险。")
        });
    }
    Some(parts.join(if prefer_english { " " } else { "" }))
}

fn direct_config_edit_plan_answer(
    outputs: &[ConfigEditObservedOutput],
    prefer_english: bool,
) -> Option<String> {
    let planned = outputs.iter().rev().find(|item| {
        config_edit_output_action(&item.value) == Some("plan_config_change")
            && !outputs.iter().any(|candidate| {
                candidate.index > item.index
                    && config_edit_output_action(&candidate.value) == Some("apply_config_change")
            })
    })?;
    let field_path = config_edit_field_label(&planned.value);
    let path = config_edit_path_label(&planned.value);
    let value = config_edit_value_label(&planned.value, "new_value");
    let would_change = planned
        .value
        .get("would_change")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    Some(match (prefer_english, value, would_change) {
        (true, Some(value), true) => format!(
            "Config change planned only: `{field_path}` would be set to `{value}` in `{path}`. No config file was written."
        ),
        (true, Some(value), false) => format!(
            "Config change planned only: `{field_path}` is already `{value}` in `{path}`. No config file was written."
        ),
        (true, None, _) => {
            format!("Config change planned only for `{field_path}` in `{path}`. No config file was written.")
        }
        (false, Some(value), true) => format!(
            "已生成配置变更计划：`{field_path}` 将设置为 `{value}`（`{path}`）。尚未写入配置。"
        ),
        (false, Some(value), false) => format!(
            "已生成配置变更计划：`{field_path}` 当前已经是 `{value}`（`{path}`）。尚未写入配置。"
        ),
        (false, None, _) => {
            format!("已生成配置变更计划：`{field_path}`（`{path}`）。尚未写入配置。")
        }
    })
}

fn direct_config_edit_validate_answer(
    outputs: &[ConfigEditObservedOutput],
    prefer_english: bool,
) -> Option<String> {
    let validation = outputs
        .iter()
        .rev()
        .find(|item| config_edit_output_action(&item.value) == Some("validate_config"))?;
    let path = config_edit_path_label(&validation.value);
    let valid = validation.value.get("valid")?.as_bool()?;
    if valid {
        return Some(if prefer_english {
            format!("Config validation passed for `{path}`.")
        } else {
            format!("配置验证通过：`{path}`。")
        });
    }
    let reason = config_edit_string_field(&validation.value, "error_text").unwrap_or("invalid");
    Some(if prefer_english {
        format!("Config validation failed for `{path}`: {reason}.")
    } else {
        format!("配置验证未通过：`{path}`，原因：{reason}。")
    })
}

fn config_edit_risk_labels(value: &serde_json::Value) -> Vec<String> {
    value
        .get("risks")
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
        .unwrap_or_default()
}

fn direct_config_edit_guard_answer(
    outputs: &[ConfigEditObservedOutput],
    prefer_english: bool,
) -> Option<String> {
    let guard = outputs
        .iter()
        .rev()
        .find(|item| config_edit_output_action(&item.value) == Some("guard_config"))?;
    let path = config_edit_path_label(&guard.value);
    let risk_count = guard
        .value
        .get("risk_count")
        .and_then(|value| value.as_u64())
        .unwrap_or_else(|| config_edit_risk_labels(&guard.value).len() as u64);
    if risk_count == 0 {
        return Some(if prefer_english {
            format!("No obvious config risks found in `{path}`.")
        } else {
            format!("`{path}` 未发现明显配置风险。")
        });
    }
    let risks = config_edit_risk_labels(&guard.value);
    let risk_text = if risks.is_empty() {
        if prefer_english {
            format!("{risk_count} risk(s)")
        } else {
            format!("{risk_count} 个风险")
        }
    } else {
        risks.join(if prefer_english { "; " } else { "；" })
    };
    Some(if prefer_english {
        format!("Found {risk_count} config risk(s) in `{path}`: {risk_text}.")
    } else {
        format!("`{path}` 发现 {risk_count} 个配置风险：{risk_text}。")
    })
}

fn direct_config_edit_read_back_answer(
    outputs: &[ConfigEditObservedOutput],
    prefer_english: bool,
) -> Option<String> {
    let read_back = outputs
        .iter()
        .rev()
        .find(|item| config_edit_output_action(&item.value) == Some("read_back"))?;
    let field_path = config_edit_field_label(&read_back.value);
    let path = config_edit_path_label(&read_back.value);
    let exists = read_back
        .value
        .get("exists")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !exists {
        return Some(if prefer_english {
            format!("`{field_path}` was not found in `{path}`.")
        } else {
            format!("`{path}` 中未找到 `{field_path}`。")
        });
    }
    let value = config_edit_value_label(&read_back.value, "value").unwrap_or_default();
    Some(if prefer_english {
        format!("`{field_path}` in `{path}` is `{value}`.")
    } else {
        format!("`{path}` 中 `{field_path}` 的当前值是 `{value}`。")
    })
}

pub(crate) fn direct_config_edit_observed_answer(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let outputs = config_edit_observed_outputs(loop_state);
    if outputs.is_empty() {
        return None;
    }
    let request_language = crate::language_policy::request_language_hint(user_text);
    let prefer_english = request_language == "en"
        || (request_language == "config_default" && prefer_english_for_user_text(state, user_text));
    let answer = direct_config_edit_apply_answer(&outputs, prefer_english)
        .or_else(|| direct_config_edit_plan_answer(&outputs, prefer_english))
        .or_else(|| direct_config_edit_guard_answer(&outputs, prefer_english))
        .or_else(|| direct_config_edit_validate_answer(&outputs, prefer_english))
        .or_else(|| direct_config_edit_read_back_answer(&outputs, prefer_english))?;
    Some((answer, config_edit_summary()))
}

fn direct_config_guard_observed_answer(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let outputs = config_edit_observed_outputs(loop_state);
    if outputs.is_empty() {
        return None;
    }
    let prefer_english = prefer_english_for_user_text(state, user_text);
    direct_config_edit_guard_answer(&outputs, prefer_english).map(|answer| {
        (
            answer,
            deterministic_observed_execution_status_summary(loop_state),
        )
    })
}

#[derive(Debug)]
struct RustClawConfigFieldObservation {
    path: String,
    field_path: String,
    exists: bool,
    value: serde_json::Value,
    value_text: Option<String>,
}

fn path_is_rustclaw_main_config(path: &str) -> bool {
    let components = Path::new(path)
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>();
    matches!(components.as_slice(), [.., "configs", "config.toml"])
}

fn rustclaw_config_path_label(path: &str) -> String {
    if path_is_rustclaw_main_config(path) {
        "configs/config.toml".to_string()
    } else {
        path.to_string()
    }
}

fn config_output_path(value: &serde_json::Value) -> Option<String> {
    value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
}

fn observed_config_field_path(value: &serde_json::Value) -> Option<String> {
    value
        .get("resolved_field_path")
        .or_else(|| value.get("field_path"))
        .or_else(|| value.get("field"))
        .or_else(|| value.get("key"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
}

fn rustclaw_config_field_observation_from_value(
    path: &str,
    value: &serde_json::Value,
) -> Option<RustClawConfigFieldObservation> {
    let field_path = observed_config_field_path(value)?;
    let field_value = value
        .get("value")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let exists = value
        .get("exists")
        .and_then(|value| value.as_bool())
        .unwrap_or(!field_value.is_null());
    let value_text = value
        .get("value_text")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string);
    Some(RustClawConfigFieldObservation {
        path: path.to_string(),
        field_path,
        exists,
        value: field_value,
        value_text,
    })
}

fn rustclaw_config_field_observations_from_output(
    value: &serde_json::Value,
) -> Vec<RustClawConfigFieldObservation> {
    let Some(action) = value.get("action").and_then(|value| value.as_str()) else {
        return Vec::new();
    };
    if !matches!(
        action,
        "extract_field" | "extract_fields" | "read_field" | "read_fields"
    ) {
        return Vec::new();
    }
    let Some(path) = config_output_path(value).filter(|path| path_is_rustclaw_main_config(path))
    else {
        return Vec::new();
    };
    if let Some(results) = value.get("results").and_then(|value| value.as_array()) {
        return results
            .iter()
            .filter_map(|item| rustclaw_config_field_observation_from_value(&path, item))
            .collect();
    }
    rustclaw_config_field_observation_from_value(&path, value)
        .into_iter()
        .collect()
}

fn rustclaw_config_field_observations(
    loop_state: &crate::agent_engine::LoopState,
) -> Vec<RustClawConfigFieldObservation> {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            step.is_ok() && matches!(step.skill.as_str(), "config_basic" | "system_basic")
        })
        .filter_map(|step| step.output.as_deref())
        .filter_map(|output| serde_json::from_str::<serde_json::Value>(output.trim()).ok())
        .flat_map(|value| rustclaw_config_field_observations_from_output(&value))
        .collect()
}

fn observed_field_value_text(observation: &RustClawConfigFieldObservation) -> Option<String> {
    observation.value_text.clone().or_else(|| {
        if observation.value.is_string() {
            observation.value.as_str().map(ToString::to_string)
        } else if observation.value.is_null() {
            None
        } else {
            Some(execution_summary_value_to_string(&observation.value))
        }
    })
}

fn observed_field_is_true(observation: &RustClawConfigFieldObservation) -> bool {
    observation.value.as_bool() == Some(true)
        || observed_field_value_text(observation)
            .is_some_and(|text| text.trim().eq_ignore_ascii_case("true") || text.trim() == "1")
}

fn observed_field_i64(observation: &RustClawConfigFieldObservation) -> Option<i64> {
    observation.value.as_i64().or_else(|| {
        observed_field_value_text(observation)?
            .trim()
            .parse::<i64>()
            .ok()
    })
}

fn observed_tools_allow_contains_wildcard(observation: &RustClawConfigFieldObservation) -> bool {
    if observation
        .value
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some("*")))
    {
        return true;
    }
    observed_field_value_text(observation).is_some_and(|text| {
        text.split(',')
            .map(|part| part.trim().trim_matches('"').trim_matches('\''))
            .any(|part| part == "*" || part == "[*]")
    })
}

fn observed_server_listen_is_public(observation: &RustClawConfigFieldObservation) -> bool {
    observed_field_value_text(observation)
        .map(|text| text.trim().trim_matches('"').to_string())
        .is_some_and(|text| text == "0.0.0.0" || text.starts_with("0.0.0.0:"))
}

fn quoted_string_label(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("\"{value}\""))
}

fn rustclaw_config_known_risk_field(field_path: &str) -> bool {
    [
        "tools.allow",
        "tools.allow_sudo",
        "tools.allow_path_outside_workspace",
        "telegram.sendfile.full_access",
        "server.listen",
        "self_extension.enabled",
        "worker.task_timeout_seconds",
    ]
    .iter()
    .any(|candidate| field_path.eq_ignore_ascii_case(candidate))
}

fn rustclaw_config_risk_label(observation: &RustClawConfigFieldObservation) -> Option<String> {
    if !observation.exists {
        return None;
    }
    let field_path = observation.field_path.trim();
    if field_path.eq_ignore_ascii_case("tools.allow") {
        return observed_tools_allow_contains_wildcard(observation)
            .then(|| "tools.allow=[\"*\"]".to_string());
    }
    if field_path.eq_ignore_ascii_case("tools.allow_sudo") {
        return observed_field_is_true(observation).then(|| "tools.allow_sudo=true".to_string());
    }
    if field_path.eq_ignore_ascii_case("tools.allow_path_outside_workspace") {
        return observed_field_is_true(observation)
            .then(|| "tools.allow_path_outside_workspace=true".to_string());
    }
    if field_path.eq_ignore_ascii_case("telegram.sendfile.full_access") {
        return observed_field_is_true(observation)
            .then(|| "telegram.sendfile.full_access=true".to_string());
    }
    if field_path.eq_ignore_ascii_case("server.listen") {
        return observed_server_listen_is_public(observation).then(|| {
            let value = observed_field_value_text(observation).unwrap_or_default();
            format!(
                "server.listen={}",
                quoted_string_label(value.trim().trim_matches('"'))
            )
        });
    }
    if field_path.eq_ignore_ascii_case("self_extension.enabled") {
        return observed_field_is_true(observation)
            .then(|| "self_extension.enabled=true".to_string());
    }
    if field_path.eq_ignore_ascii_case("worker.task_timeout_seconds") {
        let value = observed_field_i64(observation)?;
        return (value > 3600).then(|| format!("worker.task_timeout_seconds={value}"));
    }
    None
}

fn direct_rustclaw_config_field_risk_answer(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let observations = rustclaw_config_field_observations(loop_state);
    let mut known_fields = Vec::new();
    let mut risks = Vec::new();
    for observation in &observations {
        if !rustclaw_config_known_risk_field(&observation.field_path) {
            continue;
        }
        if !known_fields
            .iter()
            .any(|field: &String| field.eq_ignore_ascii_case(&observation.field_path))
        {
            known_fields.push(observation.field_path.clone());
        }
        if let Some(label) = rustclaw_config_risk_label(observation) {
            if !risks.iter().any(|existing| existing == &label) {
                risks.push(label);
            }
        }
    }
    if known_fields.len() < 2 {
        return None;
    }
    let path = observations
        .iter()
        .find(|observation| rustclaw_config_known_risk_field(&observation.field_path))
        .map(|observation| rustclaw_config_path_label(&observation.path))
        .unwrap_or_else(|| "configs/config.toml".to_string());
    let prefer_english = prefer_english_for_user_text(state, user_text);
    let answer = if risks.is_empty() {
        if prefer_english {
            format!("No obvious config risks found in `{path}`.")
        } else {
            format!("`{path}` 未发现明显配置风险。")
        }
    } else if prefer_english {
        format!(
            "Found {} config risk(s) in `{path}`: {}.",
            risks.len(),
            risks.join("; ")
        )
    } else {
        format!(
            "`{path}` 发现 {} 个配置风险：{}。",
            risks.len(),
            risks.join("；")
        )
    };
    Some((
        answer,
        deterministic_observed_execution_status_summary(loop_state),
    ))
}

fn direct_rustclaw_config_risk_answer(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    direct_config_guard_observed_answer(state, user_text, loop_state)
        .or_else(|| direct_rustclaw_config_field_risk_answer(state, user_text, loop_state))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SizeComparisonAnswerStyle {
    DeltaOnly,
    ExplainRatio,
}

fn size_comparison_answer_style(
    route: &crate::RouteResult,
    user_text: &str,
) -> SizeComparisonAnswerStyle {
    if crate::intent_router::contract_test_hint_value(user_text, "selector_answer_style")
        .as_deref()
        .is_some_and(|value| {
            matches!(
                value.trim(),
                "larger_with_sizes" | "comparison_with_sizes" | "explain_ratio"
            )
        })
    {
        return SizeComparisonAnswerStyle::ExplainRatio;
    }
    if crate::intent_router::contract_test_hint_value(user_text, "selector_answer_style")
        .as_deref()
        .is_some_and(|value| matches!(value.trim(), "delta_only" | "size_delta"))
    {
        return SizeComparisonAnswerStyle::DeltaOnly;
    }
    let _ = route;
    SizeComparisonAnswerStyle::ExplainRatio
}

fn compare_paths_size_ratio_answer_with_style(
    body: &str,
    prefer_english: bool,
    style: SizeComparisonAnswerStyle,
) -> Option<String> {
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
    if style == SizeComparisonAnswerStyle::DeltaOnly {
        if left_size == right_size {
            return Some(if prefer_english {
                format!("{left_label} and {right_label}: 0 bytes")
            } else {
                format!("{left_label} 和 {right_label}：0 字节")
            });
        }
        let (larger_label, delta) = if left_size > right_size {
            (left_label, left_size - right_size)
        } else {
            (right_label, right_size - left_size)
        };
        return Some(if prefer_english {
            format!("{larger_label}: {delta} bytes")
        } else {
            format!("{larger_label}：{delta} 字节")
        });
    }
    if left_size == right_size {
        return Some(if prefer_english {
            format!("They are the same size: {left_label} and {right_label} are both {left_size} bytes.")
        } else {
            format!("{left_label} 和 {right_label} 一样大，都是 {left_size} 字节。")
        });
    }
    let (larger_label, larger_size, smaller_label, smaller_size) = if left_size > right_size {
        (left_label, left_size, right_label, right_size)
    } else {
        (right_label, right_size, left_label, left_size)
    };
    let ratio = (smaller_size > 0).then(|| larger_size as f64 / smaller_size as f64);
    Some(match (prefer_english, ratio) {
        (true, Some(ratio)) => format!(
            "`{larger_label}` is larger: {larger_size} bytes, about {ratio:.2}x `{smaller_label}` ({smaller_size} bytes)."
        ),
        (true, None) => format!(
            "`{larger_label}` is larger: {larger_size} bytes; `{smaller_label}` is 0 bytes."
        ),
        (false, Some(ratio)) => format!(
            "`{larger_label}` 更大：{larger_size} 字节，大约是 `{smaller_label}`（{smaller_size} 字节）的 {ratio:.2} 倍。"
        ),
        (false, None) => format!(
            "`{larger_label}` 更大：{larger_size} 字节；`{smaller_label}` 为 0 字节。"
        )
    })
}

#[cfg(test)]
fn compare_paths_size_ratio_answer(body: &str, prefer_english: bool) -> Option<String> {
    compare_paths_size_ratio_answer_with_style(
        body,
        prefer_english,
        SizeComparisonAnswerStyle::ExplainRatio,
    )
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

fn compare_paths_size_facts(value: &serde_json::Value) -> Option<Vec<PathSizeFact>> {
    if value.get("action").and_then(|value| value.as_str()) != Some("compare_paths") {
        return None;
    }
    let left = value.get("left")?;
    let right = value.get("right")?;
    let left_size = left.get("size_bytes").and_then(|value| value.as_u64())?;
    let right_size = right.get("size_bytes").and_then(|value| value.as_u64())?;
    Some(vec![
        PathSizeFact {
            label: path_display_label(left, "left"),
            size_bytes: left_size,
        },
        PathSizeFact {
            label: path_display_label(right, "right"),
            size_bytes: right_size,
        },
    ])
}

fn observed_quantity_size_facts(loop_state: &LoopState) -> Vec<PathSizeFact> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "system_basic" | "fs_basic"))
        .filter_map(|step| step.output.as_deref())
        .find_map(|output| {
            let value = serde_json::from_str::<serde_json::Value>(output).ok()?;
            path_batch_size_facts(&value).or_else(|| compare_paths_size_facts(&value))
        })
        .unwrap_or_default()
}

fn latest_delivery_preserves_observed_quantity_size_facts(
    loop_state: &LoopState,
) -> Option<String> {
    let answer = loop_state
        .delivery_messages
        .iter()
        .rev()
        .find(|message| !crate::finalize::is_execution_summary_message(message))?
        .trim();
    if answer.is_empty()
        || crate::finalize::parse_delivery_token(answer).is_some()
        || crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
        || crate::finalize::is_execution_summary_message(answer)
        || looks_like_structured_machine_output(answer)
    {
        return None;
    }
    let facts = observed_quantity_size_facts(loop_state);
    if facts.len() < 2 {
        return None;
    }
    let matched = facts
        .iter()
        .filter(|fact| {
            answer.contains(&fact.label) && answer.contains(&fact.size_bytes.to_string())
        })
        .count();
    (matched >= 2).then(|| answer.to_string())
}

fn path_batch_size_comparison_answer_with_style(
    body: &str,
    prefer_english: bool,
    style: SizeComparisonAnswerStyle,
) -> Option<String> {
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
            if style == SizeComparisonAnswerStyle::DeltaOnly {
                format!("{tied}: 0 bytes")
            } else {
                format!(
                    "They are the same size: {tied} are all {} bytes.",
                    largest.size_bytes
                )
            }
        } else {
            if style == SizeComparisonAnswerStyle::DeltaOnly {
                format!("{tied}：0 字节")
            } else {
                format!("它们一样大：{tied} 都是 {} 字节。", largest.size_bytes)
            }
        });
    }
    if style == SizeComparisonAnswerStyle::DeltaOnly {
        let delta = largest.size_bytes.saturating_sub(runner_up.size_bytes);
        return Some(if prefer_english {
            format!("{}: {} bytes", largest.label, delta)
        } else {
            format!("{}：{} 字节", largest.label, delta)
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

fn compact_binary_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let bytes_f = bytes as f64;
    if bytes_f >= GIB {
        format!("{:.1} GiB", bytes_f / GIB)
    } else if bytes_f >= MIB {
        format!("{:.1} MiB", bytes_f / MIB)
    } else if bytes_f >= KIB {
        format!("{:.1} KiB", bytes_f / KIB)
    } else {
        format!("{bytes} bytes")
    }
}

fn count_inventory_size_answer_with_shape(
    body: &str,
    _prefer_english: bool,
    response_shape: crate::OutputResponseShape,
) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(|value| value.as_str()) != Some("count_inventory") {
        return None;
    }
    let counts = value.get("counts")?;
    let total_size = counts
        .get("total_size_bytes")
        .and_then(|value| value.as_u64())?;
    if matches!(response_shape, crate::OutputResponseShape::Scalar) {
        return Some(total_size.to_string());
    }
    let label = path_display_label(&value, "path");
    let compact = compact_binary_size(total_size);
    let total_entries = counts.get("total").and_then(|value| value.as_u64());
    if matches!(response_shape, crate::OutputResponseShape::OneSentence) {
        let mut parts = vec![
            format!("path={label}"),
            format!("size.bytes={total_size}"),
            format!("size.human={compact}"),
        ];
        if let Some(total) = total_entries {
            parts.push(format!("count.total={total}"));
        }
        return Some(parts.join(" "));
    }
    let mut lines = vec![
        format!("path={label}"),
        format!("size.bytes={total_size}"),
        format!("size.human={compact}"),
    ];
    if let Some(total) = total_entries {
        lines.push(format!("count.total={total}"));
    }
    Some(lines.join("\n"))
}

fn output_has_count_inventory_total(output: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return false;
    };
    if value.get("action").and_then(|value| value.as_str()) != Some("count_inventory") {
        return false;
    }
    value
        .get("counts")
        .and_then(|counts| counts.get("total"))
        .and_then(|value| value.as_u64())
        .is_some()
}

fn count_inventory_total_observation_count(loop_state: &crate::agent_engine::LoopState) -> usize {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "system_basic" | "fs_basic"))
        .filter_map(|step| step.output.as_deref())
        .filter(|output| output_has_count_inventory_total(output))
        .count()
}

fn inventory_ranked_size_list_answer(body: &str, route: &crate::RouteResult) -> Option<String> {
    if route.output_contract.response_shape != crate::OutputResponseShape::Strict {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(|value| value.as_str()) != Some("inventory_dir") {
        return None;
    }
    let sort_by = value.get("sort_by").and_then(|value| value.as_str())?;
    if !matches!(sort_by, "size_desc" | "size_asc") {
        return None;
    }
    let mut entries = value
        .get("entries")
        .and_then(|value| value.as_array())?
        .iter()
        .filter(|entry| {
            entry
                .get("kind")
                .and_then(|value| value.as_str())
                .is_none_or(|kind| kind == "file")
        })
        .filter_map(|entry| {
            let name = entry
                .get("name")
                .or_else(|| entry.get("path"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|name| !name.is_empty())?;
            let size_bytes = entry.get("size_bytes").and_then(|value| value.as_u64())?;
            Some((name.to_string(), size_bytes))
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }
    if sort_by == "size_desc" {
        entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    } else {
        entries.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    }
    Some(
        entries
            .into_iter()
            .map(|(name, size_bytes)| format!("{name} {size_bytes}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

#[cfg(test)]
fn path_batch_size_comparison_answer(body: &str, prefer_english: bool) -> Option<String> {
    path_batch_size_comparison_answer_with_style(
        body,
        prefer_english,
        SizeComparisonAnswerStyle::ExplainRatio,
    )
}

#[derive(Debug, Clone)]
struct TailLogObservation {
    path: String,
    excerpt: String,
}

fn tail_log_observation_from_step(
    step: &crate::executor::StepExecutionResult,
) -> Option<TailLogObservation> {
    if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
        return None;
    }
    let output = step.output.as_deref()?.trim();
    let value = serde_json::from_str::<serde_json::Value>(output).ok()?;
    if value.get("action").and_then(|value| value.as_str()) != Some("read_range")
        || value.get("mode").and_then(|value| value.as_str()) != Some("tail")
    {
        return None;
    }
    let requested_n = value.get("requested_n").and_then(|value| value.as_u64())?;
    if requested_n == 0 || requested_n > 50 {
        return None;
    }
    let path = value
        .get("path")
        .or_else(|| value.get("resolved_path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let excerpt = value
        .get("excerpt")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let file_name = std::path::Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(path);
    if !file_name.to_ascii_lowercase().ends_with(".log") {
        return None;
    }
    Some(TailLogObservation {
        path: path.to_string(),
        excerpt: excerpt.to_string(),
    })
}

fn latest_tail_log_observation(
    loop_state: &crate::agent_engine::LoopState,
) -> Option<TailLogObservation> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find_map(tail_log_observation_from_step)
}

#[derive(Debug, Clone, Copy, Default)]
struct LogSeverityCounts {
    info: usize,
    warn: usize,
    error: usize,
}

fn log_severity_counts(excerpt: &str) -> LogSeverityCounts {
    let mut counts = LogSeverityCounts::default();
    for token in excerpt.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        match token {
            "INFO" => counts.info += 1,
            "WARN" | "WARNING" => counts.warn += 1,
            "ERROR" | "ERR" | "FATAL" | "CRITICAL" => counts.error += 1,
            _ => {}
        }
    }
    counts
}

fn tail_log_related_name_key(path: &str) -> Option<String> {
    let file_name = std::path::Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())?;
    file_name
        .split('.')
        .next()
        .map(str::trim)
        .filter(|value| value.len() >= 3)
        .map(|value| value.to_ascii_lowercase())
}

fn collect_log_file_names_from_json(
    value: &serde_json::Value,
    related_key: &str,
    names: &mut std::collections::BTreeSet<String>,
) {
    match value {
        serde_json::Value::Object(map) => {
            let kind = map.get("kind").and_then(|value| value.as_str());
            let path = map
                .get("path")
                .or_else(|| map.get("name"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if kind == Some("file") {
                if let Some(path) = path {
                    if let Some(file_name) = std::path::Path::new(path)
                        .file_name()
                        .and_then(|value| value.to_str())
                    {
                        let lower = file_name.to_ascii_lowercase();
                        if lower.ends_with(".log") && lower.contains(related_key) {
                            names.insert(file_name.to_string());
                        }
                    }
                }
            }
            for child in map.values() {
                collect_log_file_names_from_json(child, related_key, names);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_log_file_names_from_json(item, related_key, names);
            }
        }
        _ => {}
    }
}

fn related_log_file_names(
    loop_state: &crate::agent_engine::LoopState,
    tail_path: &str,
) -> Vec<String> {
    let Some(related_key) = tail_log_related_name_key(tail_path) else {
        return Vec::new();
    };
    let mut names = std::collections::BTreeSet::new();
    for step in &loop_state.executed_step_results {
        if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
            continue;
        }
        let Some(output) = step.output.as_deref() else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) {
            collect_log_file_names_from_json(&value, &related_key, &mut names);
        }
    }
    if names.is_empty() {
        if let Some(file_name) = std::path::Path::new(tail_path)
            .file_name()
            .and_then(|value| value.to_str())
        {
            names.insert(file_name.to_string());
        }
    }
    names.into_iter().take(8).collect()
}

fn direct_log_tail_status_answer(
    _state: &AppState,
    _user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken | crate::OutputResponseShape::Scalar
        )
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::None
    {
        return None;
    }
    let observation = latest_tail_log_observation(loop_state)?;
    let counts = log_severity_counts(&observation.excerpt);
    let file_name = std::path::Path::new(&observation.path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(observation.path.as_str());
    let names = related_log_file_names(loop_state, &observation.path);
    let joined_names = if names.is_empty() {
        file_name.to_string()
    } else {
        names.join(",")
    };
    let state = if counts.error > 0 {
        "error"
    } else if counts.warn > 0 {
        "warning"
    } else {
        "ok"
    };
    let answer = format!(
        "log.files={joined_names}\nlog.tail_file={file_name}\nlog.level.info={}\nlog.level.warn={}\nlog.level.error={}\nlog.state={state}",
        counts.info, counts.warn, counts.error
    );
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
            used_evidence_ids_count: 2,
            ..Default::default()
        },
    ))
}

fn replace_delivery_with_direct_log_tail_status_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some((answer, summary)) =
        direct_log_tail_status_answer(state, user_text, loop_state, agent_run_context)
    else {
        return false;
    };
    let answer = answer.trim();
    if answer.is_empty() {
        return false;
    }
    if loop_state
        .delivery_messages
        .last()
        .map(|message| message.trim() == answer)
        .unwrap_or(false)
    {
        loop_state.last_user_visible_respond = Some(answer.to_string());
        *finalizer_summary = Some(summary);
        return true;
    }
    loop_state
        .delivery_messages
        .retain(|message| crate::finalize::is_execution_summary_message(message));
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.to_string(),
    );
    loop_state.last_user_visible_respond = Some(answer.to_string());
    *finalizer_summary = Some(summary);
    info!(
        "delivery replace_with_direct_log_tail_status task_id={}",
        task.task_id
    );
    true
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
    let prefer_english =
        prefer_english_for_agent_contextual_user_text(state, user_text, agent_run_context);
    let style = size_comparison_answer_style(route, user_text);
    if count_inventory_total_observation_count(loop_state) >= 2 {
        return None;
    }
    let answer = {
        loop_state
            .executed_step_results
            .iter()
            .rev()
            .find_map(|step| {
                if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
                    return None;
                }
                let output = step.output.as_deref()?;
                inventory_ranked_size_list_answer(output, route)
                    .or_else(|| {
                        count_inventory_size_answer_with_shape(
                            output,
                            prefer_english,
                            route.output_contract.response_shape,
                        )
                    })
                    .or_else(|| {
                        compare_paths_size_ratio_answer_with_style(output, prefer_english, style)
                    })
                    .or_else(|| {
                        path_batch_size_comparison_answer_with_style(output, prefer_english, style)
                    })
            })
    }?;
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

fn direct_directory_purpose_summary_from_size_facts(
    _state: &AppState,
    _user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::DirectoryPurposeSummary
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
    {
        return None;
    }
    let mut facts = loop_state
        .executed_step_results
        .iter()
        .rev()
        .find_map(|step| {
            if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
                return None;
            }
            let output = step.output.as_deref()?;
            let value = serde_json::from_str::<serde_json::Value>(output).ok()?;
            path_batch_size_facts(&value)
        })?;
    facts.sort_by(|a, b| {
        b.size_bytes
            .cmp(&a.size_bytes)
            .then_with(|| a.label.cmp(&b.label))
    });
    let largest = facts.first()?;
    let dir_label = route
        .output_contract
        .locator_hint
        .trim()
        .trim_end_matches(['/', '\\'])
        .trim()
        .to_string();
    let dir_label = if dir_label.is_empty() {
        ".".to_string()
    } else {
        dir_label
    };
    let subject = schema_subject_from_path_label(&largest.label);
    let file_count = facts.len() as u64;
    let answer = format!(
        "directory={dir_label}\nfile.count={file_count}\nlargest.path={}\nlargest.size_bytes={}\nschema.subject={subject}",
        largest.label, largest.size_bytes
    );
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

fn schema_subject_from_path_label(label: &str) -> String {
    let file_name = Path::new(label)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(label)
        .trim();
    let subject = file_name
        .strip_suffix(".schema.json")
        .or_else(|| file_name.strip_suffix(".json"))
        .unwrap_or(file_name)
        .replace(['_', '-'], " ");
    let subject = subject.trim();
    if subject.is_empty() {
        file_name.to_string()
    } else {
        subject.to_string()
    }
}

fn replace_delivery_with_deterministic_directory_purpose_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some((answer, summary)) = direct_directory_purpose_summary_from_size_facts(
        state,
        user_text,
        loop_state,
        agent_run_context,
    ) else {
        return false;
    };
    if loop_state
        .delivery_messages
        .last()
        .is_some_and(|message| message.trim() == answer.trim())
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
        "delivery replace_with_deterministic_directory_purpose task_id={}",
        task.task_id
    );
    true
}

fn replace_delivery_with_deterministic_quantity_comparison_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some((answer, summary)) = direct_quantity_comparison_from_compare_paths(
        state,
        user_text,
        loop_state,
        agent_run_context,
    ) else {
        return false;
    };
    if let Some(existing_answer) =
        latest_delivery_preserves_observed_quantity_size_facts(loop_state)
    {
        loop_state.last_user_visible_respond = Some(existing_answer);
        *finalizer_summary = Some(summary);
        return true;
    }
    if loop_state
        .delivery_messages
        .last()
        .is_some_and(|message| message.trim() == answer.trim())
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
        "delivery replace_with_deterministic_quantity_comparison task_id={}",
        task.task_id
    );
    true
}

fn json_scalar_display(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => Some("null".to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        }
        _ => None,
    }
}

fn compact_json_item_label(key: Option<&str>, value: &serde_json::Value) -> Option<String> {
    let key = key.map(str::trim).filter(|key| !key.is_empty());
    match (key, json_scalar_display(value)) {
        (Some(key), Some(value)) => Some(format!("{key}={value}")),
        (Some(key), None) => Some(key.to_string()),
        (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn structured_container_summary_from_value(
    field_path: &str,
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    let field_path = field_path.trim();
    if field_path.is_empty() {
        return None;
    }
    const MAX_PREVIEW_ITEMS: usize = 6;
    match value {
        serde_json::Value::Object(map) => {
            if map.is_empty() {
                return Some(if prefer_english {
                    format!("`{field_path}` is an empty object.")
                } else {
                    format!("`{field_path}` 是一个空对象。")
                });
            }
            let mut entries = map
                .iter()
                .filter_map(|(key, value)| compact_json_item_label(Some(key), value))
                .take(MAX_PREVIEW_ITEMS)
                .collect::<Vec<_>>();
            if entries.is_empty() {
                entries = map.keys().take(MAX_PREVIEW_ITEMS).cloned().collect();
            }
            let suffix = if map.len() > entries.len() {
                if prefer_english {
                    ", ..."
                } else {
                    "，..."
                }
            } else {
                ""
            };
            if prefer_english {
                Some(format!(
                    "`{field_path}` contains {} entries: {}{}.",
                    map.len(),
                    entries.join(", "),
                    suffix
                ))
            } else {
                Some(format!(
                    "`{field_path}` 包含 {} 项：{}{}。",
                    map.len(),
                    entries.join("、"),
                    suffix
                ))
            }
        }
        serde_json::Value::Array(items) => {
            if items.is_empty() {
                return Some(if prefer_english {
                    format!("`{field_path}` is an empty array.")
                } else {
                    format!("`{field_path}` 是一个空数组。")
                });
            }
            let entries = items
                .iter()
                .filter_map(|value| compact_json_item_label(None, value))
                .take(MAX_PREVIEW_ITEMS)
                .collect::<Vec<_>>();
            let suffix = if items.len() > entries.len() {
                if prefer_english {
                    ", ..."
                } else {
                    "，..."
                }
            } else {
                ""
            };
            if entries.is_empty() {
                return Some(if prefer_english {
                    format!("`{field_path}` contains {} items.", items.len())
                } else {
                    format!("`{field_path}` 包含 {} 项。", items.len())
                });
            }
            if prefer_english {
                Some(format!(
                    "`{field_path}` contains {} items: {}{}.",
                    items.len(),
                    entries.join(", "),
                    suffix
                ))
            } else {
                Some(format!(
                    "`{field_path}` 包含 {} 项：{}{}。",
                    items.len(),
                    entries.join("、"),
                    suffix
                ))
            }
        }
        _ => None,
    }
}

fn structured_container_from_extract_value(
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if !matches!(
        value.get("action").and_then(|value| value.as_str()),
        Some("extract_field" | "read_field")
    ) {
        return None;
    }
    if !value
        .get("exists")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    let field_path = value
        .get("resolved_field_path")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            value
                .get("field_path")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })?;
    structured_container_summary_from_value(field_path, value.get("value")?, prefer_english)
}

fn deterministic_structured_container_summary_answer(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if !route.output_contract.requires_content_evidence || route.output_contract.delivery_required {
        return None;
    }
    if !matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
    ) {
        return None;
    }
    if !matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None | crate::OutputSemanticKind::ContentExcerptSummary
    ) {
        return None;
    }
    let prefer_english = prefer_english_for_user_text(state, user_text);
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| {
            step.is_ok() && matches!(step.skill.as_str(), "system_basic" | "config_basic")
        })
        .filter_map(|step| step.output.as_deref())
        .filter_map(|output| serde_json::from_str::<serde_json::Value>(output).ok())
        .find_map(|value| structured_container_from_extract_value(&value, prefer_english))
}

fn direct_db_basic_observed_answer(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if route.output_contract.delivery_required || !route.output_contract.requires_content_evidence {
        return None;
    }
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    ) {
        return None;
    }
    let prefer_english = prefer_english_for_user_text(state, user_text);
    let answer = loop_state
        .executed_step_results
        .iter()
        .rev()
        .find(|step| {
            step.is_ok()
                && step.skill == "db_basic"
                && step
                    .output
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|output| !output.is_empty())
        })
        .and_then(|step| step.output.as_deref())
        .and_then(|output| db_basic_rows_answer_from_output_for_route(route, output))?;
    if answer.trim().is_empty() {
        return None;
    }
    Some((
        if prefer_english {
            answer
        } else {
            answer.replace(", ", "，")
        },
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

fn db_basic_rows_answer_from_output_for_route(
    route: &crate::RouteResult,
    output: &str,
) -> Option<String> {
    db_basic_rows_answer_from_output_with_scalar_count(
        output,
        route.output_contract.response_shape == crate::OutputResponseShape::Scalar
            && route.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarCount,
    )
}

fn db_basic_rows_answer_from_output_with_scalar_count(
    output: &str,
    scalar_count: bool,
) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    let result = value
        .get("columns")
        .and_then(|_| value.get("rows"))
        .map(|_| &value)
        .or_else(|| value.get("result"))
        .or_else(|| value.get("extra").and_then(|extra| extra.get("result")))?;
    let columns = result
        .get("columns")
        .and_then(|value| value.as_array())?
        .iter()
        .filter_map(|value| value.as_str().map(str::trim))
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if columns.is_empty() {
        return None;
    }
    let rows = result.get("rows").and_then(|value| value.as_array())?;
    if scalar_count {
        if rows.len() == 1 && columns.len() == 1 {
            return rows
                .first()
                .and_then(|row| db_row_column_value(row, &columns[0], 0));
        }
        return Some(rows.len().to_string());
    }
    if rows.is_empty() {
        return Some("No rows returned.".to_string());
    }
    if rows.len() == 1 && columns.len() == 1 {
        return rows
            .first()
            .and_then(|row| db_row_column_value(row, &columns[0], 0));
    }
    if columns.len() == 1 {
        let lines = rows
            .iter()
            .filter_map(|row| db_row_column_value(row, &columns[0], 0))
            .take(50)
            .collect::<Vec<_>>();
        return (!lines.is_empty()).then(|| lines.join("\n"));
    }

    let lines = rows
        .iter()
        .filter_map(|row| db_row_line(row, &columns))
        .take(50)
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn db_row_line(row: &serde_json::Value, columns: &[String]) -> Option<String> {
    let values = columns
        .iter()
        .enumerate()
        .filter_map(|(idx, column)| {
            db_row_column_value(row, column, idx).map(|value| format!("{column}: {value}"))
        })
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| values.join(", "))
}

fn db_row_column_value(row: &serde_json::Value, column: &str, index: usize) -> Option<String> {
    match row {
        serde_json::Value::Object(map) => map.get(column).and_then(json_scalar_display),
        serde_json::Value::Array(values) => values.get(index).and_then(json_scalar_display),
        _ => None,
    }
}

fn structured_file_format_for_path(path: &str) -> Option<&'static str> {
    match Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => Some("json"),
        Some("toml") => Some("toml"),
        _ => None,
    }
}

fn broad_structured_read_range_from_value(value: &serde_json::Value) -> Option<(String, String)> {
    if value.get("action").and_then(|value| value.as_str()) != Some("read_range") {
        return None;
    }
    if !matches!(
        value.get("mode").and_then(|value| value.as_str()),
        Some("head" | "full" | "all")
    ) {
        return None;
    }
    if value
        .get("requested_n")
        .and_then(|value| value.as_u64())
        .is_some_and(|requested_n| requested_n < 50)
    {
        return None;
    }
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    let format = value
        .get("format")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|format| matches!(*format, "json" | "toml"))
        .or_else(|| structured_file_format_for_path(path))?;
    Some((path.to_string(), format.to_string()))
}

fn latest_broad_structured_read_range(
    loop_state: &crate::agent_engine::LoopState,
) -> Option<(String, String)> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "system_basic" | "fs_basic"))
        .filter_map(|step| step.output.as_deref())
        .filter_map(|output| serde_json::from_str::<serde_json::Value>(output).ok())
        .find_map(|value| broad_structured_read_range_from_value(&value))
}

fn message_is_non_answer_separator(message: &str) -> bool {
    let chars = message
        .trim()
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<Vec<_>>();
    if chars.len() < 6 || chars.iter().any(|ch| ch.is_alphanumeric()) {
        return false;
    }
    let first = chars[0];
    chars.iter().all(|ch| *ch == first)
        || chars
            .iter()
            .all(|ch| matches!(*ch, '=' | '-' | '_' | '*' | '#' | '~'))
}

fn discard_non_answer_separator_delivery_for_broad_structured_read(
    task_id: &str,
    loop_state: &mut crate::agent_engine::LoopState,
) -> bool {
    if latest_broad_structured_read_range(loop_state).is_none() {
        return false;
    }
    let before_len = loop_state.delivery_messages.len();
    loop_state.delivery_messages.retain(|message| {
        crate::finalize::is_execution_summary_message(message)
            || !message_is_non_answer_separator(message)
    });
    let removed = before_len != loop_state.delivery_messages.len();
    if removed {
        if loop_state
            .last_user_visible_respond
            .as_deref()
            .is_some_and(message_is_non_answer_separator)
        {
            loop_state.last_user_visible_respond = None;
        }
        if loop_state
            .last_publishable_synthesis_output
            .as_deref()
            .is_some_and(message_is_non_answer_separator)
        {
            loop_state.last_publishable_synthesis_output = None;
        }
        info!(
            "delivery discard_non_answer_separator_after_structured_read task_id={}",
            task_id
        );
    }
    removed
}

fn validate_structured_file(path: &str, format: &str) -> Option<Result<(), String>> {
    let content = std::fs::read_to_string(path)
        .map_err(|err| err.to_string())
        .ok()?;
    let result = match format {
        "json" => serde_json::from_str::<serde_json::Value>(&content)
            .map(|_| ())
            .map_err(|err| err.to_string()),
        "toml" => toml::from_str::<toml::Value>(&content)
            .map(|_| ())
            .map_err(|err| err.to_string()),
        _ => return None,
    };
    Some(result)
}

fn deterministic_structured_file_validation_from_read_range(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|context| context.route_result.as_ref())?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ConfigValidation {
        return None;
    }
    let (path, format) = latest_broad_structured_read_range(loop_state)?;
    let validation = validate_structured_file(&path, &format)?;
    let prefer_english = prefer_english_for_user_text(state, user_text);
    let answer = match validation {
        Ok(()) if prefer_english => format!("pass: {format} parsed successfully"),
        Ok(()) => format!("通过：{format} 解析成功"),
        Err(err) if prefer_english => format!(
            "fail: {format} parse failed: {}",
            crate::truncate_for_agent_trace(&err)
        ),
        Err(err) => format!(
            "未通过：{format} 解析失败：{}",
            crate::truncate_for_agent_trace(&err)
        ),
    };
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

fn attach_deterministic_structured_file_validation_from_read_range(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut crate::agent_engine::LoopState,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some((answer, summary)) = deterministic_structured_file_validation_from_read_range(
        state,
        user_text,
        loop_state,
        agent_run_context,
    ) else {
        return false;
    };
    *finalizer_summary = Some(summary);
    loop_state.last_user_visible_respond = Some(answer.clone());
    append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
    info!(
        "delivery fallback_from_structured_file_validation_read_range task_id={}",
        task.task_id
    );
    true
}

fn replace_delivery_with_deterministic_rustclaw_config_risk_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut crate::agent_engine::LoopState,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some((answer, summary)) = direct_rustclaw_config_risk_answer(state, user_text, loop_state)
    else {
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
        "delivery replace_with_deterministic_rustclaw_config_risk task_id={}",
        task.task_id
    );
    true
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
    if let Some((answer, _summary)) =
        direct_config_edit_observed_answer(state, user_text, loop_state)
    {
        return answer;
    }
    if let Some((answer, _summary)) =
        direct_rustclaw_config_risk_answer(state, user_text, loop_state)
    {
        return answer;
    }
    if let Some((answer, _summary)) = direct_quantity_comparison_from_compare_paths(
        state,
        user_text,
        loop_state,
        agent_run_context,
    ) {
        return answer;
    }
    if let Some(answer) = deterministic_structured_container_summary_answer(
        state,
        user_text,
        loop_state,
        agent_run_context,
    ) {
        return answer;
    }
    if let Some((answer, _summary)) =
        direct_db_basic_observed_answer(state, user_text, loop_state, agent_run_context)
    {
        return answer;
    }
    if let Some((answer, _summary)) = deterministic_matrix_observed_shape_answer(
        state,
        task,
        user_text,
        loop_state,
        agent_run_context,
    ) {
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
    let status_summary = || deterministic_observed_execution_status_summary(loop_state);
    let deterministic_answer =
        deterministic_execution_failed_step_answer(state, user_text, loop_state, agent_run_context)
            .map(|answer| (answer, status_summary()))
            .or_else(|| {
                deterministic_observed_execution_status_answer(state, user_text, loop_state)
                    .map(|answer| (answer, status_summary()))
            })
            .or_else(|| direct_config_edit_observed_answer(state, user_text, loop_state))
            .or_else(|| direct_rustclaw_config_risk_answer(state, user_text, loop_state))
            .or_else(|| {
                direct_quantity_comparison_from_compare_paths(
                    state,
                    user_text,
                    loop_state,
                    agent_run_context,
                )
            })
            .or_else(|| {
                deterministic_structured_container_summary_answer(
                    state,
                    user_text,
                    loop_state,
                    agent_run_context,
                )
                .map(|answer| (answer, status_summary()))
            })
            .or_else(|| {
                direct_db_basic_observed_answer(state, user_text, loop_state, agent_run_context)
            })
            .or_else(|| {
                deterministic_matrix_observed_shape_answer(
                    state,
                    task,
                    user_text,
                    loop_state,
                    agent_run_context,
                )
            })
            .or_else(|| {
                deterministic_missing_observed_target_answer(
                    state,
                    user_text,
                    loop_state,
                    agent_run_context,
                )
                .map(|answer| (answer, status_summary()))
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
    if !delivery_contract_suppresses_execution_summary(
        loop_state,
        agent_run_context,
        std::slice::from_ref(&message),
    ) {
        delivery_messages.extend(execution_summaries);
    }
    delivery_messages.push(message.clone());
    let delivery_consistent =
        crate::task_journal::delivery_payload_consistent(&message, &delivery_messages);
    let has_deterministic_answer = deterministic_answer.is_some();
    let finalizer_summary = finalizer_summary.or_else(|| {
        deterministic_answer
            .as_ref()
            .map(|(_, summary)| summary.clone())
    });
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
            direct_config_edit_observed_answer(state, user_text, &loop_state)
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            info!(
                "delivery fallback_from_config_edit_observed task_id={}",
                task.task_id
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            direct_rustclaw_config_risk_answer(state, user_text, &loop_state)
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            info!(
                "delivery fallback_from_rustclaw_config_risk_observed task_id={}",
                task.task_id
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            direct_db_basic_observed_answer(state, user_text, &loop_state, agent_run_context)
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            info!(
                "delivery fallback_from_db_basic_observed task_id={}",
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
            direct_structured_observed_answer_allowing_implicit_metadata_path_facts(
                Some(state),
                &loop_state,
                agent_run_context,
            )
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
                    direct_file_token_from_observed_path_batch_facts(&loop_state, agent_run_context)
                })
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
        if !successful_content_observation_should_precede_status_summary(
            agent_run_context,
            &loop_state,
        ) {
            attach_deterministic_observed_execution_status_answer(
                state,
                task,
                user_text,
                &mut loop_state,
                &mut finalizer_summary,
            );
        }
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

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            direct_path_from_active_bound_inventory(&loop_state, agent_run_context)
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            info!(
                "delivery fallback_from_active_bound_inventory_path task_id={}",
                task.task_id
            );
        }
    }

    if loop_state.delivery_messages.is_empty()
        && should_try_observed_output_language_fallback(&loop_state, agent_run_context)
    {
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
        attach_deterministic_observed_execution_status_answer(
            state,
            task,
            user_text,
            &mut loop_state,
            &mut finalizer_summary,
        );
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

    normalize_file_token_delivery_from_auto_locator(&mut loop_state, agent_run_context);
    normalize_file_token_delivery_from_observed_paths(state, &mut loop_state, agent_run_context);
    enforce_delivery_output_contract(state, task, user_text, &mut loop_state, agent_run_context)
        .await;
    replace_placeholder_delivery_with_synthesis(task, &mut loop_state);
    replace_raw_read_delivery_with_synthesis(task, &mut loop_state, agent_run_context);
    let replaced_grounded_terminal_respond =
        replace_structured_delivery_with_grounded_terminal_respond(
            task,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        );
    let replaced_quantity_comparison = if !replaced_grounded_terminal_respond {
        replace_delivery_with_deterministic_quantity_comparison_answer(
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
    let replaced_directory_purpose =
        if !replaced_grounded_terminal_respond && !replaced_quantity_comparison {
            replace_delivery_with_deterministic_directory_purpose_answer(
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
    let replaced_contract_answer = if !replaced_grounded_terminal_respond
        && !replaced_quantity_comparison
        && !replaced_directory_purpose
    {
        replace_delivery_with_loop_contract_observed_answer(
            task,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        )
    } else {
        false
    };
    let replaced_failed_step = if !replaced_grounded_terminal_respond
        && !replaced_quantity_comparison
        && !replaced_directory_purpose
        && !replaced_contract_answer
    {
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
    let replaced_log_tail_status = if !replaced_grounded_terminal_respond
        && !replaced_quantity_comparison
        && !replaced_directory_purpose
        && !replaced_contract_answer
        && !replaced_failed_step
    {
        replace_delivery_with_direct_log_tail_status_answer(
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
    if !replaced_grounded_terminal_respond
        && !replaced_quantity_comparison
        && !replaced_directory_purpose
        && !replaced_contract_answer
        && !replaced_failed_step
        && !replaced_log_tail_status
        && !delivery_is_content_answer_candidate(
            agent_run_context,
            &loop_state,
            &loop_state.delivery_messages,
        )
    {
        replace_delivery_with_deterministic_observed_execution_status_answer(
            state,
            task,
            user_text,
            &mut loop_state,
            &mut finalizer_summary,
        );
    }
    if !replaced_grounded_terminal_respond && !replaced_log_tail_status {
        replace_delivery_with_latest_tail_read_range_answer(
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
        );
    }
    replace_delivery_with_deterministic_rustclaw_config_risk_answer(
        state,
        task,
        user_text,
        &mut loop_state,
        &mut finalizer_summary,
    );
    discard_non_answer_separator_delivery_for_broad_structured_read(&task.task_id, &mut loop_state);
    if loop_state.delivery_messages.is_empty() {
        attach_deterministic_structured_file_validation_from_read_range(
            state,
            task,
            user_text,
            &mut loop_state,
            agent_run_context,
            &mut finalizer_summary,
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
    replace_delivery_with_observed_markdown_heading_scalar(
        &task.task_id,
        &mut loop_state,
        agent_run_context,
        &mut delivery_deduped,
        &mut finalizer_summary,
    );
    replace_delivery_with_matrix_observed_shape_answer(
        state,
        task,
        user_text,
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
#[path = "loop_reply_tests.rs"]
mod tests;
