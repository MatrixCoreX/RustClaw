use std::path::Path;

use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, AskReply, ClaimedTask};

use super::{
    build_loop_journal, delivery_messages_include_delivery_token, final_reply_language_hint,
    latest_publishable_synthesis_step_matches,
    planned_delivery_is_publishable_model_language_answer, step_output_is_read_range,
    structured_json_values_from_output,
};

pub(super) fn route_requires_file_token(agent_run_context: Option<&AgentRunContext>) -> bool {
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

pub(super) fn route_requires_compound_content_file_delivery(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            route.output_contract.delivery_required
                && route.output_contract.delivery_intent == crate::OutputDeliveryIntent::FileSingle
                && route
                    .output_contract
                    .semantic_kind
                    .is_content_excerpt_summary()
        })
}

pub(super) fn route_allows_file_token_only_fallback(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    !route_requires_compound_content_file_delivery(agent_run_context)
}

fn route_locator_file_token(
    state: &AppState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if !matches!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
    ) {
        return None;
    }
    let hint = route.output_contract.locator_hint.trim();
    if hint.is_empty() {
        return None;
    }
    let path = Path::new(hint);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(path)
    };
    if !resolved.is_file() {
        return None;
    }
    let resolved = resolved.canonicalize().unwrap_or(resolved);
    Some(format!("FILE:{}", resolved.display()))
}

pub(super) fn append_compound_file_delivery_token_from_route(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    if !route_requires_compound_content_file_delivery(agent_run_context)
        || delivery_messages_include_delivery_token(&loop_state.delivery_messages)
        || !loop_state
            .delivery_messages
            .iter()
            .any(|message| planned_delivery_is_publishable_model_language_answer(message))
    {
        return false;
    }
    let Some(token) = route_locator_file_token(state, agent_run_context) else {
        return false;
    };
    append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, token);
    true
}

pub(super) fn generated_delivery_existing_file_content_synthesis_token(
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::GeneratedFileDelivery
        || !route.output_contract.delivery_required
        || route.output_contract.delivery_intent != crate::OutputDeliveryIntent::FileSingle
        || route.output_contract.response_shape != crate::OutputResponseShape::FileToken
        || !latest_publishable_synthesis_step_matches(loop_state)
        || !loop_state
            .executed_step_results
            .iter()
            .any(step_output_is_read_range)
        || loop_state
            .executed_step_results
            .iter()
            .any(step_output_is_file_write)
    {
        return None;
    }
    route_locator_file_token(state, agent_run_context)
}

fn step_output_is_file_write(step: &crate::executor::StepExecutionResult) -> bool {
    if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
        return false;
    }
    step.output.as_deref().map(str::trim).is_some_and(|output| {
        structured_json_values_from_output(output)
            .iter()
            .any(file_write_output_value)
    })
}

fn file_write_output_value(value: &serde_json::Value) -> bool {
    matches!(
        value.get("action").and_then(|value| value.as_str()),
        Some("write_text" | "append_text")
    )
}

pub(crate) fn output_excerpt_has_missing_file_evidence(output: &str) -> bool {
    if output.trim().eq_ignore_ascii_case("NOT_FOUND") {
        return true;
    }
    structured_json_values_from_output(output)
        .iter()
        .any(output_value_has_missing_file_evidence)
}

fn output_value_has_missing_file_evidence(value: &serde_json::Value) -> bool {
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
    if has_path_batch_shape
        && path_facts.is_some_and(|facts| {
            facts.iter().any(|fact| {
                fact.get("exists").and_then(|v| v.as_bool()) == Some(false)
                    && fact
                        .get("path")
                        .and_then(|v| v.as_str())
                        .is_some_and(|path| !path.trim().is_empty())
            })
        })
    {
        return true;
    }

    value
        .get("extra")
        .is_some_and(output_value_has_missing_file_evidence)
}

pub(super) fn has_missing_file_search_evidence(loop_state: &LoopState) -> bool {
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

pub(super) fn step_error_has_missing_file_evidence(
    step: &crate::executor::StepExecutionResult,
) -> bool {
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

pub(super) fn latest_file_delivery_observation_is_missing(loop_state: &LoopState) -> bool {
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

pub(super) fn should_return_missing_file_delivery_reply(
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

pub(super) fn missing_file_path_from_output(output: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(output).ok()?;
    missing_file_path_from_output_value(&value)
}

fn missing_file_path_from_output_value(value: &serde_json::Value) -> Option<String> {
    let path_from_facts = value
        .get("facts")
        .and_then(|value| value.as_array())
        .and_then(|facts| {
            facts.iter().find_map(|fact| {
                (fact.get("exists").and_then(|value| value.as_bool()) == Some(false))
                    .then(|| missing_path_from_path_fact(fact))
                    .flatten()
            })
        });
    if path_from_facts.is_some() {
        return path_from_facts;
    }

    let path_from_empty_locator = value
        .get("action")
        .and_then(|v| v.as_str())
        .is_some_and(|action| matches!(action, "find_name" | "find_path"))
        .then(|| {
            let candidate = value
                .get("patterns")
                .and_then(|patterns| patterns.as_array())
                .and_then(|patterns| patterns.first())
                .and_then(|pattern| pattern.as_str())
                .or_else(|| value.get("query").and_then(|query| query.as_str()))
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(ToString::to_string)?;
            let root = value
                .get("root")
                .and_then(|root| root.as_str())
                .map(str::trim)
                .filter(|root| !root.is_empty());
            Some(if let Some(root) = root {
                Path::new(root).join(candidate).display().to_string()
            } else {
                candidate
            })
        })
        .flatten();
    if path_from_empty_locator.is_some() {
        return path_from_empty_locator;
    }

    value
        .get("extra")
        .and_then(missing_file_path_from_output_value)
}

pub(super) fn missing_file_path_from_loop(
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

pub(super) async fn missing_file_delivery_reply_from_loop(
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

    let language_hint = final_reply_language_hint(state, task, user_text, agent_run_context);
    let missing_path = missing_file_path_from_loop(loop_state, agent_run_context);
    let message = crate::fallback::compose_missing_file_delivery_response(
        state,
        task,
        user_text,
        agent_run_context
            .and_then(|ctx| ctx.route_result.as_ref())
            .map(|route| route.resolved_intent.as_str())
            .unwrap_or(""),
        missing_path.as_deref(),
        &language_hint,
    )
    .await;
    let delivery_messages = vec![message.clone()];
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
