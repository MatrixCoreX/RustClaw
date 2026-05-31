use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};

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

#[cfg(test)]
#[allow(dead_code)]
fn persist_observed_facts(
    state: &AppState,
    task: &ClaimedTask,
    observed_facts: &ObservedFacts,
) -> Result<()> {
    let db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("acquire db for observed facts persist: {err}"))?;
    let user_key = effective_user_key(task);
    let facts_json = serde_json::to_string(observed_facts)?;
    let now_ts = crate::now_ts_u64();
    let expires_at_ts = now_ts + OBSERVED_FACTS_TTL_SECS;
    db.execute(
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

#[allow(dead_code)]
pub(crate) fn load_active_observed_facts(
    state: &AppState,
    task: &ClaimedTask,
) -> Option<ObservedFacts> {
    load_active_observed_facts_snapshot(state, task).map(|(facts, _)| facts)
}

#[allow(dead_code)]
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

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn replace_active_observed_facts_from_ask_outcome(
    state: &AppState,
    task: &ClaimedTask,
    route_result: &crate::RouteResult,
    answer_text: &str,
    answer_messages: &[String],
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    let observed_facts =
        derive_observed_facts_from_ask_outcome(answer_text, answer_messages, journal, route_result);
    let result = if observed_facts.is_empty() {
        clear_active_observed_facts(state, task)
    } else {
        persist_observed_facts(state, task, &observed_facts)
    };
    if let Err(err) = result {
        tracing::warn!(
            "observed_facts persist failed task_id={} err={}",
            task.task_id,
            err
        );
        return None;
    }
    (!observed_facts.is_empty()).then(|| task.task_id.clone())
}

pub(crate) fn sync_active_observed_facts_from_ask_outcome_tx(
    tx: &rusqlite::Transaction<'_>,
    task: &ClaimedTask,
    _prompt: &str,
    route_result: &crate::RouteResult,
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
    route_result: &crate::RouteResult,
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
    let may_capture_ordered_entries = route_contract_can_publish_ordered_entries(route_result)
        || !journal_ordered_entries.is_empty()
        || combined_contains_delivery_file_token(&combined);
    let mut ordered_entries = if may_capture_ordered_entries {
        let entries = crate::followup_frame::extract_ordered_entries_from_text(&combined);
        if entries.is_empty() {
            journal_ordered_entries
        } else {
            entries
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

    let bound_target = crate::followup_frame::derive_bound_target_from_journal(journal)
        .or_else(|| {
            crate::followup_frame::derive_bound_target_from_answer(answer_text, answer_messages)
        })
        .or_else(|| {
            let hint = route_result.output_contract.locator_hint.trim();
            (!hint.is_empty()).then(|| hint.to_string())
        });
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
        delivery_targets,
    }
}

fn route_contract_can_publish_ordered_entries(route_result: &crate::RouteResult) -> bool {
    route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::SqliteTableListing
                | crate::OutputSemanticKind::SqliteTableNamesOnly
        )
        || matches!(
            route_result.output_contract.delivery_intent,
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

#[cfg(test)]
#[path = "observed_facts_tests.rs"]
mod tests;
