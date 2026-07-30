use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

const DEFAULT_PREVIEW_BYTES: usize = 32 * 1024;
const MIN_PREVIEW_BYTES: usize = 4 * 1024;
const MAX_PREVIEW_BYTES: usize = 1024 * 1024;

pub(crate) struct PublishedArtifact {
    pub(crate) artifact_ref: Value,
    pub(crate) range_handle: Value,
}

pub(crate) fn sensitivity_aware_json_model_view(value: &Value) -> (Value, bool) {
    fn redact(value: &Value, sensitive_parent: bool, redacted: &mut bool) -> Value {
        match value {
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .map(|(key, child)| {
                        let sensitive = sensitive_parent || sensitive_field_name(key);
                        let child = if sensitive {
                            *redacted = true;
                            redacted_value(child)
                        } else {
                            redact(child, false, redacted)
                        };
                        (key.clone(), child)
                    })
                    .collect(),
            ),
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|item| redact(item, sensitive_parent, redacted))
                    .collect(),
            ),
            Value::String(text) => {
                let (safe, changed) = sensitivity_aware_text_model_view(text);
                *redacted |= changed;
                Value::String(safe)
            }
            _ => value.clone(),
        }
    }

    let mut redacted = false;
    (redact(value, false, &mut redacted), redacted)
}

pub(crate) fn sensitivity_aware_text_model_view(value: &str) -> (String, bool) {
    let mut output = Vec::new();
    let mut redacted = false;
    let mut private_key = false;
    for line in value.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("begin ") && lower.contains("private key") {
            private_key = true;
            redacted = true;
            output.push("[REDACTED private_key]".to_string());
            continue;
        }
        if private_key {
            redacted = true;
            if lower.contains("end ") && lower.contains("private key") {
                private_key = false;
            }
            continue;
        }
        if let Some(boundary) = sensitive_assignment_boundary(line) {
            redacted = true;
            let secret = line[boundary..].trim();
            output.push(format!(
                "{}[REDACTED sha256:{:x}]",
                &line[..boundary],
                Sha256::digest(secret.as_bytes())
            ));
            continue;
        }
        if let Some(start) = lower.find("bearer ") {
            redacted = true;
            output.push(format!("{}Bearer [REDACTED]", &line[..start]));
            continue;
        }
        output.push(line.to_string());
    }
    let mut safe = output.join("\n");
    if value.ends_with('\n') {
        safe.push('\n');
    }
    (safe, redacted)
}

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
    let relative_path = PathBuf::from(claw_core::workspace_state::WORKSPACE_STATE_DIR_NAME)
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

pub(crate) fn publish_existing_task_artifact(
    workspace_root: &Path,
    task_id: &str,
    namespace: &str,
    source_path: &Path,
    suffix: &str,
    media_type: &str,
    metadata: Value,
) -> io::Result<Option<PublishedArtifact>> {
    let source_metadata = fs::metadata(source_path)?;
    if !source_metadata.is_file() || source_metadata.len() == 0 {
        return Ok(None);
    }

    let artifact_id = uuid::Uuid::new_v4().to_string();
    let namespace_key = machine_path_component(namespace, "output");
    let task_key = machine_path_component(task_id, "task");
    let suffix_key = machine_path_component(suffix, "data");
    let relative_path = PathBuf::from(claw_core::workspace_state::WORKSPACE_STATE_DIR_NAME)
        .join("artifacts")
        .join(&namespace_key)
        .join(task_key)
        .join(format!("{artifact_id}.{suffix_key}"));
    let artifact_path = workspace_root.join(&relative_path);
    atomic_publish_existing(source_path, &artifact_path)?;
    let sha256 = sha256_file(source_path)?;
    let relative_path = relative_path.to_string_lossy().replace('\\', "/");
    let mut artifact_metadata = metadata.as_object().cloned().unwrap_or_default();
    artifact_metadata.insert("size_bytes".to_string(), json!(source_metadata.len()));
    artifact_metadata.insert("task_id".to_string(), Value::String(task_id.to_string()));
    artifact_metadata.insert(
        "provenance".to_string(),
        Value::String(namespace_key.clone()),
    );
    let artifact_ref = json!({
        "id": format!("{namespace_key}:{artifact_id}"),
        "path": relative_path,
        "media_type": media_type,
        "sha256": sha256,
        "metadata": artifact_metadata,
    });
    let range_handle = json!({
        "artifact_ref": artifact_ref["id"],
        "path": artifact_ref["path"],
        "start_byte": 0,
        "end_byte": source_metadata.len(),
        "read_capability": "artifact.read_range",
    });
    Ok(Some(PublishedArtifact {
        artifact_ref,
        range_handle,
    }))
}

