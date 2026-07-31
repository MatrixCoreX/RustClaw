//! Extract outbound media from structured payloads or language-neutral delivery tokens.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::channel_delivery_tokens::{
    legacy_delivery_tokens, parse_legacy_delivery_line_ref, LegacyDeliveryKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WechatOutboundKind {
    Image,
    Video,
    Audio,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WechatOutboundSource {
    LocalPath(PathBuf),
    RemoteUrl(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WechatOutboundMedia {
    pub kind: WechatOutboundKind,
    pub source: WechatOutboundSource,
}

/// Ordered media attachments to send after optional caption text (line order preserved).
pub fn extract_wechat_outbound_media(
    answer: &str,
    workspace_root: &Path,
) -> Vec<WechatOutboundMedia> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    collect_structured_media(answer, workspace_root, &mut out, &mut seen);
    for token in legacy_delivery_tokens(answer) {
        let forced_kind = match token.kind {
            LegacyDeliveryKind::Image => Some(WechatOutboundKind::Image),
            LegacyDeliveryKind::Video => Some(WechatOutboundKind::Video),
            LegacyDeliveryKind::Voice | LegacyDeliveryKind::Music => {
                Some(WechatOutboundKind::Audio)
            }
            LegacyDeliveryKind::File => Some(WechatOutboundKind::File),
            LegacyDeliveryKind::Auto => None,
        };
        push_media_reference(
            &mut out,
            &mut seen,
            workspace_root,
            token.reference,
            forced_kind,
        );
    }
    for line in answer.lines() {
        if let Some(p) = parse_written_bytes_line(line.trim()) {
            push_media_reference(&mut out, &mut seen, workspace_root, p, None);
        }
    }
    out
}

/// Back-compat: image-only paths (subset of [`extract_wechat_outbound_media`]).
pub fn extract_image_paths_from_reply(answer: &str, workspace_root: &Path) -> Vec<PathBuf> {
    extract_wechat_outbound_media(answer, workspace_root)
        .into_iter()
        .filter_map(|media| match media {
            WechatOutboundMedia {
                kind: WechatOutboundKind::Image,
                source: WechatOutboundSource::LocalPath(path),
            } => Some(path),
            _ => None,
        })
        .collect()
}

pub fn strip_wechat_delivery_lines(answer: &str) -> String {
    answer
        .lines()
        .filter(|line| {
            let t = line.trim();
            if parse_legacy_delivery_line_ref(t).is_some() {
                return false;
            }
            if line_has_structured_media(t) {
                return false;
            }
            if parse_written_bytes_line(t).is_some() {
                return false;
            }
            true
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn collect_structured_media(
    answer: &str,
    workspace_root: &Path,
    out: &mut Vec<WechatOutboundMedia>,
    seen: &mut HashSet<String>,
) {
    let trimmed = answer.trim();
    if is_json_candidate(trimmed) {
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            collect_media_from_value(&value, workspace_root, out, seen, false);
            return;
        }
    }
    for line in answer
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if !is_json_candidate(line) {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            collect_media_from_value(&value, workspace_root, out, seen, false);
        }
    }
}

fn line_has_structured_media(line: &str) -> bool {
    if !is_json_candidate(line) {
        return false;
    }
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    value_contains_structured_media(&value, false)
}

fn is_json_candidate(s: &str) -> bool {
    let trimmed = s.trim_start();
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

fn collect_media_from_value(
    value: &Value,
    workspace_root: &Path,
    out: &mut Vec<WechatOutboundMedia>,
    seen: &mut HashSet<String>,
    media_context: bool,
) {
    match value {
        Value::Object(obj) => {
            collect_media_from_object(obj, workspace_root, out, seen, media_context)
        }
        Value::Array(items) => {
            for item in items {
                collect_media_from_value(item, workspace_root, out, seen, media_context);
            }
        }
        _ => {}
    }
}

fn collect_media_from_object(
    obj: &Map<String, Value>,
    workspace_root: &Path,
    out: &mut Vec<WechatOutboundMedia>,
    seen: &mut HashSet<String>,
    media_context: bool,
) {
    if is_browser_task_artifact_manifest(obj) {
        return;
    }
    let kind = media_kind_from_object(obj);
    for key in ["output_path", "file_path", "local_path", "resolved_path"] {
        collect_string_or_strings(obj.get(key), workspace_root, out, seen, kind);
    }
    if media_context || kind.is_some() {
        collect_string_or_strings(obj.get("path"), workspace_root, out, seen, kind);
    }
    for (key, forced_kind) in [
        ("image_url", Some(WechatOutboundKind::Image)),
        ("video_url", Some(WechatOutboundKind::Video)),
        ("file_url", Some(WechatOutboundKind::File)),
        ("media_url", kind),
        ("download_url", kind),
    ] {
        collect_string_or_strings(obj.get(key), workspace_root, out, seen, forced_kind);
    }
    if media_context || kind.is_some() {
        collect_string_or_strings(obj.get("url"), workspace_root, out, seen, kind);
    }

    for (key, child) in obj {
        if matches!(key.as_str(), "text" | "error_text" | "message") {
            continue;
        }
        let child_media_context = media_context || structured_media_container_key(key);
        if child_media_context || structured_payload_container_key(key) {
            collect_media_from_value(child, workspace_root, out, seen, child_media_context);
        }
    }
}

fn value_contains_structured_media(value: &Value, media_context: bool) -> bool {
    match value {
        Value::Object(obj) => object_contains_structured_media(obj, media_context),
        Value::Array(items) => items
            .iter()
            .any(|item| value_contains_structured_media(item, media_context)),
        _ => false,
    }
}

fn object_contains_structured_media(obj: &Map<String, Value>, media_context: bool) -> bool {
    if is_browser_task_artifact_manifest(obj) {
        return false;
    }
    let kind = media_kind_from_object(obj);
    if ["output_path", "file_path", "local_path", "resolved_path"]
        .iter()
        .any(|key| value_has_string_or_strings(obj.get(*key)))
    {
        return true;
    }
    if (media_context || kind.is_some()) && value_has_string_or_strings(obj.get("path")) {
        return true;
    }
    if [
        "image_url",
        "video_url",
        "file_url",
        "media_url",
        "download_url",
    ]
    .iter()
    .any(|key| value_has_string_or_strings(obj.get(*key)))
    {
        return true;
    }
    if (media_context || kind.is_some()) && value_has_string_or_strings(obj.get("url")) {
        return true;
    }
    obj.iter().any(|(key, child)| {
        if matches!(key.as_str(), "text" | "error_text" | "message") {
            return false;
        }
        let child_media_context = media_context || structured_media_container_key(key);
        (child_media_context || structured_payload_container_key(key))
            && value_contains_structured_media(child, child_media_context)
    })
}

fn is_browser_task_artifact_manifest(obj: &Map<String, Value>) -> bool {
    obj.get("schema_version").and_then(Value::as_u64) == Some(1)
        && obj.get("id").and_then(Value::as_str).is_some()
        && obj.get("sha256").and_then(Value::as_str).is_some()
        && obj
            .get("download_url")
            .and_then(Value::as_str)
            .is_some_and(|url| {
                url.starts_with("/v1/tasks/")
                    && url.contains("/artifacts/")
                    && url.ends_with("/content")
            })
}

fn value_has_string_or_strings(value: Option<&Value>) -> bool {
    match value {
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Array(items)) => items
            .iter()
            .any(|item| matches!(item, Value::String(s) if !s.trim().is_empty())),
        _ => false,
    }
}

fn collect_string_or_strings(
    value: Option<&Value>,
    workspace_root: &Path,
    out: &mut Vec<WechatOutboundMedia>,
    seen: &mut HashSet<String>,
    forced_kind: Option<WechatOutboundKind>,
) {
    match value {
        Some(Value::String(s)) => {
            push_media_reference(out, seen, workspace_root, normalize_token(s), forced_kind);
        }
        Some(Value::Array(items)) => {
            for item in items {
                if let Value::String(s) = item {
                    push_media_reference(
                        out,
                        seen,
                        workspace_root,
                        normalize_token(s),
                        forced_kind,
                    );
                }
            }
        }
        _ => {}
    }
}

fn structured_media_container_key(key: &str) -> bool {
    matches!(
        key,
        "media"
            | "media_delivery"
            | "delivery"
            | "attachments"
            | "attachment"
            | "outputs"
            | "planned_outputs"
            | "artifacts"
            | "artifact"
            | "files"
            | "file"
    )
}

fn structured_payload_container_key(key: &str) -> bool {
    matches!(
        key,
        "extra"
            | "result"
            | "payload"
            | "output"
            | "final_result_json"
            | "async_poll_adapter_result"
            | "poll_result"
            | "skill_result"
    ) || structured_media_container_key(key)
}

fn media_kind_from_object(obj: &Map<String, Value>) -> Option<WechatOutboundKind> {
    for key in [
        "kind",
        "type",
        "media_type",
        "artifact_type",
        "output_type",
        "message_type",
    ] {
        let Some(value) = obj.get(key).and_then(Value::as_str) else {
            continue;
        };
        if let Some(kind) = media_kind_from_token(value) {
            return Some(kind);
        }
    }
    None
}

fn media_kind_from_token(token: &str) -> Option<WechatOutboundKind> {
    match token.trim().to_ascii_lowercase().as_str() {
        "image" | "image_file" | "photo" | "picture" => Some(WechatOutboundKind::Image),
        "video" | "video_file" => Some(WechatOutboundKind::Video),
        "audio" | "audio_file" | "voice" | "voice_file" | "music" | "music_file" => {
            Some(WechatOutboundKind::Audio)
        }
        "file" | "file_file" | "document" => Some(WechatOutboundKind::File),
        _ => None,
    }
}

fn normalize_token(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix('`').unwrap_or(s);
    let s = s.strip_suffix('`').unwrap_or(s);
    s.trim().trim_matches('"').trim_matches('\'').to_string()
}

fn push_media_reference(
    out: &mut Vec<WechatOutboundMedia>,
    seen: &mut HashSet<String>,
    workspace_root: &Path,
    token: String,
    forced_kind: Option<WechatOutboundKind>,
) {
    let Some(media) = parse_media_reference(workspace_root, token, forced_kind) else {
        return;
    };
    let key = media_dedupe_key(&media);
    if seen.insert(key) {
        out.push(media);
    }
}

fn parse_media_reference(
    workspace_root: &Path,
    token: String,
    forced_kind: Option<WechatOutboundKind>,
) -> Option<WechatOutboundMedia> {
    if token.is_empty() {
        return None;
    }
    if is_remote_url(&token) {
        return Some(WechatOutboundMedia {
            kind: forced_kind.unwrap_or_else(|| classify_remote_kind(&token)),
            source: WechatOutboundSource::RemoteUrl(token),
        });
    }
    let local_token = normalize_local_token(&token);
    let path = resolve_workspace_path(workspace_root, local_token)?;
    Some(WechatOutboundMedia {
        kind: forced_kind.unwrap_or_else(|| classify_path_kind(&path)),
        source: WechatOutboundSource::LocalPath(path),
    })
}

fn media_dedupe_key(media: &WechatOutboundMedia) -> String {
    match media {
        WechatOutboundMedia {
            kind,
            source: WechatOutboundSource::LocalPath(path),
        } => format!("{kind:?}:local:{}", path.display()),
        WechatOutboundMedia {
            kind,
            source: WechatOutboundSource::RemoteUrl(url),
        } => format!("{kind:?}:remote:{url}"),
    }
}

fn normalize_local_token(token: &str) -> String {
    token.strip_prefix("file://").unwrap_or(token).to_string()
}

fn resolve_workspace_path(workspace_root: &Path, token: String) -> Option<PathBuf> {
    if token.is_empty() {
        return None;
    }
    let p = Path::new(&token);
    let candidate = if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace_root.join(&token)
    };
    Some(candidate.canonicalize().unwrap_or(candidate))
}

fn is_remote_url(token: &str) -> bool {
    token.starts_with("http://") || token.starts_with("https://")
}

fn is_probably_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp"
            )
        })
        .unwrap_or(false)
}

