use rusqlite::{params, Connection};
use serde_json::Value;

use crate::{utf8_safe_prefix, AppState};

use super::{
    effective_user_key, query_recent_memories_for_chat, LLM_SHORT_TERM_MEMORY_PREFIX,
    MEMORY_ROLE_ASSISTANT, MEMORY_ROLE_USER, MEMORY_SAFETY_FLAG_INJECTION_LIKE,
    RETRIEVAL_SUCCESS_STATE_FAILED,
};

fn strip_llm_reply_memory_prefix(text: &str) -> &str {
    text.trim()
        .strip_prefix(LLM_SHORT_TERM_MEMORY_PREFIX)
        .unwrap_or_else(|| text.trim())
        .trim()
}

pub(crate) fn is_transient_assistant_context_text_basic(text: &str) -> bool {
    let trimmed = strip_llm_reply_memory_prefix(text);
    trimmed.is_empty()
        || trimmed == provider_unavailable_assistant_placeholder()
        || trimmed == clarify_assistant_placeholder()
        || crate::finalize::is_execution_summary_message(trimmed)
}

pub(super) fn is_transient_assistant_context_text(state: &AppState, text: &str) -> bool {
    let trimmed = strip_llm_reply_memory_prefix(text);
    is_transient_assistant_context_text_basic(trimmed)
        || crate::fallback::is_known_clarify_fallback_text(state, trimmed)
}

fn assistant_context_text_for_recall<'a>(state: &AppState, text: &'a str) -> Option<&'a str> {
    if is_transient_assistant_context_text(state, text) {
        None
    } else {
        Some(strip_llm_reply_memory_prefix(text))
    }
}

fn extract_last_turn_user_text_from_payload(payload_json: &str) -> Option<String> {
    let payload = serde_json::from_str::<Value>(payload_json).ok()?;
    let text = payload.get("text").and_then(Value::as_str)?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AssistantContextReplyKind {
    Normal,
    ClarifyPlaceholder,
    ProviderUnavailablePlaceholder,
}

pub(super) fn provider_unavailable_assistant_placeholder() -> &'static str {
    "[provider_unavailable_reply_omitted]"
}

pub(super) fn clarify_assistant_placeholder() -> &'static str {
    "[clarification_requested]"
}

fn raw_output_looks_like_structured_json(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .map(|value| value.is_object() || value.is_array())
        .unwrap_or(false)
}

fn raw_output_looks_like_linewise_json(text: &str) -> bool {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    !lines.is_empty()
        && lines
            .iter()
            .all(|line| raw_output_looks_like_structured_json(line))
}

fn value_is_machine_envelope(value: &Value) -> bool {
    value
        .get("output_format")
        .and_then(Value::as_str)
        .is_some_and(|format| format == "machine_json")
        && value
            .get("owner_layer")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|owner| !owner.is_empty())
}

fn visible_text_looks_like_machine_envelope(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .map(|value| value_is_machine_envelope(&value))
        .unwrap_or(false)
}

fn visible_text_looks_like_linewise_machine_envelope(text: &str) -> bool {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    !lines.is_empty()
        && lines
            .iter()
            .all(|line| visible_text_looks_like_machine_envelope(line))
}

fn normalize_read_range_excerpt_for_recent_turns(excerpt: &str) -> Option<String> {
    let lines = excerpt
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.split_once('|')
                .filter(|(prefix, _)| {
                    !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit())
                })
                .map(|(_, rest)| rest.trim_start().to_string())
                .unwrap_or_else(|| line.trim().to_string())
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn normalize_observed_listing_for_recent_turns(text: &str) -> Option<String> {
    if raw_output_looks_like_structured_json(text) || raw_output_looks_like_linewise_json(text) {
        return None;
    }
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("bash: warning: setlocale:")
                && !line.starts_with("warning: setlocale:")
        })
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if lines.len() < 2 {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn strip_ordered_list_prefix(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let digit_count = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digit_count == 0 {
        return None;
    }
    let rest = &trimmed[digit_count..];
    let stripped = if let Some(rest) = rest.strip_prefix(". ") {
        rest
    } else if let Some(rest) = rest.strip_prefix(") ") {
        rest
    } else if let Some(rest) = rest.strip_prefix("、") {
        rest
    } else {
        return None;
    };
    let stripped = stripped.trim();
    (!stripped.is_empty()).then(|| stripped.to_string())
}

fn is_ordered_list_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    let digit_count = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digit_count == 0 {
        return false;
    }
    let rest = &trimmed[digit_count..];
    rest.starts_with(". ") || rest.starts_with(") ") || rest.starts_with("、")
}

