use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{AppState, ClaimedTask};

const CLARIFY_STATE_TTL_SECS: u64 = 30 * 60;

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

#[cfg(test)]
#[allow(dead_code)]
fn persist_clarify_state(
    state: &AppState,
    task: &ClaimedTask,
    clarify_state: &ClarifyState,
) -> Result<()> {
    let db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("acquire db for clarify state persist: {err}"))?;
    let user_key = effective_user_key(task);
    let state_json = serde_json::to_string(clarify_state)?;
    db.execute(
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

#[allow(dead_code)]
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
        semantic_kind: (!matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
        ))
        .then(|| {
            route_result
                .output_contract
                .semantic_kind
                .as_str()
                .to_string()
        }),
        source_request: prompt.trim().to_string(),
        source_task_id: task_id.to_string(),
        updated_at_ts: now_ts,
        expires_at_ts: now_ts + CLARIFY_STATE_TTL_SECS,
    })
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

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn replace_active_clarify_state_from_ask_outcome(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
    route_result: &crate::RouteResult,
    answer_text: &str,
    answer_messages: &[String],
    semantic_clarify: bool,
    fuzzy_locator_suggestions: &[String],
    prior_session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> Option<String> {
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
        if let Err(err) = clear_active_clarify_state(state, task) {
            tracing::warn!(
                "clarify_state clear failed task_id={} err={}",
                task.task_id,
                err
            );
        }
        return None;
    };
    if let Err(err) = persist_clarify_state(state, task, &clarify_state) {
        tracing::warn!(
            "clarify_state persist failed task_id={} err={}",
            task.task_id,
            err
        );
        return None;
    }
    Some(clarify_state.source_task_id)
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
mod tests {
    use super::{
        clarify_question_from_answer, derive_clarify_candidate_targets,
        derive_clarify_state_for_ask_outcome, ClarifyMissingSlot,
    };

    #[test]
    fn locator_question_prefers_matching_answer_text() {
        let question = clarify_question_from_answer("LOCATOR_CLARIFY_PROMPT", &[])
            .expect("question should be extracted");
        assert_eq!(question, "LOCATOR_CLARIFY_PROMPT");
    }

    #[test]
    fn derive_locator_clarify_state_from_semantic_clarify() {
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::clarify(),
            resolved_intent: "看一下那个日志最后 5 行".to_string(),
            needs_clarify: true,
            clarify_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
            route_reason: "clarify".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                requires_content_evidence: true,
                locator_kind: crate::OutputLocatorKind::Path,
                ..crate::IntentOutputContract::default()
            },
        };
        let clarify_state = derive_clarify_state_for_ask_outcome(
            "task-1",
            "看一下那个日志最后 5 行",
            &route,
            "LOCATOR_CLARIFY_PROMPT",
            &[],
            true,
            &[],
            None,
        )
        .expect("clarify state should be derived");
        assert_eq!(clarify_state.missing_slot, ClarifyMissingSlot::Locator);
        assert!(!clarify_state.delivery_required);
        assert_eq!(clarify_state.output_shape, None);
        assert_eq!(clarify_state.semantic_kind, None);
        assert_eq!(clarify_state.source_request, "看一下那个日志最后 5 行");
    }

    #[test]
    fn derive_locator_clarify_state_preserves_non_free_output_shape() {
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::clarify(),
            resolved_intent: "看一下那个日志".to_string(),
            needs_clarify: true,
            clarify_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
            route_reason: "clarify".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
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
                locator_kind: crate::OutputLocatorKind::Path,
                ..crate::IntentOutputContract::default()
            },
        };
        let clarify_state = derive_clarify_state_for_ask_outcome(
            "task-2",
            "看一下那个日志",
            &route,
            "LOCATOR_CLARIFY_PROMPT",
            &[],
            true,
            &[],
            None,
        )
        .expect("clarify state should be derived");
        assert_eq!(
            clarify_state.output_shape.as_deref(),
            Some(crate::OutputResponseShape::OneSentence.as_str())
        );
        assert_eq!(clarify_state.semantic_kind, None);
    }

    #[test]
    fn clarify_candidate_targets_prefer_prior_observed_entries() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: Some(crate::observed_facts::ObservedFacts {
                ordered_entries: vec![
                    "README.md".to_string(),
                    "deploy.md".to_string(),
                    "README.md".to_string(),
                ],
                ..crate::observed_facts::ObservedFacts::default()
            }),
        };
        let candidates = derive_clarify_candidate_targets(&[], Some(&snapshot));
        assert_eq!(
            candidates,
            vec!["README.md".to_string(), "deploy.md".to_string()]
        );
    }

    #[test]
    fn clarify_candidate_targets_preserve_observed_entry_order() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: Some(crate::observed_facts::ObservedFacts {
                ordered_entries: vec![
                    "deploy.md".to_string(),
                    "README.md".to_string(),
                    "deploy.md".to_string(),
                ],
                ..crate::observed_facts::ObservedFacts::default()
            }),
        };
        let candidates = derive_clarify_candidate_targets(&[], Some(&snapshot));
        assert_eq!(
            candidates,
            vec!["deploy.md".to_string(), "README.md".to_string()]
        );
    }

    #[test]
    fn clarify_candidate_targets_fall_back_to_structured_fuzzy_locator_candidates() {
        let candidates = derive_clarify_candidate_targets(
            &[
                "/tmp/a/Cargo.toml".to_string(),
                "/tmp/b/Cargo.toml".to_string(),
            ],
            None,
        );
        assert_eq!(
            candidates,
            vec![
                "/tmp/a/Cargo.toml".to_string(),
                "/tmp/b/Cargo.toml".to_string()
            ]
        );
    }

    #[test]
    fn derive_clarify_state_seeds_candidate_targets_from_prior_session() {
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::clarify(),
            resolved_intent: "把那个文件发给我".to_string(),
            needs_clarify: true,
            clarify_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
            route_reason: "clarify".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: true,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::FileToken,
                delivery_required: true,
                locator_kind: crate::OutputLocatorKind::Path,
                ..crate::IntentOutputContract::default()
            },
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: Some(crate::observed_facts::ObservedFacts {
                ordered_entries: vec!["act_plan.log".to_string(), "clawd.log".to_string()],
                ..crate::observed_facts::ObservedFacts::default()
            }),
        };
        let clarify_state = derive_clarify_state_for_ask_outcome(
            "task-3",
            "把那个文件发给我",
            &route,
            "LOCATOR_CLARIFY_PROMPT",
            &[],
            true,
            &[],
            Some(&snapshot),
        )
        .expect("clarify state should be derived");
        assert_eq!(
            clarify_state.candidate_targets,
            vec!["act_plan.log".to_string(), "clawd.log".to_string()]
        );
    }
}
