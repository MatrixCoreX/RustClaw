use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

use crate::{AppState, ClaimedTask};

const FOLLOWUP_FRAME_TTL_SECS: u64 = 30 * 60;
pub(crate) const MAX_ORDERED_ENTRIES: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FollowupOpKind {
    #[default]
    Generic,
    Read,
    List,
    Delivery,
    ClarifyPending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FollowupUnresolvedSlot {
    Locator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FollowupSliceKind {
    Head,
    Tail,
    Range,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FollowupSliceSpec {
    pub(crate) kind: FollowupSliceKind,
    pub(crate) n: Option<usize>,
    pub(crate) start_line: Option<usize>,
    pub(crate) end_line: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct FollowupFrame {
    pub(crate) source_request: String,
    pub(crate) op_kind: FollowupOpKind,
    pub(crate) bound_target: Option<String>,
    pub(crate) ordered_entries: Vec<String>,
    pub(crate) selected_entry_index: Option<usize>,
    pub(crate) slice_spec: Option<FollowupSliceSpec>,
    pub(crate) output_shape: Option<String>,
    pub(crate) unresolved_slot: Option<FollowupUnresolvedSlot>,
    pub(crate) source_task_id: String,
    pub(crate) updated_at_ts: u64,
    pub(crate) expires_at_ts: u64,
}

impl FollowupFrame {
    fn is_expired(&self, now_ts: u64) -> bool {
        self.expires_at_ts <= now_ts
    }

    fn can_accept_locator_reply(&self) -> bool {
        (matches!(self.op_kind, FollowupOpKind::ClarifyPending)
            || self.unresolved_slot == Some(FollowupUnresolvedSlot::Locator))
            && !self.source_request.trim().is_empty()
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
fn persist_frame(state: &AppState, task: &ClaimedTask, frame: &FollowupFrame) -> Result<()> {
    let db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("acquire db for followup frame persist: {err}"))?;
    let user_key = effective_user_key(task);
    let frame_json = serde_json::to_string(frame)?;
    db.execute(
        "INSERT INTO followup_frames (
            user_id, chat_id, user_key, frame_json, source_task_id, updated_at_ts, expires_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(user_id, chat_id, user_key) DO UPDATE SET
            frame_json = excluded.frame_json,
            source_task_id = excluded.source_task_id,
            updated_at_ts = excluded.updated_at_ts,
            expires_at_ts = excluded.expires_at_ts",
        params![
            task.user_id,
            task.chat_id,
            user_key,
            frame_json,
            frame.source_task_id,
            frame.updated_at_ts as i64,
            frame.expires_at_ts as i64,
        ],
    )?;
    Ok(())
}

fn persist_frame_tx(
    tx: &rusqlite::Transaction<'_>,
    task: &ClaimedTask,
    frame: &FollowupFrame,
) -> Result<()> {
    let user_key = effective_user_key(task);
    let frame_json = serde_json::to_string(frame)?;
    tx.execute(
        "INSERT INTO followup_frames (
            user_id, chat_id, user_key, frame_json, source_task_id, updated_at_ts, expires_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(user_id, chat_id, user_key) DO UPDATE SET
            frame_json = excluded.frame_json,
            source_task_id = excluded.source_task_id,
            updated_at_ts = excluded.updated_at_ts,
            expires_at_ts = excluded.expires_at_ts",
        params![
            task.user_id,
            task.chat_id,
            user_key,
            frame_json,
            frame.source_task_id,
            frame.updated_at_ts as i64,
            frame.expires_at_ts as i64,
        ],
    )?;
    Ok(())
}

fn clear_frame(state: &AppState, task: &ClaimedTask) -> Result<()> {
    let db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("acquire db for followup frame clear: {err}"))?;
    let user_key = effective_user_key(task);
    db.execute(
        "DELETE FROM followup_frames
         WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3",
        params![task.user_id, task.chat_id, user_key],
    )?;
    Ok(())
}

fn clear_frame_tx(tx: &rusqlite::Transaction<'_>, task: &ClaimedTask) -> Result<()> {
    let user_key = effective_user_key(task);
    tx.execute(
        "DELETE FROM followup_frames
         WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3",
        params![task.user_id, task.chat_id, user_key],
    )?;
    Ok(())
}

pub(crate) fn load_active_followup_frame(
    state: &AppState,
    task: &ClaimedTask,
) -> Option<FollowupFrame> {
    let db = state.core.db.get().ok()?;
    let user_key = effective_user_key(task);
    let mut stmt = db
        .prepare(
            "SELECT frame_json
             FROM followup_frames
             WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3",
        )
        .ok()?;
    let frame_json = stmt
        .query_row(params![task.user_id, task.chat_id, user_key], |row| {
            row.get::<_, String>(0)
        })
        .ok()?;
    let frame = serde_json::from_str::<FollowupFrame>(&frame_json).ok()?;
    if frame.is_expired(crate::now_ts_u64()) {
        let _ = clear_frame(state, task);
        return None;
    }
    Some(frame)
}

pub(crate) fn synthesize_locator_reply_resolved_intent(
    frame: &FollowupFrame,
    locator_reply: &str,
) -> Option<String> {
    let locator_reply = locator_reply.trim();
    if !frame.can_accept_locator_reply()
        || !crate::clarify_followup::prompt_is_structural_locator_only(locator_reply)
    {
        return None;
    }
    Some(
        format!(
            "Continue the previous request that was waiting for clarification: {}\nUser now provides the missing target/content: {}",
            frame.source_request.trim(),
            locator_reply
        ),
    )
}

fn sanitize_ordered_entry_text(entry: &str) -> String {
    let mut current = entry.trim();
    if let Some(stripped) = current.strip_prefix("- ") {
        current = stripped.trim();
    }
    if let Some(stripped) = current.strip_prefix("* ") {
        current = stripped.trim();
    }
    if let Some(stripped) = current.strip_prefix("+ ") {
        current = stripped.trim();
    }
    loop {
        let before = current;
        current = current.trim_matches(|ch: char| {
            matches!(
                ch,
                '`' | '"' | '\'' | '“' | '”' | '‘' | '’' | '<' | '>' | '《' | '》'
            )
        });
        for wrapper in ["**", "__", "*", "_"] {
            if current.len() > wrapper.len() * 2
                && current.starts_with(wrapper)
                && current.ends_with(wrapper)
            {
                current = current[wrapper.len()..current.len() - wrapper.len()].trim();
            }
        }
        if current == before {
            break;
        }
    }
    current.to_string()
}

fn compact_listing_payload(line: &str) -> Option<&str> {
    if let Some((_, rest)) = line.split_once('：') {
        return Some(rest);
    }
    if let Some((_, rest)) = line.split_once(':') {
        return Some(rest);
    }
    is_bare_compact_entry_list(line).then_some(line)
}

fn is_bare_compact_entry_list(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.contains(char::is_whitespace) {
        return false;
    }
    let mut saw_separator = false;
    for ch in trimmed.chars() {
        if matches!(ch, '、' | '，' | ',') {
            saw_separator = true;
            continue;
        }
        if !(ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                '.' | '_' | '-' | '/' | '\\' | '~' | '@' | '+' | '=' | '[' | ']' | '(' | ')'
            ))
        {
            return false;
        }
    }
    saw_separator
}

fn resolve_ordered_entry_target(frame: &FollowupFrame, entry: &str) -> String {
    let sanitized = sanitize_ordered_entry_text(entry);
    let trimmed = sanitized.trim();
    if trimmed.is_empty() || Path::new(trimmed).is_absolute() {
        return trimmed.to_string();
    }
    let base = frame
        .bound_target
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("");
    if base.is_empty() {
        return trimmed.to_string();
    }
    let base_path = Path::new(base);
    let join_root = if base_path.extension().is_some() {
        base_path.parent().unwrap_or(base_path)
    } else {
        base_path
    };
    join_root.join(trimmed).display().to_string()
}

pub(crate) fn ordered_entry_target_at(frame: &FollowupFrame, index: usize) -> Option<String> {
    frame
        .ordered_entries
        .get(index)
        .map(|entry| resolve_ordered_entry_target(frame, entry))
        .filter(|target| !target.trim().is_empty())
}

pub(crate) fn selected_entry_index_for_target(
    frame: &FollowupFrame,
    target: &str,
) -> Option<usize> {
    let trimmed_target = target.trim();
    if trimmed_target.is_empty() || frame.ordered_entries.is_empty() {
        return None;
    }
    frame
        .ordered_entries
        .iter()
        .enumerate()
        .find_map(|(index, entry)| {
            let resolved = resolve_ordered_entry_target(frame, entry);
            (path_targets_equivalent(&resolved, trimmed_target)
                || path_targets_equivalent(entry.trim(), trimmed_target))
            .then_some(index)
        })
}

fn path_targets_equivalent(left: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if left == right {
        return true;
    }
    let left_path = Path::new(left);
    let right_path = Path::new(right);
    match (left_path.is_absolute(), right_path.is_absolute()) {
        (true, false) => absolute_path_has_suffix(left_path, right_path),
        (false, true) => absolute_path_has_suffix(right_path, left_path),
        _ => false,
    }
}

fn absolute_path_has_suffix(absolute: &Path, suffix: &Path) -> bool {
    if !absolute.is_absolute() {
        return false;
    }
    let absolute_components = absolute
        .components()
        .map(|component| component.as_os_str().to_owned())
        .collect::<Vec<_>>();
    let suffix_components = suffix
        .components()
        .map(|component| component.as_os_str().to_owned())
        .collect::<Vec<_>>();
    !suffix_components.is_empty()
        && absolute_components.len() >= suffix_components.len()
        && absolute_components[absolute_components.len() - suffix_components.len()..]
            == suffix_components[..]
}

fn op_kind_from_route(
    route_result: &crate::RouteResult,
    unresolved_locator: bool,
    journal: &crate::task_journal::TaskJournal,
) -> FollowupOpKind {
    if unresolved_locator {
        return FollowupOpKind::ClarifyPending;
    }
    if route_result.wants_file_delivery || route_result.output_contract.delivery_required {
        return FollowupOpKind::Delivery;
    }
    if output_contract_prefers_listing_followup(route_result)
        || journal_has_listing_observation(journal)
    {
        return FollowupOpKind::List;
    }
    if route_result.output_contract.requires_content_evidence
        && (matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Url
        ) || matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::ContentExcerptSummary
        ))
    {
        return FollowupOpKind::Read;
    }
    if route_result.output_contract.requires_content_evidence {
        return FollowupOpKind::Read;
    }
    FollowupOpKind::Generic
}

fn output_contract_prefers_listing_followup(route_result: &crate::RouteResult) -> bool {
    matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::FilePaths
            | crate::OutputSemanticKind::SqliteTableListing
            | crate::OutputSemanticKind::SqliteTableNamesOnly
    ) || matches!(
        route_result.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::DirectoryLookup
            | crate::OutputDeliveryIntent::DirectoryBatchFiles
    )
}