fn looks_like_wrapped_ordered_listing_answer(text: &str) -> bool {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    lines.len() >= 3
        && lines
            .iter()
            .skip(1)
            .filter(|line| is_ordered_list_line(line))
            .count()
            >= 2
}

fn is_delivery_token_line(line: &str) -> bool {
    let trimmed = line.trim();
    matches!(
        trimmed.split_once(':').map(|(prefix, _)| prefix),
        Some("FILE")
            | Some("IMAGE_FILE")
            | Some("IMAGE_URL")
            | Some("VIDEO_URL")
            | Some("FILE_URL")
            | Some("MEDIA_URL")
    )
}

fn extract_delivery_token_target(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let (_, rest) = trimmed.split_once(':')?;
    let rest = rest.trim();
    (!rest.is_empty()).then(|| rest.to_string())
}

fn normalize_recent_assistant_ordered_entry(entry: &str) -> Option<String> {
    let trimmed = entry
        .trim()
        .trim_matches(|c| matches!(c, '"' | '\'' | '`' | '“' | '”' | '‘' | '’'))
        .trim_end_matches(|c| matches!(c, ';' | '；' | ',' | '，' | '。'))
        .trim();
    if trimmed.is_empty() {
        return None;
    }
    if is_delivery_token_line(trimmed) {
        return extract_delivery_token_target(trimmed);
    }
    Some(trimmed.to_string())
}

fn looks_like_locatorish_recent_assistant_entry(entry: &str) -> bool {
    let trimmed = entry.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains('.')
        || (!trimmed.contains(char::is_whitespace) && trimmed.len() <= 128)
}

pub(super) fn ordered_entries_from_assistant_reply(text: &str, max_entries: usize) -> Vec<String> {
    let max_entries = max_entries.max(2);

    let numbered = text
        .lines()
        .filter_map(strip_ordered_list_prefix)
        .filter_map(|entry| normalize_recent_assistant_ordered_entry(&entry))
        .take(max_entries)
        .collect::<Vec<_>>();
    if numbered.len() >= 2 {
        return numbered;
    }

    let token_lines = text
        .lines()
        .filter_map(extract_delivery_token_target)
        .take(max_entries)
        .collect::<Vec<_>>();
    if token_lines.len() >= 2 {
        return token_lines;
    }

    let semicolon_source = text
        .rsplit_once('：')
        .map(|(_, tail)| tail)
        .or_else(|| text.rsplit_once(':').map(|(_, tail)| tail))
        .unwrap_or(text);
    let semicolon_entries = semicolon_source
        .split([';', '；'])
        .filter_map(normalize_recent_assistant_ordered_entry)
        .collect::<Vec<_>>();
    if semicolon_entries.len() >= 2
        && semicolon_entries
            .iter()
            .filter(|entry| looks_like_locatorish_recent_assistant_entry(entry))
            .count()
            >= 2
    {
        return semicolon_entries.into_iter().take(max_entries).collect();
    }

    Vec::new()
}

fn format_recent_assistant_ordered_entries(text: &str) -> Option<String> {
    let entries = ordered_entries_from_assistant_reply(text, 10);
    if entries.len() < 2 {
        return None;
    }
    Some(
        entries
            .iter()
            .enumerate()
            .map(|(idx, entry)| format!("{}:{}", idx + 1, entry))
            .collect::<Vec<_>>()
            .join(" | "),
    )
}

