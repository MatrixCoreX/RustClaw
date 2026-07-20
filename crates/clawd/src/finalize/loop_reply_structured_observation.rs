use std::path::Path;

use tracing::info;

use crate::agent_engine::AgentRunContext;
use crate::AppState;

use super::log_deterministic_delivery_record;

fn json_scalar_display(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => Some("null".to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        }
        _ => None,
    }
}

fn compact_json_item_label(key: Option<&str>, value: &serde_json::Value) -> Option<String> {
    let key = key.map(str::trim).filter(|key| !key.is_empty());
    match (key, json_scalar_display(value)) {
        (Some(key), Some(value)) => Some(format!("{key}={value}")),
        (Some(key), None) => Some(key.to_string()),
        (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn structured_container_summary_from_value(
    field_path: &str,
    value: &serde_json::Value,
    _prefer_english: bool,
) -> Option<String> {
    let field_path = field_path.trim();
    if field_path.is_empty() {
        return None;
    }
    const MAX_PREVIEW_ITEMS: usize = 6;
    match value {
        serde_json::Value::Object(map) => {
            let mut entries = map
                .iter()
                .filter_map(|(key, value)| compact_json_item_label(Some(key), value))
                .take(MAX_PREVIEW_ITEMS)
                .collect::<Vec<_>>();
            if entries.is_empty() {
                entries = map.keys().take(MAX_PREVIEW_ITEMS).cloned().collect();
            }
            let mut lines = structured_container_machine_header(field_path, "object", map.len());
            lines.push(format!("truncated={}", map.len() > entries.len()));
            for (idx, entry) in entries.iter().enumerate() {
                push_structured_machine_line(&mut lines, &format!("item.{}.label", idx + 1), entry);
            }
            Some(lines.join("\n"))
        }
        serde_json::Value::Array(items) => {
            let entries = items
                .iter()
                .filter_map(|value| compact_json_item_label(None, value))
                .take(MAX_PREVIEW_ITEMS)
                .collect::<Vec<_>>();
            let mut lines = structured_container_machine_header(field_path, "array", items.len());
            lines.push(format!("truncated={}", items.len() > entries.len()));
            for (idx, entry) in entries.iter().enumerate() {
                push_structured_machine_line(&mut lines, &format!("item.{}.label", idx + 1), entry);
            }
            Some(lines.join("\n"))
        }
        _ => None,
    }
}

fn structured_container_machine_header(
    field_path: &str,
    container_kind: &str,
    item_count: usize,
) -> Vec<String> {
    let mut lines = vec![
        "message_key=clawd.msg.structured_container.observed".to_string(),
        "reason_code=structured_container_observed".to_string(),
    ];
    push_structured_machine_line(&mut lines, "field_path", field_path);
    lines.push(format!("container_kind={container_kind}"));
    lines.push(format!("item_count={item_count}"));
    lines.push(format!("is_empty={}", item_count == 0));
    lines
}

fn push_structured_machine_line(lines: &mut Vec<String>, key: &str, value: &str) {
    let value = crate::truncate_for_agent_trace(
        &crate::visible_text::sanitize_user_visible_text(value).replace('\n', " "),
    );
    lines.push(format!("{key}={value}"));
}

fn structured_container_from_extract_value(
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if !matches!(
        value.get("action").and_then(|value| value.as_str()),
        Some("extract_field" | "read_field")
    ) {
        return None;
    }
    if !value
        .get("exists")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    let field_path = value
        .get("resolved_field_path")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            value
                .get("field_path")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })?;
    structured_container_summary_from_value(field_path, value.get("value")?, prefer_english)
}

pub(super) fn deterministic_structured_container_summary_answer(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.output_contract())?;
    if !route.requires_content_evidence || route.delivery_required {
        return None;
    }
    if !matches!(
        route.response_shape,
        crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
    ) {
        return None;
    }
    if !matches!(
        route.semantic_kind,
        crate::OutputSemanticKind::None | crate::OutputSemanticKind::ContentExcerptSummary
    ) {
        return None;
    }
    let _ = state;
    let _ = user_text;
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| {
            step.is_ok() && matches!(step.skill.as_str(), "system_basic" | "config_basic")
        })
        .filter_map(|step| step.output.as_deref())
        .filter_map(|output| serde_json::from_str::<serde_json::Value>(output).ok())
        .find_map(|value| structured_container_from_extract_value(&value, false))
}

pub(super) fn direct_db_basic_observed_answer(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.output_contract())?;
    if route.delivery_required {
        return None;
    }
    if matches!(route.response_shape, crate::OutputResponseShape::FileToken) {
        return None;
    }
    let _ = state;
    let _ = user_text;
    let answer = loop_state
        .executed_step_results
        .iter()
        .rev()
        .find(|step| {
            step.is_ok()
                && step.skill == "db_basic"
                && step
                    .output
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|output| !output.is_empty())
        })
        .and_then(|step| step.output.as_deref())
        .and_then(|output| db_basic_rows_answer_from_output_for_route(route, output))?;
    if answer.trim().is_empty() {
        return None;
    }
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            parsed: true,
            contract_ok: true,
            completion_ok: Some(true),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: 1,
            ..Default::default()
        },
    ))
}

