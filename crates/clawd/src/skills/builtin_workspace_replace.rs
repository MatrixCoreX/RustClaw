use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

use super::builtin_workspace_mutation::{atomic_write_file, run_checkpointed_workspace_mutation};
use super::builtin_workspace_patch::{canonical_workspace_root, validate_relative_patch_path};

#[path = "builtin_workspace_replace_edit.rs"]
mod edit;

const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
const MAX_PREVIEW_BYTES: usize = 64 * 1024;

struct Replacement {
    path: String,
    before: String,
    after: String,
    before_sha256: String,
    after_sha256: String,
    edits: Vec<edit::AppliedEdit>,
    replacement_count: usize,
}

pub(super) fn execute_workspace_replace_for_root(
    workspace_root: &Path,
    task_id: &str,
    args: &Map<String, Value>,
) -> Result<String, String> {
    ensure_only_keys(
        args,
        &[
            "action",
            "path",
            "old_text",
            "new_text",
            "expected_occurrences",
            "replace_all",
            "edits",
            "expected_sha256",
            "preserve_line_endings",
        ],
    )?;
    let action = required_token(args, "action")?;
    let root = canonical_workspace_root(workspace_root)?;
    let requested_path = required_nonempty_string(args, "path")?;
    let target = validate_replace_path(&root, requested_path).map_err(|error| {
        if crate::skills::parse_structured_skill_error(&error)
            .is_some_and(|parsed| parsed.error_code == "invalid_patch_path")
        {
            replace_error(
                "path_outside_workspace",
                "workspace.replace.path_outside_workspace",
                json!({"path": requested_path}),
            )
        } else {
            error
        }
    })?;
    let path = target
        .strip_prefix(&root)
        .map_err(|_| {
            replace_error(
                "path_outside_workspace",
                "workspace.replace.path_outside_workspace",
                json!({"path": requested_path}),
            )
        })?
        .to_string_lossy()
        .into_owned();
    let replacement = prepare_replacement(args, &path, &target)?;
    let preview = replacement_preview(action, &replacement);

    if action == "preview_replace_text" {
        return encode_result(preview);
    }
    if action != "replace_text" {
        return Err(replace_error(
            "unsupported_action",
            "workspace.replace.unsupported_action",
            json!({"action": action}),
        ));
    }
    if replacement.before == replacement.after {
        return Err(replace_error(
            "replacement_no_change",
            "workspace.replace.no_change",
            json!({"path": path}),
        ));
    }

    let expected_before_hash = replacement.before_sha256.clone();
    let after = replacement.after.as_bytes().to_vec();
    let mutation =
        run_checkpointed_workspace_mutation(&root, task_id, "replace_text", &target, || {
            let current = read_text_file(&target, &path)?;
            let current_hash = sha256_label(current.as_bytes());
            if current_hash != expected_before_hash {
                return Err(replace_error(
                    "replacement_precondition_failed",
                    "workspace.replace.precondition_failed",
                    json!({
                        "path": path,
                        "expected_sha256": expected_before_hash,
                        "actual_sha256": current_hash,
                    }),
                ));
            }
            atomic_write_file(&target, &after).map_err(|error| {
                replace_error(
                    "replacement_write_failed",
                    "workspace.replace.write_failed",
                    json!({
                        "path": path,
                        "io_kind": format!("{:?}", error.kind()),
                    }),
                )
            })
        })?;

    let mut result: Value = serde_json::from_str(&mutation).map_err(|error| {
        replace_error(
            "replacement_result_invalid",
            "workspace.replace.result_invalid",
            json!({"error_code": format!("{:?}", error.classify())}),
        )
    })?;
    let object = result.as_object_mut().ok_or_else(|| {
        replace_error(
            "replacement_result_invalid",
            "workspace.replace.result_invalid",
            Value::Null,
        )
    })?;
    let preview_object = preview
        .as_object()
        .expect("replacement_preview_object_invariant");
    for key in [
        "before_sha256",
        "after_sha256",
        "occurrence_count",
        "changed_byte_range",
        "changed_byte_ranges",
        "diff_preview",
        "diff_truncated",
        "path",
        "edit_count",
    ] {
        if let Some(value) = preview_object.get(key) {
            object.insert(key.to_string(), value.clone());
        }
    }
    object.insert("source".to_string(), json!("workspace_replace"));
    object.insert(
        "message_key".to_string(),
        json!("workspace.replace.applied"),
    );
    object.insert(
        "replacement_count".to_string(),
        json!(replacement.replacement_count),
    );
    object.insert("idempotency_replay".to_string(), json!(false));
    encode_result(result)
}