fn is_probably_video(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "mp4" | "mov" | "webm" | "mkv" | "m4v"
            )
        })
        .unwrap_or(false)
}

fn is_probably_audio(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "aac" | "m4a" | "mp3" | "amr" | "ogg" | "opus" | "wav" | "flac"
            )
        })
        .unwrap_or(false)
}

fn classify_path_kind(p: &Path) -> WechatOutboundKind {
    if is_probably_image(p) {
        WechatOutboundKind::Image
    } else if is_probably_video(p) {
        WechatOutboundKind::Video
    } else if is_probably_audio(p) {
        WechatOutboundKind::Audio
    } else {
        WechatOutboundKind::File
    }
}

fn classify_remote_kind(url: &str) -> WechatOutboundKind {
    let normalized = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    if normalized.ends_with(".jpg")
        || normalized.ends_with(".jpeg")
        || normalized.ends_with(".png")
        || normalized.ends_with(".webp")
        || normalized.ends_with(".gif")
        || normalized.ends_with(".bmp")
    {
        WechatOutboundKind::Image
    } else if normalized.ends_with(".mp4")
        || normalized.ends_with(".mov")
        || normalized.ends_with(".webm")
        || normalized.ends_with(".mkv")
        || normalized.ends_with(".m4v")
    {
        WechatOutboundKind::Video
    } else if normalized.ends_with(".aac")
        || normalized.ends_with(".m4a")
        || normalized.ends_with(".mp3")
        || normalized.ends_with(".amr")
        || normalized.ends_with(".ogg")
        || normalized.ends_with(".opus")
        || normalized.ends_with(".wav")
        || normalized.ends_with(".flac")
    {
        WechatOutboundKind::Audio
    } else {
        WechatOutboundKind::File
    }
}

fn parse_written_bytes_line(line: &str) -> Option<String> {
    let t = line.trim();
    let rest = t.strip_prefix("written ")?;
    let (_bytes, path_part) = rest.split_once(" bytes to ")?;
    let p = normalize_token(path_part);
    if p.is_empty() {
        None
    } else {
        Some(p)
    }
}

#[cfg(test)]
#[path = "wechat_reply_media_tests.rs"]
mod tests;
