use serde_json::Value;
use tracing::info;

use super::SinglePlanEnvelope;
use crate::{AgentAction, AppState, ClaimedTask};

fn parse_plan_action_step(step: &Value, state: &AppState) -> Option<AgentAction> {
    let raw_step = serde_json::to_string(step).ok()?;
    let normalized = crate::parse_agent_action_json_with_repair(&raw_step, state).ok()?;
    serde_json::from_value::<AgentAction>(normalized).ok()
}

fn plan_actions_follow_machine_contract(state: &AppState, actions: &[AgentAction]) -> bool {
    let mut valid = true;
    for action in actions {
        if let AgentAction::CallCapability { capability, args } = action {
            let (resolved, record) =
                crate::capability_resolver::resolve_capability_action_with_record_for_state(
                    state,
                    capability,
                    args.clone(),
                );
            if resolved.is_none() {
                info!(
                    "plan_result_capability_contract_rejected capability={} outcome={} reason_code={}",
                    capability, record.outcome, record.reason_code
                );
                valid = false;
            }
            continue;
        }
        let (executable, args) = match action {
            AgentAction::CallTool { tool, args } => (tool.as_str(), args),
            AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
            _ => continue,
        };
        let canonical = state.resolve_canonical_skill_name(executable);
        for violation in crate::schema_contract::executable_enum_violations(state, &canonical, args)
        {
            info!(
                "plan_result_enum_constraint_rejected executable={} field={} constraint=enum",
                canonical, violation.field
            );
            valid = false;
        }
    }
    valid
}

fn parse_xml_tool_parameter_value(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        Value::Null
    } else if let Some(value) =
        crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(trimmed)
    {
        value
    } else {
        Value::String(trimmed.to_string())
    }
}

pub(super) fn extract_xml_tool_call_steps(raw: &str) -> Vec<Value> {
    let mut steps = Vec::new();
    let mut search_from = 0usize;
    while let Some(invoke_rel) = raw[search_from..].find("<invoke name=\"") {
        let invoke_start = search_from + invoke_rel;
        let name_start = invoke_start + "<invoke name=\"".len();
        let Some(name_end_rel) = raw[name_start..].find('"') else {
            break;
        };
        let name_end = name_start + name_end_rel;
        let invoke_name = raw[name_start..name_end].trim();
        let Some(tag_end_rel) = raw[name_end..].find('>') else {
            break;
        };
        let body_start = name_end + tag_end_rel + 1;
        let Some(close_rel) = raw[body_start..].find("</invoke>") else {
            break;
        };
        let body_end = body_start + close_rel;
        let body = &raw[body_start..body_end];
        search_from = body_end + "</invoke>".len();

        let mut params = serde_json::Map::new();
        let mut param_search = 0usize;
        while let Some(param_rel) = body[param_search..].find("<parameter name=\"") {
            let param_start = param_search + param_rel;
            let name_start = param_start + "<parameter name=\"".len();
            let Some(name_end_rel) = body[name_start..].find('"') else {
                break;
            };
            let name_end = name_start + name_end_rel;
            let param_name = body[name_start..name_end].trim();
            let Some(tag_end_rel) = body[name_end..].find('>') else {
                break;
            };
            let value_start = name_end + tag_end_rel + 1;
            let Some(close_rel) = body[value_start..].find("</parameter>") else {
                break;
            };
            let value_end = value_start + close_rel;
            params.insert(
                param_name.to_string(),
                parse_xml_tool_parameter_value(&body[value_start..value_end]),
            );
            param_search = value_end + "</parameter>".len();
        }

        let step = match invoke_name {
            "call_skill" => {
                let skill = params.get("skill").and_then(|v| v.as_str()).map(str::trim);
                let args = params
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                skill.map(|skill| {
                    serde_json::json!({
                        "type": "call_skill",
                        "skill": skill,
                        "args": args,
                    })
                })
            }
            "call_tool" => {
                let tool = params.get("tool").and_then(|v| v.as_str()).map(str::trim);
                let args = params
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                tool.map(|tool| {
                    serde_json::json!({
                        "type": "call_tool",
                        "tool": tool,
                        "args": args,
                    })
                })
            }
            "call_capability" => {
                let capability = params
                    .get("capability")
                    .and_then(|v| v.as_str())
                    .map(str::trim);
                let args = params
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                capability.map(|capability| {
                    serde_json::json!({
                        "type": "call_capability",
                        "capability": capability,
                        "args": args,
                    })
                })
            }
            other => {
                let args = params
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                Some(serde_json::json!({
                    "type": "call_skill",
                    "skill": other,
                    "args": args,
                }))
            }
        };

        if let Some(step) = step {
            steps.push(step);
        }
    }
    steps
}

