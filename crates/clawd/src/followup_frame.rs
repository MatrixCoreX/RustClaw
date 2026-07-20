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
    CodeWorkspace,
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

fn dedupe_ordered_entries(entries: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::with_capacity(entries.len());
    for entry in entries {
        if !deduped.iter().any(|seen| seen == &entry) {
            deduped.push(entry);
        }
    }
    deduped
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
    if trimmed.is_empty() {
        return false;
    }
    let parts = trimmed
        .split(['、', '，', ','])
        .map(str::trim)
        .collect::<Vec<_>>();
    parts.len() >= 2 && parts.into_iter().all(is_compact_entry_token)
}

fn is_compact_entry_token(token: &str) -> bool {
    let trimmed = token.trim();
    if trimmed.is_empty() || trimmed.contains(char::is_whitespace) {
        return false;
    }
    trimmed.chars().all(|ch| {
        ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                '.' | '_' | '-' | '/' | '\\' | '~' | '@' | '+' | '=' | '[' | ']' | '(' | ')'
            )
    })
}

fn markdown_bullet_entry(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let payload = ["- ", "* ", "+ "]
        .into_iter()
        .find_map(|prefix| trimmed.strip_prefix(prefix))?;
    let entry = sanitize_ordered_entry_text(payload);
    (!entry.trim().is_empty()).then_some(entry)
}

fn markdown_bullet_block_entries(text: &str) -> Vec<String> {
    let mut entries = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if entries.len() >= 2 {
                break;
            }
            entries.clear();
            continue;
        }
        if let Some(entry) = markdown_bullet_entry(trimmed) {
            entries.push(entry);
            if entries.len() >= MAX_ORDERED_ENTRIES {
                break;
            }
            continue;
        }
        if entries.len() >= 2 {
            break;
        }
        entries.clear();
    }
    if entries.len() >= 2 {
        dedupe_ordered_entries(entries)
    } else {
        Vec::new()
    }
}

fn structural_ordered_entry_has_signal(entry: &str) -> bool {
    let trimmed = entry.trim();
    Path::new(trimmed).is_absolute()
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains('.')
        || trimmed.contains('_')
        || trimmed.contains('-')
        || trimmed.contains('@')
        || trimmed.contains('=')
}

fn is_structural_ordered_entry_token(entry: &str) -> bool {
    let trimmed = entry.trim();
    if trimmed.is_empty()
        || trimmed.contains(char::is_whitespace)
        || trimmed.chars().any(char::is_control)
    {
        return false;
    }
    trimmed.chars().all(|ch| {
        ch.is_alphanumeric()
            || matches!(
                ch,
                '.' | '_'
                    | '-'
                    | '/'
                    | '\\'
                    | '~'
                    | '@'
                    | '+'
                    | '='
                    | '['
                    | ']'
                    | '('
                    | ')'
                    | '%'
                    | ':'
            )
    })
}

pub(crate) fn ordered_entries_are_structural_tokens(entries: &[String]) -> bool {
    entries.len() >= 2
        && entries
            .iter()
            .all(|entry| is_structural_ordered_entry_token(entry))
        && entries
            .iter()
            .any(|entry| structural_ordered_entry_has_signal(entry))
}

fn structural_ordered_entries(entries: &[String]) -> Vec<String> {
    let structural = entries
        .iter()
        .filter(|entry| is_structural_ordered_entry_token(entry))
        .cloned()
        .collect::<Vec<_>>();
    if structural.len() >= 2
        && structural
            .iter()
            .any(|entry| structural_ordered_entry_has_signal(entry))
    {
        structural
    } else {
        Vec::new()
    }
}