fn extract_observed_step_text_for_recent_turns(value: &Value) -> Option<String> {
    let step_results = value
        .get("task_journal")
        .and_then(|v| v.get("trace"))
        .and_then(|v| v.get("step_results"))
        .and_then(Value::as_array)?;
    for step in step_results.iter().rev() {
        let skill = step
            .get("skill")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let output = step
            .get("output_excerpt")
            .or_else(|| step.get("output"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty());
        if skill == "system_basic" {
            let output = output?;
            let parsed = serde_json::from_str::<Value>(output).ok()?;
            let action = parsed
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if action == "read_range" {
                if let Some(excerpt) = parsed
                    .get("excerpt")
                    .and_then(Value::as_str)
                    .and_then(normalize_read_range_excerpt_for_recent_turns)
                {
                    let path = parsed
                        .get("resolved_path")
                        .or_else(|| parsed.get("path"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|v| !v.is_empty());
                    return Some(match path {
                        Some(path) => format!("read_range path={path}\n{excerpt}"),
                        None => excerpt,
                    });
                }
            }
        }
    }
    None
}

fn extract_observed_listing_text_for_recent_turns(value: &Value) -> Option<String> {
    let step_results = value
        .get("task_journal")
        .and_then(|v| v.get("trace"))
        .and_then(|v| v.get("step_results"))
        .and_then(Value::as_array)?;
    for step in step_results.iter().rev() {
        let skill = step
            .get("skill")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let output = step
            .get("output_excerpt")
            .or_else(|| step.get("output"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())?;
        if matches!(skill, "run_cmd" | "list_dir") {
            if let Some(listing) = normalize_observed_listing_for_recent_turns(output) {
                return Some(listing);
            }
        }
        if skill == "system_basic" {
            let parsed = serde_json::from_str::<Value>(output).ok()?;
            let action = parsed
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if action == "inventory_dir" {
                if let Some(listing) = parsed
                    .get("entries")
                    .and_then(Value::as_array)
                    .map(|entries| {
                        entries
                            .iter()
                            .filter_map(|entry| entry.get("name").and_then(Value::as_str))
                            .map(str::trim)
                            .filter(|name| !name.is_empty())
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                    })
                    .filter(|entries| entries.len() >= 2)
                    .map(|entries| entries.join("\n"))
                {
                    return Some(listing);
                }
            }
        }
    }
    None
}

pub(super) fn extract_result_text_for_recent_turns(value: &Value) -> Option<String> {
    if let Some(observed_text) = extract_observed_step_text_for_recent_turns(value) {
        return Some(observed_text);
    }
    let direct_text = value
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string);
    let first_message = value
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string);
    let final_answer = value
        .get("task_journal")
        .and_then(|v| v.get("summary"))
        .and_then(|v| v.get("final_answer"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string);
    let should_prefer_observed = direct_text
        .iter()
        .chain(first_message.iter())
        .chain(final_answer.iter())
        .any(|text| {
            visible_text_looks_like_machine_envelope(text)
                || visible_text_looks_like_linewise_machine_envelope(text)
        });
    if should_prefer_observed {
        if let Some(observed_text) = extract_observed_step_text_for_recent_turns(value) {
            return Some(observed_text);
        }
    }
    let should_prefer_observed_listing = direct_text
        .iter()
        .chain(first_message.iter())
        .chain(final_answer.iter())
        .any(|text| looks_like_wrapped_ordered_listing_answer(text));
    if should_prefer_observed_listing {
        if let Some(observed_listing) = extract_observed_listing_text_for_recent_turns(value) {
            return Some(observed_listing);
        }
    }
    direct_text.or(first_message).or(final_answer).or_else(|| {
        value
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(ToString::to_string)
    })
}

pub(super) fn classify_assistant_context_reply_kind(
    parsed_result: Option<&Value>,
    assistant_text: &str,
    // The caller owns localized fallback detection. This function only reads
    // structured task metadata to avoid phrase-based language branching here.
    is_fallback: impl Fn(&str) -> bool,
) -> AssistantContextReplyKind {
    if is_fallback(assistant_text) {
        return AssistantContextReplyKind::ProviderUnavailablePlaceholder;
    }
    let summary = parsed_result
        .and_then(|value| value.get("task_journal"))
        .and_then(|value| value.get("summary"));
    let final_status = summary
        .and_then(|value| value.get("final_status"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if final_status.eq_ignore_ascii_case("clarify") {
        return AssistantContextReplyKind::ClarifyPlaceholder;
    }
    AssistantContextReplyKind::Normal
}

fn extract_last_turn_assistant_text_from_task(
    state: &AppState,
    status: &str,
    result_json: Option<&str>,
    error_text: Option<&str>,
) -> Option<String> {
    if status.eq_ignore_ascii_case(RETRIEVAL_SUCCESS_STATE_FAILED) {
        if let Some(err) = error_text.map(str::trim).filter(|v| !v.is_empty()) {
            return Some(err.to_string());
        }
    }
    let result_json = result_json.map(str::trim).filter(|v| !v.is_empty())?;
    let parsed = serde_json::from_str::<Value>(result_json).ok();
    let assistant_text = if let Some(val) = parsed.as_ref() {
        extract_result_text_for_recent_turns(val)
    } else {
        None
    };
    let assistant_text = assistant_text.unwrap_or_else(|| result_json.to_string());
    match classify_assistant_context_reply_kind(parsed.as_ref(), &assistant_text, |t| {
        crate::fallback::is_known_clarify_fallback_text(state, t)
    }) {
        AssistantContextReplyKind::Normal => Some(assistant_text),
        AssistantContextReplyKind::ClarifyPlaceholder => {
            Some(clarify_assistant_placeholder().to_string())
        }
        AssistantContextReplyKind::ProviderUnavailablePlaceholder => {
            Some(provider_unavailable_assistant_placeholder().to_string())
        }
    }
}

fn query_recent_terminal_ask_turn_for_chat(
    state: &AppState,
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
) -> anyhow::Result<Option<(String, String)>> {
    let mut stmt = db.prepare(
        "SELECT payload_json, result_json, error_text, status
         FROM tasks
         WHERE user_id = ?1
           AND chat_id = ?2
           AND user_key = ?3
           AND kind = 'ask'
           AND status IN ('succeeded', 'failed')
         ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) DESC
         LIMIT 8",
    )?;
    let rows = stmt.query_map(params![user_id, chat_id, user_key], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    for row in rows {
        let (payload_json, result_json, error_text, status) = row?;
        let Some(user_text) = extract_last_turn_user_text_from_payload(&payload_json) else {
            continue;
        };
        let Some(assistant_text) = extract_last_turn_assistant_text_from_task(
            state,
            &status,
            result_json.as_deref(),
            error_text.as_deref(),
        ) else {
            continue;
        };
        if assistant_text == provider_unavailable_assistant_placeholder()
            || assistant_text == clarify_assistant_placeholder()
            || crate::fallback::is_known_clarify_fallback_text(state, &assistant_text)
        {
            continue;
        }
        if assistant_text.trim().is_empty() {
            continue;
        }
        return Ok(Some((user_text, assistant_text)));
    }
    Ok(None)
}

fn query_recent_terminal_ask_turns_for_chat(
    state: &AppState,
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    limit: usize,
) -> anyhow::Result<Vec<(String, String)>> {
    let limit = limit.max(1).min(12);
    let mut stmt = db.prepare(
        "SELECT payload_json, result_json, error_text, status
         FROM tasks
         WHERE user_id = ?1
           AND chat_id = ?2
           AND user_key = ?3
           AND kind = 'ask'
           AND status IN ('succeeded', 'failed')
         ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) DESC
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(params![user_id, chat_id, user_key, limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut out: Vec<(String, String)> = Vec::new();
    for row in rows {
        let (payload_json, result_json, error_text, status) = row?;
        let Some(user_text) = extract_last_turn_user_text_from_payload(&payload_json) else {
            continue;
        };
        let Some(assistant_text) = extract_last_turn_assistant_text_from_task(
            state,
            &status,
            result_json.as_deref(),
            error_text.as_deref(),
        ) else {
            continue;
        };
        if assistant_text == provider_unavailable_assistant_placeholder()
            || assistant_text == clarify_assistant_placeholder()
            || crate::fallback::is_known_clarify_fallback_text(state, &assistant_text)
        {
            continue;
        }
        if assistant_text.trim().is_empty() {
            continue;
        }
        out.push((user_text, assistant_text));
    }
    Ok(out)
}

fn format_last_turn_full_context(
    user_content: &str,
    assistant_content: &str,
    max_segment_chars: usize,
    max_total_chars: usize,
) -> String {
    let user_text = utf8_safe_prefix(user_content.trim(), max_segment_chars).to_string();
    let assistant_text = utf8_safe_prefix(assistant_content.trim(), max_segment_chars).to_string();
    let formatted = format!(
        "[LAST_TURN_FULL]\nUser: {}\nAssistant: {}\n[/LAST_TURN_FULL]",
        user_text, assistant_text
    );
    if formatted.len() > max_total_chars {
        let truncated = utf8_safe_prefix(&formatted, max_total_chars).to_string();
        if !truncated.ends_with("[/LAST_TURN_FULL]") {
            let mut out = truncated;
            if out.len() + 18 <= max_total_chars {
                out.push_str("[/LAST_TURN_FULL]");
            }
            out
        } else {
            truncated
        }
    } else {
        formatted
    }
}

pub(crate) fn build_recent_turns_full_context(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
    max_turns: usize,
    max_segment_chars: usize,
    max_total_chars: usize,
) -> String {
    let user_key = effective_user_key(user_key, user_id, chat_id);
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => return "<none>".to_string(),
    };
    let max_turns = max_turns.max(1).min(10);
    let max_segment_chars = max_segment_chars.max(128);
    let max_total_chars = max_total_chars.max(512);
    let turns = query_recent_terminal_ask_turns_for_chat(
        state, &db, user_id, chat_id, &user_key, max_turns,
    )
    .unwrap_or_default();
    if turns.is_empty() {
        return "<none>".to_string();
    }
    let mut out = String::from("### RECENT_TURNS_FULL\n");
    for (idx, (user_text, assistant_text)) in turns.iter().enumerate() {
        let relative = -((idx as i64) + 1);
        let user_view = utf8_safe_prefix(user_text.trim(), max_segment_chars).to_string();
        let assistant_view = utf8_safe_prefix(assistant_text.trim(), max_segment_chars).to_string();
        let turn_block = format!(
            "[TURN {}]\nUser: {}\nAssistant: {}\n[/TURN]\n",
            relative, user_view, assistant_view
        );
        if out.len() + turn_block.len() > max_total_chars {
            break;
        }
        out.push_str(&turn_block);
    }
    if out.trim() == "### RECENT_TURNS_FULL" {
        "<none>".to_string()
    } else {
        out
    }
}

pub(crate) fn build_last_turn_full_context(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
    max_segment_chars: usize,
    max_total_chars: usize,
) -> String {
    let user_key = effective_user_key(user_key, user_id, chat_id);
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => return "<none>".to_string(),
    };
    if let Ok(Some((user_text, assistant_text))) =
        query_recent_terminal_ask_turn_for_chat(state, &db, user_id, chat_id, &user_key)
    {
        return format_last_turn_full_context(
            &user_text,
            &assistant_text,
            max_segment_chars,
            max_total_chars,
        );
    }
    let recent = match query_recent_memories_for_chat(&db, user_id, chat_id, &user_key, 10) {
        Ok(v) => v,
        Err(_) => return "<none>".to_string(),
    };
    let mut found_assistant: Option<(String, String)> = None;
    let mut found_user: Option<(String, String)> = None;
    for (role, content, safety_flag) in &recent {
        if found_assistant.is_none() && role == MEMORY_ROLE_ASSISTANT {
            if let Some(assistant_text) = assistant_context_text_for_recall(state, content) {
                found_assistant = Some((assistant_text.to_string(), safety_flag.clone()));
            }
            continue;
        }
        if found_assistant.is_some() && found_user.is_none() && role == MEMORY_ROLE_USER {
            found_user = Some((content.clone(), safety_flag.clone()));
            break;
        }
    }
    let (user_content, _) = match found_user {
        Some(v) => v,
        None => return "<none>".to_string(),
    };
    let (assistant_content, _) = match found_assistant {
        Some(v) => v,
        None => return "<none>".to_string(),
    };
    format_last_turn_full_context(
        &user_content,
        &assistant_content,
        max_segment_chars,
        max_total_chars,
    )
}

pub(crate) fn build_recent_assistant_replies_context(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
    max_replies: usize,
    preview_chars: usize,
) -> String {
    let max_replies = max_replies.max(1);
    let preview_chars = preview_chars.max(48);
    let user_key = effective_user_key(user_key, user_id, chat_id);
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => return "<none>".to_string(),
    };

    let rows = query_recent_memories_for_chat(&db, user_id, chat_id, &user_key, max_replies * 6)
        .unwrap_or_default();
    if rows.is_empty() {
        return "<none>".to_string();
    }

    let mut lines: Vec<String> = Vec::new();
    for (role, content, safety_flag) in rows {
        if role != MEMORY_ROLE_ASSISTANT {
            continue;
        }
        if state.policy.memory.safety_filter_enabled
            && safety_flag == MEMORY_SAFETY_FLAG_INJECTION_LIKE
        {
            continue;
        }
        let Some(trimmed_content) = assistant_context_text_for_recall(state, &content) else {
            continue;
        };
        let reply_index = lines.len() + 1;
        let relative_index = -(reply_index as i64);
        let preview = utf8_safe_prefix(trimmed_content, preview_chars)
            .replace('\n', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if preview.is_empty() {
            continue;
        }
        let has_code_block = if content.contains("```") {
            "true"
        } else {
            "false"
        };
        let mut line = format!(
            "- turn_id=assistant[{}] relative_index={} short_preview={} has_code_block={}",
            relative_index, relative_index, preview, has_code_block
        );
        if let Some(ordered_entries) = format_recent_assistant_ordered_entries(trimmed_content) {
            line.push_str(" ordered_entries=");
            line.push_str(&ordered_entries);
        }
        lines.push(line);
        if lines.len() >= max_replies {
            break;
        }
    }

    if lines.is_empty() {
        "<none>".to_string()
    } else {
        format!("### RECENT_ASSISTANT_REPLIES\n{}", lines.join("\n"))
    }
}
