use anyhow::Result;
use regex::Regex;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

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

fn alias_binding_prefix_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)(?:先记一下|再记一下|记一下|记住|remember this|remember that|note that)"#)
            .expect("valid alias binding prefix regex")
    })
}

fn quoted_alias_binding_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"[\"“'`](?P<alias>[^\"”'`]+)[\"”'`]\s*(?:就是|指向|指|是|改成|means|is|becomes)\s*(?P<target>[^，,；;。]+)"#,
        )
        .expect("valid quoted alias binding regex")
    })
}

fn bare_alias_binding_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)(?P<alias>that [a-z0-9][a-z0-9 _/\-]*|那个[^，,；;。 ]+|这(?:个|份)[^，,；;。 ]+)\s*(?:就是|指向|指|是|改成)\s*(?P<target>[^，,；;。]+)"#,
        )
        .expect("valid bare alias binding regex")
    })
}

fn parse_session_alias_bindings(prompt: &str) -> Vec<SessionAliasBinding> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() || !alias_binding_prefix_re().is_match(trimmed) {
        return Vec::new();
    }
    let now_ts = crate::now_ts_u64();
    let mut out = Vec::new();
    let mut occupied_ranges = Vec::new();
    for captures in quoted_alias_binding_re().captures_iter(trimmed) {
        let Some(alias) = captures.name("alias") else {
            continue;
        };
        let Some(target) = captures.name("target") else {
            continue;
        };
        if let Some(full_match) = captures.get(0) {
            occupied_ranges.push(full_match.range());
        }
        let alias = alias.as_str().trim();
        let Some(target) = normalize_alias_target(target.as_str()) else {
            continue;
        };
        if alias.is_empty() {
            continue;
        }
        out.push(SessionAliasBinding {
            alias: alias.to_string(),
            target,
            updated_at_ts: now_ts,
        });
    }
    for captures in bare_alias_binding_re().captures_iter(trimmed) {
        let Some(full_match) = captures.get(0) else {
            continue;
        };
        if occupied_ranges
            .iter()
            .any(|occupied| full_match.start() < occupied.end && occupied.start < full_match.end())
        {
            continue;
        }
        let Some(alias) = captures.name("alias") else {
            continue;
        };
        let Some(target) = captures.name("target") else {
            continue;
        };
        let alias = alias.as_str().trim();
        let Some(target) = normalize_alias_target(target.as_str()) else {
            continue;
        };
        if alias.is_empty()
            || out
                .iter()
                .any(|existing| existing.alias.eq_ignore_ascii_case(alias))
        {
            continue;
        }
        out.push(SessionAliasBinding {
            alias: alias.to_string(),
            target,
            updated_at_ts: now_ts,
        });
    }
    out.truncate(MAX_SESSION_ALIAS_BINDINGS);
    out
}

fn binding_only_prompt_tail(prompt: &str) -> String {
    let mut tail = quoted_alias_binding_re()
        .replace_all(prompt, "")
        .to_string();
    tail = bare_alias_binding_re().replace_all(&tail, "").to_string();
    tail = alias_binding_prefix_re().replace_all(&tail, "").to_string();
    for marker in ["后面我说", "后面说", "以后我说", "以后说"] {
        tail = tail.replace(marker, "");
    }
    tail.trim_matches(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                ',' | '，' | '.' | '。' | ';' | '；' | ':' | '：' | '、' | '(' | ')' | '（' | '）'
            )
    })
    .to_string()
}

pub(crate) fn prompt_is_binding_only_session_alias_definition(prompt: &str) -> bool {
    let bindings = parse_session_alias_bindings(prompt);
    if bindings.is_empty() {
        return false;
    }
    let tail = binding_only_prompt_tail(prompt);
    tail.is_empty()
        || matches!(
            tail.as_str(),
            "," | "，" | "." | "。" | ";" | "；" | ":" | "："
        )
}

