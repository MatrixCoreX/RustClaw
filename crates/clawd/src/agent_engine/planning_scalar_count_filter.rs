use super::scalar_count_explicit_path::ScalarCountInventoryKind;
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ScalarCountFilterHint {
    target_kind: ScalarCountInventoryKind,
    include_hidden: Option<bool>,
    recursive: Option<bool>,
    extensions: Vec<String>,
}

#[cfg(test)]
pub(super) fn scalar_count_filter_hint_from_turn_analysis(
    turn_analysis: Option<&crate::turn_context::TurnAnalysis>,
) -> Option<ScalarCountFilterHint> {
    let filter = turn_analysis?
        .state_patch
        .as_ref()?
        .get("scalar_count_filter")?
        .as_object()?;
    let target_kind = match filter.get("target_kind").and_then(Value::as_str) {
        Some(raw) => parse_scalar_count_filter_target_kind(raw)?,
        None => ScalarCountInventoryKind::Any,
    };
    let include_hidden = filter.get("include_hidden").and_then(Value::as_bool);
    let recursive = filter.get("recursive").and_then(Value::as_bool);
    let extensions = scalar_count_filter_extensions_from_value(filter.get("extensions"));
    if target_kind == ScalarCountInventoryKind::Any
        && include_hidden.is_none()
        && recursive.is_none()
        && extensions.is_empty()
    {
        return None;
    }
    Some(ScalarCountFilterHint {
        target_kind,
        include_hidden,
        recursive,
        extensions,
    })
}

#[cfg(test)]
fn scalar_count_filter_extensions_from_value(raw: Option<&Value>) -> Vec<String> {
    let mut out = Vec::new();
    match raw {
        Some(Value::String(value)) => push_scalar_count_filter_extension(&mut out, value),
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(value) = item.as_str() {
                    push_scalar_count_filter_extension(&mut out, value);
                }
            }
        }
        _ => {}
    }
    out
}

#[cfg(test)]
fn push_scalar_count_filter_extension(out: &mut Vec<String>, value: &str) {
    let Some(value) = normalize_extension_filter_text(value) else {
        return;
    };
    if !out.iter().any(|existing| existing == &value) {
        out.push(value);
    }
}

#[cfg(test)]
fn parse_scalar_count_filter_target_kind(raw: &str) -> Option<ScalarCountInventoryKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "any" => Some(ScalarCountInventoryKind::Any),
        "file" => Some(ScalarCountInventoryKind::Files),
        "dir" => Some(ScalarCountInventoryKind::Dirs),
        _ => None,
    }
}

pub(super) fn scalar_count_filter_hint_from_route(
    route: &RouteResult,
) -> Option<ScalarCountFilterHint> {
    let filter = &route.output_contract.self_extension.scalar_count_filter;
    if !filter.has_constraints() {
        return None;
    }
    let target_kind = match filter.target_kind {
        crate::OutputScalarCountTargetKind::Any => ScalarCountInventoryKind::Any,
        crate::OutputScalarCountTargetKind::File => ScalarCountInventoryKind::Files,
        crate::OutputScalarCountTargetKind::Dir => ScalarCountInventoryKind::Dirs,
    };
    Some(ScalarCountFilterHint {
        target_kind,
        include_hidden: filter.include_hidden,
        recursive: filter.recursive,
        extensions: filter.extensions.clone(),
    })
}

#[cfg(test)]
pub(super) fn scalar_count_filter_hint_for_route_or_turn(
    route: &RouteResult,
    turn_analysis: Option<&crate::turn_context::TurnAnalysis>,
) -> Option<ScalarCountFilterHint> {
    scalar_count_filter_hint_from_route(route)
        .or_else(|| scalar_count_filter_hint_from_turn_analysis(turn_analysis))
}

pub(super) fn apply_scalar_count_filter_hint(
    out: &mut serde_json::Map<String, Value>,
    hint: &ScalarCountFilterHint,
) {
    match hint.target_kind {
        ScalarCountInventoryKind::Any => {}
        ScalarCountInventoryKind::Files => {
            out.insert("kind_filter".to_string(), Value::String("file".to_string()));
            out.insert("count_files".to_string(), Value::Bool(true));
            out.insert("count_dirs".to_string(), Value::Bool(false));
            out.insert("files_only".to_string(), Value::Bool(true));
            out.insert("dirs_only".to_string(), Value::Bool(false));
        }
        ScalarCountInventoryKind::Dirs => {
            out.insert("kind_filter".to_string(), Value::String("dir".to_string()));
            out.insert("count_files".to_string(), Value::Bool(false));
            out.insert("count_dirs".to_string(), Value::Bool(true));
            out.insert("dirs_only".to_string(), Value::Bool(true));
            out.insert("files_only".to_string(), Value::Bool(false));
        }
    }
    if let Some(include_hidden) = hint.include_hidden {
        out.insert("include_hidden".to_string(), Value::Bool(include_hidden));
    } else if hint.target_kind != ScalarCountInventoryKind::Any {
        out.insert("include_hidden".to_string(), Value::Bool(false));
    }
    if let Some(recursive) = hint.recursive {
        out.insert("recursive".to_string(), Value::Bool(recursive));
    }
    if !hint.extensions.is_empty() {
        out.insert(
            "ext_filter".to_string(),
            Value::Array(hint.extensions.iter().cloned().map(Value::String).collect()),
        );
        if !matches!(hint.target_kind, ScalarCountInventoryKind::Dirs) {
            out.insert("kind_filter".to_string(), Value::String("file".to_string()));
            out.insert("count_files".to_string(), Value::Bool(true));
            out.insert("count_dirs".to_string(), Value::Bool(false));
            out.insert("files_only".to_string(), Value::Bool(true));
            out.insert("dirs_only".to_string(), Value::Bool(false));
        }
    }
}