fn journal_has_listing_observation(journal: &crate::task_journal::TaskJournal) -> bool {
    journal.step_results.iter().rev().any(|step| {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            return false;
        }
        if step.skill == "list_dir" {
            return true;
        }
        if step.skill != "system_basic" {
            return false;
        }
        step.output_excerpt
            .as_deref()
            .and_then(parse_journal_step_output)
            .and_then(|value| {
                value
                    .get("action")
                    .and_then(Value::as_str)
                    .map(|action| matches!(action, "inventory_dir" | "list_dir"))
            })
            .unwrap_or(false)
    })
}

pub(crate) fn extract_ordered_entries_from_text(text: &str) -> Vec<String> {
    let mut entries = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("FILE:") {
            let path = trimmed.trim_start_matches("FILE:").trim();
            if !path.is_empty() {
                entries.push(path.to_string());
            }
            continue;
        }
        let mut chars = trimmed.chars().peekable();
        let mut digits = String::new();
        while chars.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            digits.push(chars.next().unwrap_or_default());
        }
        if !digits.is_empty() && matches!(chars.peek(), Some('.' | ')' | '、')) {
            let _ = chars.next();
            let rest = sanitize_ordered_entry_text(&chars.collect::<String>());
            if !rest.is_empty() {
                entries.push(rest);
            }
        }
        if entries.len() >= MAX_ORDERED_ENTRIES {
            break;
        }
    }
    if entries.len() >= 2 {
        return entries;
    }
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(compact) = compact_listing_payload(trimmed) else {
            continue;
        };
        let compact_entries = compact
            .split(['、', '，', ','])
            .map(sanitize_ordered_entry_text)
            .map(|token| token.trim_end_matches(['。', '.']).to_string())
            .filter(|token| !token.is_empty())
            .filter(|token| !token.contains(char::is_whitespace))
            .take(MAX_ORDERED_ENTRIES)
            .collect::<Vec<_>>();
        if compact_entries.len() >= 2 {
            return compact_entries;
        }
    }
    let mut contiguous_lines = Vec::new();
    let mut saw_listing_block = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if saw_listing_block && contiguous_lines.len() >= 2 {
                break;
            }
            continue;
        }
        saw_listing_block = true;
        contiguous_lines.push(sanitize_ordered_entry_text(trimmed));
        if contiguous_lines.len() >= MAX_ORDERED_ENTRIES {
            break;
        }
    }
    if contiguous_lines.len() >= 2 {
        return contiguous_lines;
    }
    let simple_lines = text
        .lines()
        .map(sanitize_ordered_entry_text)
        .filter(|line| !line.is_empty())
        .take(MAX_ORDERED_ENTRIES)
        .collect::<Vec<_>>();
    if simple_lines.len() >= 2 {
        return simple_lines;
    }
    Vec::new()
}