fn merge_alias_bindings(
    prior_state: Option<&ConversationState>,
    prompt: &str,
) -> Vec<SessionAliasBinding> {
    let mut alias_bindings = prior_state
        .map(|state| state.alias_bindings.clone())
        .unwrap_or_default();
    let parsed = parse_session_alias_bindings(prompt);
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

fn next_last_primary_task_prompt(
    prior_state: Option<&ConversationState>,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    resolved_prompt_for_execution: &str,
) -> Option<String> {
    if should_preserve_active_session_pointers(turn_analysis) {
        return prior_state.and_then(|state| state.last_primary_task_prompt.clone());
    }
    let trimmed = resolved_prompt_for_execution.trim();
    if trimmed.is_empty() {
        return prior_state.and_then(|state| state.last_primary_task_prompt.clone());
    }
    if route_result.is_clarify_gate()
        && !should_persist_clarify_primary_task_prompt(route_result, turn_analysis)
    {
        return prior_state.and_then(|state| state.last_primary_task_prompt.clone());
    }
    Some(trimmed.to_string())
}

fn next_last_primary_task_output(
    prior_state: Option<&ConversationState>,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    answer_text: &str,
    answer_messages: &[String],
) -> Option<String> {
    if should_preserve_active_session_pointers(turn_analysis) || route_result.is_clarify_gate() {
        return prior_state.and_then(|state| state.last_primary_task_output.clone());
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
    latest_output.or_else(|| prior_state.and_then(|state| state.last_primary_task_output.clone()))
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

pub(crate) fn session_alias_target_for_prompt(
    prompt: &str,
    session_snapshot: Option<&ActiveSessionSnapshot>,
) -> Option<String> {
    if prompt.trim().is_empty() || prompt_is_binding_only_session_alias_definition(prompt) {
        return None;
    }
    let bindings = session_snapshot
        .and_then(|snapshot| snapshot.conversation_state.as_ref())
        .map(|state| state.alias_bindings.as_slice())
        .unwrap_or(&[]);
    let mut sorted = bindings.iter().collect::<Vec<_>>();
    sorted.sort_by(|a, b| b.alias.chars().count().cmp(&a.alias.chars().count()));
    sorted
        .into_iter()
        .find(|binding| prompt.contains(&binding.alias))
        .map(|binding| binding.target.clone())
}

pub(crate) fn rewrite_prompt_with_alias_bindings(
    prompt: &str,
    session_snapshot: Option<&ActiveSessionSnapshot>,
) -> Option<String> {
    if prompt.trim().is_empty() || prompt_is_binding_only_session_alias_definition(prompt) {
        return None;
    }
    let bindings = session_snapshot
        .and_then(|snapshot| snapshot.conversation_state.as_ref())
        .map(|state| state.alias_bindings.as_slice())
        .unwrap_or(&[]);
    if bindings.is_empty() {
        return None;
    }
    let mut rewritten = prompt.to_string();
    let mut matched = false;
    let mut sorted = bindings.iter().collect::<Vec<_>>();
    sorted.sort_by(|a, b| b.alias.chars().count().cmp(&a.alias.chars().count()));
    for binding in sorted {
        if rewritten.contains(&binding.alias) {
            rewritten = rewritten.replace(
                &binding.alias,
                &spaced_alias_target_substitution(&binding.target),
            );
            matched = true;
        }
    }
    matched.then_some(rewritten.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn spaced_alias_target_substitution(target: &str) -> String {
    format!(" {target} ")
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
    journal: &crate::task_journal::TaskJournal,
) {
    let prior_session_snapshot = load_active_session_snapshot(state, task);
    let prior_state = load_active_conversation_state(state, task);
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
    let preserve_active_session_pointers = should_preserve_active_session_pointers(turn_analysis);
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
            alias_bindings: merge_alias_bindings(prior_state.as_ref(), prompt),
            last_primary_task_prompt: next_last_primary_task_prompt(
                prior_state.as_ref(),
                route_result,
                turn_analysis,
                resolved_prompt_for_execution,
            ),
            last_primary_task_output: next_last_primary_task_output(
                prior_state.as_ref(),
                route_result,
                turn_analysis,
                answer_text,
                answer_messages,
            ),
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
        effective_locale_hint, load_active_session_snapshot, next_last_primary_task_prompt,
        normalized_locale_hint, prompt_is_binding_only_session_alias_definition,
        rewrite_prompt_with_alias_bindings, session_alias_target_for_prompt, ActiveSessionPointers,
        ActiveSessionSnapshot, ConversationState, SessionAliasBinding,
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
    fn binding_only_session_alias_definition_is_detected() {
        assert!(prompt_is_binding_only_session_alias_definition(
            "先记一下，后面我说“那个文件”就是 /tmp/README.md"
        ));
        assert!(!prompt_is_binding_only_session_alias_definition(
            "先记一下，后面我说“那个文件”就是 /tmp/README.md，然后读一下它开头"
        ));
    }

    #[test]
    fn rewrite_prompt_with_alias_bindings_rewrites_bound_aliases() {
        let snapshot = ActiveSessionSnapshot {
            conversation_state: Some(ConversationState {
                alias_bindings: vec![
                    SessionAliasBinding {
                        alias: "那个文件".to_string(),
                        target: "/tmp/README.md".to_string(),
                        updated_at_ts: 1,
                    },
                    SessionAliasBinding {
                        alias: "甲".to_string(),
                        target: "/tmp/a.md".to_string(),
                        updated_at_ts: 2,
                    },
                ],
                ..ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let rewritten = rewrite_prompt_with_alias_bindings(
            "把那个文件开头读 10 行，然后顺手说甲是干什么的",
            Some(&snapshot),
        )
        .expect("rewrite");
        assert!(rewritten.contains("/tmp/README.md"));
        assert!(rewritten.contains("/tmp/a.md"));
        assert!(rewritten.contains("把 /tmp/README.md 开头读 10 行"));
    }

    #[test]
    fn session_alias_target_for_prompt_finds_matching_binding() {
        let snapshot = ActiveSessionSnapshot {
            conversation_state: Some(ConversationState {
                alias_bindings: vec![SessionAliasBinding {
                    alias: "那个日志".to_string(),
                    target: "/tmp/app.log".to_string(),
                    updated_at_ts: 1,
                }],
                ..ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert_eq!(
            session_alias_target_for_prompt("看一下那个日志最近 20 行", Some(&snapshot)),
            Some("/tmp/app.log".to_string())
        );
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
        let route_result = crate::RouteResult {
            routed_mode: crate::RoutedMode::AskClarify,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::AskClarify),
            resolved_intent: "帮我写个方案".to_string(),
            needs_clarify: true,
            clarify_question: "请补充主题".to_string(),
            route_reason: "clarify".to_string(),
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
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
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
        );
        assert_eq!(persisted.as_deref(), Some("帮我写个方案"));
    }

    #[test]
    fn clarify_task_prompt_without_turn_analysis_is_preserved_when_not_locator_driven() {
        let route_result = crate::RouteResult {
            routed_mode: crate::RoutedMode::AskClarify,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::AskClarify),
            resolved_intent: "Help me write a proposal".to_string(),
            needs_clarify: true,
            clarify_question: "What is the topic and audience?".to_string(),
            route_reason: "missing_task_slots".to_string(),
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
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let persisted =
            next_last_primary_task_prompt(None, &route_result, None, "Help me write a proposal");
        assert_eq!(persisted.as_deref(), Some("Help me write a proposal"));
    }
}