pub(crate) fn publish_canonical_evidence_artifact(
    workspace_root: &Path,
    task_id: &str,
    evidence_id: &str,
    bytes: &[u8],
) -> io::Result<PublishedArtifact> {
    let sha256 = format!("{:x}", Sha256::digest(bytes));
    let task_key = machine_path_component(task_id, "task");
    let relative_path = PathBuf::from(claw_core::workspace_state::WORKSPACE_STATE_DIR_NAME)
        .join("artifacts")
        .join("canonical-evidence")
        .join(task_key)
        .join(format!("{sha256}.json"));
    let artifact_path = workspace_root.join(&relative_path);
    if !artifact_path.is_file() {
        atomic_write(&artifact_path, bytes)?;
        restrict_evidence_permissions(&artifact_path)?;
    }
    let relative_path = relative_path.to_string_lossy().replace('\\', "/");
    let artifact_ref = json!({
        "id": evidence_id,
        "path": relative_path,
        "media_type": "application/json",
        "sha256": sha256,
        "metadata": {
            "size_bytes": bytes.len(),
            "task_id": task_id,
            "provenance": "canonical_capability_result",
            "sensitivity": "task_owner_restricted",
            "retention": "task_retention_policy",
            "storage_protection": {
                "access_control": "task_owner_and_runtime",
                "private_file_mode": "0600",
                "private_directory_mode": "0700",
                "at_rest_encryption": "deployment_storage_policy",
            },
        },
    });
    let range_handle = json!({
        "artifact_ref": evidence_id,
        "path": artifact_ref["path"],
        "start_byte": 0,
        "end_byte": bytes.len(),
        "read_capability": "artifact.read_range",
    });
    Ok(PublishedArtifact {
        artifact_ref,
        range_handle,
    })
}

fn sensitive_field_name(key: &str) -> bool {
    let normalized = key.trim().to_ascii_lowercase().replace(['-', '.'], "_");
    [
        "api_key",
        "apikey",
        "access_token",
        "refresh_token",
        "authorization",
        "password",
        "passwd",
        "secret",
        "client_secret",
        "private_key",
        "cookie",
        "set_cookie",
        "user_key",
    ]
    .iter()
    .any(|candidate| normalized == *candidate || normalized.ends_with(&format!("_{candidate}")))
}

fn sensitive_assignment_boundary(line: &str) -> Option<usize> {
    let lower = line.to_ascii_lowercase();
    let sensitive = [
        "api_key",
        "apikey",
        "access_token",
        "refresh_token",
        "authorization",
        "password",
        "passwd",
        "client_secret",
        "private_key",
        "user_key",
    ];
    if !sensitive.iter().any(|key| lower.contains(key)) {
        return None;
    }
    line.find('=')
        .or_else(|| line.find(':'))
        .map(|index| index + 1)
}

fn redacted_value(value: &Value) -> Value {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    json!({
        "redacted": true,
        "sha256": format!("{:x}", Sha256::digest(&bytes)),
        "value_kind": match value {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        },
    })
}

#[cfg(unix)]
fn restrict_evidence_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    if let Some(parent) = path.parent() {
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn restrict_evidence_permissions(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn preview_limit_bytes() -> usize {
    claw_core::product_identity::env_string("SKILL_OUTPUT_PREVIEW_BYTES")
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
        io::Error::new(io::ErrorKind::InvalidInput, "artifact_path_parent_missing")
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

fn atomic_publish_existing(source: &Path, destination: &Path) -> io::Result<()> {
    let parent = destination.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "artifact_path_parent_missing")
    })?;
    fs::create_dir_all(parent)?;
    let temp_path = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        if fs::hard_link(source, &temp_path).is_err() {
            fs::copy(source, &temp_path)?;
            File::open(&temp_path)?.sync_all()?;
        }
        fs::rename(&temp_path, destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn sha256_file(path: &Path) -> io::Result<String> {
    use std::io::Read;

    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
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