fn validate_replace_path(root: &Path, requested_path: &str) -> Result<PathBuf, String> {
    let requested = Path::new(requested_path);
    if !requested.is_absolute() {
        return validate_relative_patch_path(root, requested_path);
    }

    let relative = requested
        .strip_prefix(root)
        .map_err(|_| super::builtin_workspace_patch::invalid_path_error(requested_path))?;
    let relative = relative
        .to_str()
        .ok_or_else(|| super::builtin_workspace_patch::invalid_path_error(requested_path))?;
    validate_relative_patch_path(root, relative)
}

fn prepare_replacement(
    args: &Map<String, Value>,
    requested_path: &str,
    target: &Path,
) -> Result<Replacement, String> {
    let before = read_text_file(target, requested_path)?;
    if let Some(expected) = optional_nonempty_string(args, "expected_sha256") {
        let actual = sha256_label(before.as_bytes());
        if normalize_hash(expected) != actual {
            return Err(replace_error(
                "replacement_precondition_failed",
                "workspace.replace.precondition_failed",
                json!({
                    "path": requested_path,
                    "expected_sha256": expected,
                    "actual_sha256": actual,
                }),
            ));
        }
    }
    let preserve_line_endings = optional_bool(args, "preserve_line_endings")?.unwrap_or(true);
    let outcome =
        edit::apply_requested_edits(args, requested_path, &before, preserve_line_endings)?;

    Ok(Replacement {
        path: requested_path.to_string(),
        before_sha256: sha256_label(before.as_bytes()),
        after_sha256: sha256_label(outcome.after.as_bytes()),
        before,
        after: outcome.after,
        edits: outcome.edits,
        replacement_count: outcome.replacement_count,
    })
}

fn replacement_preview(action: &str, replacement: &Replacement) -> Value {
    let mut raw_preview = format!("--- a/{0}\n+++ b/{0}\n", replacement.path);
    let mut changed_byte_ranges = Vec::new();
    for applied in &replacement.edits {
        raw_preview.push_str(&format!(
            "@@ staged edit {} occurrences {} @@\n-{}\n+{}\n",
            applied.index, applied.occurrence_count, applied.old_text, applied.new_text,
        ));
        changed_byte_ranges.extend(applied.ranges.iter().map(|range| {
            json!({
                "edit_index": applied.index,
                "coordinate_space": "staged_before_edit",
                "before_start": range.before_start,
                "before_end": range.before_end,
                "after_start": range.after_start,
                "after_end": range.after_end,
                "ranges_truncated": applied.ranges_truncated,
            })
        }));
    }
    let (diff_preview, diff_truncated) = bounded_utf8(&raw_preview, MAX_PREVIEW_BYTES);
    let changed_byte_range = changed_byte_ranges.first().cloned().unwrap_or(Value::Null);
    json!({
        "schema_version": 1,
        "source": "workspace_replace",
        "status": "ok",
        "action": action,
        "message_key": "workspace.replace.preview_ready",
        "path": replacement.path,
        "occurrence_count": replacement.replacement_count,
        "edit_count": replacement.edits.len(),
        "before_sha256": replacement.before_sha256,
        "after_sha256": replacement.after_sha256,
        "changed_byte_range": changed_byte_range,
        "changed_byte_ranges": changed_byte_ranges,
        "would_change": replacement.before != replacement.after,
        "diff_preview": diff_preview,
        "diff_truncated": diff_truncated,
    })
}

