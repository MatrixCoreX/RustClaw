use serde_json::{json, Map, Value};

#[path = "builtin_workspace_replace_edit_args.rs"]
mod edit_args;

use edit_args::{ensure_edit_keys, invalid_edit, optional_bool, required_string};

const MAX_EDITS: usize = 128;
const MAX_FRAGMENT_BYTES: usize = 1024 * 1024;
const MAX_OCCURRENCES: usize = 10_000;
const MAX_REPORTED_RANGES: usize = 64;

pub(super) struct AppliedEdit {
    pub index: usize,
    pub old_text: String,
    pub new_text: String,
    pub occurrence_count: usize,
    pub ranges: Vec<AppliedRange>,
    pub ranges_truncated: bool,
}

pub(super) struct AppliedRange {
    pub before_start: usize,
    pub before_end: usize,
    pub after_start: usize,
    pub after_end: usize,
}

pub(super) struct EditOutcome {
    pub after: String,
    pub edits: Vec<AppliedEdit>,
    pub replacement_count: usize,
}

pub(super) fn apply_requested_edits(
    args: &Map<String, Value>,
    path: &str,
    before: &str,
    preserve_line_endings: bool,
) -> Result<EditOutcome, String> {
    let has_batch = args.contains_key("edits");
    let has_direct = args.contains_key("old_text") || args.contains_key("new_text");
    if has_batch
        && (has_direct
            || args.contains_key("expected_occurrences")
            || args.contains_key("replace_all"))
    {
        return Err(edit_error(
            "conflicting_edit_modes",
            "workspace.replace.conflicting_edit_modes",
            json!({"path": path}),
        ));
    }

    let mut staged = before.to_string();
    let mut applied = Vec::new();
    if has_batch {
        let edits = args.get("edits").and_then(Value::as_array).ok_or_else(|| {
            edit_error(
                "invalid_edits",
                "workspace.replace.invalid_edits",
                json!({"path": path}),
            )
        })?;
        if edits.is_empty() || edits.len() > MAX_EDITS {
            return Err(edit_error(
                "invalid_edits",
                "workspace.replace.invalid_edits",
                json!({"path": path, "edit_count": edits.len(), "max_edits": MAX_EDITS}),
            ));
        }
        for (index, value) in edits.iter().enumerate() {
            let edit = value.as_object().ok_or_else(|| invalid_edit(path, index))?;
            ensure_edit_keys(edit, path, index)?;
            apply_one(
                edit,
                path,
                index,
                preserve_line_endings,
                &mut staged,
                &mut applied,
            )?;
        }
    } else {
        apply_one(
            args,
            path,
            0,
            preserve_line_endings,
            &mut staged,
            &mut applied,
        )?;
    }

    Ok(EditOutcome {
        replacement_count: applied.iter().map(|edit| edit.occurrence_count).sum(),
        after: staged,
        edits: applied,
    })
}