fn parse_journal_step_output(output: &str) -> Option<Value> {
    serde_json::from_str::<Value>(output.trim()).ok()
}

pub(crate) fn derive_bound_target_from_journal(
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    for step in journal.step_results.iter().rev() {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        let Some(value) = parse_journal_step_output(output) else {
            continue;
        };
        if let Some(path) = value
            .get("resolved_path")
            .or_else(|| value.get("path"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            return Some(path.to_string());
        }
    }
    None
}

pub(crate) fn derive_slice_spec_from_journal(
    journal: &crate::task_journal::TaskJournal,
) -> Option<FollowupSliceSpec> {
    for step in journal.step_results.iter().rev() {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        let Some(value) = parse_journal_step_output(output) else {
            continue;
        };
        if value.get("action").and_then(Value::as_str) != Some("read_range") {
            continue;
        }
        let kind = match value.get("mode").and_then(Value::as_str) {
            Some("head") => FollowupSliceKind::Head,
            Some("tail") => FollowupSliceKind::Tail,
            Some("range") => FollowupSliceKind::Range,
            _ => continue,
        };
        return Some(FollowupSliceSpec {
            kind,
            n: value
                .get("n")
                .or_else(|| value.get("requested_n"))
                .and_then(Value::as_u64)
                .map(|value| value as usize),
            start_line: value
                .get("start_line")
                .and_then(Value::as_u64)
                .map(|value| value as usize),
            end_line: value
                .get("end_line")
                .and_then(Value::as_u64)
                .map(|value| value as usize),
        });
    }
    None
}

pub(crate) fn derive_ordered_entries_from_journal(
    journal: &crate::task_journal::TaskJournal,
) -> Vec<String> {
    for step in journal.step_results.iter().rev() {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        if step.skill == "list_dir" {
            let entries = output
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(MAX_ORDERED_ENTRIES)
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            if entries.len() >= 2 {
                return entries;
            }
        }
        if step.skill == "system_basic" {
            let Some(value) = parse_journal_step_output(output) else {
                continue;
            };
            if value.get("action").and_then(Value::as_str) == Some("inventory_dir") {
                let entries = value
                    .get("names")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .take(MAX_ORDERED_ENTRIES)
                    .map(ToString::to_string)
                    .collect::<Vec<_>>();
                if entries.len() >= 2 {
                    return entries;
                }
            }
        }
    }
    Vec::new()
}

pub(crate) fn derive_bound_target_from_answer(
    answer_text: &str,
    answer_messages: &[String],
) -> Option<String> {
    let mut candidates = crate::extract_delivery_file_tokens(answer_text);
    for message in answer_messages {
        candidates.extend(crate::extract_delivery_file_tokens(message));
    }
    candidates
        .into_iter()
        .find_map(|token| crate::delivery_utils::extract_file_path_from_delivery_token(&token))
}

fn answer_contains_multiple_delivery_targets(
    answer_text: &str,
    answer_messages: &[String],
) -> bool {
    let mut targets = crate::extract_delivery_file_tokens(answer_text)
        .into_iter()
        .filter_map(|token| crate::delivery_utils::extract_file_path_from_delivery_token(&token))
        .collect::<Vec<_>>();
    for message in answer_messages {
        targets.extend(
            crate::extract_delivery_file_tokens(message)
                .into_iter()
                .filter_map(|token| {
                    crate::delivery_utils::extract_file_path_from_delivery_token(&token)
                }),
        );
    }
    targets.sort();
    targets.dedup();
    targets.len() >= 2
}

fn merge_frame_with_prior(
    mut frame: FollowupFrame,
    prior_frame: Option<&FollowupFrame>,
) -> FollowupFrame {
    let Some(prior) = prior_frame else {
        if let Some(bound_target) = frame.bound_target.as_deref() {
            frame.selected_entry_index = selected_entry_index_for_target(&frame, bound_target);
        }
        return frame;
    };
    if frame.ordered_entries.is_empty()
        && !prior.ordered_entries.is_empty()
        && frame
            .bound_target
            .as_deref()
            .and_then(|target| selected_entry_index_for_target(prior, target))
            .is_some()
    {
        frame.ordered_entries = prior.ordered_entries.clone();
    }
    if frame.selected_entry_index.is_none() {
        if let Some(bound_target) = frame.bound_target.as_deref() {
            frame.selected_entry_index = selected_entry_index_for_target(&frame, bound_target);
        }
    }
    frame
}

fn derive_frame_for_ask_outcome(
    prior_frame: Option<&FollowupFrame>,
    task_id: &str,
    prompt: &str,
    route_result: &crate::RouteResult,
    answer_text: &str,
    answer_messages: &[String],
    semantic_clarify: bool,
    journal: &crate::task_journal::TaskJournal,
) -> FollowupFrame {
    let now_ts = crate::now_ts_u64();
    let observed_facts = crate::observed_facts::derive_observed_facts_from_ask_outcome(
        answer_text,
        answer_messages,
        journal,
        route_result,
    );
    let unresolved_locator = semantic_clarify
        && matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Url
                | crate::OutputLocatorKind::CurrentWorkspace
                | crate::OutputLocatorKind::None
        )
        && (route_result.needs_clarify
            || route_result.output_contract.locator_hint.trim().is_empty());
    let op_kind = op_kind_from_route(route_result, unresolved_locator, journal);
    let should_extract_answer_entries = matches!(op_kind, FollowupOpKind::List)
        || (matches!(op_kind, FollowupOpKind::Delivery)
            && answer_contains_multiple_delivery_targets(answer_text, answer_messages));
    let mut ordered_entries = if should_extract_answer_entries {
        observed_facts.ordered_entries.clone()
    } else {
        Vec::new()
    };
    ordered_entries.truncate(MAX_ORDERED_ENTRIES);
    let frame = FollowupFrame {
        source_request: prompt.trim().to_string(),
        op_kind,
        bound_target: (!unresolved_locator)
            .then(|| {
                observed_facts.bound_target.clone().or_else(|| {
                    let hint = route_result.output_contract.locator_hint.trim();
                    (!hint.is_empty()).then(|| hint.to_string())
                })
            })
            .flatten(),
        ordered_entries,
        selected_entry_index: None,
        slice_spec: observed_facts.slice_spec.clone(),
        output_shape: Some(
            route_result
                .output_contract
                .response_shape
                .as_str()
                .to_string(),
        ),
        unresolved_slot: unresolved_locator.then_some(FollowupUnresolvedSlot::Locator),
        source_task_id: task_id.to_string(),
        updated_at_ts: now_ts,
        expires_at_ts: now_ts + FOLLOWUP_FRAME_TTL_SECS,
    };
    merge_frame_with_prior(frame, prior_frame)
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn replace_active_frame_from_ask_outcome(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
    route_result: &crate::RouteResult,
    answer_text: &str,
    answer_messages: &[String],
    semantic_clarify: bool,
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    if semantic_clarify {
        if let Err(err) = clear_frame(state, task) {
            tracing::warn!(
                "followup_frame clear failed task_id={} err={}",
                task.task_id,
                err
            );
        }
        return None;
    }
    let prior_frame =
        crate::conversation_state::load_active_session_snapshot(state, task).active_followup_frame;
    let frame = derive_frame_for_ask_outcome(
        prior_frame.as_ref(),
        &task.task_id,
        prompt,
        route_result,
        answer_text,
        answer_messages,
        semantic_clarify,
        journal,
    );
    if let Err(err) = persist_frame(state, task, &frame) {
        tracing::warn!(
            "followup_frame persist failed task_id={} err={}",
            task.task_id,
            err
        );
        return None;
    }
    Some(frame.source_task_id)
}

pub(crate) fn sync_active_frame_from_ask_outcome_tx(
    tx: &rusqlite::Transaction<'_>,
    task: &ClaimedTask,
    prompt: &str,
    route_result: &crate::RouteResult,
    answer_text: &str,
    answer_messages: &[String],
    semantic_clarify: bool,
    journal: &crate::task_journal::TaskJournal,
    prior_frame: Option<&FollowupFrame>,
) -> Result<Option<String>> {
    if semantic_clarify {
        clear_frame_tx(tx, task)?;
        return Ok(None);
    }
    let frame = derive_frame_for_ask_outcome(
        prior_frame,
        &task.task_id,
        prompt,
        route_result,
        answer_text,
        answer_messages,
        semantic_clarify,
        journal,
    );
    persist_frame_tx(tx, task, &frame)?;
    Ok(Some(frame.source_task_id))
}

#[cfg(test)]
mod tests {
    use super::{
        extract_ordered_entries_from_text, load_active_followup_frame, persist_frame,
        replace_active_frame_from_ask_outcome, synthesize_locator_reply_resolved_intent,
        FollowupFrame, FollowupOpKind, FollowupSliceKind, FollowupSliceSpec,
        FollowupUnresolvedSlot,
    };
    use crate::{
        runtime::AppState, AskMode, IntentOutputContract, OutputLocatorKind, RouteResult,
        RoutedMode,
    };

    #[test]
    fn locator_reply_resolved_intent_uses_persisted_request() {
        let frame = FollowupFrame {
            source_request: "看一下那个 model io log 最后 4 行，再一句话说有什么现象".to_string(),
            op_kind: FollowupOpKind::ClarifyPending,
            unresolved_slot: Some(FollowupUnresolvedSlot::Locator),
            ..FollowupFrame::default()
        };
        let rewritten =
            synthesize_locator_reply_resolved_intent(&frame, "/tmp/device_local/logs/model_io.log")
                .expect("frame should accept locator reply");
        assert!(rewritten.contains("看一下那个 model io log 最后 4 行"));
        assert!(rewritten.contains("/tmp/device_local/logs/model_io.log"));
    }

    #[test]
    fn locator_reply_resolved_intent_rejects_non_locator_new_request() {
        let frame = FollowupFrame {
            source_request: "看一下那个 model io log 最后 4 行，再一句话说有什么现象".to_string(),
            op_kind: FollowupOpKind::ClarifyPending,
            unresolved_slot: Some(FollowupUnresolvedSlot::Locator),
            ..FollowupFrame::default()
        };
        assert!(synthesize_locator_reply_resolved_intent(&frame, "今天天气怎么样").is_none());
    }

    #[test]
    fn extracts_ordered_entries_from_compact_listing_sentence() {
        let entries = extract_ordered_entries_from_text(
            "列表：act_plan.log、clawd.log、clawd.run.log、feishud.log、install_ops.log。",
        );
        assert_eq!(
            entries,
            vec![
                "act_plan.log",
                "clawd.log",
                "clawd.run.log",
                "feishud.log",
                "install_ops.log"
            ]
        );
    }

    #[test]
    fn extracts_ordered_entries_from_bare_compact_listing() {
        let entries = extract_ordered_entries_from_text(
            "act_plan.log,clawd.log,clawd.run.log,feishud.log,install_ops.log",
        );
        assert_eq!(
            entries,
            vec![
                "act_plan.log",
                "clawd.log",
                "clawd.run.log",
                "feishud.log",
                "install_ops.log"
            ]
        );
    }

    #[test]
    fn ignores_prose_prefixed_compact_listing_without_delimiter() {
        let entries = extract_ordered_entries_from_text(
            "前5个条目act_plan.log、clawd.log、clawd.run.log、feishud.log、install_ops.log",
        );
        assert!(
            entries.is_empty(),
            "compact follow-up extraction should not depend on language-specific prefix filters"
        );
    }

    #[test]
    fn extracts_ordered_entries_from_listing_block_before_summary_paragraph() {
        let entries = extract_ordered_entries_from_text(
            "act_plan.log\nclawd.log\nclawd.run.log\nfeishud.log\ninstall_ops.log\n\n这个目录主要放运行日志和排查记录。",
        );
        assert_eq!(
            entries,
            vec![
                "act_plan.log",
                "clawd.log",
                "clawd.run.log",
                "feishud.log",
                "install_ops.log"
            ]
        );
    }

    #[test]
    fn extracts_ordered_entries_from_markdown_numbered_listing() {
        let entries = extract_ordered_entries_from_text(
            "**logs 目录下前 5 个文件：**\n\n1. `act_plan.log`\n2. `clawd.log`\n3. `clawd.run.log`\n4. `feishud.log`\n5. `install_ops.log`\n",
        );
        assert_eq!(
            entries,
            vec![
                "act_plan.log",
                "clawd.log",
                "clawd.run.log",
                "feishud.log",
                "install_ops.log"
            ]
        );
    }

    #[test]
    fn persisted_followup_frame_round_trips_with_slice_and_entries() {
        let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        let task = crate::ClaimedTask {
            task_id: "task-followup-frame".to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: Some("test-user".to_string()),
            channel: "ui".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        };
        let mut journal =
            crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "system_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    serde_json::json!({
                        "action": "read_range",
                        "resolved_path": "/tmp/logs/model_io.log",
                        "mode": "tail",
                        "n": 4,
                        "excerpt": "1|a\n2|b\n3|c\n4|d"
                    })
                    .to_string(),
                ),
                ..Default::default()
            });
        let route_result = RouteResult {
            routed_mode: RoutedMode::Act,
            ask_mode: AskMode::from_routed_mode(RoutedMode::Act),
            resolved_intent: "看一下那个 model io log 最后 4 行，再一句话说有什么现象".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "test".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "model_io.log".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        replace_active_frame_from_ask_outcome(
            &state,
            &task,
            "看一下那个 model io log 最后 4 行，再一句话说有什么现象",
            &route_result,
            "a\nb\nc\nd",
            &[],
            false,
            &journal,
        );
        let frame = load_active_followup_frame(&state, &task).expect("frame should load");
        assert_eq!(
            frame.bound_target.as_deref(),
            Some("/tmp/logs/model_io.log")
        );
        assert_eq!(
            frame.slice_spec,
            Some(FollowupSliceSpec {
                kind: FollowupSliceKind::Tail,
                n: Some(4),
                start_line: None,
                end_line: None,
            })
        );
    }

    #[test]
    fn compact_listing_answer_persists_ordered_entries_for_followup() {
        let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        let task = crate::ClaimedTask {
            task_id: "task-followup-compact-list".to_string(),
            user_id: 11,
            chat_id: 12,
            user_key: Some("test-user".to_string()),
            channel: "ui".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        };
        let journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
        let route_result = RouteResult {
            routed_mode: RoutedMode::Act,
            ask_mode: AskMode::from_routed_mode(RoutedMode::Act),
            resolved_intent: "列出 logs 目录下前 5 个文件名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "test".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::FileNames,
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        replace_active_frame_from_ask_outcome(
            &state,
            &task,
            "先列出 logs 目录下前 5 个文件名",
            &route_result,
            "列表：act_plan.log、clawd.log、clawd.run.log、feishud.log、install_ops.log。",
            &[],
            false,
            &journal,
        );
        let frame = load_active_followup_frame(&state, &task).expect("frame should load");
        assert_eq!(frame.op_kind, FollowupOpKind::List);
        assert_eq!(frame.bound_target.as_deref(), Some("logs"));
        assert_eq!(
            frame.ordered_entries,
            vec![
                "act_plan.log",
                "clawd.log",
                "clawd.run.log",
                "feishud.log",
                "install_ops.log"
            ]
        );
        assert_eq!(frame.selected_entry_index, None);
    }

    #[test]
    fn visible_listing_answer_overrides_full_journal_listing_for_followup() {
        let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        let task = crate::ClaimedTask {
            task_id: "task-followup-visible-list".to_string(),
            user_id: 13,
            chat_id: 14,
            user_key: Some("test-user".to_string()),
            channel: "ui".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        };
        let mut journal =
            crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "list_dir".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    "act_plan.log\nclawd.log\nclawd.run.log\nfeishud.log\ninstall_ops.log\nnl_manual_qwen.run.log\nservice_ops.log\n".to_string(),
                ),
                ..Default::default()
            });
        let route_result = RouteResult {
            routed_mode: RoutedMode::Act,
            ask_mode: AskMode::from_routed_mode(RoutedMode::Act),
            resolved_intent: "列出 logs 目录下前 5 个文件名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "test".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        replace_active_frame_from_ask_outcome(
            &state,
            &task,
            "先列出 logs 目录下前 5 个文件名",
            &route_result,
            "act_plan.log\nclawd.log\nclawd.run.log\nfeishud.log\ninstall_ops.log",
            &[],
            false,
            &journal,
        );
        let frame = load_active_followup_frame(&state, &task).expect("frame should load");
        assert_eq!(
            frame.ordered_entries,
            vec![
                "act_plan.log",
                "clawd.log",
                "clawd.run.log",
                "feishud.log",
                "install_ops.log"
            ]
        );
    }

    #[test]
    fn selected_target_turn_inherits_prior_ordered_entries_and_index() {
        let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        let task = crate::ClaimedTask {
            task_id: "task-followup-selected-entry".to_string(),
            user_id: 21,
            chat_id: 22,
            user_key: Some("test-user".to_string()),
            channel: "ui".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        };
        let prior_frame = FollowupFrame {
            source_request: "先列出 logs 目录下前 4 个文件名".to_string(),
            op_kind: FollowupOpKind::List,
            bound_target: Some("logs".to_string()),
            ordered_entries: vec![
                "act_plan.log".to_string(),
                "clawd.log".to_string(),
                "clawd.run.log".to_string(),
                "feishud.log".to_string(),
            ],
            source_task_id: "older-task".to_string(),
            updated_at_ts: 1,
            expires_at_ts: crate::now_ts_u64() + 300,
            ..FollowupFrame::default()
        };
        persist_frame(&state, &task, &prior_frame).expect("persist prior frame");

        let mut journal =
            crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "system_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    serde_json::json!({
                        "action": "read_range",
                        "resolved_path": "logs/clawd.log",
                        "mode": "tail",
                        "n": 2,
                        "excerpt": "x\ny"
                    })
                    .to_string(),
                ),
                ..Default::default()
            });
        let route_result = RouteResult {
            routed_mode: RoutedMode::Act,
            ask_mode: AskMode::from_routed_mode(RoutedMode::Act),
            resolved_intent: "看第二个最后 2 行".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "test".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "logs/clawd.log".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        replace_active_frame_from_ask_outcome(
            &state,
            &task,
            "看第二个最后 2 行",
            &route_result,
            "line1\nline2",
            &[],
            false,
            &journal,
        );
        let frame = load_active_followup_frame(&state, &task).expect("frame should load");
        assert_eq!(frame.ordered_entries, prior_frame.ordered_entries);
        assert_eq!(frame.selected_entry_index, Some(1));
        assert_eq!(frame.bound_target.as_deref(), Some("logs/clawd.log"));
    }

    #[test]
    fn delivery_answer_sets_bound_target_from_file_token_and_inherits_selection() {
        let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        let task = crate::ClaimedTask {
            task_id: "task-followup-delivery-entry".to_string(),
            user_id: 31,
            chat_id: 32,
            user_key: Some("test-user".to_string()),
            channel: "ui".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        };
        let prior_frame = FollowupFrame {
            source_request: "先列出 logs 目录下前 4 个文件名".to_string(),
            op_kind: FollowupOpKind::List,
            bound_target: Some("logs".to_string()),
            ordered_entries: vec![
                "act_plan.log".to_string(),
                "clawd.log".to_string(),
                "clawd.run.log".to_string(),
                "feishud.log".to_string(),
            ],
            source_task_id: "older-task".to_string(),
            updated_at_ts: 1,
            expires_at_ts: crate::now_ts_u64() + 300,
            ..FollowupFrame::default()
        };
        persist_frame(&state, &task, &prior_frame).expect("persist prior frame");
        let journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
        let route_result = RouteResult {
            routed_mode: RoutedMode::Act,
            ask_mode: AskMode::from_routed_mode(RoutedMode::Act),
            resolved_intent: "把第二个发给我".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "test".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: true,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::FileToken,
                requires_content_evidence: false,
                delivery_required: true,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::FileSingle,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        replace_active_frame_from_ask_outcome(
            &state,
            &task,
            "把第二个发给我",
            &route_result,
            "FILE:logs/clawd.log",
            &["FILE:logs/clawd.log".to_string()],
            false,
            &journal,
        );
        let frame = load_active_followup_frame(&state, &task).expect("frame should load");
        assert_eq!(frame.bound_target.as_deref(), Some("logs/clawd.log"));
        assert_eq!(frame.selected_entry_index, Some(1));
        assert_eq!(frame.ordered_entries, prior_frame.ordered_entries);
    }

    #[test]
    fn delivery_answer_with_absolute_file_token_still_inherits_relative_listing_selection() {
        let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        let task = crate::ClaimedTask {
            task_id: "task-followup-delivery-absolute-entry".to_string(),
            user_id: 41,
            chat_id: 42,
            user_key: Some("test-user".to_string()),
            channel: "ui".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        };
        let prior_frame = FollowupFrame {
            source_request: "先列出 logs 目录下前 4 个文件名".to_string(),
            op_kind: FollowupOpKind::List,
            bound_target: Some("logs".to_string()),
            ordered_entries: vec![
                "act_plan.log".to_string(),
                "clawd.log".to_string(),
                "clawd.run.log".to_string(),
                "feishud.log".to_string(),
            ],
            source_task_id: "older-task".to_string(),
            updated_at_ts: 1,
            expires_at_ts: crate::now_ts_u64() + 300,
            ..FollowupFrame::default()
        };
        persist_frame(&state, &task, &prior_frame).expect("persist prior frame");
        let journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
        let route_result = RouteResult {
            routed_mode: RoutedMode::Act,
            ask_mode: AskMode::from_routed_mode(RoutedMode::Act),
            resolved_intent: "把第二个发给我".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "test".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: true,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::FileToken,
                requires_content_evidence: false,
                delivery_required: true,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::FileSingle,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        replace_active_frame_from_ask_outcome(
            &state,
            &task,
            "把第二个发给我",
            &route_result,
            "FILE:/home/guagua/rustclaw/logs/clawd.log",
            &["FILE:/home/guagua/rustclaw/logs/clawd.log".to_string()],
            false,
            &journal,
        );
        let frame = load_active_followup_frame(&state, &task).expect("frame should load");
        assert_eq!(
            frame.bound_target.as_deref(),
            Some("/home/guagua/rustclaw/logs/clawd.log")
        );
        assert_eq!(frame.selected_entry_index, Some(1));
        assert_eq!(frame.ordered_entries, prior_frame.ordered_entries);
    }

    #[test]
    fn clarify_outcome_clears_active_followup_frame() {
        let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        let task = crate::ClaimedTask {
            task_id: "task-followup-clarify".to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: Some("test-user".to_string()),
            channel: "ui".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        };
        let journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
        let route_result = RouteResult {
            routed_mode: RoutedMode::AskClarify,
            ask_mode: AskMode::from_routed_mode(RoutedMode::AskClarify),
            resolved_intent: "看一下那个 README 开头，然后一句话总结".to_string(),
            needs_clarify: true,
            clarify_question: "请提供具体文件路径".to_string(),
            route_reason: "fresh_content_deictic_requires_locator".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        replace_active_frame_from_ask_outcome(
            &state,
            &task,
            "读一下那个 README 开头，然后一句话总结",
            &route_result,
            "请提供具体文件路径。",
            &[],
            true,
            &journal,
        );
        assert!(
            load_active_followup_frame(&state, &task).is_none(),
            "clarify outcomes should be represented by ClarifyState, not a duplicate followup frame"
        );
    }

    #[test]
    fn clarify_outcome_with_stale_locator_hint_still_clears_followup_frame() {
        let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        let task = crate::ClaimedTask {
            task_id: "task-followup-stale-locator".to_string(),
            user_id: 3,
            chat_id: 4,
            user_key: Some("test-user".to_string()),
            channel: "ui".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        };
        let journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "prompt");
        let route_result = RouteResult {
            routed_mode: RoutedMode::Act,
            ask_mode: AskMode::from_routed_mode(RoutedMode::Act),
            resolved_intent: "看一下那个模型日志最后 5 行".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "memory_alias".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "/tmp/rustclaw-workspace/old/logs/model_io.log".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        replace_active_frame_from_ask_outcome(
            &state,
            &task,
            "看看那个模型日志最后 5 行",
            &route_result,
            "LOCATOR_CLARIFY_PROMPT",
            &["LOCATOR_CLARIFY_PROMPT".to_string()],
            true,
            &journal,
        );
        assert!(
            load_active_followup_frame(&state, &task).is_none(),
            "clarify outcomes should not leave a stale followup frame behind"
        );
    }
}
