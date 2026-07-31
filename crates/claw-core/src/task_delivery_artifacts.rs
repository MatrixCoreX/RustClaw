//! Convert clawd task artifact manifests into safe, channel-ready delivery tokens.
//!
//! Channel adapters already understand language-neutral `IMAGE_FILE:`, `VIDEO_FILE:`,
//! and `FILE:` tokens. This module makes the structured task artifact manifest the
//! primary source, while retaining those tokens as the adapter boundary.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::wechat_reply_media::strip_wechat_delivery_lines;

const TASK_ARTIFACT_SCHEMA_VERSION: u32 = 1;
const MAX_TASK_ARTIFACTS: usize = 32;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct TaskDeliveryArtifactManifest {
    schema_version: u32,
    id: String,
    filename: String,
    kind: String,
    mime_type: String,
    size_bytes: u64,
    sha256: String,
    download_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveryPreference {
    Default,
    Enabled,
    Disabled,
}

/// Merge structured task artifacts into the messages consumed by native channel adapters.
///
/// When artifacts are available, legacy delivery-token lines are removed and replaced with
/// tokens pointing at clawd's immutable task delivery copies. If a task explicitly disables
/// delivery, all delivery-token lines are removed. If any manifest cannot be resolved safely,
/// the original messages are retained as a compatibility fallback.
pub fn merge_task_artifact_delivery_messages(
    task_id: &str,
    result_json: Option<&Value>,
    workspace_root: &Path,
    messages: Vec<String>,
) -> Vec<String> {
    let preference = task_delivery_preference(result_json);
    if preference == DeliveryPreference::Disabled {
        return messages_without_delivery_lines(messages);
    }

    let manifests = task_artifact_manifests(task_id, result_json);
    if manifests.is_empty() {
        return messages;
    }
    let tokens = manifests
        .iter()
        .filter_map(|manifest| {
            validated_task_artifact_path(workspace_root, task_id, manifest)
                .map(|path| artifact_delivery_token(manifest, &path))
        })
        .collect::<Vec<_>>();

    // Keep the legacy source-path tokens when even one structured artifact is unavailable.
    // This preserves delivery during rolling upgrades or after a partial artifact cleanup.
    if tokens.len() != manifests.len() {
        return messages;
    }

    let mut merged = messages_without_delivery_lines(messages);
    let token_block = tokens.join("\n");
    if let Some(last) = merged.last_mut() {
        if !last.trim().is_empty() {
            last.push('\n');
        }
        last.push_str(&token_block);
    } else {
        merged.push(token_block);
    }
    merged
}

fn messages_without_delivery_lines(messages: Vec<String>) -> Vec<String> {
    messages
        .into_iter()
        .map(|message| strip_wechat_delivery_lines(&message).trim().to_string())
        .filter(|message| !message.is_empty())
        .collect()
}

fn task_delivery_preference(result_json: Option<&Value>) -> DeliveryPreference {
    let flags = result_json
        .and_then(|result| result.pointer("/task_journal/trace/capability_results"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|result| {
            result
                .pointer("/data/extra/delivery/deliver_to_user")
                .and_then(Value::as_bool)
        })
        .collect::<Vec<_>>();
    if flags.iter().any(|enabled| *enabled) {
        DeliveryPreference::Enabled
    } else if flags.is_empty() {
        DeliveryPreference::Default
    } else {
        DeliveryPreference::Disabled
    }
}

fn task_artifact_manifests(
    task_id: &str,
    result_json: Option<&Value>,
) -> Vec<TaskDeliveryArtifactManifest> {
    if machine_component(task_id).is_none() {
        return Vec::new();
    }
    result_json
        .and_then(|result| result.get("artifacts"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            serde_json::from_value::<TaskDeliveryArtifactManifest>(value.clone()).ok()
        })
        .filter(|manifest| valid_manifest(task_id, manifest))
        .take(MAX_TASK_ARTIFACTS)
        .collect()
}

fn valid_manifest(task_id: &str, manifest: &TaskDeliveryArtifactManifest) -> bool {
    manifest.schema_version == TASK_ARTIFACT_SCHEMA_VERSION
        && machine_component(&manifest.id).is_some()
        && !manifest.filename.trim().is_empty()
        && safe_filename(&manifest.filename) == manifest.filename
        && manifest.sha256.len() == 64
        && manifest.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        && manifest.download_url == format!("/v1/tasks/{task_id}/artifacts/{}/content", manifest.id)
}

fn validated_task_artifact_path(
    workspace_root: &Path,
    task_id: &str,
    manifest: &TaskDeliveryArtifactManifest,
) -> Option<PathBuf> {
    let workspace = workspace_root.canonicalize().ok()?;
    let task_component = machine_component(task_id)?;
    let artifact_component = machine_component(&manifest.id)?;
    crate::workspace_state::known_workspace_state_roots(&workspace)
        .into_iter()
        .find_map(|state_root| {
            let task_root = state_root
                .join("artifacts")
                .join("delivery")
                .join(&task_component);
            let candidate = task_root
                .join(&artifact_component)
                .join(safe_filename(&manifest.filename));
            let canonical_task_root = task_root.canonicalize().ok()?;
            let canonical = candidate.canonicalize().ok()?;
            let metadata = canonical.metadata().ok()?;
            (canonical.starts_with(canonical_task_root)
                && metadata.is_file()
                && metadata.len() == manifest.size_bytes)
                .then_some(canonical)
        })
}

fn artifact_delivery_token(manifest: &TaskDeliveryArtifactManifest, path: &Path) -> String {
    let kind = manifest.kind.trim().to_ascii_lowercase();
    let mime = manifest.mime_type.trim().to_ascii_lowercase();
    let prefix = if kind == "image" || mime.starts_with("image/") {
        "IMAGE_FILE:"
    } else if kind == "video" || mime.starts_with("video/") {
        "VIDEO_FILE:"
    } else {
        "FILE:"
    };
    format!("{prefix}{}", path.display())
}

fn machine_component(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':')))
    .then(|| value.to_string())
}

fn safe_filename(value: &str) -> String {
    let mut name = value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            {
                '_'
            } else {
                ch
            }
        })
        .take(180)
        .collect::<String>();
    name = name.trim_matches(['.', ' ']).to_string();
    if name.is_empty() || name == "." || name == ".." {
        "artifact.bin".to_string()
    } else {
        name
    }
}

#[cfg(test)]
#[path = "task_delivery_artifacts_tests.rs"]
mod tests;
