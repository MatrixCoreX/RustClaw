use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AppState, ClaimedTask};

const MAX_SESSION_ALIAS_BINDINGS: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct SessionAliasBinding {
    pub(crate) alias: String,
    pub(crate) target: String,
    pub(crate) updated_at_ts: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct ConversationState {
    pub(crate) active_followup_task_id: Option<String>,
    pub(crate) active_clarify_task_id: Option<String>,
    pub(crate) active_observed_facts_task_id: Option<String>,
    #[serde(default)]
    pub(crate) alias_bindings: Vec<SessionAliasBinding>,
    #[serde(default)]
    pub(crate) last_primary_task_prompt: Option<String>,
    #[serde(default)]
    pub(crate) last_primary_task_output: Option<String>,
    pub(crate) locale_hint: Option<String>,
    pub(crate) last_task_id: String,
    pub(crate) updated_at_ts: u64,
}

pub(crate) struct ActiveSessionSnapshot {
    #[allow(dead_code)]
    pub(crate) conversation_state: Option<ConversationState>,
    pub(crate) active_followup_frame: Option<crate::followup_frame::FollowupFrame>,
    pub(crate) active_clarify_state: Option<crate::clarify_state::ClarifyState>,
    pub(crate) active_observed_facts: Option<crate::observed_facts::ObservedFacts>,
}

#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub(crate) struct ActiveSessionPointers {
    pub(crate) active_followup_task_id: Option<String>,
    pub(crate) active_clarify_task_id: Option<String>,
    pub(crate) active_observed_facts_task_id: Option<String>,
}

fn effective_user_key(task: &ClaimedTask) -> String {
    task.user_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("anon:{}:{}", task.user_id, task.chat_id))
}

fn normalized_locale_hint(payload: Option<&Value>) -> Option<String> {
    payload
        .and_then(|value| {
            value
                .get("response_language")
                .or_else(|| value.get("language"))
                .or_else(|| value.get("locale"))
        })
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn normalize_alias_target(raw_target: &str) -> Option<String> {
    let trimmed = raw_target
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | '“' | '”' | '‘' | '’'))
        .trim();
    if trimmed.is_empty() {
        return None;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(trimmed);
    crate::intent::locator_extractor::extract_explicit_locator_for_fallback(trimmed)
        .map(|locator| locator.locator_hint)
        .or_else(|| surface.single_filename_candidate().map(ToString::to_string))
        .or_else(|| Some(trimmed.to_string()))
}

fn normalize_explicit_alias_target(raw_target: &str) -> Option<String> {
    let trimmed = raw_target
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | '“' | '”' | '‘' | '’'))
        .trim();
    if trimmed.is_empty() {
        return None;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(trimmed);
    crate::intent::locator_extractor::extract_explicit_locator_for_fallback(trimmed)
        .map(|locator| locator.locator_hint)
        .or_else(|| surface.single_filename_candidate().map(ToString::to_string))
}

fn normalized_alias_surface_for_match(raw: &str) -> String {
    let mut out = String::new();
    let mut pending_space = false;
    for ch in raw.trim().chars() {
        let mapped = if matches!(ch, '_' | '-') { ' ' } else { ch };
        if mapped.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space && !out.ends_with(' ') {
            out.push(' ');
        }
        for lower in mapped.to_lowercase() {
            out.push(lower);
        }
        pending_space = false;
    }
    out.trim().to_string()
}

pub(crate) fn alias_surface_matches_prompt(prompt: &str, alias: &str) -> bool {
    let alias = normalized_alias_surface_for_match(alias);
    if alias.is_empty() {
        return false;
    }
    normalized_alias_surface_for_match(prompt).contains(&alias)
}

pub(crate) fn single_alias_binding_mentioned_in_prompt<'a>(
    bindings: &'a [SessionAliasBinding],
    prompt: &str,
) -> Option<&'a SessionAliasBinding> {
    let mut matches = alias_bindings_mentioned_in_prompt(bindings, prompt);
    if matches.is_empty() {
        return None;
    }
    let target = matches[0].target.trim();
    if matches.len() == 1
        || matches
            .iter()
            .all(|binding| binding.target.trim() == target)
    {
        matches.sort_by_key(|binding| {
            std::cmp::Reverse(
                normalized_alias_surface_for_match(&binding.alias)
                    .chars()
                    .count(),
            )
        });
        return Some(matches.remove(0));
    }
    None
}

pub(crate) fn alias_bindings_mentioned_in_prompt<'a>(
    bindings: &'a [SessionAliasBinding],
    prompt: &str,
) -> Vec<&'a SessionAliasBinding> {
    let mut matches = bindings
        .iter()
        .filter(|binding| alias_surface_matches_prompt(prompt, &binding.alias))
        .collect::<Vec<_>>();
    matches.dedup_by(|left, right| left.alias == right.alias && left.target == right.target);
    matches
}

pub(crate) fn session_alias_bindings_from_state_patch(
    state_patch: Option<&Value>,
) -> Vec<SessionAliasBinding> {
    let Some(state_patch) = state_patch else {
        return Vec::new();
    };
    let now_ts = crate::now_ts_u64();
    let mut out = Vec::new();
    if let Some(alias_bindings) = state_patch
        .get("alias_bindings")
        .and_then(|value| value.as_array())
    {
        for item in alias_bindings {
            let Some(alias) = item
                .get("alias")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let Some(target) = item
                .get("target")
                .and_then(|value| value.as_str())
                .and_then(normalize_alias_target)
            else {
                continue;
            };
            if out
                .iter()
                .any(|existing: &SessionAliasBinding| existing.alias.eq_ignore_ascii_case(alias))
            {
                continue;
            }
            out.push(SessionAliasBinding {
                alias: alias.to_string(),
                target,
                updated_at_ts: now_ts,
            });
            if out.len() >= MAX_SESSION_ALIAS_BINDINGS {
                return out;
            }
        }
    }
    let Some(obj) = state_patch.as_object() else {
        return out;
    };
    for (key, value) in obj {
        let alias_and_target = compatibility_alias_key(key)
            .and_then(|alias| {
                compatibility_alias_target(value)
                    .and_then(normalize_alias_target)
                    .map(|target| (alias, target))
            })
            .or_else(|| {
                direct_alias_map_key(key).and_then(|alias| {
                    compatibility_alias_target(value)
                        .and_then(normalize_explicit_alias_target)
                        .map(|target| (alias, target))
                })
            });
        let Some((alias, target)) = alias_and_target else {
            continue;
        };
        if out
            .iter()
            .any(|existing: &SessionAliasBinding| existing.alias.eq_ignore_ascii_case(&alias))
        {
            continue;
        }
        out.push(SessionAliasBinding {
            alias,
            target,
            updated_at_ts: now_ts,
        });
        if out.len() >= MAX_SESSION_ALIAS_BINDINGS {
            break;
        }
    }
    out
}

pub(crate) fn state_patch_is_alias_bindings_only(state_patch: &Value) -> bool {
    let Some(obj) = state_patch.as_object() else {
        return false;
    };
    !obj.is_empty()
        && obj.iter().all(|(key, value)| {
            if !json_value_is_meaningful(value) {
                return true;
            }
            if key == "alias_bindings" {
                return value.as_array().is_some_and(|items| {
                    !items.is_empty()
                        && items.iter().all(|item| {
                            let alias = item
                                .get("alias")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|alias| !alias.is_empty());
                            let target = item
                                .get("target")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|target| !target.is_empty());
                            alias.is_some() && target.is_some()
                        })
                });
            }
            compatibility_alias_key(key).is_some()
                && compatibility_alias_target(value)
                    .and_then(normalize_alias_target)
                    .is_some()
                || direct_alias_map_key(key).is_some()
                    && compatibility_alias_target(value)
                        .and_then(normalize_explicit_alias_target)
                        .is_some()
        })
}

fn json_value_is_meaningful(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => items.iter().any(json_value_is_meaningful),
        Value::Object(map) => map.values().any(json_value_is_meaningful),
        _ => true,
    }
}

fn compatibility_alias_key(key: &str) -> Option<String> {
    let trimmed = key.trim();
    let alias = trimmed
        .strip_suffix("_alias")
        .or_else(|| trimmed.strip_suffix("Alias"))?
        .trim_matches(|ch: char| ch == '_' || ch == '-' || ch.is_whitespace())
        .trim();
    (!alias.is_empty()).then(|| alias.to_string())
}

fn direct_alias_map_key(key: &str) -> Option<String> {
    let trimmed = key.trim();
    if trimmed.is_empty() || state_patch_schema_key(trimmed) {
        return None;
    }
    Some(trimmed.to_string())
}

fn state_patch_schema_key(key: &str) -> bool {
    matches!(
        key,
        "alias_bindings"
            | "active_task_boundary"
            | "audience"
            | "constraints"
            | "deictic_reference"
            | "deliverable"
            | "filename_only"
            | "format"
            | "ordered_entry_ref"
            | "ordered_entry_reference"
            | "output_format"
            | "primary_task_update"
            | "quantity_comparison"
            | "scope"
            | "target"
    )
}

fn compatibility_alias_target(value: &Value) -> Option<&str> {
    if let Some(target) = value.as_str() {
        return Some(target);
    }
    value
        .as_object()
        .and_then(|obj| obj.get("target").or_else(|| obj.get("path")))
        .and_then(Value::as_str)
}

