use serde_json::{json, Map, Value};

pub(super) fn ensure_edit_keys(
    edit: &Map<String, Value>,
    path: &str,
    index: usize,
) -> Result<(), String> {
    const ALLOWED: &[&str] = &[
        "old_text",
        "new_text",
        "expected_occurrences",
        "replace_all",
    ];
    if let Some(key) = edit.keys().find(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(super::edit_error(
            "unexpected_edit_arg",
            "workspace.replace.unexpected_edit_arg",
            json!({"path": path, "edit_index": index, "arg": key}),
        ));
    }
    Ok(())
}

pub(super) fn required_string<'a>(
    edit: &'a Map<String, Value>,
    key: &str,
    path: &str,
    index: usize,
) -> Result<&'a str, String> {
    edit.get(key).and_then(Value::as_str).ok_or_else(|| {
        super::edit_error(
            "missing_edit_arg",
            "workspace.replace.missing_edit_arg",
            json!({"path": path, "edit_index": index, "arg": key}),
        )
    })
}

pub(super) fn optional_bool(
    edit: &Map<String, Value>,
    key: &str,
    path: &str,
    index: usize,
) -> Result<Option<bool>, String> {
    match edit.get(key) {
        None => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(super::edit_error(
            "invalid_edit_arg_type",
            "workspace.replace.invalid_edit_arg_type",
            json!({"path": path, "edit_index": index, "arg": key, "expected_type": "boolean"}),
        )),
    }
}

pub(super) fn invalid_edit(path: &str, index: usize) -> String {
    super::edit_error(
        "invalid_edit",
        "workspace.replace.invalid_edit",
        json!({"path": path, "edit_index": index}),
    )
}
