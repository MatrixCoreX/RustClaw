use serde_json::Value;
use std::path::{Path, PathBuf};

fn normalized_scope_patch_hint_text(raw: &str) -> Option<String> {
    let mut value = raw.trim().trim_matches(['"', '\'']).trim();
    if value.is_empty() {
        return None;
    }
    loop {
        let lower = value.to_ascii_lowercase();
        let stripped = if lower.ends_with("_only") {
            value[..value.len().saturating_sub("_only".len())].trim()
        } else if lower.ends_with("-only") {
            value[..value.len().saturating_sub("-only".len())].trim()
        } else if lower.ends_with(" only") {
            value[..value.len().saturating_sub(" only".len())].trim()
        } else {
            value
        };
        if stripped == value {
            break;
        }
        value = stripped.trim_matches(['_', '-', ' ']).trim();
        if value.is_empty() {
            return None;
        }
    }
    let simple_scope_token = value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/' | '\\' | '.'));
    if !simple_scope_token || matches!(value, "." | "./" | "/" | "\\") {
        return None;
    }
    Some(value.to_string())
}

pub(super) fn scope_patch_hint_value(value: &Value) -> Option<String> {
    match value {
        Value::String(raw) => normalized_scope_patch_hint_text(raw),
        Value::Array(values) => values.iter().find_map(scope_patch_hint_value),
        Value::Object(map) => [
            "scope",
            "module",
            "area",
            "section",
            "topic",
            "focus",
            "target_scope",
        ]
        .iter()
        .filter_map(|key| map.get(*key))
        .find_map(scope_patch_hint_value),
        _ => None,
    }
}

pub(super) fn locator_hint_is_unset_or_broad(hint: &str) -> bool {
    let hint = hint.trim();
    hint.is_empty() || matches!(hint, "." | "./" | "/" | "\\") || Path::new(hint).is_absolute()
}

fn locator_hint_names_workspace_root(hint: &str, workspace_root: &Path) -> bool {
    let Some(root_name) = workspace_root.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let normalized_root = normalize_locator_identity_token(root_name);
    let normalized_hint = normalize_locator_identity_token(hint);
    !normalized_root.is_empty() && normalized_hint == normalized_root
}

pub(super) fn locator_hint_points_to_workspace_root(hint: &str, workspace_root: &Path) -> bool {
    if locator_hint_names_workspace_root(hint, workspace_root) {
        return true;
    }
    let hint = hint.trim();
    if hint.is_empty() || hint.contains('\n') {
        return false;
    }
    let candidate = Path::new(hint);
    let candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace_root.join(candidate)
    };
    normalize_compare_path(candidate) == normalize_compare_path(workspace_root.to_path_buf())
}

fn normalize_locator_identity_token(value: &str) -> String {
    value
        .trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\''
                    | '`'
                    | ','
                    | '.'
                    | '，'
                    | '。'
                    | ':'
                    | '：'
                    | ';'
                    | '；'
                    | ')'
                    | '('
                    | ']'
                    | '['
                    | '）'
                    | '（'
                    | '】'
                    | '【'
                    | '>'
                    | '<'
                    | '》'
                    | '《'
            )
        })
        .to_ascii_lowercase()
}

fn normalize_compare_path(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

pub(super) fn locator_hint_compare_path(
    locator_hint: &str,
    workspace_root: &Path,
) -> Option<PathBuf> {
    let hint = locator_hint.trim();
    if hint.is_empty()
        || hint.contains('\n')
        || hint.contains('|')
        || locator_hint_looks_like_multi_target_list(hint, workspace_root)
        || hint.contains("->")
        || hint.starts_with("http://")
        || hint.starts_with("https://")
    {
        return None;
    }
    let path = Path::new(hint);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    Some(normalize_compare_path(path))
}

fn locator_hint_looks_like_multi_target_list(hint: &str, workspace_root: &Path) -> bool {
    if serde_json::from_str::<serde_json::Value>(hint)
        .ok()
        .and_then(|value| value.as_array().map(|items| items.len() > 1))
        .unwrap_or(false)
    {
        return true;
    }

    if !hint.contains(',') && !hint.contains(';') && !hint.contains('、') {
        return false;
    }

    let path = Path::new(hint);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    if path.exists() {
        return false;
    }

    hint.split([',', ';', '、'])
        .filter(|part| !part.trim().is_empty())
        .take(2)
        .count()
        > 1
}

pub(super) fn first_compare_path_from_text(text: &str, workspace_root: &Path) -> Option<PathBuf> {
    let locator = crate::intent::locator_extractor::extract_explicit_locator_for_fallback(text)?;
    locator_hint_compare_path(&locator.locator_hint, workspace_root)
}

pub(super) fn compare_path_targets_current_anchor(candidate: &Path, current_anchor: &Path) -> bool {
    candidate == current_anchor || candidate.starts_with(current_anchor)
}
