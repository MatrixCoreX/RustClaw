use serde_json::Value;
use std::path::{Path, PathBuf};

pub(crate) const CLAWD_USER_NAMED_OUTPUT_PATH_ARG: &str = "_clawd_user_named_output_path";

fn structured_write_path_arg(normalized_skill: &str, args: &Value) -> Option<String> {
    let obj = args.as_object()?;
    match normalized_skill {
        "write_file" => obj.get("path").and_then(Value::as_str),
        "fs_basic" => {
            let action = obj.get("action").and_then(Value::as_str)?.trim();
            if matches!(action, "write_text" | "append_text") {
                obj.get("path").and_then(Value::as_str)
            } else {
                None
            }
        }
        _ => None,
    }
    .map(str::trim)
    .filter(|path| !path.is_empty())
    .map(ToString::to_string)
}

fn structured_write_has_content_arg(args: &Value) -> bool {
    let Some(obj) = args.as_object() else {
        return false;
    };
    ["content", "text", "data", "body"].iter().any(|key| {
        obj.get(*key)
            .and_then(Value::as_str)
            .is_some_and(|v| !v.is_empty())
    })
}

fn resolve_workspace_candidate_path(workspace_root: &Path, raw_path: &str) -> Option<PathBuf> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let raw = Path::new(trimmed);
    if raw
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return None;
    }
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    };
    candidate.starts_with(&root).then_some(candidate)
}

fn request_surface_names_user_output_path(request_text: &str, path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let file_name = file_name.trim();
    if file_name.is_empty() {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request_text);
    surface
        .filename_candidates_excluding_field_selectors()
        .into_iter()
        .any(|candidate| {
            let trimmed = candidate.trim();
            let candidate_file_name = Path::new(trimmed)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(trimmed)
                .trim();
            !candidate_file_name.is_empty() && candidate_file_name.eq_ignore_ascii_case(file_name)
        })
}

pub(crate) fn action_is_user_named_new_workspace_write(
    workspace_root: &Path,
    request_text: &str,
    normalized_skill: &str,
    args: &Value,
) -> bool {
    if !structured_write_has_content_arg(args) {
        return false;
    }
    let Some(raw_path) = structured_write_path_arg(normalized_skill, args) else {
        return false;
    };
    let Some(candidate) = resolve_workspace_candidate_path(workspace_root, &raw_path) else {
        return false;
    };
    !candidate.exists() && request_surface_names_user_output_path(request_text, &candidate)
}
