use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

const DEFAULT_PREVIEW_BYTES: usize = 32 * 1024;
const MIN_PREVIEW_BYTES: usize = 4 * 1024;
const MAX_PREVIEW_BYTES: usize = 1024 * 1024;

pub(crate) fn spill_skill_text_if_needed(
    workspace_root: &Path,
    task_id: &str,
    skill_name: &str,
    text: &mut String,
    extra: &mut Option<Value>,
) -> io::Result<bool> {
    let preview_limit = preview_limit_bytes();
    if text.len() <= preview_limit {
        return Ok(false);
    }

    let full_bytes = text.as_bytes();
    let total_bytes = full_bytes.len();
    let preview_end = utf8_prefix_end(text, preview_limit);
    let preview = text[..preview_end].to_string();
    let is_json = serde_json::from_str::<Value>(text).is_ok();
    let extension = if is_json { "json" } else { "txt" };
    let media_type = if is_json {
        "application/json"
    } else {
        "text/plain; charset=utf-8"
    };
    let artifact_id = uuid::Uuid::new_v4().to_string();
    let task_key = machine_path_component(task_id, "task");
    let skill_key = machine_path_component(skill_name, "skill");
    let relative_path = PathBuf::from(".rustclaw")
        .join("artifacts")
        .join("skill-output")
        .join(task_key)
        .join(format!("{skill_key}-{artifact_id}.{extension}"));
    let artifact_path = workspace_root.join(&relative_path);
    atomic_write(&artifact_path, full_bytes)?;

    let sha256 = format!("{:x}", Sha256::digest(full_bytes));
    let relative_path = relative_path.to_string_lossy().replace('\\', "/");
    let artifact_ref = json!({
        "id": format!("skill-output:{artifact_id}"),
        "path": relative_path,
        "media_type": media_type,
        "sha256": sha256,
        "metadata": {
            "size_bytes": total_bytes,
            "task_id": task_id,
            "skill": skill_name,
            "provenance": "skill_output",
        },
    });
    let range_handle = json!({
        "artifact_ref": artifact_ref["id"],
        "path": artifact_ref["path"],
        "start_byte": 0,
        "end_byte": total_bytes,
        "read_capability": "artifact.read_range",
    });

    let map = extra_object(extra);
    append_unique(map, "artifact_refs", artifact_ref.clone());
    append_unique(map, "artifacts", artifact_ref);
    append_unique(map, "range_handles", range_handle);
    map.insert("output_truncated".to_string(), Value::Bool(true));
    map.insert("truncated".to_string(), Value::Bool(true));
    map.insert("output_total_bytes".to_string(), json!(total_bytes));
    map.insert("output_preview_bytes".to_string(), json!(preview_end));
    map.insert("output_sha256".to_string(), Value::String(sha256));
    let page = json!({
        "cursor": 0,
        "start_byte": 0,
        "end_byte": preview_end,
        "total_bytes": total_bytes,
        "limit_bytes": preview_limit,
        "has_more": true,
        "next_cursor": preview_end,
        "next_start_byte": preview_end,
    });
    if map.contains_key("page") {
        map.insert("output_page".to_string(), page);
    } else {
        map.insert("page".to_string(), page);
    }

    *text = if is_json {
        json!({
            "schema_version": 1,
            "status_code": "output_truncated",
            "preview": preview,
            "artifact_ref": format!("skill-output:{artifact_id}"),
            "next_cursor": preview_end,
            "total_bytes": total_bytes,
        })
        .to_string()
    } else {
        preview
    };
    Ok(true)
}

fn preview_limit_bytes() -> usize {
    std::env::var("RUSTCLAW_SKILL_OUTPUT_PREVIEW_BYTES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_PREVIEW_BYTES)
        .clamp(MIN_PREVIEW_BYTES, MAX_PREVIEW_BYTES)
}

fn utf8_prefix_end(value: &str, limit: usize) -> usize {
    let mut end = value.len().min(limit);
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    end
}

fn machine_path_component(value: &str, fallback: &str) -> String {
    let component = value
        .trim()
        .chars()
        .take(96)
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    if component.is_empty() {
        fallback.to_string()
    } else {
        component
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "artifact path has no parent")
    })?;
    fs::create_dir_all(parent)?;
    let temp_path = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = File::create(&temp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp_path, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn extra_object(extra: &mut Option<Value>) -> &mut Map<String, Value> {
    let current = extra.take();
    let value = match current {
        Some(Value::Object(map)) => Value::Object(map),
        Some(value) => json!({
            "schema_version": 1,
            "source": "skill_output_artifact",
            "value": value,
        }),
        None => json!({
            "schema_version": 1,
            "source": "skill_output_artifact",
        }),
    };
    *extra = Some(value);
    extra
        .as_mut()
        .and_then(Value::as_object_mut)
        .expect("extra initialized as object")
}

fn append_unique(map: &mut Map<String, Value>, key: &str, value: Value) {
    let values = map
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Some(values) = values.as_array_mut() {
        if !values.iter().any(|existing| existing == &value) {
            values.push(value);
        }
    }
}

#[cfg(test)]
#[path = "skill_output_artifact_tests.rs"]
mod tests;
