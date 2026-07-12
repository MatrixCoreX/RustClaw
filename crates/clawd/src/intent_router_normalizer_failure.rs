use std::fmt::Display;

use tracing::{info, warn};

use super::contract_hint::contract_hint_fallback_decision;
use super::directory_observation::directory_pair_fallback_decision;
use super::explicit_path_facts::explicit_surface_path_facts_fallback_decision;
use super::inline_transform::inline_json_transform_fallback_decision;
use super::parse_failed_fallback::{
    empty_clarify_decision, parse_failed_explicit_existing_path_observation_fallback_decision,
};
use super::{
    normalizer_output_from_fallback, IntentNormalizerOutput, IntentOutputContract,
    OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape, OutputSemanticKind,
    RouteDecision, ScheduleKind,
};
use crate::intent::surface_signals::PromptSurfaceSignals;
use crate::{AppState, ClaimedTask};
use serde_json::Value;

pub(super) fn normalizer_prompt_missing_fallback_output(
    state: &AppState,
    task: &ClaimedTask,
    req: &str,
    surface_req: &str,
    err: &(impl Display + ?Sized),
) -> IntentNormalizerOutput {
    warn!(
        "intent_normalizer prompt load failed, falling back to safe clarify: task_id={} err={}",
        task.task_id, err
    );
    if let Some(fallback) = inline_json_transform_fallback_decision(req) {
        info!(
            "{} intent_normalizer task_id={} prompt_missing_inline_json_transform_fallback input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback(
            req,
            "prompt_missing_inline_json_transform_fallback",
            fallback,
            None,
        );
    }
    if let Some(fallback) = directory_pair_fallback_decision(state, surface_req) {
        info!(
            "{} intent_normalizer task_id={} prompt_missing_directory_pair_fallback reason={} input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            fallback.reason,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback(
            req,
            "prompt_missing_directory_pair_fallback",
            fallback,
            None,
        );
    }
    let fallback = empty_clarify_decision(req, "normalizer_prompt_missing");
    normalizer_output_from_fallback(req, "prompt_missing_safe_clarify", fallback, None)
}

pub(super) fn normalizer_llm_failed_fallback_output(
    state: &AppState,
    task: &ClaimedTask,
    req: &str,
    surface_req: &str,
    req_surface: &PromptSurfaceSignals,
    err: &(impl Display + ?Sized),
) -> IntentNormalizerOutput {
    warn!(
        "intent_normalizer llm failed, falling back to safe clarify: task_id={} err={}",
        task.task_id, err
    );
    if let Some(fallback) = contract_hint_fallback_decision(
        req,
        req_surface,
        &state.skill_rt.workspace_root,
        "normalizer_unavailable_contract_hint",
    ) {
        info!(
            "{} intent_normalizer task_id={} llm_failed_contract_hint_fallback reason={} input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            fallback.reason,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback(
            req,
            "llm_failed_contract_hint_fallback",
            fallback,
            None,
        );
    }
    if let Some(fallback) = inline_json_transform_fallback_decision(req) {
        info!(
            "{} intent_normalizer task_id={} llm_failed_inline_json_transform_fallback input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback(
            req,
            "llm_failed_inline_json_transform_fallback",
            fallback,
            None,
        );
    }
    if let Some(fallback) = directory_pair_fallback_decision(state, surface_req) {
        info!(
            "{} intent_normalizer task_id={} llm_failed_directory_pair_fallback reason={} input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            fallback.reason,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback(
            req,
            "llm_failed_directory_pair_fallback",
            fallback,
            None,
        );
    }
    if let Some(fallback) = explicit_surface_path_facts_fallback_decision(
        surface_req,
        req_surface,
        &state.skill_rt.workspace_root,
    ) {
        info!(
            "{} intent_normalizer task_id={} llm_failed_explicit_surface_fallback reason={} input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            fallback.reason,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback(
            req,
            "llm_failed_structured_surface_fallback",
            fallback,
            None,
        );
    }
    if let Some(fallback) = parse_failed_explicit_existing_path_observation_fallback_decision(
        req,
        &state.skill_rt.workspace_root,
    ) {
        info!(
            "{} intent_normalizer task_id={} llm_failed_existing_path_fallback reason={} input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            fallback.reason,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback(
            req,
            "llm_failed_existing_path_observation_fallback",
            fallback,
            None,
        );
    }
    let fallback = empty_clarify_decision(req, "normalizer_llm_failed");
    normalizer_output_from_fallback(
        req,
        "llm_failed_safe_clarify",
        fallback,
        Some(crate::fallback::ClarifyFallbackSource::LlmUnavailable),
    )
}