fn merge_alias_bindings(
    prior_state: Option<&ConversationState>,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Vec<SessionAliasBinding> {
    let mut alias_bindings = prior_state
        .map(|state| state.alias_bindings.clone())
        .unwrap_or_default();
    let parsed = session_alias_bindings_from_state_patch(
        turn_analysis.and_then(|analysis| analysis.state_patch.as_ref()),
    );
    if parsed.is_empty() {
        return alias_bindings;
    }
    for binding in parsed {
        alias_bindings.retain(|existing| existing.alias != binding.alias);
        alias_bindings.push(binding);
    }
    if alias_bindings.len() > MAX_SESSION_ALIAS_BINDINGS {
        let start = alias_bindings.len() - MAX_SESSION_ALIAS_BINDINGS;
        alias_bindings = alias_bindings.split_off(start);
    }
    alias_bindings
}

fn merge_alias_bindings_for_turn(
    prior_state: Option<&ConversationState>,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    prompt: &str,
    route_result: &crate::RouteResult,
    resolved_prompt_for_execution: &str,
) -> Vec<SessionAliasBinding> {
    let mut alias_bindings = merge_alias_bindings(prior_state, turn_analysis);
    for binding in structural_alias_bindings_from_prompt(
        prior_state,
        prompt,
        route_result,
        resolved_prompt_for_execution,
    ) {
        alias_bindings.retain(|existing| existing.alias != binding.alias);
        alias_bindings.push(binding);
    }
    if alias_bindings.len() > MAX_SESSION_ALIAS_BINDINGS {
        let start = alias_bindings.len() - MAX_SESSION_ALIAS_BINDINGS;
        alias_bindings = alias_bindings.split_off(start);
    }
    alias_bindings
}

fn structural_alias_bindings_from_prompt(
    prior_state: Option<&ConversationState>,
    prompt: &str,
    route_result: &crate::RouteResult,
    resolved_prompt_for_execution: &str,
) -> Vec<SessionAliasBinding> {
    let mut out = Vec::new();
    if let Some(binding) =
        structural_alias_binding_from_prompt(prompt, route_result, resolved_prompt_for_execution)
    {
        out.push(binding);
    }
    let rebinds = structural_alias_rebinds_from_prompt(prior_state, prompt);
    if !rebinds.is_empty() {
        out.extend(rebinds);
    } else if route_result.should_refresh_long_term_memory {
        out.extend(structural_alias_bindings_from_single_locator_prefix(prompt));
    }
    out
}

pub(crate) fn structural_alias_rebind_from_prompt(
    prior_state: Option<&ConversationState>,
    prompt: &str,
) -> Option<SessionAliasBinding> {
    structural_alias_rebinds_from_prompt(prior_state, prompt)
        .into_iter()
        .next()
}

pub(crate) fn structural_alias_rebinds_from_prompt(
    prior_state: Option<&ConversationState>,
    prompt: &str,
) -> Vec<SessionAliasBinding> {
    let Some(prior) = prior_state else {
        return Vec::new();
    };
    let target = match single_current_prompt_locator_target(prompt) {
        Some(target) if !target.trim().is_empty() => target,
        _ => return Vec::new(),
    };
    let now_ts = crate::now_ts_u64();
    alias_bindings_mentioned_in_prompt(&prior.alias_bindings, prompt)
        .into_iter()
        .filter(|existing| existing.target != target)
        .map(|existing| SessionAliasBinding {
            alias: existing.alias.clone(),
            target: target.clone(),
            updated_at_ts: now_ts,
        })
        .collect()
}

fn structural_alias_bindings_from_single_locator_prefix(prompt: &str) -> Vec<SessionAliasBinding> {
    let Some((surface, target)) = single_current_prompt_locator_surface_and_target(prompt) else {
        return Vec::new();
    };
    let Some(idx) = prompt.find(&surface) else {
        return Vec::new();
    };
    let prefix = prompt[..idx].trim();
    let aliases = alias_suffix_candidates_from_prefix(prefix);
    let now_ts = crate::now_ts_u64();
    aliases
        .into_iter()
        .map(|alias| SessionAliasBinding {
            alias,
            target: target.clone(),
            updated_at_ts: now_ts,
        })
        .collect()
}

fn single_current_prompt_locator_target(prompt: &str) -> Option<String> {
    single_current_prompt_locator_surface_and_target(prompt).map(|(_, target)| target)
}

fn single_current_prompt_locator_surface_and_target(prompt: &str) -> Option<(String, String)> {
    let mut locators =
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(prompt);
    locators.dedup_by(|left, right| left.locator_hint == right.locator_hint);
    if locators.len() != 1 {
        return None;
    }
    let surface = locators.remove(0).locator_hint;
    let target = normalize_alias_target(&surface)?;
    Some((surface, target))
}

fn alias_suffix_candidates_from_prefix(prefix: &str) -> Vec<String> {
    let tokens = prefix
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|ch: char| {
                    ch.is_ascii_punctuation()
                        || matches!(ch, '，' | '。' | '；' | '：' | '“' | '”' | '‘' | '’')
                })
                .trim()
        })
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if tokens.len() < 3 {
        return Vec::new();
    }
    let base = &tokens[..tokens.len() - 1];
    let mut out = Vec::new();
    for len in 2..=base.len().min(4) {
        let candidate = base[base.len() - len..].join(" ");
        if structural_alias_candidate_is_safe(&candidate)
            && !out
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(&candidate))
        {
            out.push(candidate);
        }
    }
    out
}

pub(crate) fn structural_alias_binding_from_prompt(
    prompt: &str,
    route_result: &crate::RouteResult,
    resolved_prompt_for_execution: &str,
) -> Option<SessionAliasBinding> {
    if !route_result.is_chat_gate() || route_result.output_contract.requires_content_evidence {
        return None;
    }
    let alias = single_structural_quoted_alias(prompt)?;
    let target = single_structural_locator_target([
        prompt,
        resolved_prompt_for_execution,
        route_result.resolved_intent.as_str(),
        route_result.output_contract.locator_hint.as_str(),
    ])?;
    Some(SessionAliasBinding {
        alias,
        target,
        updated_at_ts: crate::now_ts_u64(),
    })
}

fn single_structural_quoted_alias(text: &str) -> Option<String> {
    let mut candidates = Vec::new();
    for (open, close) in [('“', '”'), ('"', '"'), ('\'', '\''), ('`', '`')] {
        let mut inside = false;
        let mut start = 0usize;
        for (idx, ch) in text.char_indices() {
            if !inside && ch == open {
                inside = true;
                start = idx + ch.len_utf8();
                continue;
            }
            if inside && ch == close {
                if let Some(candidate) = text
                    .get(start..idx)
                    .map(str::trim)
                    .filter(|candidate| structural_alias_candidate_is_safe(candidate))
                {
                    candidates.push(candidate.to_string());
                }
                inside = false;
            }
        }
    }
    candidates.sort();
    candidates.dedup();
    (candidates.len() == 1).then(|| candidates.remove(0))
}

fn structural_alias_candidate_is_safe(candidate: &str) -> bool {
    let char_count = candidate.chars().count();
    if !(1..=80).contains(&char_count) {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(candidate);
    !surface.has_concrete_locator_hint()
        && crate::intent::locator_extractor::extract_explicit_locator_for_fallback(candidate)
            .is_none()
}

fn single_structural_locator_target<'a>(
    sources: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let mut targets = Vec::new();
    for source in sources {
        let Some(target) =
            crate::intent::locator_extractor::extract_explicit_locator_for_fallback(source)
                .map(|locator| locator.locator_hint)
                .and_then(|target| normalize_alias_target(&target))
        else {
            continue;
        };
        if !targets.iter().any(|existing| existing == &target) {
            targets.push(target);
        }
    }
    (targets.len() == 1).then(|| targets.remove(0))
}

fn next_last_primary_task_prompt(
    prior_state: Option<&ConversationState>,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    journal: &crate::task_journal::TaskJournal,
    prompt: &str,
    resolved_prompt_for_execution: &str,
) -> Option<String> {
    if standalone_preference_or_memory_turn_clears_primary_task(route_result, turn_analysis) {
        return None;
    }
    if should_preserve_active_session_pointers(turn_analysis) {
        return prior_state.and_then(|state| state.last_primary_task_prompt.clone());
    }
    let prior_prompt = prior_state.and_then(|state| state.last_primary_task_prompt.clone());
    if standalone_answer_candidate_request_should_not_promote(
        route_result,
        turn_analysis,
        resolved_prompt_for_execution,
    ) {
        return prior_prompt;
    }
    if standalone_task_request_preserves_prior_primary(
        prior_prompt.as_deref(),
        route_result,
        turn_analysis,
    ) {
        return prior_prompt;
    }
    let user_prompt = prompt.trim();
    let resolved_prompt = resolved_prompt_for_execution.trim();
    let current_prompt = if user_prompt.is_empty() {
        resolved_prompt
    } else {
        user_prompt
    };
    if current_prompt.is_empty() {
        return prior_prompt;
    }
    if unannotated_structured_listing_starts_primary_task(route_result, turn_analysis, journal) {
        return Some(current_prompt.to_string());
    }
    let Some(turn_type) = turn_analysis.and_then(|analysis| analysis.turn_type) else {
        if unannotated_evidence_backed_deliverable_starts_primary_task(route_result, turn_analysis)
        {
            return Some(current_prompt.to_string());
        }
        if standalone_contextual_chat_result_starts_primary_task(route_result, turn_analysis) {
            return Some(current_prompt.to_string());
        }
        if route_result.is_clarify_gate() && prior_prompt.is_none() {
            return Some(current_prompt.to_string());
        }
        return prior_prompt;
    };
    if !is_primary_task_turn_type(turn_type) {
        return prior_prompt;
    }
    if route_result.is_clarify_gate()
        && !should_persist_clarify_primary_task_prompt(route_result, turn_analysis)
    {
        return prior_prompt;
    }
    match turn_type {
        crate::intent_router::TurnType::TaskRequest => Some(current_prompt.to_string()),
        crate::intent_router::TurnType::TaskReplace => Some(current_prompt.to_string()),
        crate::intent_router::TurnType::TaskAppend
        | crate::intent_router::TurnType::TaskCorrect
        | crate::intent_router::TurnType::TaskScopeUpdate => Some(merge_primary_task_prompt(
            prior_prompt.as_deref(),
            current_prompt,
            turn_type,
            turn_analysis.and_then(|analysis| analysis.state_patch.as_ref()),
        )),
        _ => prior_prompt,
    }
}

