use std::fmt::Display;

use tracing::{info, warn};

use super::contract_hint::contract_hint_fallback_decision;
use super::directory_observation::directory_pair_fallback_decision;
use super::explicit_path_facts::explicit_surface_path_facts_fallback_decision;
use super::inline_transform::inline_json_transform_fallback_decision;
use super::parse_failed_fallback::{
    empty_clarify_decision, parse_failed_explicit_existing_path_observation_fallback_decision,
};
use super::{normalizer_output_from_fallback, IntentNormalizerOutput};
use crate::intent::surface_signals::PromptSurfaceSignals;
use crate::{AppState, ClaimedTask};

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