fn resolve_ordered_entry_target(frame: &FollowupFrame, entry: &str) -> String {
    let sanitized = sanitize_ordered_entry_text(entry);
    let trimmed = sanitized.trim();
    if trimmed.is_empty() || Path::new(trimmed).is_absolute() {
        return trimmed.to_string();
    }
    let entry_path = Path::new(trimmed);
    if entry_path.components().count() > 1 && entry_path.exists() {
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
    if let Some(entry_parent) = entry_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        let parent_already_in_base = if join_root.is_absolute() {
            absolute_path_has_suffix(join_root, entry_parent)
        } else {
            join_root.ends_with(entry_parent)
        };
        if parent_already_in_base {
            return trimmed.to_string();
        }
    }
    join_root.join(trimmed).display().to_string()
}

#[cfg(test)]
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
    route_result: &crate::IntentOutputContract,
    unresolved_locator: bool,
    journal: &crate::task_journal::TaskJournal,
) -> FollowupOpKind {
    if unresolved_locator {
        return FollowupOpKind::ClarifyPending;
    }
    if derive_code_workspace_bound_target_from_route_and_journal(route_result, journal).is_some() {
        return FollowupOpKind::CodeWorkspace;
    }
    if route_result.delivery_required || route_result.delivery_required {
        return FollowupOpKind::Delivery;
    }
    if output_contract_prefers_listing_followup(route_result)
        || journal_has_listing_observation(journal)
    {
        return FollowupOpKind::List;
    }
    if !crate::observed_facts::route_allows_observed_bound_target(route_result) {
        return FollowupOpKind::Generic;
    }
    if route_result.requires_content_evidence
        && (matches!(
            route_result.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Url
        ) || route_result.semantic_kind_is_any(&[
            crate::OutputSemanticKind::ContentExcerptSummary,
            crate::OutputSemanticKind::ContentExcerptWithSummary,
        ]))
    {
        return FollowupOpKind::Read;
    }
    if route_result.requires_content_evidence {
        return FollowupOpKind::Read;
    }
    FollowupOpKind::Generic
}

fn output_contract_prefers_listing_followup(route_result: &crate::IntentOutputContract) -> bool {
    route_result.semantic_kind_is_any(&[
        crate::OutputSemanticKind::FileNames,
        crate::OutputSemanticKind::DirectoryNames,
        crate::OutputSemanticKind::DirectoryEntryGroups,
        crate::OutputSemanticKind::FilePaths,
    ]) || matches!(
        route_result.delivery_intent,
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
        step.output_excerpt
            .as_deref()
            .and_then(parse_journal_step_output)
            .map(|value| listing_json_has_entries(&value))
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
        return dedupe_ordered_entries(entries);
    }
    let bullet_entries = markdown_bullet_block_entries(text);
    if bullet_entries.len() >= 2 {
        return bullet_entries;
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
            return dedupe_ordered_entries(compact_entries);
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
        let structural_entries = structural_ordered_entries(&contiguous_lines);
        if structural_entries.len() >= 2 {
            return dedupe_ordered_entries(structural_entries);
        }
        return dedupe_ordered_entries(contiguous_lines);
    }
    let simple_lines = text
        .lines()
        .map(sanitize_ordered_entry_text)
        .filter(|line| !line.is_empty())
        .take(MAX_ORDERED_ENTRIES)
        .collect::<Vec<_>>();
    if simple_lines.len() >= 2 {
        return dedupe_ordered_entries(simple_lines);
    }
    Vec::new()
}

fn parse_journal_step_output(output: &str) -> Option<Value> {
    serde_json::from_str::<Value>(output.trim()).ok()
}

fn nonempty_string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
}

fn bound_target_from_journal_output_value(value: &Value) -> Option<String> {
    for source in [Some(value), value.get("extra")].into_iter().flatten() {
        if let Some(path) = nonempty_string_field(source, "resolved_path")
            .or_else(|| nonempty_string_field(source, "path"))
        {
            return Some(path);
        }
    }
    None
}

