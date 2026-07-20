use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::{AppState, ClaimedTask};

const OBSERVED_FACTS_TTL_SECS: u64 = 30 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct ObservedFacts {
    pub(crate) bound_target: Option<String>,
    pub(crate) ordered_entries: Vec<String>,
    pub(crate) selected_entry_index: Option<usize>,
    pub(crate) observed_entry_count: Option<usize>,
    pub(crate) slice_spec: Option<crate::followup_frame::FollowupSliceSpec>,
    pub(crate) output_shape: Option<String>,
    pub(crate) delivery_targets: Vec<String>,
}

impl ObservedFacts {
    pub(crate) fn is_empty(&self) -> bool {
        self.bound_target.is_none()
            && self.ordered_entries.is_empty()
            && self.selected_entry_index.is_none()
            && self.observed_entry_count.is_none()
            && self.slice_spec.is_none()
            && self.delivery_targets.is_empty()
    }
}

fn effective_user_key(task: &ClaimedTask) -> String {
    task.user_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("anon:{}:{}", task.user_id, task.chat_id))
}

fn persist_observed_facts_tx(
    tx: &rusqlite::Transaction<'_>,
    task: &ClaimedTask,
    observed_facts: &ObservedFacts,
) -> Result<()> {
    let user_key = effective_user_key(task);
    let facts_json = serde_json::to_string(observed_facts)?;
    let now_ts = crate::now_ts_u64();
    let expires_at_ts = now_ts + OBSERVED_FACTS_TTL_SECS;
    tx.execute(
        "INSERT INTO observed_facts_states (
            user_id, chat_id, user_key, facts_json, source_task_id, updated_at_ts, expires_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(user_id, chat_id, user_key) DO UPDATE SET
            facts_json = excluded.facts_json,
            source_task_id = excluded.source_task_id,
            updated_at_ts = excluded.updated_at_ts,
            expires_at_ts = excluded.expires_at_ts",
        params![
            task.user_id,
            task.chat_id,
            user_key,
            facts_json,
            task.task_id,
            now_ts as i64,
            expires_at_ts as i64,
        ],
    )?;
    Ok(())
}

pub(crate) fn clear_active_observed_facts(state: &AppState, task: &ClaimedTask) -> Result<()> {
    let db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("acquire db for observed facts clear: {err}"))?;
    let user_key = effective_user_key(task);
    db.execute(
        "DELETE FROM observed_facts_states
         WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3",
        params![task.user_id, task.chat_id, user_key],
    )?;
    Ok(())
}

fn clear_active_observed_facts_tx(
    tx: &rusqlite::Transaction<'_>,
    task: &ClaimedTask,
) -> Result<()> {
    let user_key = effective_user_key(task);
    tx.execute(
        "DELETE FROM observed_facts_states
         WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3",
        params![task.user_id, task.chat_id, user_key],
    )?;
    Ok(())
}

