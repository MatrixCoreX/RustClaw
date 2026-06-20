use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{AppState, ClaimedTask};

const CLARIFY_STATE_TTL_SECS: u64 = 30 * 60;
const CLARIFY_STATE_RESOLVED_INTENT_MARKER: &str = "[RESOLVED_INTENT]";
const STRUCTURED_FIELD_SELECTOR_TOKEN_PREFIX: &str = "structured_field_selector=";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ClarifyMissingSlot {
    Locator,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ClarifyState {
    pub(crate) missing_slot: ClarifyMissingSlot,
    pub(crate) pending_question: String,
    pub(crate) candidate_targets: Vec<String>,
    pub(crate) delivery_required: bool,
    pub(crate) output_shape: Option<String>,
    pub(crate) semantic_kind: Option<String>,
    pub(crate) source_request: String,
    pub(crate) source_task_id: String,
    pub(crate) updated_at_ts: u64,
    pub(crate) expires_at_ts: u64,
}

fn effective_user_key(task: &ClaimedTask) -> String {
    task.user_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("anon:{}:{}", task.user_id, task.chat_id))
}

fn persist_clarify_state_tx(
    tx: &rusqlite::Transaction<'_>,
    task: &ClaimedTask,
    clarify_state: &ClarifyState,
) -> Result<()> {
    let user_key = effective_user_key(task);
    let state_json = serde_json::to_string(clarify_state)?;
    tx.execute(
        "INSERT INTO clarify_states (
            user_id, chat_id, user_key, state_json, source_task_id, updated_at_ts, expires_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(user_id, chat_id, user_key) DO UPDATE SET
            state_json = excluded.state_json,
            source_task_id = excluded.source_task_id,
            updated_at_ts = excluded.updated_at_ts,
            expires_at_ts = excluded.expires_at_ts",
        params![
            task.user_id,
            task.chat_id,
            user_key,
            state_json,
            clarify_state.source_task_id,
            clarify_state.updated_at_ts as i64,
            clarify_state.expires_at_ts as i64,
        ],
    )?;
    Ok(())
}

pub(crate) fn clear_active_clarify_state(state: &AppState, task: &ClaimedTask) -> Result<()> {
    let db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("acquire db for clarify state clear: {err}"))?;
    let user_key = effective_user_key(task);
    db.execute(
        "DELETE FROM clarify_states
         WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3",
        params![task.user_id, task.chat_id, user_key],
    )?;
    Ok(())
}

fn clear_active_clarify_state_tx(tx: &rusqlite::Transaction<'_>, task: &ClaimedTask) -> Result<()> {
    let user_key = effective_user_key(task);
    tx.execute(
        "DELETE FROM clarify_states
         WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3",
        params![task.user_id, task.chat_id, user_key],
    )?;
    Ok(())
}

pub(crate) fn load_active_clarify_state(
    state: &AppState,
    task: &ClaimedTask,
) -> Option<ClarifyState> {
    let db = state.core.db.get().ok()?;
    let user_key = effective_user_key(task);
    let mut stmt = db
        .prepare(
            "SELECT state_json
             FROM clarify_states
             WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3",
        )
        .ok()?;
    let state_json = stmt
        .query_row(params![task.user_id, task.chat_id, user_key], |row| {
            row.get::<_, String>(0)
        })
        .ok()?;
    let clarify_state = serde_json::from_str::<ClarifyState>(&state_json).ok()?;
    if clarify_state.expires_at_ts <= crate::now_ts_u64() {
        let _ = clear_active_clarify_state(state, task);
        return None;
    }
    Some(clarify_state)
}

fn clarify_question_from_answer(answer_text: &str, answer_messages: &[String]) -> Option<String> {
    answer_messages
        .iter()
        .map(|message| message.trim())
        .find(|message| !message.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            let answer = answer_text.trim();
            (!answer.is_empty()).then(|| answer.to_string())
        })
}

fn derive_clarify_state_for_ask_outcome(
    task_id: &str,
    prompt: &str,
    route_result: &crate::RouteResult,
    answer_text: &str,
    answer_messages: &[String],
    semantic_clarify: bool,
    fuzzy_locator_suggestions: &[String],
    prior_session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> Option<ClarifyState> {
    if !semantic_clarify {
        return None;
    }
    let pending_question =
        clarify_question_from_answer(answer_text, answer_messages).or_else(|| {
            let route_question = route_result.clarify_question.trim();
            (!route_question.is_empty()).then(|| route_question.to_string())
        })?;
    let candidate_targets =
        derive_clarify_candidate_targets(fuzzy_locator_suggestions, prior_session_snapshot);
    let now_ts = crate::now_ts_u64();
    let semantic_kind = clarify_state_semantic_kind(route_result);
    Some(ClarifyState {
        missing_slot: ClarifyMissingSlot::Locator,
        pending_question,
        candidate_targets,
        delivery_required: route_result.wants_file_delivery
            || route_result.output_contract.delivery_required
            || matches!(
                route_result.output_contract.response_shape,
                crate::OutputResponseShape::FileToken
            ),
        output_shape: (!matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Free
        ))
        .then(|| {
            route_result
                .output_contract
                .response_shape
                .as_str()
                .to_string()
        }),
        semantic_kind,
        source_request: clarify_state_source_request(prompt, route_result),
        source_task_id: task_id.to_string(),
        updated_at_ts: now_ts,
        expires_at_ts: now_ts + CLARIFY_STATE_TTL_SECS,
    })
}