fn db_basic_rows_answer_from_output_for_route(
    route: &crate::IntentOutputContract,
    output: &str,
) -> Option<String> {
    db_basic_rows_answer_from_output_with_scalar_count(
        output,
        route.clone().response_shape == crate::OutputResponseShape::Scalar
            && route.semantic_kind_is(crate::OutputSemanticKind::ScalarCount),
    )
}

fn db_basic_rows_answer_from_output_with_scalar_count(
    output: &str,
    scalar_count: bool,
) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    let result = value
        .get("columns")
        .and_then(|_| value.get("rows"))
        .map(|_| &value)
        .or_else(|| value.get("result"))
        .or_else(|| value.get("extra").and_then(|extra| extra.get("result")))?;
    let columns = result
        .get("columns")
        .and_then(|value| value.as_array())?
        .iter()
        .filter_map(|value| value.as_str().map(str::trim))
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if columns.is_empty() {
        return None;
    }
    let rows = result.get("rows").and_then(|value| value.as_array())?;
    if scalar_count {
        if rows.len() == 1 && columns.len() == 1 {
            return rows
                .first()
                .and_then(|row| db_row_column_value(row, &columns[0], 0));
        }
        return Some(rows.len().to_string());
    }
    if rows.is_empty() {
        let mut lines = vec![
            "message_key=clawd.msg.db.rows.observed".to_string(),
            "reason_code=db_rows_observed".to_string(),
            "row_count=0".to_string(),
            format!("column_count={}", columns.len()),
        ];
        for (idx, column) in columns.iter().enumerate() {
            push_structured_machine_line(&mut lines, &format!("column.{}", idx + 1), column);
        }
        return Some(lines.join("\n"));
    }
    if rows.len() == 1 && columns.len() == 1 {
        return rows
            .first()
            .and_then(|row| db_row_column_value(row, &columns[0], 0));
    }
    if columns.len() == 1 {
        let lines = rows
            .iter()
            .filter_map(|row| db_row_column_value(row, &columns[0], 0))
            .take(50)
            .collect::<Vec<_>>();
        return (!lines.is_empty()).then(|| lines.join("\n"));
    }

    let lines = rows
        .iter()
        .filter_map(|row| db_row_line(row, &columns))
        .take(50)
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn db_row_line(row: &serde_json::Value, columns: &[String]) -> Option<String> {
    let values = columns
        .iter()
        .enumerate()
        .filter_map(|(idx, column)| {
            db_row_column_value(row, column, idx).map(|value| format!("{column}: {value}"))
        })
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| values.join(", "))
}

fn db_row_column_value(row: &serde_json::Value, column: &str, index: usize) -> Option<String> {
    match row {
        serde_json::Value::Object(map) => map.get(column).and_then(json_scalar_display),
        serde_json::Value::Array(values) => values.get(index).and_then(json_scalar_display),
        _ => None,
    }
}

fn structured_file_format_for_path(path: &str) -> Option<&'static str> {
    match Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => Some("json"),
        Some("toml") => Some("toml"),
        _ => None,
    }
}

fn broad_structured_read_range_from_value(value: &serde_json::Value) -> Option<(String, String)> {
    if value.get("action").and_then(|value| value.as_str()) != Some("read_range") {
        return None;
    }
    if !matches!(
        value.get("mode").and_then(|value| value.as_str()),
        Some("head" | "full" | "all")
    ) {
        return None;
    }
    if value
        .get("requested_n")
        .and_then(|value| value.as_u64())
        .is_some_and(|requested_n| requested_n < 50)
    {
        return None;
    }
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    let format = value
        .get("format")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|format| matches!(*format, "json" | "toml"))
        .or_else(|| structured_file_format_for_path(path))?;
    Some((path.to_string(), format.to_string()))
}

fn latest_broad_structured_read_range(
    loop_state: &crate::agent_engine::LoopState,
) -> Option<(String, String)> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "system_basic" | "fs_basic"))
        .filter_map(|step| step.output.as_deref())
        .filter_map(|output| serde_json::from_str::<serde_json::Value>(output).ok())
        .find_map(|value| broad_structured_read_range_from_value(&value))
}

pub(super) fn message_is_non_answer_separator(message: &str) -> bool {
    crate::finalize::is_non_answer_separator_message(message)
}

pub(super) fn discard_non_answer_separator_delivery_for_broad_structured_read(
    task_id: &str,
    loop_state: &mut crate::agent_engine::LoopState,
) -> bool {
    if latest_broad_structured_read_range(loop_state).is_none() {
        return false;
    }
    let before_len = loop_state.delivery_messages.len();
    loop_state.delivery_messages.retain(|message| {
        crate::finalize::is_execution_summary_message(message)
            || !message_is_non_answer_separator(message)
    });
    let removed = before_len != loop_state.delivery_messages.len();
    if removed {
        if loop_state
            .last_user_visible_respond
            .as_deref()
            .is_some_and(message_is_non_answer_separator)
        {
            loop_state.last_user_visible_respond = None;
        }
        if loop_state
            .last_publishable_synthesis_output
            .as_deref()
            .is_some_and(message_is_non_answer_separator)
        {
            loop_state.last_publishable_synthesis_output = None;
        }
        info!(
            "delivery discard_non_answer_separator_after_structured_read task_id={}",
            task_id
        );
        log_deterministic_delivery_record(
            task_id,
            "discard_non_answer_separator_after_structured_read",
            "discarded",
            None,
            0,
        );
    }
    removed
}