pub(super) fn normalizer_parse_failed_fallback_output(
    state: &AppState,
    task: &ClaimedTask,
    req: &str,
    surface_req: &str,
    req_surface: &PromptSurfaceSignals,
    llm_out: &str,
) -> IntentNormalizerOutput {
    warn!(
        "intent_normalizer parse failed, falling back to safe clarify: task_id={} raw={}",
        task.task_id,
        crate::truncate_for_log(llm_out)
    );
    if let Some(fallback) = inline_json_transform_fallback_decision(req) {
        info!(
            "{} intent_normalizer task_id={} parse_failed_inline_json_transform_fallback input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback(
            req,
            "parse_failed_inline_json_transform_fallback",
            fallback,
            None,
        );
    }
    if let Some(fallback) = directory_pair_fallback_decision(state, surface_req) {
        info!(
            "{} intent_normalizer task_id={} parse_failed_directory_pair_fallback reason={} input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            fallback.reason,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback(
            req,
            "parse_failed_directory_pair_fallback",
            fallback,
            None,
        );
    }
    if let Some(fallback) = contract_hint_fallback_decision(
        req,
        req_surface,
        &state.skill_rt.workspace_root,
        "normalizer_parse_failed_contract_hint",
    ) {
        info!(
            "{} intent_normalizer task_id={} parse_failed_contract_hint_fallback reason={} input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            fallback.reason,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback(
            req,
            "parse_failed_contract_hint_fallback",
            fallback,
            None,
        );
    }
    if let Some(fallback) = explicit_surface_path_facts_fallback_decision(
        surface_req,
        req_surface,
        &state.skill_rt.workspace_root,
    ) {
        info!(
            "{} intent_normalizer task_id={} parse_failed_explicit_surface_fallback reason={} input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            fallback.reason,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback(
            req,
            "parse_failed_structured_surface_fallback",
            fallback,
            None,
        );
    }
    if let Some(fallback) = parse_failed_executable_contract_fallback_decision(req, llm_out) {
        info!(
            "{} intent_normalizer task_id={} parse_failed_executable_contract_fallback reason={} input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            fallback.reason,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback(
            req,
            "parse_failed_executable_contract_fallback",
            fallback,
            None,
        );
    }
    if let Some(fallback) = parse_failed_explicit_existing_path_observation_fallback_decision(
        req,
        &state.skill_rt.workspace_root,
    ) {
        info!(
            "{} intent_normalizer task_id={} parse_failed_existing_path_fallback reason={} input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            fallback.reason,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback(
            req,
            "parse_failed_existing_path_observation_fallback",
            fallback,
            None,
        );
    }
    let fallback = empty_clarify_decision(req, "normalizer_parse_failed");
    normalizer_output_from_fallback(req, "parse_failed_safe_clarify", fallback, None)
}

pub(super) fn parse_failed_executable_contract_fallback_decision(
    req: &str,
    llm_out: &str,
) -> Option<RouteDecision> {
    let value = serde_json::from_str::<Value>(llm_out.trim()).ok()?;
    if !raw_normalizer_execution_recipe_declares_execution(&value) {
        return None;
    }
    let has_required_machine_fields = raw_json_nonempty(
        value
            .pointer("/state_patch/required_machine_fields")
            .or_else(|| value.pointer("/output_contract/required_machine_fields")),
    );
    let raw_shape = value
        .pointer("/output_contract/response_shape")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let response_shape = if has_required_machine_fields
        || matches!(raw_shape.as_str(), "strict" | "json" | "exact")
    {
        OutputResponseShape::Strict
    } else {
        OutputResponseShape::Free
    };
    Some(RouteDecision {
        resolved_user_intent: req.trim().to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "normalizer_parse_failed_executable_contract_fallback:executable_contract_preserved_for_agent_loop".to_string(),
        confidence: Some(0.50),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: String::new(),
            ..Default::default()
        },
    })
}

fn raw_normalizer_execution_recipe_declares_execution(value: &Value) -> bool {
    let Some(recipe) = value.get("execution_recipe").and_then(Value::as_object) else {
        return false;
    };
    for key in [
        "kind",
        "command",
        "cmd",
        "shell_command",
        "execution_mode",
        "async_adapter_kind",
    ] {
        let Some(text) = recipe.get(key).and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        if !text.is_empty() && !text.eq_ignore_ascii_case("none") {
            return true;
        }
    }
    false
}

fn raw_json_nonempty(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(object)) => !object.is_empty(),
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(Value::Bool(_) | Value::Number(_)) => true,
        Some(Value::Null) | None => false,
    }
}
