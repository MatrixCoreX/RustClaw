use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::AppState;

pub(crate) fn render_prompt_template(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut rendered = template.to_string();
    for (key, value) in replacements {
        rendered = rendered.replace(key, value);
    }
    rendered
}

pub(crate) fn log_prompt_render(
    task_id: &str,
    prompt_name: &str,
    prompt_file: &str,
    round: Option<usize>,
) {
    match round {
        Some(round) => {
            tracing::info!(
                "{} prompt_invocation task_id={} prompt_name={} prompt_file={} prompt_dynamic=true note=dynamic_built_prompt round={}",
                crate::highlight_tag("prompt"),
                task_id,
                prompt_name,
                prompt_file,
                round
            );
        }
        None => {
            tracing::info!(
                "{} prompt_invocation task_id={} prompt_name={} prompt_file={} prompt_dynamic=true note=dynamic_built_prompt",
                crate::highlight_tag("prompt"),
                task_id,
                prompt_name,
                prompt_file
            );
        }
    }
}

pub(crate) fn parse_llm_json_extract_then_raw<T: DeserializeOwned>(raw: &str) -> Option<T> {
    extract_json_object(raw)
        .and_then(|s| serde_json::from_str::<T>(&s).ok())
        .or_else(|| serde_json::from_str::<T>(raw.trim()).ok())
}

pub(crate) fn parse_llm_json_extract_or_any<T: DeserializeOwned>(raw: &str) -> Option<T> {
    extract_json_object(raw)
        .or_else(|| extract_first_json_object_any(raw))
        .and_then(|s| serde_json::from_str::<T>(&s).ok())
}

pub(crate) fn parse_llm_json_raw_or_any<T: DeserializeOwned>(raw: &str) -> Option<T> {
    serde_json::from_str::<T>(raw.trim()).ok().or_else(|| {
        extract_first_json_object_any(raw).and_then(|s| serde_json::from_str::<T>(&s).ok())
    })
}

pub(crate) fn extract_first_json_object_any(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i;
            let mut depth = 0usize;
            let mut in_string = false;
            let mut escaped = false;
            let mut j = i;
            while j < bytes.len() {
                let c = bytes[j];
                if in_string {
                    if escaped {
                        escaped = false;
                    } else if c == b'\\' {
                        escaped = true;
                    } else if c == b'"' {
                        in_string = false;
                    }
                } else if c == b'"' {
                    in_string = true;
                } else if c == b'{' {
                    depth += 1;
                } else if c == b'}' {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    if depth == 0 {
                        return Some(text[start..=j].to_string());
                    }
                }
                j += 1;
            }
            i = j;
        }
        i += 1;
    }
    None
}

pub(crate) fn extract_json_object(text: &str) -> Option<String> {
    extract_agent_action_objects(text).into_iter().next()
}

pub(crate) fn extract_agent_action_objects(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i;
            let mut depth = 0usize;
            let mut in_string = false;
            let mut escaped = false;
            let mut j = i;

            while j < bytes.len() {
                let c = bytes[j];
                if in_string {
                    if escaped {
                        escaped = false;
                    } else if c == b'\\' {
                        escaped = true;
                    } else if c == b'"' {
                        in_string = false;
                    }
                } else if c == b'"' {
                    in_string = true;
                } else if c == b'{' {
                    depth += 1;
                } else if c == b'}' {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    if depth == 0 {
                        let candidate = &text[start..=j];
                        if is_agent_action_candidate(candidate) {
                            out.push(candidate.to_string());
                        }
                    }
                }
                j += 1;
            }
            i = j;
        }
        i += 1;
    }
    out
}

pub(crate) fn parse_agent_action_json_with_repair(
    raw: &str,
    state: &AppState,
) -> Result<Value, String> {
    let parsed = match serde_json::from_str::<Value>(raw) {
        Ok(v) => Ok(v),
        Err(first_err) => {
            let repaired = repair_invalid_json_escapes(raw);
            match serde_json::from_str::<Value>(&repaired) {
                Ok(v) => Ok(v),
                Err(second_err) => Err(format!(
                    "initial parse error: {first_err}; repaired parse error: {second_err}"
                )),
            }
        }
    }?;
    Ok(normalize_agent_action_shape(parsed, state))
}