pub(super) async fn parse_single_plan_actions(
    raw: &str,
    state: &AppState,
    task: &ClaimedTask,
) -> Option<Vec<AgentAction>> {
    if plan_json_has_unterminated_string(raw) {
        info!(
            "plan_result_unterminated_json_string_rejected task_id={}",
            task.task_id
        );
        return None;
    }
    let mut step_values = Vec::new();
    match crate::prompt_utils::validate_against_schema::<Value>(
        raw,
        crate::prompt_utils::PromptSchemaId::PlanResult,
    ) {
        Ok(validated) => {
            if !validated.raw_parse_ok || validated.schema_normalized {
                info!(
                    "plan_result schema_parse_recovery task_id={} raw_parse_ok={} schema_normalized={}",
                    task.task_id, validated.raw_parse_ok, validated.schema_normalized
                );
            }
            match validated.value {
                Value::Object(map) => {
                    if let Some(steps) = map.get("steps").and_then(|v| v.as_array()) {
                        step_values.extend(steps.iter().cloned());
                    } else {
                        step_values.push(Value::Object(map));
                    }
                }
                Value::Array(arr) => step_values.extend(arr),
                other => step_values.push(other),
            }
        }
        Err(err) if err.is_contract_violation() => {
            if err.contract_violations_only_under("$.output_contract") {
                if let Some(Value::Object(map)) =
                    crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
                {
                    if let Some(steps) = map.get("steps").and_then(Value::as_array) {
                        step_values.extend(steps.iter().cloned());
                        info!(
                            "plan_result_output_contract_discarded task_id={} reason=invalid_optional_output_contract",
                            task.task_id
                        );
                    }
                }
            }
            if step_values.is_empty() {
                info!(
                    "plan_result_schema_contract_rejected task_id={} error={}",
                    task.task_id, err
                );
                return None;
            }
            // Each recovered step is still validated below against the action
            // schema, registry, resolver, and enum contracts.
        }
        Err(_) => {}
    }
    if step_values.is_empty() {
        if let Some(value) =
            crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
        {
            match value {
                Value::Object(map) => {
                    if let Some(steps) = map.get("steps").and_then(|v| v.as_array()) {
                        step_values.extend(steps.iter().cloned());
                    } else {
                        step_values.push(Value::Object(map));
                    }
                }
                Value::Array(arr) => step_values.extend(arr),
                other => step_values.push(other),
            }
        }
    }
    if step_values.is_empty() {
        for candidate in crate::prompt_utils::extract_agent_action_objects(raw) {
            if let Ok(value) = serde_json::from_str::<Value>(&candidate) {
                step_values.push(value);
            }
        }
    }
    if step_values.is_empty() {
        step_values.extend(extract_xml_tool_call_steps(raw));
    }
    if step_values.is_empty() {
        if let Some(actions) = recover_malformed_terminal_actions(raw) {
            return Some(actions);
        }
    }
    if step_values.is_empty() {
        let value = crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(raw)?;
        let env = serde_json::from_value::<SinglePlanEnvelope>(value).ok()?;
        step_values.extend(env.steps);
    }
    if step_values.is_empty() {
        return None;
    }

    let mut actions = Vec::new();
    for step in step_values {
        let Some(action) = parse_plan_action_step(&step, state) else {
            continue;
        };
        match action {
            AgentAction::Think { .. } => {}
            AgentAction::Respond { content } => actions.push(AgentAction::Respond { content }),
            _ => actions.push(action),
        }
    }
    if actions.is_empty() {
        None
    } else if !plan_actions_follow_machine_contract(state, &actions) {
        info!(
            "plan_result_registry_contract_rejected task_id={} reason=invalid_machine_contract",
            task.task_id
        );
        None
    } else {
        Some(actions)
    }
}

fn plan_json_has_unterminated_string(raw: &str) -> bool {
    let Some(json_start) = raw.find(['{', '[']) else {
        return false;
    };
    let mut in_string = false;
    let mut escaped = false;
    for ch in raw[json_start..].chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && in_string {
            escaped = true;
        } else if ch == '"' {
            in_string = !in_string;
        }
    }
    in_string
}

fn recover_malformed_terminal_actions(raw: &str) -> Option<Vec<AgentAction>> {
    let (respond_type_start, respond) = recover_malformed_respond_action(raw)?;
    let mut actions = Vec::new();
    if let Some(synthesize) = recover_malformed_synthesize_answer_before(raw, respond_type_start) {
        actions.push(synthesize);
    }
    actions.push(respond);
    Some(actions)
}

