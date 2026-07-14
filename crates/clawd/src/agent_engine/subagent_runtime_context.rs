use serde_json::{json, Value};
use std::{fs::File, io::Read, path::Path};

use super::{machine_ref_or_empty, SubagentActionOptions, SubagentRuntimeConfig};

const MAX_SUBAGENT_CONTEXT_EVIDENCE_REFS: usize = 6;
const DEFAULT_SUBAGENT_CONTEXT_EVIDENCE_CHARS: usize = 2048;
const MAX_SUBAGENT_CONTEXT_EVIDENCE_CHARS: usize = 4096;
const MAX_SUBAGENT_CONTEXT_EVIDENCE_BYTES: u64 = 32_768;

pub(super) fn context_evidence_summary(
    context_refs: &[Value],
    options: &SubagentActionOptions,
    config: &SubagentRuntimeConfig,
) -> Value {
    let requested_refs = context_refs
        .iter()
        .filter_map(|item| item.get("ref").and_then(Value::as_str))
        .filter(|item| !item.trim().is_empty())
        .take(MAX_SUBAGENT_CONTEXT_EVIDENCE_REFS)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if requested_refs.is_empty() {
        return json!({
            "schema_version": 1,
            "present": false,
            "status": "no_context_refs",
            "item_count": 0,
            "available_count": 0,
            "items": [],
        });
    }

    let Some(root) = config.context_evidence_root.as_ref() else {
        return json!({
            "schema_version": 1,
            "present": false,
            "status": "context_evidence_root_unavailable",
            "item_count": requested_refs.len(),
            "available_count": 0,
            "items": requested_refs
                .iter()
                .map(|reference| {
                    context_evidence_unavailable_item(
                        reference,
                        "context_evidence_root_unavailable",
                    )
                })
                .collect::<Vec<_>>(),
        });
    };
    let Some(root_canonical) = root.canonicalize().ok() else {
        return json!({
            "schema_version": 1,
            "present": false,
            "status": "workspace_root_unavailable",
            "item_count": requested_refs.len(),
            "available_count": 0,
            "items": requested_refs
                .iter()
                .map(|reference| {
                    context_evidence_unavailable_item(reference, "workspace_root_unavailable")
                })
                .collect::<Vec<_>>(),
        });
    };

    let max_chars = context_evidence_max_chars(options);
    let items = requested_refs
        .iter()
        .map(|reference| context_evidence_item(root, &root_canonical, reference, max_chars))
        .collect::<Vec<_>>();
    let available_count = items
        .iter()
        .filter(|item| item.get("status").and_then(Value::as_str) == Some("available"))
        .count();
    json!({
        "schema_version": 1,
        "present": available_count > 0,
        "status": if available_count > 0 { "available" } else { "unavailable" },
        "item_count": items.len(),
        "available_count": available_count,
        "max_context_chars": max_chars,
        "items": items,
    })
}

pub(super) fn context_evidence_combined_excerpt(summary: &Value) -> String {
    summary
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .filter(|item| item.get("status").and_then(Value::as_str) == Some("available"))
        .filter_map(|item| {
            let excerpt = item
                .get("content_excerpt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            if excerpt.is_empty() {
                return None;
            }
            let path = item
                .get("path")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .or_else(|| item.get("ref").and_then(Value::as_str))
                .unwrap_or_default();
            Some(format!("{path}\n{excerpt}"))
        })
        .collect::<Vec<_>>()
        .join("\n---\n")
}

pub(super) fn context_evidence_paths(summary: &Value) -> Vec<String> {
    summary
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .filter(|item| item.get("status").and_then(Value::as_str) == Some("available"))
        .filter_map(|item| item.get("path").and_then(Value::as_str))
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub(super) fn context_evidence_action(summary: &Value) -> &'static str {
    if context_evidence_has_available_excerpt(summary) {
        "read_text_range"
    } else {
        "subagent_context_evidence"
    }
}

pub(super) fn context_evidence_has_available_excerpt(summary: &Value) -> bool {
    summary
        .get("available_count")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        > 0
}

pub(super) fn context_evidence_summary_from_items(items: Vec<Value>) -> Value {
    let available_count = items
        .iter()
        .filter(|item| item.get("status").and_then(Value::as_str) == Some("available"))
        .count();
    json!({
        "schema_version": 1,
        "present": available_count > 0,
        "status": if available_count > 0 { "available" } else { "unavailable" },
        "item_count": items.len(),
        "available_count": available_count,
        "items": items,
    })
}

fn context_evidence_unavailable_item(reference: &str, status: &str) -> Value {
    json!({
        "schema_version": 1,
        "ref": machine_ref_or_empty(reference.trim()),
        "status": status,
        "path": "",
        "content_excerpt": "",
        "excerpt_char_count": 0,
    })
}