fn is_agent_action_candidate(candidate: &str) -> bool {
    if let Ok(v) = serde_json::from_str::<Value>(candidate) {
        return v.get("type").is_some()
            || v.get("action").is_some()
            || v.get("tool").is_some()
            || v.get("skill").is_some();
    }
    candidate.contains("\"type\"")
        || candidate.contains("\"action\"")
        || candidate.contains("\"tool\"")
        || candidate.contains("\"skill\"")
}

fn repair_invalid_json_escapes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 16);
    let mut in_string = false;
    let mut escaped = false;

    for ch in raw.chars() {
        if !in_string {
            if ch == '"' {
                in_string = true;
            }
            out.push(ch);
            continue;
        }

        if escaped {
            let valid = matches!(ch, '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' | 'u');
            if !valid {
                out.push('\\');
            }
            out.push(ch);
            escaped = false;
            continue;
        }

        match ch {
            '\\' => {
                out.push(ch);
                escaped = true;
            }
            '"' => {
                out.push(ch);
                in_string = false;
            }
            _ => out.push(ch),
        }
    }

    out
}

fn normalize_agent_action_shape(value: Value, state: &AppState) -> Value {
    let Some(obj) = value.as_object() else {
        return value;
    };
    let Some(raw_type) = obj.get("type").and_then(|v| v.as_str()) else {
        if let Some(skill) = obj.get("skill").and_then(|v| v.as_str()) {
            let normalized_skill = state.resolve_canonical_skill_name(skill.trim());
            if state.is_builtin_skill(&normalized_skill) {
                let args = obj.get("args").cloned().unwrap_or_else(|| json!({}));
                return json!({
                    "type": "call_skill",
                    "skill": normalized_skill,
                    "args": args,
                });
            }
        }
        if let Some(tool) = obj.get("tool").and_then(|v| v.as_str()) {
            let normalized_tool = state.resolve_canonical_skill_name(tool.trim());
            let args = obj.get("args").cloned().unwrap_or_else(|| json!({}));
            return json!({
                "type": "call_skill",
                "skill": normalized_tool,
                "args": args,
            });
        }
        if let Some(content) = obj.get("content").and_then(|v| v.as_str()) {
            return json!({
                "type": "respond",
                "content": content,
            });
        }
        return value;
    };
    let step_type = raw_type.trim().to_ascii_lowercase();
    if matches!(
        step_type.as_str(),
        "think" | "call_tool" | "call_skill" | "respond"
    ) {
        if step_type == "call_tool" {
            if let Some(tool) = obj.get("tool").and_then(|v| v.as_str()) {
                let normalized_tool = state.resolve_canonical_skill_name(tool.trim());
                let args = obj.get("args").cloned().unwrap_or_else(|| json!({}));
                return json!({
                    "type": "call_skill",
                    "skill": normalized_tool,
                    "args": args,
                });
            }
        }
        return value;
    }

    let args = collect_bare_action_args(obj);
    if state.is_builtin_skill(&step_type) {
        return json!({
            "type": "call_skill",
            "skill": step_type,
            "args": args,
        });
    }

    let normalized_skill = state.resolve_canonical_skill_name(&step_type);
    if state.is_builtin_skill(&normalized_skill) {
        return json!({
            "type": "call_skill",
            "skill": normalized_skill,
            "args": args,
        });
    }

    value
}

fn collect_bare_action_args(obj: &serde_json::Map<String, Value>) -> Value {
    let mut args = obj
        .get("args")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    for (key, value) in obj {
        if matches!(key.as_str(), "type" | "args" | "tool" | "skill") {
            continue;
        }
        args.insert(key.clone(), value.clone());
    }
    Value::Object(args)
}