fn action_and_path_from_journal_output_value(value: &Value) -> Option<(String, String)> {
    for source in [Some(value), value.get("extra")].into_iter().flatten() {
        let Some(action) = nonempty_string_field(source, "action") else {
            continue;
        };
        let Some(path) = nonempty_string_field(source, "resolved_path")
            .or_else(|| nonempty_string_field(source, "effective_path"))
            .or_else(|| nonempty_string_field(source, "path"))
        else {
            continue;
        };
        return Some((action, path));
    }
    None
}

fn cwd_from_journal_output_value(value: &Value) -> Option<String> {
    for source in [Some(value), value.get("extra")].into_iter().flatten() {
        if let Some(cwd) = nonempty_string_field(source, "cwd")
            .or_else(|| nonempty_string_field(source, "working_dir"))
        {
            return Some(cwd);
        }
    }
    None
}

fn path_parent_string(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    let path = Path::new(path);
    let parent = if path.extension().is_some() {
        path.parent()?
    } else {
        path
    };
    let rendered = parent.to_string_lossy().trim().to_string();
    (!rendered.is_empty()).then_some(rendered)
}

fn code_workspace_write_action(action: &str) -> bool {
    matches!(action, "write_text" | "append_text" | "shell_write")
}

fn code_workspace_validation_action(action: &str) -> bool {
    matches!(
        action,
        "run_cmd" | "process_basic" | "read_range" | "read_text_range" | "grep_text"
    )
}

fn path_looks_like_code_or_test_artifact(path: &str) -> bool {
    let lower = path.trim().to_ascii_lowercase();
    const CODE_EXTENSIONS: &[&str] = &[
        ".py", ".rs", ".js", ".jsx", ".ts", ".tsx", ".go", ".java", ".kt", ".c", ".cc", ".cpp",
        ".h", ".hpp", ".cs", ".php", ".rb", ".swift", ".scala", ".sh", ".bash", ".zsh", ".fish",
        ".ps1", ".sql",
    ];
    CODE_EXTENSIONS
        .iter()
        .any(|extension| lower.ends_with(extension))
}

fn path_looks_like_test_artifact(path: &str) -> bool {
    let path = path.trim().replace('\\', "/");
    let basename = Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path.as_str())
        .to_ascii_lowercase();
    basename.starts_with("test_")
        || basename.ends_with("_test.py")
        || basename.ends_with(".test.js")
        || basename.ends_with(".spec.js")
        || basename.ends_with(".test.ts")
        || basename.ends_with(".spec.ts")
        || basename.ends_with("_test.rs")
}

fn push_unique_path(paths: &mut Vec<String>, path: String) {
    if path.trim().is_empty() || paths.iter().any(|existing| existing == &path) {
        return;
    }
    paths.push(path);
}

pub(crate) fn derive_code_workspace_bound_target_from_journal(
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    let mut write_dirs = Vec::new();
    let mut validation_dirs = Vec::new();
    let mut saw_validation = false;

    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            continue;
        }
        if matches!(step.skill.as_str(), "run_cmd" | "process_basic") {
            saw_validation = true;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        let Some(value) = parse_journal_step_output(output.trim()) else {
            continue;
        };
        if let Some((action, path)) = action_and_path_from_journal_output_value(&value) {
            if code_workspace_write_action(&action) && path_looks_like_code_or_test_artifact(&path)
            {
                if let Some(parent) = path_parent_string(&path) {
                    push_unique_path(&mut write_dirs, parent);
                }
            }
            if code_workspace_validation_action(&action)
                && path_looks_like_code_or_test_artifact(&path)
            {
                saw_validation = true;
                if let Some(parent) = path_parent_string(&path) {
                    push_unique_path(&mut validation_dirs, parent);
                }
            }
        }
        if matches!(step.skill.as_str(), "run_cmd" | "process_basic") {
            if let Some(cwd) = cwd_from_journal_output_value(&value)
                .or_else(|| nonempty_string_field(&value, "cwd"))
            {
                push_unique_path(&mut validation_dirs, cwd);
            }
        }
    }

    if write_dirs.is_empty() || !saw_validation {
        return None;
    }
    write_dirs
        .iter()
        .find(|dir| validation_dirs.iter().any(|candidate| candidate == *dir))
        .cloned()
        .or_else(|| {
            (write_dirs.len() == 1)
                .then(|| write_dirs.first().cloned())
                .flatten()
        })
}