fn merge_primary_task_prompt(
    prior_prompt: Option<&str>,
    current_prompt: &str,
    turn_type: crate::intent_router::TurnType,
    state_patch: Option<&Value>,
) -> String {
    let prior = prior_prompt
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(prior) = prior else {
        return current_prompt.to_string();
    };
    if prior == current_prompt {
        return prior.to_string();
    }
    let label = match turn_type {
        crate::intent_router::TurnType::TaskCorrect => "Correction",
        crate::intent_router::TurnType::TaskScopeUpdate => "Scope update",
        _ => "Additional instruction",
    };
    let patch = state_patch
        .and_then(render_primary_task_state_patch)
        .map(|patch| format!("\nStructured update: {patch}"))
        .unwrap_or_default();
    if prior.starts_with("Task so far:\n") {
        format!("{prior}\n\n{label}: {current_prompt}{patch}")
    } else {
        format!("Task so far:\n{prior}\n\n{label}: {current_prompt}{patch}")
    }
}

fn render_primary_task_state_patch(state_patch: &Value) -> Option<String> {
    match state_patch {
        Value::Null => None,
        Value::Object(map) if map.is_empty() => None,
        Value::Array(items) if items.is_empty() => None,
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        other => serde_json::to_string(other).ok(),
    }
}

fn is_primary_task_turn_type(turn_type: crate::intent_router::TurnType) -> bool {
    matches!(
        turn_type,
        crate::intent_router::TurnType::TaskRequest
            | crate::intent_router::TurnType::TaskAppend
            | crate::intent_router::TurnType::TaskReplace
            | crate::intent_router::TurnType::TaskCorrect
            | crate::intent_router::TurnType::TaskScopeUpdate
    )
}

fn should_track_primary_task_output(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let Some(turn_type) = turn_analysis.and_then(|analysis| analysis.turn_type) else {
        return false;
    };
    is_primary_task_turn_type(turn_type)
}

fn active_primary_followup_turn(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    matches!(
        (
            turn_analysis.and_then(|analysis| analysis.turn_type),
            turn_analysis.and_then(|analysis| analysis.target_task_policy),
        ),
        (
            Some(
                crate::intent_router::TurnType::TaskAppend
                    | crate::intent_router::TurnType::TaskCorrect
                    | crate::intent_router::TurnType::TaskReplace
                    | crate::intent_router::TurnType::TaskScopeUpdate
            ),
            Some(
                crate::intent_router::TargetTaskPolicy::ReuseActive
                    | crate::intent_router::TargetTaskPolicy::ReplaceActive
            )
        )
    )
}

fn active_primary_non_success_preserves_prior(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    active_primary_followup_turn(turn_analysis)
        && matches!(
            journal.final_status,
            Some(
                crate::task_journal::TaskJournalFinalStatus::Clarify
                    | crate::task_journal::TaskJournalFinalStatus::Failure
                    | crate::task_journal::TaskJournalFinalStatus::ResumeFailure
            )
        )
}

fn model_fallback_preserves_primary_state(
    fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    matches!(
        fallback_source,
        Some(
            crate::fallback::ClarifyFallbackSource::LlmUnavailable
                | crate::fallback::ClarifyFallbackSource::EmptyResponse
        )
    ) && matches!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Clarify)
    )
}

fn prior_last_primary_task_output(prior_state: Option<&ConversationState>) -> Option<String> {
    prior_state.and_then(|state| state.last_primary_task_output.clone())
}

fn standalone_contextual_chat_result_starts_primary_task(
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let allowed_turn = turn_analysis.is_none()
        || matches!(
            (
                turn_analysis.and_then(|analysis| analysis.turn_type),
                turn_analysis.and_then(|analysis| analysis.target_task_policy),
            ),
            (
                Some(crate::intent_router::TurnType::TaskRequest),
                Some(crate::intent_router::TargetTaskPolicy::Standalone)
            )
        );
    if !allowed_turn
        || !route_result.is_chat_gate()
        || route_result.needs_clarify
        || route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
        || !matches!(
            route_result.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
    {
        return false;
    }
    matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::QuantityComparison
            | crate::OutputSemanticKind::RecentScalarEqualityCheck
    )
}

fn unannotated_evidence_backed_deliverable_starts_primary_task(
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    if turn_analysis.is_some()
        || route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
    {
        return false;
    }

    true
}

fn unannotated_structured_listing_starts_primary_task(
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    turn_analysis.is_none()
        && !route_result.needs_clarify
        && route_result.is_execute_gate()
        && !crate::followup_frame::derive_ordered_entries_from_journal(journal).is_empty()
}

fn standalone_preference_or_memory_turn_clears_primary_task(
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    matches!(
        turn_analysis.and_then(|analysis| analysis.turn_type),
        Some(crate::intent_router::TurnType::PreferenceOrMemory)
    ) && !matches!(
        turn_analysis.and_then(|analysis| analysis.target_task_policy),
        Some(
            crate::intent_router::TargetTaskPolicy::ReuseActive
                | crate::intent_router::TargetTaskPolicy::ReplaceActive
        )
    ) && route_result.is_chat_gate()
        && !route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
}

fn state_patch_requests_primary_task_replacement(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let Some(patch) = turn_analysis.and_then(|analysis| analysis.state_patch.as_ref()) else {
        return false;
    };
    let primary_update = patch
        .get("primary_task_update")
        .and_then(|value| value.as_str())
        .map(str::trim);
    let active_boundary = patch
        .get("active_task_boundary")
        .and_then(|value| value.as_str())
        .map(str::trim);
    matches!(primary_update, Some("replace")) || matches!(active_boundary, Some("new_deliverable"))
}

fn standalone_task_request_preserves_prior_primary(
    prior_primary_task_prompt: Option<&str>,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    if standalone_contextual_chat_result_starts_primary_task(route_result, turn_analysis) {
        return false;
    }
    if state_patch_requests_primary_task_replacement(turn_analysis) {
        return false;
    }
    prior_primary_task_prompt
        .map(str::trim)
        .is_some_and(|prompt| !prompt.is_empty())
        && matches!(
            turn_analysis.and_then(|analysis| analysis.turn_type),
            Some(crate::intent_router::TurnType::TaskRequest)
        )
        && matches!(
            turn_analysis.and_then(|analysis| analysis.target_task_policy),
            Some(crate::intent_router::TargetTaskPolicy::Standalone)
        )
        && route_result.is_chat_gate()
        && !route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
}

fn standalone_answer_candidate_request_should_not_promote(
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    resolved_prompt_for_execution: &str,
) -> bool {
    matches!(
        turn_analysis.and_then(|analysis| analysis.turn_type),
        Some(crate::intent_router::TurnType::TaskRequest)
    ) && matches!(
        turn_analysis.and_then(|analysis| analysis.target_task_policy),
        Some(crate::intent_router::TargetTaskPolicy::Standalone)
    ) && route_result.is_chat_gate()
        && !route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
        && (matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        ) || resolved_prompt_for_execution
            .lines()
            .any(|line| line.trim_start().starts_with("answer_candidate:")))
}