fn recover_malformed_respond_action(raw: &str) -> Option<(usize, AgentAction)> {
    if !raw.contains("\"steps\"") && !raw.contains("\"actions\"") {
        return None;
    }
    let mut respond_type_start = None;
    let mut cursor = 0usize;
    while let Some(start) = json_string_field_value_start_from(raw, "type", cursor) {
        let value = recover_json_string_until_next_quote(&raw[start..])?;
        let next_cursor = start.saturating_add(value.len()).saturating_add(1);
        if value.trim().eq_ignore_ascii_case("respond") {
            respond_type_start = Some(start);
        }
        cursor = next_cursor;
    }
    let respond_type_start = respond_type_start?;
    let content_start = json_string_field_value_start_from(raw, "content", respond_type_start)?;
    let content = recover_malformed_json_tail_string(&raw[content_start..])?;
    let content = decode_json_like_string(&content);
    (!content.trim().is_empty()).then_some((respond_type_start, AgentAction::Respond { content }))
}

fn recover_malformed_synthesize_answer_before(raw: &str, end: usize) -> Option<AgentAction> {
    let mut synthesize_type_start = None;
    let mut cursor = 0usize;
    while let Some(start) = json_string_field_value_start_from(raw, "type", cursor) {
        if start >= end {
            break;
        }
        let value = recover_json_string_until_next_quote(&raw[start..])?;
        let next_cursor = start.saturating_add(value.len()).saturating_add(1);
        if value.trim().eq_ignore_ascii_case("synthesize_answer") {
            synthesize_type_start = Some(start);
        }
        cursor = next_cursor;
    }
    let synthesize_type_start = synthesize_type_start?;
    let refs_start =
        json_array_field_value_start_from(raw, "evidence_refs", synthesize_type_start, end)?;
    let refs_raw = recover_json_array_slice(&raw[refs_start..end])?;
    let refs = serde_json::from_str::<Vec<String>>(refs_raw).ok()?;
    Some(AgentAction::SynthesizeAnswer {
        evidence_refs: refs,
    })
}

fn json_string_field_value_start_from(raw: &str, field: &str, from: usize) -> Option<usize> {
    let marker = format!("\"{field}\"");
    for (rel_idx, _) in raw[from..].match_indices(&marker) {
        let idx = from + rel_idx;
        let mut cursor = idx + marker.len();
        cursor = skip_ascii_ws(raw, cursor);
        if raw[cursor..].chars().next()? != ':' {
            continue;
        }
        cursor += ':'.len_utf8();
        cursor = skip_ascii_ws(raw, cursor);
        if raw[cursor..].chars().next()? == '"' {
            return Some(cursor + '"'.len_utf8());
        }
    }
    None
}

fn json_array_field_value_start_from(
    raw: &str,
    field: &str,
    from: usize,
    to: usize,
) -> Option<usize> {
    let marker = format!("\"{field}\"");
    let search_end = to.min(raw.len());
    for (rel_idx, _) in raw[from..search_end].match_indices(&marker) {
        let idx = from + rel_idx;
        let mut cursor = idx + marker.len();
        cursor = skip_ascii_ws(raw, cursor);
        if raw[cursor..].chars().next()? != ':' {
            continue;
        }
        cursor += ':'.len_utf8();
        cursor = skip_ascii_ws(raw, cursor);
        if cursor < search_end && raw[cursor..].chars().next()? == '[' {
            return Some(cursor);
        }
    }
    None
}

fn skip_ascii_ws(raw: &str, mut cursor: usize) -> usize {
    while let Some(ch) = raw[cursor..].chars().next() {
        if !ch.is_ascii_whitespace() {
            break;
        }
        cursor += ch.len_utf8();
    }
    cursor
}

fn recover_json_string_until_next_quote(raw: &str) -> Option<String> {
    let mut escaped = false;
    for (idx, ch) in raw.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            return Some(raw[..idx].to_string());
        }
    }
    None
}

fn recover_json_array_slice(raw: &str) -> Option<&str> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, ch) in raw.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '[' => depth += 1,
            ']' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    return Some(&raw[..=idx]);
                }
            }
            _ => {}
        }
    }
    None
}

fn recover_malformed_json_tail_string(raw: &str) -> Option<String> {
    let trimmed = raw.trim_end();
    let mut end = trimmed.len();
    while let Some((idx, ch)) = trimmed[..end].char_indices().next_back() {
        if matches!(ch, '}' | ']') || ch.is_ascii_whitespace() {
            end = idx;
            continue;
        }
        break;
    }
    let (quote_idx, quote) = trimmed[..end].char_indices().next_back()?;
    (quote == '"').then(|| trimmed[..quote_idx].to_string())
}

fn decode_json_like_string(raw: &str) -> String {
    serde_json::from_str::<String>(&format!("\"{raw}\"")).unwrap_or_else(|_| {
        raw.replace("\\n", "\n")
            .replace("\\r", "\r")
            .replace("\\t", "\t")
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    })
}

#[cfg(test)]
#[path = "planning_parse_tests.rs"]
mod tests;