fn apply_one(
    edit: &Map<String, Value>,
    path: &str,
    index: usize,
    preserve_line_endings: bool,
    staged: &mut String,
    applied: &mut Vec<AppliedEdit>,
) -> Result<(), String> {
    let old_text = required_string(edit, "old_text", path, index)?;
    let new_text = required_string(edit, "new_text", path, index)?;
    if old_text.is_empty() {
        return Err(edit_error(
            "empty_old_text",
            "workspace.replace.empty_old_text",
            json!({"path": path, "edit_index": index}),
        ));
    }
    if old_text.len() > MAX_FRAGMENT_BYTES || new_text.len() > MAX_FRAGMENT_BYTES {
        return Err(edit_error(
            "replacement_fragment_too_large",
            "workspace.replace.fragment_too_large",
            json!({
                "path": path,
                "edit_index": index,
                "old_text_bytes": old_text.len(),
                "new_text_bytes": new_text.len(),
                "max_fragment_bytes": MAX_FRAGMENT_BYTES,
            }),
        ));
    }

    let replace_all = optional_bool(edit, "replace_all", path, index)?.unwrap_or(false);
    let expected = expected_occurrences(edit, path, index, replace_all)?;
    let matches = staged
        .match_indices(old_text)
        .map(|(start, _)| start)
        .collect::<Vec<_>>();
    let actual = matches.len();
    if actual == 0 {
        return Err(occurrence_error(path, index, expected, actual, replace_all));
    }
    if actual > MAX_OCCURRENCES {
        return Err(edit_error(
            "replacement_occurrence_limit_exceeded",
            "workspace.replace.occurrence_limit_exceeded",
            json!({"path": path, "edit_index": index, "actual_occurrences": actual, "max_occurrences": MAX_OCCURRENCES}),
        ));
    }
    if expected.is_some_and(|value| value != actual) || (!replace_all && actual != 1) {
        return Err(occurrence_error(path, index, expected, actual, replace_all));
    }

    let replacement = if preserve_line_endings && super::uses_crlf(staged) {
        super::normalize_to_crlf(new_text)
    } else {
        new_text.to_string()
    };
    let byte_delta = replacement.len() as isize - old_text.len() as isize;
    let ranges = matches
        .iter()
        .enumerate()
        .take(MAX_REPORTED_RANGES)
        .map(|(ordinal, start)| {
            let after_start = (*start as isize + byte_delta * ordinal as isize) as usize;
            AppliedRange {
                before_start: *start,
                before_end: start + old_text.len(),
                after_start,
                after_end: after_start + replacement.len(),
            }
        })
        .collect();
    *staged = if replace_all {
        staged.replace(old_text, &replacement)
    } else {
        staged.replacen(old_text, &replacement, 1)
    };
    applied.push(AppliedEdit {
        index,
        old_text: old_text.to_string(),
        new_text: replacement,
        occurrence_count: actual,
        ranges,
        ranges_truncated: actual > MAX_REPORTED_RANGES,
    });
    Ok(())
}

fn expected_occurrences(
    edit: &Map<String, Value>,
    path: &str,
    index: usize,
    replace_all: bool,
) -> Result<Option<usize>, String> {
    let expected = match edit.get("expected_occurrences") {
        None if replace_all => None,
        None => Some(1),
        Some(value) => value
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| (1..=MAX_OCCURRENCES).contains(value))
            .map(Some)
            .ok_or_else(|| invalid_expected(path, index))?,
    };
    if !replace_all && expected != Some(1) {
        return Err(edit_error(
            "invalid_expected_occurrences",
            "workspace.replace.replace_all_required",
            json!({"path": path, "edit_index": index, "expected_occurrences": expected, "replace_all_required": true}),
        ));
    }
    Ok(expected)
}

fn invalid_expected(path: &str, index: usize) -> String {
    edit_error(
        "invalid_expected_occurrences",
        "workspace.replace.invalid_expected_occurrences",
        json!({"path": path, "edit_index": index, "maximum": MAX_OCCURRENCES}),
    )
}

fn occurrence_error(
    path: &str,
    index: usize,
    expected: Option<usize>,
    actual: usize,
    replace_all: bool,
) -> String {
    let error_code = if actual == 0 {
        "replacement_target_not_found"
    } else if expected == Some(1) && actual > 1 {
        "replacement_target_ambiguous"
    } else {
        "replacement_occurrence_mismatch"
    };
    edit_error(
        error_code,
        if actual == 0 {
            "workspace.replace.target_not_found"
        } else if error_code == "replacement_target_ambiguous" {
            "workspace.replace.target_ambiguous"
        } else {
            "workspace.replace.occurrence_mismatch"
        },
        json!({
            "path": path,
            "edit_index": index,
            "expected_occurrences": expected,
            "actual_occurrences": actual,
            "replace_all": replace_all,
        }),
    )
}

fn edit_error(error_code: &str, message_key: &str, details: Value) -> String {
    super::replace_error(error_code, message_key, details)
}