fn clarify_state_source_request(prompt: &str, route_result: &crate::RouteResult) -> String {
    let source = prompt.trim();
    let resolved = route_result.resolved_intent.trim();
    let request = if resolved.is_empty() || resolved == source {
        source.to_string()
    } else {
        format!("{source}\n{CLARIFY_STATE_RESOLVED_INTENT_MARKER}\n{resolved}")
    };
    append_structured_field_selector_token(
        request,
        route_result
            .output_contract
            .self_extension
            .structured_field_selector
            .as_deref(),
    )
}

fn append_structured_field_selector_token(mut text: String, selector: Option<&str>) -> String {
    let Some(selector) = selector.and_then(normalize_structured_field_selector_token) else {
        return text;
    };
    let token = format!("{STRUCTURED_FIELD_SELECTOR_TOKEN_PREFIX}{selector}");
    if text.split_whitespace().any(|part| part == token) {
        return text;
    }
    if !text.ends_with(char::is_whitespace) && !text.is_empty() {
        text.push(' ');
    }
    text.push_str(&token);
    text
}

pub(crate) fn structured_field_selector_token_from_text(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|part| {
        part.strip_prefix(STRUCTURED_FIELD_SELECTOR_TOKEN_PREFIX)
            .and_then(normalize_structured_field_selector_token)
    })
}

pub(crate) fn normalize_structured_field_selector_token(raw: &str) -> Option<String> {
    let selector = raw.trim();
    if selector.is_empty()
        || selector.chars().count() > 256
        || selector.chars().any(char::is_control)
        || selector.chars().any(char::is_whitespace)
        || selector.contains('\\')
        || selector.contains("://")
        || selector.starts_with('{')
        || selector.starts_with('[')
        || selector.ends_with('}')
        || selector.ends_with(']')
    {
        return None;
    }
    if selector.starts_with('/') {
        return selector
            .split('/')
            .skip(1)
            .all(|segment| !segment.trim().is_empty())
            .then(|| selector.to_string());
    }
    selector
        .chars()
        .all(|ch| {
            ch.is_ascii_alphanumeric()
                || matches!(ch, '_' | '-' | '$' | '@' | '.' | '/' | '*' | '[' | ']')
        })
        .then(|| selector.to_string())
}

fn clarify_state_semantic_kind(route_result: &crate::RouteResult) -> Option<String> {
    if !matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    ) {
        return Some(
            route_result
                .output_contract
                .semantic_kind
                .as_str()
                .to_string(),
        );
    }
    let non_content_locator_probe = !route_result.wants_file_delivery
        && !route_result.output_contract.delivery_required
        && !route_result.output_contract.requires_content_evidence
        && matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        )
        && matches!(
            route_result.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        );
    if non_content_locator_probe {
        return Some(
            crate::OutputSemanticKind::ExistenceWithPath
                .as_str()
                .to_string(),
        );
    }
    None
}

fn derive_clarify_candidate_targets(
    fuzzy_locator_suggestions: &[String],
    prior_session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> Vec<String> {
    let mut candidates = prior_session_snapshot
        .and_then(|snapshot| snapshot.active_followup_frame.as_ref())
        .map(|frame| frame.ordered_entries.clone())
        .filter(|entries| !entries.is_empty())
        .or_else(|| {
            prior_session_snapshot
                .and_then(|snapshot| snapshot.active_observed_facts.as_ref())
                .map(|facts| {
                    if !facts.ordered_entries.is_empty() {
                        facts.ordered_entries.clone()
                    } else if facts.delivery_targets.len() >= 2 {
                        facts.delivery_targets.clone()
                    } else {
                        Vec::new()
                    }
                })
                .filter(|entries| !entries.is_empty())
        })
        .or_else(|| {
            (!fuzzy_locator_suggestions.is_empty()).then(|| fuzzy_locator_suggestions.to_vec())
        })
        .unwrap_or_default();
    if candidates.is_empty() {
        if let Some(bound_target) = prior_session_snapshot
            .and_then(|snapshot| snapshot.active_observed_facts.as_ref())
            .and_then(|facts| facts.bound_target.clone())
        {
            candidates.push(bound_target);
        }
    }
    let mut deduped = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if deduped.iter().any(|existing| existing == &candidate) {
            continue;
        }
        deduped.push(candidate);
    }
    let mut candidates = deduped;
    candidates.truncate(crate::followup_frame::MAX_ORDERED_ENTRIES);
    candidates
}

pub(crate) fn sync_active_clarify_state_from_ask_outcome_tx(
    tx: &rusqlite::Transaction<'_>,
    task: &ClaimedTask,
    prompt: &str,
    route_result: &crate::RouteResult,
    answer_text: &str,
    answer_messages: &[String],
    semantic_clarify: bool,
    fuzzy_locator_suggestions: &[String],
    prior_session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> Result<Option<String>> {
    let Some(clarify_state) = derive_clarify_state_for_ask_outcome(
        &task.task_id,
        prompt,
        route_result,
        answer_text,
        answer_messages,
        semantic_clarify,
        fuzzy_locator_suggestions,
        prior_session_snapshot,
    ) else {
        clear_active_clarify_state_tx(tx, task)?;
        return Ok(None);
    };
    persist_clarify_state_tx(tx, task, &clarify_state)?;
    Ok(Some(clarify_state.source_task_id))
}

#[cfg(test)]
#[path = "clarify_state_tests.rs"]
mod tests;