pub(crate) fn derive_code_workspace_bound_target_from_route_and_journal(
    route_result: &crate::IntentOutputContract,
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    let _ = route_result;
    derive_code_workspace_bound_target_from_journal(journal)
        .or_else(|| derive_read_validated_code_workspace_bound_target_from_journal(journal))
}

fn derive_read_validated_code_workspace_bound_target_from_journal(
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    let mut source_dirs = Vec::new();
    let mut test_dirs = Vec::new();
    let mut saw_validation_command = false;

    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            continue;
        }
        if matches!(step.skill.as_str(), "run_cmd" | "process_basic") {
            saw_validation_command = true;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        let Some(value) = parse_journal_step_output(output.trim()) else {
            continue;
        };
        let Some((action, path)) = action_and_path_from_journal_output_value(&value) else {
            continue;
        };
        if !matches!(action.as_str(), "read_range" | "read_text_range")
            || !path_looks_like_code_or_test_artifact(&path)
        {
            continue;
        }
        let Some(parent) = path_parent_string(&path) else {
            continue;
        };
        if path_looks_like_test_artifact(&path) {
            push_unique_path(&mut test_dirs, parent);
        } else {
            push_unique_path(&mut source_dirs, parent);
        }
    }

    if !saw_validation_command || source_dirs.is_empty() || test_dirs.is_empty() {
        return None;
    }
    source_dirs
        .iter()
        .find(|dir| test_dirs.iter().any(|candidate| candidate == *dir))
        .cloned()
        .or_else(|| {
            (source_dirs.len() == 1 && test_dirs.len() == 1)
                .then(|| source_dirs.first().cloned())
                .flatten()
        })
}

fn is_listing_json(value: &Value) -> bool {
    value
        .get("action")
        .and_then(Value::as_str)
        .is_some_and(|action| {
            matches!(
                action,
                "inventory_dir" | "list_dir" | "tree_summary" | "find_name" | "grep_text"
            )
        })
}

fn ordered_string_entries_from_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(sanitize_ordered_entry_text)
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty())
        .take(MAX_ORDERED_ENTRIES)
        .collect::<Vec<_>>()
}

fn entry_name_from_listing_value(value: &Value) -> Option<String> {
    if let Some(name) = value.as_str() {
        return Some(name.to_string());
    }
    let object = value.as_object()?;
    for key in ["name", "file_name", "path", "relative_path"] {
        if let Some(text) = object
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            return Some(text.to_string());
        }
    }
    None
}

fn entry_name_from_tree_child(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    for key in ["name", "file_name"] {
        if let Some(text) = object
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            return Some(text.to_string());
        }
    }
    let path_text = object
        .get("relative_path")
        .or_else(|| object.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    Path::new(path_text)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
        .or_else(|| Some(path_text.to_string()))
}

fn ordered_entries_from_listing_json_shallow(value: &Value) -> Vec<String> {
    if !is_listing_json(value) {
        return Vec::new();
    }
    match value.get("action").and_then(Value::as_str) {
        Some("find_name") => {
            let entries = ordered_string_entries_from_array(value, "results");
            if entries.len() >= 2 {
                return entries;
            }
        }
        Some("grep_text") => {
            let entries = ordered_string_entries_from_array(value, "name_results");
            if entries.len() >= 2 {
                return entries;
            }
        }
        _ => {}
    }
    for key in ["names", "entries"] {
        let entries = value
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(entry_name_from_listing_value)
            .map(|entry| sanitize_ordered_entry_text(&entry))
            .map(|entry| entry.trim().to_string())
            .filter(|entry| !entry.is_empty())
            .take(MAX_ORDERED_ENTRIES)
            .collect::<Vec<_>>();
        if entries.len() >= 2 {
            return entries;
        }
    }
    if value.get("action").and_then(Value::as_str) == Some("tree_summary") {
        let entries = value
            .pointer("/tree/children")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(entry_name_from_tree_child)
            .map(|entry| sanitize_ordered_entry_text(&entry))
            .map(|entry| entry.trim().to_string())
            .filter(|entry| !entry.is_empty())
            .take(MAX_ORDERED_ENTRIES)
            .collect::<Vec<_>>();
        if entries.len() >= 2 {
            return entries;
        }
    }
    Vec::new()
}