pub(crate) fn load_active_observed_facts_snapshot(
    state: &AppState,
    task: &ClaimedTask,
) -> Option<(ObservedFacts, String)> {
    let db = state.core.db.get().ok()?;
    let user_key = effective_user_key(task);
    let mut stmt = db
        .prepare(
            "SELECT facts_json, source_task_id, expires_at_ts
             FROM observed_facts_states
             WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3",
        )
        .ok()?;
    let (facts_json, source_task_id, expires_at_ts) = stmt
        .query_row(params![task.user_id, task.chat_id, user_key], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .ok()?;
    if expires_at_ts <= crate::now_ts_u64() as i64 {
        let _ = clear_active_observed_facts(state, task);
        return None;
    }
    serde_json::from_str::<ObservedFacts>(&facts_json)
        .ok()
        .filter(|facts| !facts.is_empty())
        .map(|facts| (facts, source_task_id))
}

pub(crate) fn sync_active_observed_facts_from_ask_outcome_tx(
    tx: &rusqlite::Transaction<'_>,
    task: &ClaimedTask,
    _prompt: &str,
    route_result: &crate::IntentOutputContract,
    answer_text: &str,
    answer_messages: &[String],
    journal: &crate::task_journal::TaskJournal,
) -> Result<Option<String>> {
    let observed_facts =
        derive_observed_facts_from_ask_outcome(answer_text, answer_messages, journal, route_result);
    if observed_facts.is_empty() {
        clear_active_observed_facts_tx(tx, task)?;
        return Ok(None);
    }
    persist_observed_facts_tx(tx, task, &observed_facts)?;
    Ok(Some(task.task_id.clone()))
}

pub(crate) fn derive_observed_facts_from_ask_outcome(
    answer_text: &str,
    answer_messages: &[String],
    journal: &crate::task_journal::TaskJournal,
    route_result: &crate::IntentOutputContract,
) -> ObservedFacts {
    let mut combined = answer_text.trim().to_string();
    let publishable_messages = answer_messages
        .iter()
        .filter(|message| !crate::finalize::is_execution_summary_message(message))
        .collect::<Vec<_>>();
    if !publishable_messages.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(
            &publishable_messages
                .iter()
                .map(|message| message.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }

    let journal_ordered_entries =
        crate::followup_frame::derive_ordered_entries_from_journal(journal);
    let visible_ordered_entries =
        crate::followup_frame::extract_ordered_entries_from_text(&combined);
    let may_capture_ordered_entries = route_contract_can_publish_ordered_entries(route_result)
        || !journal_ordered_entries.is_empty()
        || combined_contains_delivery_file_token(&combined);
    let visible_entries_are_structural =
        crate::followup_frame::ordered_entries_are_structural_tokens(&visible_ordered_entries);
    let mut ordered_entries = if may_capture_ordered_entries || visible_entries_are_structural {
        if visible_ordered_entries.is_empty() {
            journal_ordered_entries
        } else {
            visible_ordered_entries
        }
    } else {
        Vec::new()
    };
    ordered_entries.truncate(crate::followup_frame::MAX_ORDERED_ENTRIES);

    let mut delivery_targets = crate::extract_delivery_file_tokens(answer_text)
        .into_iter()
        .filter_map(|token| crate::delivery_utils::extract_file_path_from_delivery_token(&token))
        .collect::<Vec<_>>();
    for message in publishable_messages {
        delivery_targets.extend(
            crate::extract_delivery_file_tokens(message)
                .into_iter()
                .filter_map(|token| {
                    crate::delivery_utils::extract_file_path_from_delivery_token(&token)
                }),
        );
    }
    delivery_targets.sort();
    delivery_targets.dedup();

    let bound_target = route_allows_observed_bound_target(route_result)
        .then(|| {
            crate::followup_frame::derive_bound_target_from_journal(journal)
                .or_else(|| {
                    crate::followup_frame::derive_bound_target_from_answer(
                        answer_text,
                        answer_messages,
                    )
                })
                .or_else(|| scalar_path_answer_bound_target(&combined, route_result))
                .or_else(|| {
                    let hint = route_result.locator_hint.trim();
                    (!hint.is_empty()).then(|| hint.to_string())
                })
        })
        .flatten();
    let selected_entry_index = bound_target.as_deref().and_then(|target| {
        crate::followup_frame::selected_entry_index_for_target(
            &crate::followup_frame::FollowupFrame {
                bound_target: bound_target.clone(),
                ordered_entries: ordered_entries.clone(),
                ..crate::followup_frame::FollowupFrame::default()
            },
            target,
        )
    });

    let observed_entry_count = (!ordered_entries.is_empty()).then_some(ordered_entries.len());

    ObservedFacts {
        bound_target,
        ordered_entries,
        selected_entry_index,
        observed_entry_count,
        slice_spec: crate::followup_frame::derive_slice_spec_from_journal(journal),
        output_shape: (!matches!(
            route_result.response_shape,
            crate::OutputResponseShape::Free
        ))
        .then(|| route_result.response_shape.as_str().to_string()),
        delivery_targets,
    }
}

pub(crate) fn route_allows_observed_bound_target(
    route_result: &crate::IntentOutputContract,
) -> bool {
    !route_uses_non_binding_workspace_evidence(route_result)
}

fn route_uses_non_binding_workspace_evidence(route_result: &crate::IntentOutputContract) -> bool {
    route_result.semantic_kind_is(crate::OutputSemanticKind::WorkspaceProjectSummary)
}

fn route_contract_can_publish_ordered_entries(route_result: &crate::IntentOutputContract) -> bool {
    route_result.delivery_required
        || route_result.delivery_required
        || route_result.semantic_kind_is_any(&[
            crate::OutputSemanticKind::FileNames,
            crate::OutputSemanticKind::DirectoryNames,
            crate::OutputSemanticKind::DirectoryEntryGroups,
            crate::OutputSemanticKind::FilePaths,
        ])
        || matches!(
            route_result.delivery_intent,
            crate::OutputDeliveryIntent::DirectoryLookup
                | crate::OutputDeliveryIntent::DirectoryBatchFiles
        )
}

fn combined_contains_delivery_file_token(combined: &str) -> bool {
    combined
        .lines()
        .map(str::trim_start)
        .any(|line| line.starts_with("FILE:"))
}

fn scalar_path_answer_bound_target(
    combined: &str,
    route_result: &crate::IntentOutputContract,
) -> Option<String> {
    if !scalar_path_contract_can_bind_target(route_result) {
        return None;
    }
    let mut lines = combined
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let line = lines.next()?;
    if lines.next().is_some() {
        return None;
    }
    normalize_scalar_path_bound_target(
        line,
        route_result.semantic_kind_is(crate::OutputSemanticKind::ScalarPathOnly),
    )
}

fn scalar_path_contract_can_bind_target(route_result: &crate::IntentOutputContract) -> bool {
    route_result.semantic_kind_is(crate::OutputSemanticKind::ScalarPathOnly)
        || (matches!(
            route_result.response_shape,
            crate::OutputResponseShape::Scalar
        ) && route_result.requires_content_evidence
            && matches!(
                route_result.locator_kind,
                crate::OutputLocatorKind::Path
                    | crate::OutputLocatorKind::Filename
                    | crate::OutputLocatorKind::CurrentWorkspace
            ))
}

fn normalize_scalar_path_bound_target(
    candidate: &str,
    allow_single_component: bool,
) -> Option<String> {
    let candidate = candidate.trim().trim_matches(|ch: char| {
        matches!(
            ch,
            '`' | '"' | '\'' | '“' | '”' | '‘' | '’' | '<' | '>' | '《' | '》'
        )
    });
    if candidate.is_empty()
        || candidate.starts_with("FILE:")
        || candidate.starts_with("http://")
        || candidate.starts_with("https://")
        || candidate.starts_with('{')
        || candidate.starts_with('[')
        || candidate.chars().any(char::is_control)
    {
        return None;
    }
    let path = Path::new(candidate);
    if path.is_absolute() || candidate.contains('/') || candidate.contains('\\') {
        return Some(candidate.to_string());
    }
    if allow_single_component
        && path.components().count() == 1
        && candidate.contains('.')
        && !candidate.chars().any(char::is_whitespace)
    {
        return Some(candidate.to_string());
    }
    None
}

#[cfg(test)]
#[path = "observed_facts_tests.rs"]
mod tests;
