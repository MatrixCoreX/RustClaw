use serde::Deserialize;
use serde_json::Value;

use crate::{OutputScalarCountFilter, OutputScalarCountTargetKind};

#[derive(Debug, Clone, Deserialize, Default)]
struct ScalarCountFilterOut {
    #[serde(default)]
    target_kind: String,
    #[serde(default)]
    include_hidden: Option<bool>,
    #[serde(default)]
    recursive: Option<bool>,
    #[serde(default)]
    extensions: Option<Value>,
}

pub(super) fn parse_scalar_count_filter(raw: Option<Value>) -> OutputScalarCountFilter {
    let Some(raw @ Value::Object(_)) = raw else {
        return OutputScalarCountFilter::default();
    };
    let Ok(raw) = serde_json::from_value::<ScalarCountFilterOut>(raw) else {
        return OutputScalarCountFilter::default();
    };
    OutputScalarCountFilter {
        target_kind: parse_scalar_count_target_kind(&raw.target_kind).unwrap_or_default(),
        include_hidden: raw.include_hidden,
        recursive: raw.recursive,
        extensions: parse_scalar_count_filter_extensions(raw.extensions.as_ref()),
    }
}

pub(super) fn parse_scalar_count_target_kind(s: &str) -> Option<OutputScalarCountTargetKind> {
    match s.trim().to_ascii_lowercase().as_str() {
        "any" => Some(OutputScalarCountTargetKind::Any),
        "file" => Some(OutputScalarCountTargetKind::File),
        "dir" => Some(OutputScalarCountTargetKind::Dir),
        _ => None,
    }
}

fn parse_scalar_count_filter_extensions(raw: Option<&Value>) -> Vec<String> {
    fn push_extension(out: &mut Vec<String>, value: &str) {
        let value = value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim()
            .trim_start_matches("*.")
            .trim_start_matches('.')
            .to_ascii_lowercase();
        if value.is_empty()
            || value.len() > 32
            || value.contains(['*', '?', '.', '/', '\\'])
            || !value
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
        {
            return;
        }
        if !out.iter().any(|existing| existing == &value) {
            out.push(value);
        }
    }

    let mut out = Vec::new();
    match raw {
        Some(Value::String(value)) => push_extension(&mut out, value),
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(value) = item.as_str() {
                    push_extension(&mut out, value);
                }
            }
        }
        _ => {}
    }
    out
}

pub(super) fn normalize_scalar_count_filter_contract_field(
    contract: &mut serde_json::Map<String, Value>,
) {
    let Some(value) = contract.get_mut("scalar_count_filter") else {
        return;
    };
    let Some(filter) = value.as_object_mut() else {
        *value = Value::Null;
        return;
    };
    filter.retain(|key, _| {
        matches!(
            key.as_str(),
            "target_kind" | "include_hidden" | "recursive" | "extensions"
        )
    });
    if let Some(target_kind) = filter.get("target_kind").cloned() {
        let normalized = target_kind
            .as_str()
            .map(super::normalize_schema_token)
            .unwrap_or_default();
        let target_kind = match normalized.as_str() {
            "file" => "file",
            "dir" | "directory" | "folder" => "dir",
            "any" | "" => "any",
            _ => "any",
        };
        filter.insert(
            "target_kind".to_string(),
            Value::String(target_kind.to_string()),
        );
    }
    for key in ["include_hidden", "recursive"] {
        if filter.get(key).is_some_and(|value| !value.is_boolean()) {
            filter.insert(key.to_string(), Value::Null);
        }
    }
    if let Some(raw) = filter.get("extensions").cloned() {
        let extensions = parse_scalar_count_filter_extensions(Some(&raw))
            .into_iter()
            .map(Value::String)
            .collect::<Vec<_>>();
        if extensions.is_empty() {
            filter.remove("extensions");
        } else {
            filter.insert("extensions".to_string(), Value::Array(extensions));
            filter.insert("target_kind".to_string(), Value::String("file".to_string()));
        }
    }
}