fn ordered_entries_from_listing_json_value(value: &Value, depth: usize) -> Vec<String> {
    if depth > 4 {
        return Vec::new();
    }
    let entries = ordered_entries_from_listing_json_shallow(value);
    if entries.len() >= 2 {
        return entries;
    }
    if let Some(extra) = value.get("extra") {
        let entries = ordered_entries_from_listing_json_value(extra, depth + 1);
        if entries.len() >= 2 {
            return entries;
        }
    }
    Vec::new()
}

fn ordered_entries_from_listing_json(value: &Value) -> Vec<String> {
    ordered_entries_from_listing_json_value(value, 0)
}

fn listing_json_has_entries(value: &Value) -> bool {
    ordered_entries_from_listing_json(value).len() >= 2
}

pub(crate) fn derive_bound_target_from_journal(
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    if let Some(code_workspace) = derive_code_workspace_bound_target_from_journal(journal) {
        return Some(code_workspace);
    }
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
        if let Some(path) = bound_target_from_journal_output_value(&value) {
            return Some(path);
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
        if let Some(value) = parse_journal_step_output(output) {
            let entries = ordered_entries_from_listing_json(&value);
            if entries.len() >= 2 {
                return entries;
            }
        }
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

fn single_visible_scalar_answer(answer_text: &str, answer_messages: &[String]) -> Option<String> {
    if let Some(candidate) = single_nonempty_line(answer_text) {
        return Some(candidate);
    }
    let mut visible_lines = Vec::new();
    for message in answer_messages {
        if crate::finalize::is_execution_summary_message(message) {
            continue;
        }
        if let Some(candidate) = single_nonempty_line(message) {
            visible_lines.push(candidate);
        } else if message.lines().any(|line| !line.trim().is_empty()) {
            return None;
        }
    }
    if visible_lines.len() == 1 {
        visible_lines.pop()
    } else {
        None
    }
}

fn single_nonempty_line(text: &str) -> Option<String> {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let line = lines.next()?;
    if lines.next().is_some() {
        return None;
    }
    Some(sanitize_ordered_entry_text(line))
}

fn selected_entry_index_from_visible_scalar_answer(
    prior: &FollowupFrame,
    answer_text: &str,
    answer_messages: &[String],
) -> Option<usize> {
    let target = single_visible_scalar_answer(answer_text, answer_messages)?;
    selected_entry_index_for_target(prior, &target)
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
    if frame.selected_entry_index.is_some()
        && frame.ordered_entries.is_empty()
        && !prior.ordered_entries.is_empty()
    {
        frame.ordered_entries = prior.ordered_entries.clone();
        if frame.bound_target.is_none() {
            frame.bound_target = prior.bound_target.clone();
        }
        if matches!(frame.op_kind, FollowupOpKind::Generic)
            && matches!(prior.op_kind, FollowupOpKind::List)
        {
            frame.op_kind = FollowupOpKind::List;
        }
    }
    if frame.selected_entry_index.is_none() {
        if let Some(bound_target) = frame.bound_target.as_deref() {
            frame.selected_entry_index = selected_entry_index_for_target(&frame, bound_target);
        }
    }
    frame
}

fn frame_has_session_anchor(frame: &FollowupFrame) -> bool {
    !matches!(frame.op_kind, FollowupOpKind::Generic)
        || frame
            .bound_target
            .as_deref()
            .is_some_and(|target| !target.trim().is_empty())
        || !frame.ordered_entries.is_empty()
        || frame.selected_entry_index.is_some()
        || frame.slice_spec.is_some()
        || frame.unresolved_slot.is_some()
}

fn derive_frame_for_ask_outcome(
    prior_frame: Option<&FollowupFrame>,
    task_id: &str,
    prompt: &str,
    route_result: &crate::IntentOutputContract,
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
            route_result.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Url
                | crate::OutputLocatorKind::CurrentWorkspace
                | crate::OutputLocatorKind::None
        )
        && (false || route_result.locator_hint.trim().is_empty());
    let op_kind = op_kind_from_route(route_result, unresolved_locator, journal);
    let code_workspace_bound_target = matches!(op_kind, FollowupOpKind::CodeWorkspace)
        .then(|| derive_code_workspace_bound_target_from_route_and_journal(route_result, journal))
        .flatten();
    let should_extract_answer_entries = matches!(op_kind, FollowupOpKind::List)
        || (matches!(op_kind, FollowupOpKind::Delivery)
            && answer_contains_multiple_delivery_targets(answer_text, answer_messages))
        || observed_facts.ordered_entries.len() >= 2;
    let mut ordered_entries = if should_extract_answer_entries {
        observed_facts.ordered_entries.clone()
    } else {
        Vec::new()
    };
    ordered_entries.truncate(MAX_ORDERED_ENTRIES);
    let selected_entry_index = prior_frame.and_then(|prior| {
        selected_entry_index_from_visible_scalar_answer(prior, answer_text, answer_messages)
    });
    let frame = FollowupFrame {
        source_request: prompt.trim().to_string(),
        op_kind,
        bound_target: (!unresolved_locator)
            .then(|| {
                if code_workspace_bound_target.is_some() {
                    return code_workspace_bound_target.clone();
                }
                if !crate::observed_facts::route_allows_observed_bound_target(route_result) {
                    return None;
                }
                observed_facts.bound_target.clone().or_else(|| {
                    let hint = route_result.locator_hint.trim();
                    (!hint.is_empty()).then(|| hint.to_string())
                })
            })
            .flatten(),
        ordered_entries,
        selected_entry_index,
        slice_spec: observed_facts.slice_spec.clone(),
        output_shape: Some(route_result.response_shape.as_str().to_string()),
        unresolved_slot: unresolved_locator.then_some(FollowupUnresolvedSlot::Locator),
        source_task_id: task_id.to_string(),
        updated_at_ts: now_ts,
        expires_at_ts: now_ts + FOLLOWUP_FRAME_TTL_SECS,
    };
    merge_frame_with_prior(frame, prior_frame)
}

#[cfg(test)]
pub(crate) fn replace_active_frame_from_ask_outcome(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
    route_result: &crate::IntentOutputContract,
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
    if !frame_has_session_anchor(&frame) {
        if let Some(prior_frame) = prior_frame.as_ref() {
            return Some(prior_frame.source_task_id.clone());
        }
        if let Err(err) = clear_frame(state, task) {
            tracing::warn!(
                "followup_frame clear empty generic failed task_id={} err={}",
                task.task_id,
                err
            );
        }
        return None;
    }
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
    route_result: &crate::IntentOutputContract,
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
    if !frame_has_session_anchor(&frame) {
        if let Some(prior_frame) = prior_frame {
            return Ok(Some(prior_frame.source_task_id.clone()));
        }
        clear_frame_tx(tx, task)?;
        return Ok(None);
    }
    persist_frame_tx(tx, task, &frame)?;
    Ok(Some(frame.source_task_id))
}

#[cfg(test)]
#[path = "followup_frame_tests.rs"]
mod tests;
