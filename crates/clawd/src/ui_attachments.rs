use std::path::{Component, Path};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde_json::{json, Value};

use crate::AppState;

const MAX_UI_ATTACHMENTS: usize = 10;
const MAX_UI_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
const MAX_UI_TOTAL_ATTACHMENT_BYTES: usize = 60 * 1024 * 1024;

#[derive(Debug, Clone)]
struct UiAttachmentInput {
    name: String,
    mime_type: String,
    kind: String,
    data_url: String,
}

#[derive(Debug, Clone)]
struct MaterializedAttachment {
    name: String,
    mime_type: String,
    kind: String,
    rel_path: String,
    size: usize,
}

pub(crate) fn materialize_ui_task_attachments(
    state: &AppState,
    payload: &mut Value,
    effective_user_id: i64,
    effective_chat_id: i64,
    call_id: &str,
) -> Result<(), String> {
    let inputs = collect_ui_attachment_inputs(payload);
    if inputs.is_empty() {
        return Ok(());
    }
    if inputs.len() > MAX_UI_ATTACHMENTS {
        return Err("ui_attachments_too_many".to_string());
    }

    let mut total_bytes = 0usize;
    let mut materialized = Vec::new();
    for (index, input) in inputs.into_iter().enumerate() {
        let (bytes, data_mime_type) = decode_data_url(&input.data_url)?;
        if bytes.len() > MAX_UI_ATTACHMENT_BYTES {
            return Err("ui_attachment_too_large".to_string());
        }
        total_bytes = total_bytes.saturating_add(bytes.len());
        if total_bytes > MAX_UI_TOTAL_ATTACHMENT_BYTES {
            return Err("ui_attachments_total_too_large".to_string());
        }
        let mime_type = if input.mime_type.trim().is_empty() {
            data_mime_type.unwrap_or_else(|| default_mime_for_kind(&input.kind).to_string())
        } else {
            input.mime_type
        };
        let safe_name = safe_upload_filename(&input.name, &input.kind, &mime_type);
        let rel_path = format!(
            "data/ui/{}/{}/{}/{:02}-{}",
            effective_user_id,
            effective_chat_id,
            safe_path_token(call_id),
            index,
            safe_name
        );
        let abs_path = state.skill_rt.workspace_root.join(&rel_path);
        if !path_is_under_root(&state.skill_rt.workspace_root, &abs_path) {
            return Err("ui_attachment_path_outside_workspace".to_string());
        }
        if let Some(parent) = abs_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("ui_attachment_create_dir_failed: {err}"))?;
        }
        std::fs::write(&abs_path, &bytes)
            .map_err(|err| format!("ui_attachment_write_failed: {err}"))?;
        materialized.push(MaterializedAttachment {
            name: safe_name,
            mime_type,
            kind: normalize_kind(&input.kind, ""),
            rel_path,
            size: bytes.len(),
        });
    }

    rewrite_payload_attachments(payload, &materialized);
    Ok(())
}

pub(crate) fn prompt_with_ui_attachment_context(prompt: &str, payload: &Value) -> String {
    let paths = attachment_context_paths(payload);
    if paths.is_empty() {
        return prompt.to_string();
    }
    let mut lines = Vec::new();
    lines.push("[RUSTCLAW_ATTACHMENT_CONTEXT]".to_string());
    for (kind, path) in paths {
        lines.push(format!("{kind}_path={path}"));
    }
    lines.push("[/RUSTCLAW_ATTACHMENT_CONTEXT]".to_string());
    let context = lines.join("\n");
    if prompt.trim().is_empty() {
        context
    } else {
        format!("{}\n\n{}", prompt.trim(), context)
    }
}

fn collect_ui_attachment_inputs(payload: &Value) -> Vec<UiAttachmentInput> {
    if let Some(items) = payload.get("attachments").and_then(Value::as_array) {
        return items
            .iter()
            .filter_map(|item| attachment_input_from_value(item, None))
            .collect();
    }

    let mut out = Vec::new();
    if let Some(items) = payload.get("images").and_then(Value::as_array) {
        for item in items {
            if let Some(input) = attachment_input_from_value(item, Some("image")) {
                out.push(input);
            }
        }
    }
    if let Some(audio) = payload.get("audio") {
        if let Some(input) = attachment_input_from_value(audio, Some("audio")) {
            out.push(input);
        }
    }
    out
}

fn attachment_input_from_value(
    value: &Value,
    forced_kind: Option<&str>,
) -> Option<UiAttachmentInput> {
    let obj = value.as_object()?;
    let data_url = obj
        .get("base64")
        .or_else(|| obj.get("data_url"))
        .or_else(|| obj.get("dataUrl"))
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    if data_url.is_empty() {
        return None;
    }
    let mime_type = obj
        .get("mime_type")
        .or_else(|| obj.get("mimeType"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    let kind = forced_kind
        .or_else(|| obj.get("kind").and_then(Value::as_str))
        .map(|v| normalize_kind(v, &mime_type))
        .unwrap_or_else(|| normalize_kind("", &mime_type));
    let name = obj
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default_name_for_kind(&kind))
        .to_string();
    Some(UiAttachmentInput {
        name,
        mime_type,
        kind,
        data_url,
    })
}

fn decode_data_url(raw: &str) -> Result<(Vec<u8>, Option<String>), String> {
    let trimmed = raw.trim();
    let (mime_type, encoded) = if let Some(rest) = trimmed.strip_prefix("data:") {
        let Some((meta, body)) = rest.split_once(',') else {
            return Err("ui_attachment_invalid_data_url".to_string());
        };
        if !meta.to_ascii_lowercase().contains(";base64") {
            return Err("ui_attachment_data_url_not_base64".to_string());
        }
        let mime = meta
            .split(';')
            .next()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned);
        (mime, body.trim())
    } else {
        (None, trimmed)
    };
    let compact = encoded.split_whitespace().collect::<String>();
    let bytes = BASE64_STANDARD
        .decode(compact.as_bytes())
        .map_err(|_| "ui_attachment_base64_decode_failed".to_string())?;
    Ok((bytes, mime_type))
}

