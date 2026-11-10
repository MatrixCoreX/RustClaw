use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Delivery carries only bytes from a successful write, never a model-authored
/// FILE token or a subsequently reopened, mutable source path.
pub(super) fn validate(
    args: &Map<String, Value>,
    task_id: &str,
    append: bool,
) -> Result<(), String> {
    let deliver = match args.get("deliver_to_user") {
        None | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        _ => return Err("file_delivery_flag_invalid".into()),
    };
    if deliver && append {
        return Err("file_delivery_requires_complete_overwrite".into());
    }
    if let Some(refs) = args.get("replaces_artifact_refs") {
        let refs = refs
            .as_array()
            .ok_or("file_delivery_replacements_invalid")?;
        if !refs.is_empty() && !deliver {
            return Err("file_delivery_replacement_requires_delivery".into());
        }
        let prefix = format!("artifact:task/{task_id}/");
        for value in refs {
            let id = value
                .as_str()
                .and_then(|value| value.strip_prefix(&prefix))
                .ok_or("file_delivery_replacement_task_mismatch")?;
            if id.is_empty()
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
            {
                return Err("file_delivery_replacement_ref_invalid".into());
            }
        }
    }
    Ok(())
}

pub(super) fn publish(
    workspace: &Path,
    task_id: &str,
    args: &Value,
    extra: &mut Option<Value>,
) -> Result<(), String> {
    if args.get("deliver_to_user").and_then(Value::as_bool) != Some(true) {
        return Ok(());
    }
    if extra
        .as_ref()
        .and_then(|value| value.get("action"))
        .and_then(Value::as_str)
        != Some("write_text")
    {
        return Ok(());
    }
    let content = args
        .get("content")
        .and_then(Value::as_str)
        .ok_or("file_delivery_content_missing")?;
    let filename = args
        .get("path")
        .and_then(Value::as_str)
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .ok_or("file_delivery_filename_missing")?;
    let digest = format!("{:x}", Sha256::digest(content.as_bytes()));
    // Each revision has its own path. Old refs cannot silently change content.
    let namespace = format!("generated-file-{}", &digest[..32]);
    let mut published = crate::skill_output_artifact::publish_task_text_artifact(
        workspace, task_id, &namespace, filename, content,
        json!({"producer": "filesystem_text_delivery", "replaces_artifact_refs": args.get("replaces_artifact_refs").cloned().unwrap_or(json!([]))}),
    ).map_err(|_| "file_delivery_publication_failed")?;
    let artifact = published
        .artifact_ref
        .as_object_mut()
        .ok_or("file_delivery_artifact_invalid")?;
    artifact.insert("filename".into(), json!(filename));
    let snapshot = workspace.join(
        artifact["path"]
            .as_str()
            .ok_or("file_delivery_path_missing")?,
    );
    artifact.insert("path".into(), json!(snapshot));
    artifact.insert("size_bytes".into(), json!(content.len()));
    artifact.insert("visibility".into(), json!("user_delivery"));
    artifact.insert("owner_task_id".into(), json!(task_id));
    let id = artifact["id"]
        .as_str()
        .ok_or("file_delivery_artifact_id_missing")?
        .to_string();
    artifact.insert(
        "artifact_ref".into(),
        json!(claw_core::task_delivery_artifacts::canonical_task_artifact_ref(task_id, &id)),
    );
    let extra = extra.get_or_insert_with(|| json!({}));
    let mut artifacts = extra
        .get("artifacts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    artifacts.push(published.artifact_ref);
    extra["artifacts"] = json!(artifacts);
    extra["delivery"] = json!({"deliver_to_user": true});
    Ok(())
}

#[cfg(test)]
#[path = "generated_file_delivery_tests.rs"]
mod tests;