fn next_last_primary_task_output(
    prior_state: Option<&ConversationState>,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    journal: &crate::task_journal::TaskJournal,
    resolved_prompt_for_execution: &str,
    answer_text: &str,
    answer_messages: &[String],
) -> Option<String> {
    if standalone_preference_or_memory_turn_clears_primary_task(route_result, turn_analysis) {
        return None;
    }
    if active_primary_non_success_preserves_prior(turn_analysis, journal) {
        return prior_last_primary_task_output(prior_state);
    }
    if should_preserve_active_session_pointers(turn_analysis)
        || route_result.is_clarify_gate()
        || standalone_answer_candidate_request_should_not_promote(
            route_result,
            turn_analysis,
            resolved_prompt_for_execution,
        )
        || standalone_task_request_preserves_prior_primary(
            prior_state.and_then(|state| state.last_primary_task_prompt.as_deref()),
            route_result,
            turn_analysis,
        )
        || !should_track_primary_task_output(turn_analysis)
    {
        if unannotated_evidence_backed_deliverable_starts_primary_task(route_result, turn_analysis)
        {
            let latest_output = answer_text
                .trim()
                .is_empty()
                .then(|| {
                    answer_messages
                        .iter()
                        .map(String::as_str)
                        .find(|text| !text.trim().is_empty())
                        .map(str::to_string)
                })
                .flatten()
                .or_else(|| {
                    let trimmed = answer_text.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                });
            return latest_output.or_else(|| prior_last_primary_task_output(prior_state));
        }
        if unannotated_structured_listing_starts_primary_task(route_result, turn_analysis, journal)
        {
            let latest_output = answer_text
                .trim()
                .is_empty()
                .then(|| {
                    answer_messages
                        .iter()
                        .map(String::as_str)
                        .find(|text| !text.trim().is_empty())
                        .map(str::to_string)
                })
                .flatten()
                .or_else(|| {
                    let trimmed = answer_text.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                });
            return latest_output.or_else(|| prior_last_primary_task_output(prior_state));
        }
        if standalone_contextual_chat_result_starts_primary_task(route_result, turn_analysis) {
            let latest_output = answer_text
                .trim()
                .is_empty()
                .then(|| {
                    answer_messages
                        .iter()
                        .map(String::as_str)
                        .find(|text| !text.trim().is_empty())
                        .map(str::to_string)
                })
                .flatten()
                .or_else(|| {
                    let trimmed = answer_text.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                });
            return latest_output.or_else(|| prior_last_primary_task_output(prior_state));
        }
        return prior_last_primary_task_output(prior_state);
    }
    let latest_output = answer_text
        .trim()
        .is_empty()
        .then(|| {
            answer_messages
                .iter()
                .map(String::as_str)
                .find(|text| !text.trim().is_empty())
                .map(str::to_string)
        })
        .flatten()
        .or_else(|| {
            let trimmed = answer_text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });
    latest_output.or_else(|| prior_last_primary_task_output(prior_state))
}

fn should_persist_clarify_primary_task_prompt(
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    if matches!(
        turn_analysis.and_then(|analysis| analysis.turn_type),
        Some(
            crate::intent_router::TurnType::TaskRequest
                | crate::intent_router::TurnType::TaskAppend
                | crate::intent_router::TurnType::TaskReplace
                | crate::intent_router::TurnType::TaskCorrect
                | crate::intent_router::TurnType::TaskScopeUpdate
        )
    ) {
        return true;
    }
    route_result.is_clarify_gate()
        && matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
        && matches!(
            route_result.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
        && matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
        )
}