fn read_text_file(path: &Path, requested_path: &str) -> Result<String, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        replace_error(
            if error.kind() == std::io::ErrorKind::NotFound {
                "replacement_target_not_found"
            } else {
                "replacement_target_inspection_failed"
            },
            if error.kind() == std::io::ErrorKind::NotFound {
                "workspace.replace.target_not_found"
            } else {
                "workspace.replace.target_inspection_failed"
            },
            json!({
                "path": requested_path,
                "io_kind": format!("{:?}", error.kind()),
            }),
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(replace_error(
            "unsupported_file_type",
            "workspace.replace.unsupported_file_type",
            json!({"path": requested_path}),
        ));
    }
    if metadata.len() > MAX_FILE_BYTES as u64 {
        return Err(replace_error(
            "replacement_target_too_large",
            "workspace.replace.target_too_large",
            json!({
                "path": requested_path,
                "size_bytes": metadata.len(),
                "max_file_bytes": MAX_FILE_BYTES,
            }),
        ));
    }
    let bytes = fs::read(path).map_err(|error| {
        replace_error(
            "replacement_read_failed",
            "workspace.replace.read_failed",
            json!({
                "path": requested_path,
                "io_kind": format!("{:?}", error.kind()),
            }),
        )
    })?;
    if bytes.contains(&0) {
        return Err(replace_error(
            "binary_file_unsupported",
            "workspace.replace.binary_file_unsupported",
            json!({"path": requested_path}),
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        replace_error(
            "non_utf8_file_unsupported",
            "workspace.replace.non_utf8_file_unsupported",
            json!({"path": requested_path}),
        )
    })
}

fn uses_crlf(text: &str) -> bool {
    text.contains("\r\n")
        && text.as_bytes().iter().enumerate().all(|(index, byte)| {
            *byte != b'\n' || (index > 0 && text.as_bytes()[index - 1] == b'\r')
        })
}

fn normalize_to_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

fn normalize_hash(value: &str) -> String {
    if value.starts_with("sha256:") {
        value.to_ascii_lowercase()
    } else {
        format!("sha256:{}", value.to_ascii_lowercase())
    }
}

fn sha256_label(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn bounded_utf8(text: &str, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text.to_string(), false);
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_string(), true)
}

fn ensure_only_keys(args: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    if let Some(key) = args.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(replace_error(
            "unexpected_arg",
            "workspace.replace.unexpected_arg",
            json!({"arg": key}),
        ));
    }
    Ok(())
}

fn required_token<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    required_nonempty_string(args, key).map(str::trim)
}

fn required_nonempty_string<'a>(
    args: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    required_string(args, key).and_then(|value| {
        if value.trim().is_empty() {
            Err(replace_error(
                "empty_arg",
                "workspace.replace.empty_arg",
                json!({"arg": key}),
            ))
        } else {
            Ok(value)
        }
    })
}

fn required_string<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    args.get(key).and_then(Value::as_str).ok_or_else(|| {
        replace_error(
            "missing_arg",
            "workspace.replace.missing_arg",
            json!({"arg": key}),
        )
    })
}

fn optional_nonempty_string<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn optional_bool(args: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(replace_error(
            "invalid_arg_type",
            "workspace.replace.invalid_arg_type",
            json!({"arg": key, "expected_type": "boolean"}),
        )),
    }
}

fn encode_result(value: Value) -> Result<String, String> {
    serde_json::to_string(&value).map_err(|error| {
        replace_error(
            "replacement_result_encode_failed",
            "workspace.replace.result_encode_failed",
            json!({"error_code": format!("{:?}", error.classify())}),
        )
    })
}

fn replace_error(error_code: &str, message_key: &str, details: Value) -> String {
    super::builtin_error(
        "workspace_patch",
        error_code,
        message_key,
        None,
        None,
        Some(json!({
            "error_code": error_code,
            "message_key": message_key,
            "details": details,
        })),
    )
}

#[cfg(test)]
#[path = "builtin_workspace_replace_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "builtin_workspace_replace_batch_tests.rs"]
mod batch_tests;