fn context_evidence_item(
    root: &Path,
    root_canonical: &Path,
    reference: &str,
    max_chars: usize,
) -> Value {
    let reference = reference.trim();
    if reference.is_empty() {
        return context_evidence_unavailable_item(reference, "empty_ref");
    }
    let requested_path = Path::new(reference);
    let candidate = if requested_path.is_absolute() {
        requested_path.to_path_buf()
    } else {
        root.join(requested_path)
    };
    let Some(canonical) = candidate.canonicalize().ok() else {
        return context_evidence_unavailable_item(reference, "not_found");
    };
    if !canonical.starts_with(root_canonical) {
        return context_evidence_unavailable_item(reference, "outside_workspace");
    }
    let relative_path = canonical
        .strip_prefix(root_canonical)
        .ok()
        .and_then(|path| path.to_str())
        .map(|path| path.replace('\\', "/"))
        .unwrap_or_default();
    let safe_path = machine_ref_or_empty(&relative_path).to_string();
    if !canonical.is_file() {
        return json!({
            "schema_version": 1,
            "ref": machine_ref_or_empty(reference),
            "status": "not_file",
            "path": safe_path,
            "content_excerpt": "",
            "excerpt_char_count": 0,
        });
    }
    let file_size_bytes = canonical.metadata().ok().map(|meta| meta.len());
    let Some(raw_excerpt) = read_bounded_utf8_text(&canonical, max_chars) else {
        return json!({
            "schema_version": 1,
            "ref": machine_ref_or_empty(reference),
            "status": "read_failed",
            "path": safe_path,
            "content_excerpt": "",
            "excerpt_char_count": 0,
            "file_size_bytes": file_size_bytes,
        });
    };
    let redacted_excerpt = redact_sensitive_excerpt_lines(&raw_excerpt);
    let (content_excerpt, excerpt_strategy) =
        bounded_head_tail_excerpt(&redacted_excerpt, max_chars);
    let excerpt_char_count = content_excerpt.chars().count();
    let observed_char_count = redacted_excerpt.chars().count();
    json!({
        "schema_version": 1,
        "ref": machine_ref_or_empty(reference),
        "status": "available",
        "path": safe_path,
        "content_excerpt": content_excerpt,
        "excerpt_strategy": excerpt_strategy,
        "excerpt_char_count": excerpt_char_count,
        "observed_char_count": observed_char_count,
        "file_size_bytes": file_size_bytes,
        "line_count_observed": redacted_excerpt.lines().count(),
        "truncated": observed_char_count > excerpt_char_count
            || file_size_bytes.is_some_and(|size| size > MAX_SUBAGENT_CONTEXT_EVIDENCE_BYTES),
    })
}

fn context_evidence_max_chars(options: &SubagentActionOptions) -> usize {
    options
        .context_slice
        .as_ref()
        .and_then(|value| {
            value
                .get("max_context_chars")
                .and_then(Value::as_u64)
                .or_else(|| value.get("max_chars").and_then(Value::as_u64))
        })
        .or_else(|| {
            options
                .budget
                .as_ref()
                .and_then(|value| value.get("max_context_chars").and_then(Value::as_u64))
        })
        .and_then(|value| usize::try_from(value).ok())
        .map(|value| value.clamp(256, MAX_SUBAGENT_CONTEXT_EVIDENCE_CHARS))
        .unwrap_or(DEFAULT_SUBAGENT_CONTEXT_EVIDENCE_CHARS)
}

fn read_bounded_utf8_text(path: &Path, max_chars: usize) -> Option<String> {
    let byte_limit = ((max_chars as u64).saturating_mul(16).saturating_add(4096))
        .min(MAX_SUBAGENT_CONTEXT_EVIDENCE_BYTES);
    let mut file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(byte_limit)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn redact_sensitive_excerpt_lines(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line_has_sensitive_marker(line) {
                "[REDACTED_SENSITIVE_LINE]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn line_has_sensitive_marker(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "api_key",
        "apikey",
        "authorization",
        "bearer",
        "password",
        "private_key",
        "secret",
        "token",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn take_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn bounded_head_tail_excerpt(text: &str, max_chars: usize) -> (String, &'static str) {
    let observed_chars = text.chars().count();
    if observed_chars <= max_chars {
        return (text.to_string(), "full");
    }
    if max_chars < 512 {
        return (take_chars(text, max_chars), "head");
    }
    let separator = "\n...\n";
    let separator_chars = separator.chars().count();
    let body_budget = max_chars.saturating_sub(separator_chars);
    let head_chars = body_budget / 2;
    let tail_chars = body_budget.saturating_sub(head_chars);
    let head = text.chars().take(head_chars).collect::<String>();
    let tail = text
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    (format!("{head}{separator}{tail}"), "head_tail")
}
