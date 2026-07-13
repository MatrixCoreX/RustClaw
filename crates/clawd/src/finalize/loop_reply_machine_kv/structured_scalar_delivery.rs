use serde_json::Value;

use crate::agent_engine::{append_delivery_message, LoopState};

pub(super) fn replace_current_field_selector_with_value(
    task_id: &str,
    loop_state: &mut LoopState,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    current: &str,
) -> bool {
    let Some(answer) = latest_value_for_current_field_selector(loop_state, current) else {
        return false;
    };
    delivery_messages.clear();
    delivery_messages.push(answer.clone());
    loop_state.delivery_messages.clear();
    append_delivery_message(task_id, &mut loop_state.delivery_messages, answer.clone());
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: loop_state.executed_step_results.len(),
        ..Default::default()
    });
    true
}

fn latest_value_for_current_field_selector(
    loop_state: &LoopState,
    current: &str,
) -> Option<String> {
    let selector = current.trim();
    if selector.is_empty() || selector.contains('\n') {
        return None;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok())
        .filter_map(|step| step.output.as_deref())
        .filter_map(|output| serde_json::from_str::<Value>(output.trim()).ok())
        .find_map(|value| value_for_field_selector(&value, selector, 0))
        .filter(|answer| answer.trim() != selector)
}

fn value_for_field_selector(value: &Value, selector: &str, depth: usize) -> Option<String> {
    if depth > 3 {
        return None;
    }
    if action_is_field_read(value) {
        if let Some(results) = value.get("results").and_then(Value::as_array) {
            if let Some(answer) = results
                .iter()
                .find_map(|item| value_from_field_item(item, selector))
            {
                return Some(answer);
            }
        }
        if let Some(answer) = value_from_field_item(value, selector) {
            return Some(answer);
        }
    }
    if let Some(answer) = value
        .get("extra")
        .and_then(|extra| value_for_field_selector(extra, selector, depth.saturating_add(1)))
    {
        return Some(answer);
    }
    value
        .get("text")
        .and_then(Value::as_str)
        .and_then(|text| serde_json::from_str::<Value>(text.trim()).ok())
        .and_then(|nested| value_for_field_selector(&nested, selector, depth.saturating_add(1)))
}

fn action_is_field_read(value: &Value) -> bool {
    matches!(
        value.get("action").and_then(Value::as_str).map(str::trim),
        Some("extract_field" | "read_field" | "extract_fields" | "read_fields")
    )
}

fn value_from_field_item(value: &Value, selector: &str) -> Option<String> {
    let field_path = value
        .get("resolved_field_path")
        .or_else(|| value.get("field_path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|field| !field.is_empty())?;
    if field_path != selector {
        return None;
    }
    if value.get("exists").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    value
        .get("value_text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
        .or_else(|| value.get("value").and_then(scalar_value_text))
}

fn scalar_value_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => {
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_string())
        }
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::Array(_) | Value::Object(_) => Some(value.to_string()),
    }
}