fn rewrite_payload_attachments(payload: &mut Value, attachments: &[MaterializedAttachment]) {
    let Some(obj) = payload.as_object_mut() else {
        return;
    };
    let mut all = Vec::new();
    let mut images = Vec::new();
    let mut audios = Vec::new();
    let mut files = Vec::new();
    for attachment in attachments {
        let entry = json!({
            "name": attachment.name,
            "mime_type": attachment.mime_type,
            "kind": attachment.kind,
            "path": attachment.rel_path,
            "size": attachment.size,
        });
        all.push(entry.clone());
        match attachment.kind.as_str() {
            "image" => images.push(entry),
            "audio" => audios.push(entry),
            _ => files.push(entry),
        }
    }
    obj.insert("attachments".to_string(), Value::Array(all));
    if !images.is_empty() {
        obj.insert("images".to_string(), Value::Array(images));
    }
    if !audios.is_empty() {
        obj.insert("audio".to_string(), audios[0].clone());
        obj.insert("audios".to_string(), Value::Array(audios));
    }
    if !files.is_empty() {
        obj.insert("files".to_string(), Value::Array(files));
    }
}

fn attachment_context_paths(payload: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(items) = payload.get("attachments").and_then(Value::as_array) {
        for item in items {
            if let Some(path) = item.get("path").and_then(Value::as_str).map(str::trim) {
                if !path.is_empty() {
                    let kind = item
                        .get("kind")
                        .and_then(Value::as_str)
                        .map(|v| normalize_kind(v, ""))
                        .unwrap_or_else(|| "file".to_string());
                    out.push((kind, path.to_string()));
                }
            }
        }
    }
    out
}

fn normalize_kind(raw: &str, mime_type: &str) -> String {
    let raw = raw.trim().to_ascii_lowercase();
    if matches!(raw.as_str(), "image" | "audio" | "file") {
        return raw;
    }
    let mime = mime_type.trim().to_ascii_lowercase();
    if mime.starts_with("image/") {
        "image".to_string()
    } else if mime.starts_with("audio/") {
        "audio".to_string()
    } else {
        "file".to_string()
    }
}

fn default_mime_for_kind(kind: &str) -> &'static str {
    match kind {
        "image" => "image/png",
        "audio" => "audio/webm",
        _ => "application/octet-stream",
    }
}

fn default_name_for_kind(kind: &str) -> &'static str {
    match kind {
        "image" => "image.png",
        "audio" => "audio.webm",
        _ => "attachment.bin",
    }
}

fn safe_upload_filename(raw_name: &str, kind: &str, mime_type: &str) -> String {
    let basename = Path::new(raw_name)
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or_else(|| default_name_for_kind(kind));
    let mut safe = basename
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    safe = safe.trim_matches('.').to_string();
    if safe.is_empty() {
        safe = default_name_for_kind(kind).to_string();
    }
    if Path::new(&safe).extension().is_none() {
        safe.push('.');
        safe.push_str(default_extension_for(kind, mime_type));
    }
    safe
}

fn safe_path_token(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn default_extension_for(kind: &str, mime_type: &str) -> &'static str {
    let mime = mime_type.trim().to_ascii_lowercase();
    if mime.contains("wav") {
        "wav"
    } else if mime.contains("mpeg") || mime.contains("mp3") {
        "mp3"
    } else if mime.contains("mp4") || mime.contains("m4a") {
        "m4a"
    } else if mime.contains("ogg") || mime.contains("opus") {
        "ogg"
    } else if mime.contains("webm") {
        "webm"
    } else if mime.contains("png") {
        "png"
    } else if mime.contains("jpeg") || mime.contains("jpg") {
        "jpg"
    } else if mime.contains("webp") {
        "webp"
    } else if kind == "audio" {
        "webm"
    } else if kind == "image" {
        "png"
    } else {
        "bin"
    }
}

fn path_is_under_root(root: &Path, path: &Path) -> bool {
    let normalized = normalize_path_lexically(path);
    normalized.starts_with(normalize_path_lexically(root))
}

fn normalize_path_lexically(path: &Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_url_decodes_mime_and_bytes() {
        let (bytes, mime) =
            decode_data_url("data:text/plain;base64,aGVsbG8=").expect("decode data url");
        assert_eq!(bytes, b"hello");
        assert_eq!(mime.as_deref(), Some("text/plain"));
    }

    #[test]
    fn safe_upload_filename_adds_extension() {
        assert_eq!(
            safe_upload_filename("../voice", "audio", "audio/webm"),
            "voice.webm"
        );
    }
}
