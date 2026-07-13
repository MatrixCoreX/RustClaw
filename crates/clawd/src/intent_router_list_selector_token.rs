use serde_json::{Map, Number, Value};

use crate::{OutputListSelector, OutputScalarCountTargetKind};

pub(super) fn list_selector_machine_token_value(raw: &str) -> Option<Value> {
    let selector = parse_list_selector_machine_token(raw)?;
    let mut out = Map::new();
    if selector.target_kind_specified {
        out.insert(
            "target_kind".to_string(),
            Value::String(target_kind_token(selector.target_kind).to_string()),
        );
    }
    if let Some(limit) = selector.limit {
        out.insert("limit".to_string(), Value::Number(Number::from(limit)));
    }
    if let Some(sort_by) = selector.sort_by {
        out.insert("sort_by".to_string(), Value::String(sort_by));
    }
    if let Some(include_hidden) = selector.include_hidden {
        out.insert("include_hidden".to_string(), Value::Bool(include_hidden));
    }
    Some(Value::Object(out))
}

pub(super) fn parse_list_selector_machine_token(raw: &str) -> Option<OutputListSelector> {
    let normalized = normalize_machine_selector_token(raw)?;
    let parts: Vec<&str> = normalized
        .split(['_', '-', '.'])
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let target_kind = selector_target_kind_from_parts(&parts);
    let target_kind_specified = target_kind.is_some();
    let limit = parts
        .iter()
        .find_map(|part| part.parse::<u64>().ok())
        .map(|value| value.clamp(1, 1000));
    let sort_by = selector_sort_by_from_parts(&parts).map(str::to_string);
    let include_hidden = selector_include_hidden_from_parts(&parts);
    if !target_kind_specified && limit.is_none() && sort_by.is_none() && include_hidden.is_none() {
        return None;
    }
    Some(OutputListSelector {
        target_kind: target_kind.unwrap_or_default(),
        target_kind_specified,
        limit,
        sort_by,
        include_metadata: None,
        include_hidden,
    })
}

fn normalize_machine_selector_token(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return None;
    }
    Some(trimmed.to_ascii_lowercase().trim_matches('_').to_string())
}

fn selector_target_kind_from_parts(parts: &[&str]) -> Option<OutputScalarCountTargetKind> {
    if parts
        .iter()
        .any(|part| matches!(*part, "file" | "files" | "filename" | "filenames"))
    {
        return Some(OutputScalarCountTargetKind::File);
    }
    if parts.iter().any(|part| {
        matches!(
            *part,
            "dir" | "dirs" | "directory" | "directories" | "folder" | "folders"
        )
    }) {
        return Some(OutputScalarCountTargetKind::Dir);
    }
    parts
        .iter()
        .any(|part| *part == "any")
        .then_some(OutputScalarCountTargetKind::Any)
}

fn selector_sort_by_from_parts(parts: &[&str]) -> Option<&'static str> {
    for window in parts.windows(2) {
        match window {
            ["name", "desc"] => return Some("name_desc"),
            ["mtime", "desc"] => return Some("mtime_desc"),
            ["mtime", "asc"] => return Some("mtime_asc"),
            ["size", "desc"] => return Some("size_desc"),
            ["size", "asc"] => return Some("size_asc"),
            _ => {}
        }
    }
    if parts
        .windows(3)
        .any(|window| window == ["sort", "by", "name"])
        || parts.windows(2).any(|window| window == ["sort", "name"])
    {
        return Some("name");
    }
    None
}

fn selector_include_hidden_from_parts(parts: &[&str]) -> Option<bool> {
    for window in parts.windows(2) {
        match window {
            ["no", "hidden"] | ["exclude", "hidden"] => return Some(false),
            ["include", "hidden"] | ["with", "hidden"] => return Some(true),
            _ => {}
        }
    }
    parts.iter().any(|part| *part == "hidden").then_some(true)
}

fn target_kind_token(target_kind: OutputScalarCountTargetKind) -> &'static str {
    match target_kind {
        OutputScalarCountTargetKind::Any => "any",
        OutputScalarCountTargetKind::File => "file",
        OutputScalarCountTargetKind::Dir => "dir",
    }
}