fn effective_locale_hint(
    prior_state: Option<&ConversationState>,
    payload: Option<&Value>,
) -> Option<String> {
    normalized_locale_hint(payload).or_else(|| {
        prior_state
            .and_then(|state| state.locale_hint.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}

fn persist_conversation_state(
    state: &AppState,
    task: &ClaimedTask,
    conversation_state: &ConversationState,
) -> Result<()> {
    let db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("acquire db for conversation state persist: {err}"))?;
    let user_key = effective_user_key(task);
    let state_json = serde_json::to_string(conversation_state)?;
    db.execute(
        "INSERT INTO conversation_states (
            user_id, chat_id, user_key, state_json, last_task_id, updated_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(user_id, chat_id, user_key) DO UPDATE SET
            state_json = excluded.state_json,
            last_task_id = excluded.last_task_id,
            updated_at_ts = excluded.updated_at_ts",
        params![
            task.user_id,
            task.chat_id,
            user_key,
            state_json,
            conversation_state.last_task_id,
            conversation_state.updated_at_ts as i64,
        ],
    )?;
    Ok(())
}

fn persist_conversation_state_tx(
    tx: &rusqlite::Transaction<'_>,
    task: &ClaimedTask,
    conversation_state: &ConversationState,
) -> Result<()> {
    let user_key = effective_user_key(task);
    let state_json = serde_json::to_string(conversation_state)?;
    tx.execute(
        "INSERT INTO conversation_states (
            user_id, chat_id, user_key, state_json, last_task_id, updated_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(user_id, chat_id, user_key) DO UPDATE SET
            state_json = excluded.state_json,
            last_task_id = excluded.last_task_id,
            updated_at_ts = excluded.updated_at_ts",
        params![
            task.user_id,
            task.chat_id,
            user_key,
            state_json,
            conversation_state.last_task_id,
            conversation_state.updated_at_ts as i64,
        ],
    )?;
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn load_active_conversation_state(
    state: &AppState,
    task: &ClaimedTask,
) -> Option<ConversationState> {
    let db = state.core.db.get().ok()?;
    let user_key = effective_user_key(task);
    let mut stmt = db
        .prepare(
            "SELECT state_json
             FROM conversation_states
             WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3",
        )
        .ok()?;
    let state_json = stmt
        .query_row(params![task.user_id, task.chat_id, user_key], |row| {
            row.get::<_, String>(0)
        })
        .ok()?;
    serde_json::from_str::<ConversationState>(&state_json).ok()
}

#[allow(dead_code)]
pub(crate) fn replace_active_conversation_state_from_session_snapshot(
    state: &AppState,
    task: &ClaimedTask,
    payload: Option<&Value>,
) {
    let prior_state = load_active_conversation_state(state, task);
    let followup = crate::followup_frame::load_active_followup_frame(state, task);
    let clarify = crate::clarify_state::load_active_clarify_state(state, task);
    let observed_facts = crate::observed_facts::load_active_observed_facts_snapshot(state, task);
    let conversation_state = ConversationState {
        active_followup_task_id: followup.map(|frame| frame.source_task_id),
        active_clarify_task_id: clarify.map(|clarify| clarify.source_task_id),
        active_observed_facts_task_id: observed_facts.map(|(_, source_task_id)| source_task_id),
        alias_bindings: prior_state
            .as_ref()
            .map(|state| state.alias_bindings.clone())
            .unwrap_or_default(),
        last_primary_task_prompt: prior_state
            .as_ref()
            .and_then(|state| state.last_primary_task_prompt.clone()),
        last_primary_task_output: prior_state
            .as_ref()
            .and_then(|state| state.last_primary_task_output.clone()),
        locale_hint: effective_locale_hint(prior_state.as_ref(), payload),
        last_task_id: task.task_id.clone(),
        updated_at_ts: crate::now_ts_u64(),
    };
    if let Err(err) = persist_conversation_state(state, task, &conversation_state) {
        tracing::warn!(
            "conversation_state persist failed task_id={} err={}",
            task.task_id,
            err
        );
    }
}

#[cfg(test)]
pub(crate) fn replace_active_conversation_state_with_pointers(
    state: &AppState,
    task: &ClaimedTask,
    payload: Option<&Value>,
    pointers: ActiveSessionPointers,
) {
    let prior_state = load_active_conversation_state(state, task);
    let conversation_state = ConversationState {
        active_followup_task_id: pointers.active_followup_task_id,
        active_clarify_task_id: pointers.active_clarify_task_id,
        active_observed_facts_task_id: pointers.active_observed_facts_task_id,
        alias_bindings: prior_state
            .as_ref()
            .map(|state| state.alias_bindings.clone())
            .unwrap_or_default(),
        last_primary_task_prompt: prior_state
            .as_ref()
            .and_then(|state| state.last_primary_task_prompt.clone()),
        last_primary_task_output: prior_state
            .as_ref()
            .and_then(|state| state.last_primary_task_output.clone()),
        locale_hint: effective_locale_hint(prior_state.as_ref(), payload),
        last_task_id: task.task_id.clone(),
        updated_at_ts: crate::now_ts_u64(),
    };
    if let Err(err) = persist_conversation_state(state, task, &conversation_state) {
        tracing::warn!(
            "conversation_state persist failed task_id={} err={}",
            task.task_id,
            err
        );
    }
}

pub(crate) fn update_active_session_from_ask_outcome(
    state: &AppState,
    task: &ClaimedTask,
    payload: Option<&Value>,
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    resolved_prompt_for_execution: &str,
    answer_text: &str,
    answer_messages: &[String],
    semantic_clarify: bool,
    fuzzy_locator_suggestions: &[String],
    journal: &crate::task_journal::TaskJournal,
    clarify_fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
) {
    let prior_session_snapshot = load_active_session_snapshot(state, task);
    let prior_state = load_active_conversation_state(state, task);
    let preserve_primary_task_for_clarifying_output =
        active_primary_non_success_preserves_prior(turn_analysis, journal)
            || model_fallback_preserves_primary_state(clarify_fallback_source, journal);
    let mut db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            tracing::warn!(
                "conversation_state tx acquire failed task_id={} err={}",
                task.task_id,
                err
            );
            return;
        }
    };
    let preserve_active_session_pointers = should_preserve_active_session_pointers(turn_analysis)
        || preserve_primary_task_for_clarifying_output;
    if preserve_active_session_pointers {
        tracing::info!(
            "conversation_state preserve_active_session_pointers task_id={} turn_type={}",
            task.task_id,
            turn_analysis
                .and_then(|analysis| analysis.turn_type)
                .map(crate::intent_router::TurnType::as_str)
                .unwrap_or("unknown")
        );
    }
    let tx = match db.transaction() {
        Ok(tx) => tx,
        Err(err) => {
            tracing::warn!(
                "conversation_state begin tx failed task_id={} err={}",
                task.task_id,
                err
            );
            return;
        }
    };
    let result: Result<()> = (|| {
        let (active_followup_task_id, active_clarify_task_id, active_observed_facts_task_id) =
            if preserve_active_session_pointers {
                (
                    prior_state
                        .as_ref()
                        .and_then(|state| state.active_followup_task_id.clone()),
                    prior_state
                        .as_ref()
                        .and_then(|state| state.active_clarify_task_id.clone()),
                    prior_state
                        .as_ref()
                        .and_then(|state| state.active_observed_facts_task_id.clone()),
                )
            } else {
                let active_followup_task_id =
                    crate::followup_frame::sync_active_frame_from_ask_outcome_tx(
                        &tx,
                        task,
                        prompt,
                        route_result,
                        answer_text,
                        answer_messages,
                        semantic_clarify,
                        journal,
                        prior_session_snapshot.active_followup_frame.as_ref(),
                    )?;
                let active_clarify_task_id =
                    crate::clarify_state::sync_active_clarify_state_from_ask_outcome_tx(
                        &tx,
                        task,
                        prompt,
                        route_result,
                        answer_text,
                        answer_messages,
                        semantic_clarify,
                        fuzzy_locator_suggestions,
                        Some(&prior_session_snapshot),
                    )?;
                let active_observed_facts_task_id =
                    crate::observed_facts::sync_active_observed_facts_from_ask_outcome_tx(
                        &tx,
                        task,
                        prompt,
                        route_result,
                        answer_text,
                        answer_messages,
                        journal,
                    )?;
                (
                    active_followup_task_id,
                    active_clarify_task_id,
                    active_observed_facts_task_id,
                )
            };
        let conversation_state = ConversationState {
            active_followup_task_id,
            active_clarify_task_id,
            active_observed_facts_task_id,
            alias_bindings: merge_alias_bindings_for_turn(
                prior_state.as_ref(),
                turn_analysis,
                prompt,
                route_result,
                resolved_prompt_for_execution,
            ),
            last_primary_task_prompt: if preserve_primary_task_for_clarifying_output {
                prior_state
                    .as_ref()
                    .and_then(|state| state.last_primary_task_prompt.clone())
            } else {
                next_last_primary_task_prompt(
                    prior_state.as_ref(),
                    route_result,
                    turn_analysis,
                    journal,
                    prompt,
                    resolved_prompt_for_execution,
                )
            },
            last_primary_task_output: if preserve_primary_task_for_clarifying_output {
                prior_state
                    .as_ref()
                    .and_then(|state| state.last_primary_task_output.clone())
            } else {
                next_last_primary_task_output(
                    prior_state.as_ref(),
                    route_result,
                    turn_analysis,
                    journal,
                    resolved_prompt_for_execution,
                    answer_text,
                    answer_messages,
                )
            },
            locale_hint: effective_locale_hint(prior_state.as_ref(), payload),
            last_task_id: task.task_id.clone(),
            updated_at_ts: crate::now_ts_u64(),
        };
        persist_conversation_state_tx(&tx, task, &conversation_state)?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(err) = result {
        tracing::warn!(
            "conversation_state transactional sync failed task_id={} err={}",
            task.task_id,
            err
        );
    }
}

fn should_preserve_active_session_pointers(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let Some(turn_type) = turn_analysis.and_then(|analysis| analysis.turn_type) else {
        return false;
    };
    matches!(
        turn_type,
        crate::intent_router::TurnType::RunControl
            | crate::intent_router::TurnType::ApprovalDecision
            | crate::intent_router::TurnType::StatusQuery
            | crate::intent_router::TurnType::FeedbackOrError
            | crate::intent_router::TurnType::PreferenceOrMemory
    )
}

fn load_authoritative_followup_frame(
    state: &AppState,
    task: &ClaimedTask,
    expected_task_id: Option<&str>,
) -> Option<crate::followup_frame::FollowupFrame> {
    let frame = crate::followup_frame::load_active_followup_frame(state, task)?;
    match expected_task_id {
        Some(expected) if frame.source_task_id == expected => Some(frame),
        Some(_) => None,
        None => Some(frame),
    }
}

fn load_authoritative_clarify_state(
    state: &AppState,
    task: &ClaimedTask,
    expected_task_id: Option<&str>,
) -> Option<crate::clarify_state::ClarifyState> {
    let clarify_state = crate::clarify_state::load_active_clarify_state(state, task)?;
    match expected_task_id {
        Some(expected) if clarify_state.source_task_id == expected => Some(clarify_state),
        Some(_) => None,
        None => Some(clarify_state),
    }
}

fn load_authoritative_observed_facts(
    state: &AppState,
    task: &ClaimedTask,
    expected_task_id: Option<&str>,
) -> Option<crate::observed_facts::ObservedFacts> {
    let (facts, source_task_id) =
        crate::observed_facts::load_active_observed_facts_snapshot(state, task)?;
    match expected_task_id {
        Some(expected) if source_task_id == expected => Some(facts),
        Some(_) => None,
        None => Some(facts),
    }
}

pub(crate) fn load_active_session_snapshot(
    state: &AppState,
    task: &ClaimedTask,
) -> ActiveSessionSnapshot {
    let conversation_state = load_active_conversation_state(state, task);
    let (active_followup_frame, active_clarify_state, active_observed_facts) =
        if let Some(conversation_state) = conversation_state.as_ref() {
            (
                load_authoritative_followup_frame(
                    state,
                    task,
                    conversation_state.active_followup_task_id.as_deref(),
                ),
                load_authoritative_clarify_state(
                    state,
                    task,
                    conversation_state.active_clarify_task_id.as_deref(),
                ),
                load_authoritative_observed_facts(
                    state,
                    task,
                    conversation_state.active_observed_facts_task_id.as_deref(),
                ),
            )
        } else {
            (
                load_authoritative_followup_frame(state, task, None),
                load_authoritative_clarify_state(state, task, None),
                load_authoritative_observed_facts(state, task, None),
            )
        };
    ActiveSessionSnapshot {
        conversation_state,
        active_followup_frame,
        active_clarify_state,
        active_observed_facts,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        effective_locale_hint, load_active_session_snapshot, normalized_locale_hint,
        ActiveSessionPointers, ActiveSessionSnapshot, ConversationState, SessionAliasBinding,
    };
    use crate::runtime::AppState;
    use crate::ClaimedTask;
    use rusqlite::params;
    use serde_json::json;

    #[test]
    fn locale_hint_prefers_response_language_then_language_then_locale() {
        assert_eq!(
            normalized_locale_hint(Some(
                &json!({"response_language":"en-US","language":"zh-CN"})
            )),
            Some("en-US".to_string())
        );
        assert_eq!(
            normalized_locale_hint(Some(&json!({"language":"zh-CN"}))),
            Some("zh-CN".to_string())
        );
        assert_eq!(
            normalized_locale_hint(Some(&json!({"locale":"en-US"}))),
            Some("en-US".to_string())
        );
        assert_eq!(normalized_locale_hint(Some(&json!({}))), None);
    }

    #[test]
    fn effective_locale_hint_preserves_prior_locale_when_payload_is_empty() {
        let prior_state = ConversationState {
            locale_hint: Some("en-US".to_string()),
            ..ConversationState::default()
        };
        assert_eq!(
            effective_locale_hint(Some(&prior_state), Some(&json!({}))),
            Some("en-US".to_string())
        );
        assert_eq!(
            effective_locale_hint(Some(&prior_state), Some(&json!({"language":"zh-CN"}))),
            Some("zh-CN".to_string())
        );
    }

    #[test]
    fn conversation_state_defaults_are_empty() {
        let state = ConversationState::default();
        assert!(state.active_followup_task_id.is_none());
        assert!(state.active_clarify_task_id.is_none());
        assert!(state.active_observed_facts_task_id.is_none());
        assert!(state.alias_bindings.is_empty());
    }

    #[test]
    fn active_session_snapshot_defaults_to_empty() {
        let snapshot = ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(snapshot.conversation_state.is_none());
        assert!(snapshot.active_followup_frame.is_none());
        assert!(snapshot.active_clarify_state.is_none());
        assert!(snapshot.active_observed_facts.is_none());
    }

    fn route_result_for_test(ask_mode: crate::AskMode, needs_clarify: bool) -> crate::RouteResult {
        crate::RouteResult {
            ask_mode,
            resolved_intent: String::new(),
            needs_clarify,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(0.8),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        }
    }

    fn empty_journal_for_test() -> crate::task_journal::TaskJournal {
        crate::task_journal::TaskJournal::new("test")
    }

    fn journal_with_final_status(
        status: crate::task_journal::TaskJournalFinalStatus,
    ) -> crate::task_journal::TaskJournal {
        let mut journal = crate::task_journal::TaskJournal::new("test");
        journal.record_final_status(status);
        journal
    }

    fn next_last_primary_task_prompt(
        prior_state: Option<&ConversationState>,
        route_result: &crate::RouteResult,
        turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
        prompt: &str,
        resolved_prompt_for_execution: &str,
    ) -> Option<String> {
        super::next_last_primary_task_prompt(
            prior_state,
            route_result,
            turn_analysis,
            &empty_journal_for_test(),
            prompt,
            resolved_prompt_for_execution,
        )
    }

    fn next_last_primary_task_output(
        prior_state: Option<&ConversationState>,
        route_result: &crate::RouteResult,
        turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
        resolved_prompt_for_execution: &str,
        answer_text: &str,
        answer_messages: &[String],
    ) -> Option<String> {
        super::next_last_primary_task_output(
            prior_state,
            route_result,
            turn_analysis,
            &empty_journal_for_test(),
            resolved_prompt_for_execution,
            answer_text,
            answer_messages,
        )
    }

    #[test]
    fn plain_chat_without_task_turn_does_not_promote_primary_task() {
        let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
        let promoted = next_last_primary_task_prompt(
            None,
            &route_result,
            None,
            "刚才记住的编号是什么？",
            "RC-CONT-CN-0428-A",
        );
        assert!(promoted.is_none());

        let prior_state = ConversationState {
            last_primary_task_prompt: Some("帮我写个方案".to_string()),
            ..ConversationState::default()
        };
        let preserved = next_last_primary_task_prompt(
            Some(&prior_state),
            &route_result,
            None,
            "刚才记住的编号是什么？",
            "RC-CONT-CN-0428-A",
        );
        assert_eq!(preserved.as_deref(), Some("帮我写个方案"));
    }

    #[test]
    fn standalone_task_request_preserves_existing_primary_task() {
        let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
        let turn_analysis = crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        };
        let prior_state = ConversationState {
            last_primary_task_prompt: Some("帮我写个方案".to_string()),
            last_primary_task_output: Some("三条登录模块要点".to_string()),
            ..ConversationState::default()
        };

        let prompt = next_last_primary_task_prompt(
            Some(&prior_state),
            &route_result,
            Some(&turn_analysis),
            "问一个独立概念问题",
            "问一个独立概念问题",
        );
        let output = next_last_primary_task_output(
            Some(&prior_state),
            &route_result,
            Some(&turn_analysis),
            "问一个独立概念问题",
            "独立概念回答",
            &[],
        );

        assert_eq!(prompt.as_deref(), Some("帮我写个方案"));
        assert_eq!(output.as_deref(), Some("三条登录模块要点"));
    }

    #[test]
    fn standalone_new_deliverable_replaces_existing_primary_task() {
        let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
        let turn_analysis = crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(json!({
                "primary_task_update": "replace",
                "active_task_boundary": "new_deliverable"
            })),
            attachment_processing_required: false,
        };
        let prior_state = ConversationState {
            last_primary_task_prompt: Some("Write a short release note for RustClaw".to_string()),
            last_primary_task_output: Some(
                "RustClaw is easier for non-technical users.".to_string(),
            ),
            ..ConversationState::default()
        };

        let prompt = next_last_primary_task_prompt(
            Some(&prior_state),
            &route_result,
            Some(&turn_analysis),
            "Write one deployment note that mentions Python 3.10",
            "Write one deployment note that mentions Python 3.10",
        );
        let output = next_last_primary_task_output(
            Some(&prior_state),
            &route_result,
            Some(&turn_analysis),
            "Write one deployment note that mentions Python 3.10",
            "RustClaw deployment should use Python 3.10.",
            &[],
        );

        assert_eq!(
            prompt.as_deref(),
            Some("Write one deployment note that mentions Python 3.10")
        );
        assert_eq!(
            output.as_deref(),
            Some("RustClaw deployment should use Python 3.10.")
        );
    }

    #[test]
    fn standalone_task_request_without_prior_can_start_primary_task() {
        let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
        let turn_analysis = crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        };

        let prompt = next_last_primary_task_prompt(
            None,
            &route_result,
            Some(&turn_analysis),
            "帮我写个方案",
            "帮我写个方案",
        );
        let output = next_last_primary_task_output(
            None,
            &route_result,
            Some(&turn_analysis),
            "帮我写个方案",
            "方案正文",
            &[],
        );

        assert_eq!(prompt.as_deref(), Some("帮我写个方案"));
        assert_eq!(output.as_deref(), Some("方案正文"));
    }

    #[test]
    fn active_task_non_success_preserves_prior_primary_output() {
        let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
        let turn_analysis = crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(json!({"target": "Python 3.10 -> Python 3.11"})),
            attachment_processing_required: false,
        };
        let prior_state = ConversationState {
            last_primary_task_prompt: Some("Write a short release note for RustClaw".to_string()),
            last_primary_task_output: Some(
                "1. Manage settings easily\n2. Track work clearly\n3. Communicate naturally"
                    .to_string(),
            ),
            ..ConversationState::default()
        };
        let journal =
            journal_with_final_status(crate::task_journal::TaskJournalFinalStatus::Clarify);

        assert!(super::active_primary_non_success_preserves_prior(
            Some(&turn_analysis),
            &journal
        ));
        let output = super::next_last_primary_task_output(
            Some(&prior_state),
            &route_result,
            Some(&turn_analysis),
            &journal,
            "Correction: mention Python 3.11, not Python 3.10.",
            "The model is temporarily unavailable.",
            &[],
        );

        assert_eq!(
            output.as_deref(),
            Some("1. Manage settings easily\n2. Track work clearly\n3. Communicate naturally")
        );
    }

    #[test]
    fn model_fallback_preserves_primary_state_from_structured_source() {
        let journal =
            journal_with_final_status(crate::task_journal::TaskJournalFinalStatus::Clarify);

        assert!(super::model_fallback_preserves_primary_state(
            Some(crate::fallback::ClarifyFallbackSource::LlmUnavailable),
            &journal
        ));
        assert!(super::model_fallback_preserves_primary_state(
            Some(crate::fallback::ClarifyFallbackSource::EmptyResponse),
            &journal
        ));
        assert!(!super::model_fallback_preserves_primary_state(
            Some(crate::fallback::ClarifyFallbackSource::IntentUnresolved),
            &journal
        ));
    }

    #[test]
    fn standalone_preference_or_memory_turn_clears_prior_primary_task() {
        let mut route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
        route_result.should_refresh_long_term_memory = true;
        route_result.agent_display_name_hint = "巡检爪".to_string();
        let turn_analysis = crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        };
        let prior_state = ConversationState {
            last_primary_task_prompt: Some(
                "compare README.md and AGENTS.md, tell me which one is larger".to_string(),
            ),
            last_primary_task_output: Some("README.md is larger.".to_string()),
            ..ConversationState::default()
        };

        let prompt = next_last_primary_task_prompt(
            Some(&prior_state),
            &route_result,
            Some(&turn_analysis),
            "后面我要提到你的时候，统一按“巡检爪”这个称呼来",
            "用户要求统一使用“巡检爪”作为称呼",
        );
        let output = next_last_primary_task_output(
            Some(&prior_state),
            &route_result,
            Some(&turn_analysis),
            "用户要求统一使用“巡检爪”作为称呼",
            "已记住：巡检爪。",
            &[],
        );

        assert!(prompt.is_none());
        assert!(output.is_none());
    }

    #[test]
    fn memory_grounded_comparison_chat_becomes_latest_primary_task() {
        let mut route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
        route_result.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
        let prior_state = ConversationState {
            last_primary_task_prompt: Some(
                "再看一下 scripts/nl_tests/fixtures/device_local/logs 目录有多少个直接子项，只输出数字"
                    .to_string(),
            ),
            last_primary_task_output: Some("2".to_string()),
            ..ConversationState::default()
        };

        let prompt = next_last_primary_task_prompt(
            Some(&prior_state),
            &route_result,
            Some(&crate::intent_router::TurnAnalysis {
                turn_type: Some(crate::intent_router::TurnType::TaskRequest),
                target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
                should_interrupt_active_run: false,
                state_patch: None,
                attachment_processing_required: false,
            }),
            "上上个目录和上个目录相比，哪个直接子项更多？只用一句话",
            "比较 docs（3个直接子项）和 logs（2个直接子项）的直接子项数量。",
        );
        let output = next_last_primary_task_output(
            Some(&prior_state),
            &route_result,
            Some(&crate::intent_router::TurnAnalysis {
                turn_type: Some(crate::intent_router::TurnType::TaskRequest),
                target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
                should_interrupt_active_run: false,
                state_patch: None,
                attachment_processing_required: false,
            }),
            "比较 docs（3个直接子项）和 logs（2个直接子项）的直接子项数量。",
            "上上个目录（docs）的直接子项更多，有3个，而上个目录（logs）是2个。",
            &[],
        );

        assert_eq!(
            prompt.as_deref(),
            Some("上上个目录和上个目录相比，哪个直接子项更多？只用一句话")
        );
        assert_eq!(
            output.as_deref(),
            Some("上上个目录（docs）的直接子项更多，有3个，而上个目录（logs）是2个。")
        );
    }

    #[test]
    fn standalone_answer_candidate_request_without_prior_does_not_start_primary_task() {
        let mut route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
        route_result.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        let turn_analysis = crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        };
        let resolved = "查询之前记住的编号\nanswer_candidate: RC-CONT-CN-0428-A";

        let prompt = next_last_primary_task_prompt(
            None,
            &route_result,
            Some(&turn_analysis),
            "刚才让你记住的编号是什么？只回答编号。",
            resolved,
        );
        let output = next_last_primary_task_output(
            None,
            &route_result,
            Some(&turn_analysis),
            resolved,
            "RC-CONT-CN-0428-A",
            &[],
        );

        assert!(prompt.is_none());
        assert!(output.is_none());
    }

    #[test]
    fn standalone_scalar_chat_request_without_answer_marker_does_not_start_primary_task() {
        let mut route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
        route_result.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        let turn_analysis = crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        };
        let resolved = "Answer the continuous test marker, which is RC-CONT-EN-0428-B.";

        let prompt = next_last_primary_task_prompt(
            None,
            &route_result,
            Some(&turn_analysis),
            "What continuous test marker did I ask you to remember? Answer only the marker.",
            resolved,
        );
        let output = next_last_primary_task_output(
            None,
            &route_result,
            Some(&turn_analysis),
            resolved,
            "RC-CONT-EN-0428-B",
            &[],
        );

        assert!(prompt.is_none());
        assert!(output.is_none());
    }

    #[test]
    fn evidence_backed_standalone_task_replaces_prior_scalar_primary_task() {
        let mut route_result =
            route_result_for_test(crate::AskMode::planner_execute_chat_wrapped(), false);
        route_result.output_contract.requires_content_evidence = true;
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        let turn_analysis = crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        };
        let prior_state = ConversationState {
            last_primary_task_prompt: Some(
                "What continuous test marker did I ask you to remember?".to_string(),
            ),
            last_primary_task_output: Some("RC-CONT-EN-0428-B".to_string()),
            ..ConversationState::default()
        };

        let prompt = next_last_primary_task_prompt(
            Some(&prior_state),
            &route_result,
            Some(&turn_analysis),
            "Write a short release note for RustClaw.",
            "Write a short release note for RustClaw.",
        );
        let output = next_last_primary_task_output(
            Some(&prior_state),
            &route_result,
            Some(&turn_analysis),
            "Write a short release note for RustClaw.",
            "RustClaw 0.1.7 is now available.",
            &[],
        );

        assert_eq!(
            prompt.as_deref(),
            Some("Write a short release note for RustClaw.")
        );
        assert_eq!(output.as_deref(), Some("RustClaw 0.1.7 is now available."));
    }

    #[test]
    fn unannotated_evidence_backed_deliverable_starts_primary_task() {
        let mut route_result =
            route_result_for_test(crate::AskMode::planner_execute_chat_wrapped(), false);
        route_result.output_contract.requires_content_evidence = true;
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;

        let prompt = next_last_primary_task_prompt(
            None,
            &route_result,
            None,
            "Write a short release note for RustClaw",
            "Write a short release note for RustClaw",
        );
        let output = next_last_primary_task_output(
            None,
            &route_result,
            None,
            "Write a short release note for RustClaw",
            "RustClaw 0.1.7 is easier to update and operate.",
            &[],
        );

        assert_eq!(
            prompt.as_deref(),
            Some("Write a short release note for RustClaw")
        );
        assert_eq!(
            output.as_deref(),
            Some("RustClaw 0.1.7 is easier to update and operate.")
        );
    }

    #[test]
    fn unannotated_structured_listing_replaces_prior_primary_task() {
        let route_result = route_result_for_test(crate::AskMode::planner_execute_plain(), false);
        let mut journal = crate::task_journal::TaskJournal::new("list");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "fs_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    serde_json::json!({
                        "action": "inventory_dir",
                        "resolved_path": "/tmp/logs",
                        "names": ["act_plan.log", "clawd.log", "clawd.run.log"]
                    })
                    .to_string(),
                ),
                ..Default::default()
            });
        let prior_state = ConversationState {
            last_primary_task_prompt: Some("先列出 document 目录下前 5 个文件名".to_string()),
            last_primary_task_output: Some(
                "builtin_write_smoke.txt\nfull_suite_trace_note.txt".to_string(),
            ),
            ..ConversationState::default()
        };

        let prompt = super::next_last_primary_task_prompt(
            Some(&prior_state),
            &route_result,
            None,
            &journal,
            "那 logs 目录下前 5 个文件名呢",
            "列出 logs 目录下前 5 个文件名",
        );
        let output = super::next_last_primary_task_output(
            Some(&prior_state),
            &route_result,
            None,
            &journal,
            "列出 logs 目录下前 5 个文件名",
            "act_plan.log\nclawd.log\nclawd.run.log",
            &[],
        );

        assert_eq!(prompt.as_deref(), Some("那 logs 目录下前 5 个文件名呢"));
        assert_eq!(
            output.as_deref(),
            Some("act_plan.log\nclawd.log\nclawd.run.log")
        );
    }

    #[test]
    fn unannotated_scalar_evidence_result_does_not_start_primary_task() {
        let mut route_result =
            route_result_for_test(crate::AskMode::planner_execute_chat_wrapped(), false);
        route_result.output_contract.requires_content_evidence = true;
        route_result.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route_result.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;

        let prompt = next_last_primary_task_prompt(
            None,
            &route_result,
            None,
            "Count files under logs",
            "Count files under logs",
        );
        let output = next_last_primary_task_output(
            None,
            &route_result,
            None,
            "Count files under logs",
            "2",
            &[],
        );

        assert!(prompt.is_none());
        assert!(output.is_none());
    }

    #[test]
    fn task_append_persists_compact_primary_without_runtime_envelope() {
        let route_result = route_result_for_test(crate::AskMode::direct_answer(), false);
        let prior_state = ConversationState {
            last_primary_task_prompt: Some("帮我写个方案".to_string()),
            ..ConversationState::default()
        };
        let persisted = next_last_primary_task_prompt(
            Some(&prior_state),
            &route_result,
            Some(&crate::intent_router::TurnAnalysis {
                turn_type: Some(crate::intent_router::TurnType::TaskAppend),
                target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
                should_interrupt_active_run: false,
                state_patch: Some(json!({"audience":"boss"})),
                attachment_processing_required: false,
            }),
            "面向老板",
            "Current task:\n帮我写个方案\n\nKeep the same task...",
        )
        .expect("primary prompt");
        assert!(persisted.contains("帮我写个方案"));
        assert!(persisted.contains("面向老板"));
        assert!(persisted.contains("\"audience\":\"boss\""));
        assert!(!persisted.contains("Continuity rules"));
        assert!(!persisted.contains("Current task:"));
    }

    #[test]
    fn repeated_task_append_keeps_single_task_so_far_header() {
        let persisted = super::merge_primary_task_prompt(
            Some("Task so far:\n帮我写个方案\n\nAdditional instruction: 面向老板"),
            "不要太技术",
            crate::intent_router::TurnType::TaskAppend,
            None,
        );
        assert_eq!(persisted.matches("Task so far:").count(), 1);
        assert!(persisted.contains("Additional instruction: 面向老板"));
        assert!(persisted.contains("Additional instruction: 不要太技术"));
    }

    #[test]
    fn authoritative_snapshot_filters_components_by_task_ids() {
        let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        let task = ClaimedTask {
            task_id: "task-2".to_string(),
            user_id: 7,
            chat_id: 9,
            user_key: Some("user-key".to_string()),
            channel: "telegram".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        };
        {
            let db = state.core.db.get().expect("db");
            db.execute(
                "INSERT INTO followup_frames (
                    user_id, chat_id, user_key, frame_json, source_task_id, updated_at_ts, expires_at_ts
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    task.user_id,
                    task.chat_id,
                    "user-key",
                    serde_json::to_string(&crate::followup_frame::FollowupFrame {
                        source_request: "read file".to_string(),
                        source_task_id: "task-old".to_string(),
                        updated_at_ts: crate::now_ts_u64(),
                        expires_at_ts: crate::now_ts_u64() + 60,
                        ..crate::followup_frame::FollowupFrame::default()
                    })
                    .expect("frame json"),
                    "task-old",
                    crate::now_ts_u64() as i64,
                    (crate::now_ts_u64() + 60) as i64,
                ],
            )
            .expect("insert followup");
            db.execute(
                "INSERT INTO conversation_states (
                    user_id, chat_id, user_key, state_json, last_task_id, updated_at_ts
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    task.user_id,
                    task.chat_id,
                    "user-key",
                    serde_json::to_string(&ConversationState {
                        active_followup_task_id: Some("task-2".to_string()),
                        active_clarify_task_id: None,
                        active_observed_facts_task_id: None,
                        alias_bindings: Vec::new(),
                        last_primary_task_prompt: None,
                        last_primary_task_output: None,
                        locale_hint: None,
                        last_task_id: "task-2".to_string(),
                        updated_at_ts: crate::now_ts_u64(),
                    })
                    .expect("conversation state json"),
                    "task-2",
                    crate::now_ts_u64() as i64,
                ],
            )
            .expect("insert conversation state");
        }

        let snapshot = load_active_session_snapshot(&state, &task);
        assert!(snapshot.active_followup_frame.is_none());
    }

    #[test]
    fn replace_active_conversation_state_with_pointers_persists_ids() {
        let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        let task = ClaimedTask {
            task_id: "task-3".to_string(),
            user_id: 11,
            chat_id: 12,
            user_key: Some("user-key".to_string()),
            channel: "telegram".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        };
        super::replace_active_conversation_state_with_pointers(
            &state,
            &task,
            Some(&json!({"response_language":"en-US"})),
            ActiveSessionPointers {
                active_followup_task_id: Some("task-f".to_string()),
                active_clarify_task_id: Some("task-c".to_string()),
                active_observed_facts_task_id: Some("task-o".to_string()),
            },
        );
        let loaded = super::load_active_conversation_state(&state, &task).expect("state");
        assert_eq!(loaded.active_followup_task_id.as_deref(), Some("task-f"));
        assert_eq!(loaded.active_clarify_task_id.as_deref(), Some("task-c"));
        assert_eq!(
            loaded.active_observed_facts_task_id.as_deref(),
            Some("task-o")
        );
        assert_eq!(loaded.locale_hint.as_deref(), Some("en-US"));
    }

    #[test]
    fn merge_alias_bindings_prefers_structured_state_patch() {
        let prior = ConversationState {
            alias_bindings: vec![SessionAliasBinding {
                alias: "那个文件".to_string(),
                target: "/tmp/old.md".to_string(),
                updated_at_ts: 1,
            }],
            ..ConversationState::default()
        };
        let turn_analysis = crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: None,
            should_interrupt_active_run: false,
            state_patch: Some(json!({
                "alias_bindings": [
                    {"alias": "那个文件", "target": "/tmp/new.md"},
                    {"alias": "那个日志", "target": "/tmp/app.log"}
                ]
            })),
            attachment_processing_required: false,
        };
        let merged = super::merge_alias_bindings(Some(&prior), Some(&turn_analysis));
        assert_eq!(merged.len(), 2);
        assert!(merged
            .iter()
            .any(|binding| binding.alias == "那个文件" && binding.target == "/tmp/new.md"));
        assert!(merged.iter().any(|binding| {
            binding.alias == "那个日志" && binding.target == "/tmp/app.log"
        }));
        assert!(!merged
            .iter()
            .any(|binding| binding.target == "/tmp/regex.md"));
    }

    #[test]
    fn merge_alias_bindings_accepts_alias_key_compatibility_patch() {
        let turn_analysis = crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: None,
            should_interrupt_active_run: false,
            state_patch: Some(json!({
                "that_file_alias": "/tmp/device/README.md",
                "currentFolderAlias": {"target": "/tmp/device/docs"}
            })),
            attachment_processing_required: false,
        };

        let merged = super::merge_alias_bindings(None, Some(&turn_analysis));

        assert_eq!(merged.len(), 2);
        assert!(merged.iter().any(
            |binding| binding.alias == "that_file" && binding.target == "/tmp/device/README.md"
        ));
        assert!(merged.iter().any(
            |binding| binding.alias == "currentFolder" && binding.target == "/tmp/device/docs"
        ));
    }

    #[test]
    fn alias_surface_match_accepts_user_defined_separator_variants() {
        let bindings = vec![SessionAliasBinding {
            alias: "note_file".to_string(),
            target: "/tmp/release_checklist.md".to_string(),
            updated_at_ts: 1,
        }];

        let matched = super::single_alias_binding_mentioned_in_prompt(
            &bindings,
            "What does the note file refer to?",
        )
        .expect("alias should match across separator variants");

        assert_eq!(matched.target, "/tmp/release_checklist.md");
    }

    #[test]
    fn memory_turn_with_single_locator_derives_short_alias_suffixes() {
        let mut route = route_result_for_test(crate::AskMode::direct_answer(), false);
        route.should_refresh_long_term_memory = true;
        let merged = super::merge_alias_bindings_for_turn(
            None,
            None,
            "Remember that the note file means scripts/nl_tests/fixtures/device_local/docs/service_notes.md. Reply only confirmed.",
            &route,
            "",
        );

        assert!(merged.iter().any(|binding| {
            binding.alias == "note file"
                && binding.target == "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
        }));
    }

    #[test]
    fn current_locator_rebinds_existing_alias_without_language_phrase_table() {
        let prior = ConversationState {
            alias_bindings: vec![SessionAliasBinding {
                alias: "note file".to_string(),
                target: "scripts/nl_tests/fixtures/device_local/docs/service_notes.md".to_string(),
                updated_at_ts: 1,
            }],
            ..ConversationState::default()
        };

        let binding = super::structural_alias_rebind_from_prompt(
            Some(&prior),
            "Correction: the note file now means scripts/nl_tests/fixtures/device_local/docs/release_checklist.md. Reply only updated.",
        )
        .expect("existing alias should rebind to the current locator");

        assert_eq!(binding.alias, "note file");
        assert_eq!(
            binding.target,
            "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
        );
    }

    #[test]
    fn current_locator_rebinds_all_mentioned_alias_surfaces() {
        let prior = ConversationState {
            alias_bindings: vec![
                SessionAliasBinding {
                    alias: "note file".to_string(),
                    target: "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
                        .to_string(),
                    updated_at_ts: 1,
                },
                SessionAliasBinding {
                    alias: "the note file".to_string(),
                    target: "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
                        .to_string(),
                    updated_at_ts: 1,
                },
            ],
            ..ConversationState::default()
        };

        let bindings = super::structural_alias_rebinds_from_prompt(
            Some(&prior),
            "Correction: the note file now means scripts/nl_tests/fixtures/device_local/docs/release_checklist.md. Reply only updated.",
        );

        assert_eq!(bindings.len(), 2);
        assert!(bindings.iter().all(|binding| {
            binding.target == "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
        }));
    }

    #[test]
    fn state_patch_accepts_path_like_direct_alias_map() {
        let patch = json!({
            "甲文件": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
        });

        assert!(super::state_patch_is_alias_bindings_only(&patch));
        let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].alias, "甲文件");
        assert_eq!(
            bindings[0].target,
            "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
        );
    }

    #[test]
    fn state_patch_rejects_non_locator_direct_alias_map() {
        let patch = json!({
            "甲文件": "the checklist from before"
        });

        assert!(!super::state_patch_is_alias_bindings_only(&patch));
        assert!(super::session_alias_bindings_from_state_patch(Some(&patch)).is_empty());
    }

    #[test]
    fn structural_prompt_alias_binding_uses_quote_and_single_locator() {
        let mut route = route_result_for_test(crate::AskMode::direct_answer(), false);
        route.risk_ceiling = crate::RiskCeiling::Low;

        let binding = super::structural_alias_binding_from_prompt(
            "先记一下，后面我说“那个文件”就是 /tmp/device/README.md",
            &route,
            "remember that quoted alias maps to /tmp/device/README.md",
        )
        .expect("binding");

        assert_eq!(binding.alias, "那个文件");
        assert_eq!(binding.target, "/tmp/device/README.md");
    }

    #[test]
    fn merge_alias_bindings_ignores_prompt_text_without_structured_patch() {
        let prior = ConversationState {
            alias_bindings: vec![SessionAliasBinding {
                alias: "那个文件".to_string(),
                target: "/tmp/old.md".to_string(),
                updated_at_ts: 1,
            }],
            ..ConversationState::default()
        };
        let turn_analysis = crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: None,
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        };
        let merged = super::merge_alias_bindings(Some(&prior), Some(&turn_analysis));
        assert_eq!(merged, prior.alias_bindings);
    }

    #[test]
    fn meta_turn_types_preserve_active_session_pointers() {
        for turn_type in [
            crate::intent_router::TurnType::RunControl,
            crate::intent_router::TurnType::ApprovalDecision,
            crate::intent_router::TurnType::StatusQuery,
            crate::intent_router::TurnType::FeedbackOrError,
            crate::intent_router::TurnType::PreferenceOrMemory,
        ] {
            assert!(super::should_preserve_active_session_pointers(Some(
                &crate::intent_router::TurnAnalysis {
                    turn_type: Some(turn_type),
                    target_task_policy: None,
                    should_interrupt_active_run: false,
                    state_patch: None,
                    attachment_processing_required: false,
                }
            )));
        }
        assert!(!super::should_preserve_active_session_pointers(Some(
            &crate::intent_router::TurnAnalysis {
                turn_type: Some(crate::intent_router::TurnType::TaskAppend),
                target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
                should_interrupt_active_run: false,
                state_patch: None,
                attachment_processing_required: false,
            }
        )));
    }

    #[test]
    fn clarify_task_request_persists_primary_prompt_for_followups() {
        let mut route_result = route_result_for_test(crate::AskMode::clarify(), true);
        route_result.resolved_intent = "帮我写个方案".to_string();
        route_result.clarify_question = "请补充主题".to_string();
        route_result.route_reason = "clarify".to_string();
        let persisted = next_last_primary_task_prompt(
            None,
            &route_result,
            Some(&crate::intent_router::TurnAnalysis {
                turn_type: Some(crate::intent_router::TurnType::TaskRequest),
                target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
                should_interrupt_active_run: false,
                state_patch: None,
                attachment_processing_required: false,
            }),
            "帮我写个方案",
            "帮我写个方案",
        );
        assert_eq!(persisted.as_deref(), Some("帮我写个方案"));
    }

    #[test]
    fn clarify_task_prompt_without_turn_analysis_is_preserved_when_not_locator_driven() {
        let mut route_result = route_result_for_test(crate::AskMode::clarify(), true);
        route_result.resolved_intent = "Help me write a proposal".to_string();
        route_result.clarify_question = "What is the topic and audience?".to_string();
        route_result.route_reason = "missing_task_slots".to_string();
        let persisted = next_last_primary_task_prompt(
            None,
            &route_result,
            None,
            "Help me write a proposal",
            "Help me write a proposal",
        );
        assert_eq!(persisted.as_deref(), Some("Help me write a proposal"));
    }
}
