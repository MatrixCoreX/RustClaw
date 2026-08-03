//! Convert clawd task artifact manifests into safe, channel-ready delivery tokens.
//!
//! Channel adapters already understand language-neutral `IMAGE_FILE:`, `VIDEO_FILE:`,
//! and `FILE:` tokens. This module makes the structured task artifact manifest the
//! primary source, while retaining those tokens as the adapter boundary.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::{
    channel_delivery_tokens::legacy_delivery_tokens,
    wechat_reply_media::strip_wechat_delivery_lines,
};

const TASK_ARTIFACT_SCHEMA_VERSION: u32 = 2;
const LEGACY_TASK_ARTIFACT_SCHEMA_VERSION: u32 = 1;
const MAX_TASK_ARTIFACTS: usize = 32;
const ASYNC_JOB_COMPLETION_SOURCE: &str = "async_job_completion_checkpoint";
const ASYNC_JOB_TERMINAL_OBSERVATION_POINTERS: &[&str] = &[
    "/task_journal/trace/task_checkpoint/boundary_context/async_job_terminal_observation",
    "/task_journal/summary/task_checkpoint/boundary_context/async_job_terminal_observation",
    "/task_checkpoint/boundary_context/async_job_terminal_observation",
    "/task_lifecycle/resume_executor_result_projection/final_result_json/task_journal/trace/task_checkpoint/boundary_context/async_job_terminal_observation",
    "/task_lifecycle/resume_executor_result_projection/final_result_json/task_journal/summary/task_checkpoint/boundary_context/async_job_terminal_observation",
];

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct TaskDeliveryArtifactManifest {
    schema_version: u32,
    id: String,
    #[serde(default)]
    artifact_ref: String,
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
    let explicit_references = messages
        .iter()
        .flat_map(|message| legacy_delivery_tokens(message))
        .map(|token| token.reference)
        .collect::<Vec<_>>();
    let selected_manifests = manifests
        .iter()
        .filter(|manifest| {
            if explicit_references.is_empty() {
                !internal_runtime_artifact(manifest)
            } else {
                explicit_references
                    .iter()
                    .any(|reference| manifest_matches_reference(manifest, reference))
            }
        })
        .collect::<Vec<_>>();
    if selected_manifests.is_empty() {
        return messages;
    }
    if !explicit_references.is_empty()
        && !explicit_references.iter().all(|reference| {
            selected_manifests
                .iter()
                .any(|manifest| manifest_matches_reference(manifest, reference))
        })
    {
        return messages;
    }
    let tokens = selected_manifests
        .iter()
        .filter_map(|manifest| {
            validated_task_artifact_path(workspace_root, task_id, manifest)
                .map(|path| artifact_delivery_token(manifest, &path))
        })
        .collect::<Vec<_>>();

    // Keep the legacy source-path tokens when even one structured artifact is unavailable.
    // This preserves delivery during rolling upgrades or after a partial artifact cleanup.
    if tokens.len() != selected_manifests.len() {
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

fn internal_runtime_artifact(manifest: &TaskDeliveryArtifactManifest) -> bool {
    manifest.id.starts_with("skill-output:")
}

fn manifest_matches_reference(manifest: &TaskDeliveryArtifactManifest, reference: &str) -> bool {
    let reference = reference.trim();
    reference == manifest.artifact_ref
        || reference == manifest.id
        || Path::new(reference)
            .file_name()
            .and_then(|name| name.to_str())
            == Some(manifest.filename.as_str())
}

fn messages_without_delivery_lines(messages: Vec<String>) -> Vec<String> {
    messages
        .into_iter()
        .map(|message| strip_wechat_delivery_lines(&message).trim().to_string())
        .filter(|message| !message.is_empty())
        .collect()
}

fn task_delivery_preference(result_json: Option<&Value>) -> DeliveryPreference {
    let Some(result_json) = result_json else {
        return DeliveryPreference::Default;
    };
    let mut flags = Vec::new();
    for pointer in [
        "/extra/delivery/deliver_to_user",
        "/final_result_json/extra/delivery/deliver_to_user",
    ] {
        if let Some(flag) = result_json.pointer(pointer).and_then(Value::as_bool) {
            flags.push(flag);
        }
    }
    for pointer in [
        "/task_journal/trace/capability_results",
        "/final_result_json/task_journal/trace/capability_results",
    ] {
        let Some(results) = result_json.pointer(pointer).and_then(Value::as_array) else {
            continue;
        };
        flags.extend(results.iter().filter_map(|result| {
            result
                .pointer("/data/extra/delivery/deliver_to_user")
                .and_then(Value::as_bool)
        }));
    }
    if let Some(final_result) = trusted_async_job_terminal_final_result(result_json) {
        if let Some(flag) = final_result
            .pointer("/extra/delivery/deliver_to_user")
            .and_then(Value::as_bool)
        {
            flags.push(flag);
        }
    }
    if flags.iter().any(|enabled| *enabled) {
        DeliveryPreference::Enabled
    } else if flags.is_empty() {
        DeliveryPreference::Default
    } else {
        DeliveryPreference::Disabled
    }
}

/// Return the structured result of a trusted, successfully completed async job.
///
/// Async execution stores its terminal payload below the task checkpoint instead of
/// duplicating it into the pre-resume capability result. Artifact materialization and
/// channel delivery share this decoder so they cannot drift onto different paths.
pub fn trusted_async_job_terminal_final_result(result: &Value) -> Option<&Value> {
    ASYNC_JOB_TERMINAL_OBSERVATION_POINTERS
        .iter()
        .filter_map(|pointer| result.pointer(pointer))
        .find_map(|observation| {
            let trusted = observation.get("schema_version").and_then(Value::as_u64) == Some(1)
                && observation.get("source").and_then(Value::as_str)
                    == Some(ASYNC_JOB_COMPLETION_SOURCE)
                && observation.get("status").and_then(Value::as_str) == Some("succeeded");
            trusted
                .then(|| observation.get("final_result_json"))
                .flatten()
                .filter(|value| value.is_object())
        })
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
    let expected_ref = canonical_task_artifact_ref(task_id, &manifest.id);
    matches!(
        manifest.schema_version,
        TASK_ARTIFACT_SCHEMA_VERSION | LEGACY_TASK_ARTIFACT_SCHEMA_VERSION
    ) && (expected_ref.as_deref() == Some(manifest.artifact_ref.as_str())
        || (manifest.schema_version == LEGACY_TASK_ARTIFACT_SCHEMA_VERSION
            && manifest.artifact_ref.is_empty()))
        && machine_component(&manifest.id).is_some()
        && !manifest.filename.trim().is_empty()
        && safe_filename(&manifest.filename) == manifest.filename
        && manifest.sha256.len() == 64
        && manifest.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        && manifest.download_url == format!("/v1/tasks/{task_id}/artifacts/{}/content", manifest.id)
}

/// Canonical, channel-neutral address for an immutable task artifact.
pub fn canonical_task_artifact_ref(task_id: &str, artifact_id: &str) -> Option<String> {
    let task_id = machine_component(task_id)?;
    let artifact_id = machine_component(artifact_id)?;
    Some(format!("artifact:task/{task_id}/{artifact_id}"))
}

/// Returns true only for an existing regular file inside the runtime-managed
/// immutable task delivery tree. Channel adapters use this to decide whether
/// a failed native upload can safely point users to the Web task-detail copy.
pub fn is_managed_task_delivery_artifact_path(workspace_root: &Path, path: &Path) -> bool {
    let Ok(canonical_path) = path.canonicalize() else {
        return false;
    };
    if !canonical_path.is_file() {
        return false;
    }
    crate::workspace_state::known_workspace_state_roots(workspace_root)
        .into_iter()
        .filter_map(|state_root| {
            state_root
                .join("artifacts")
                .join("delivery")
                .canonicalize()
                .ok()
        })
        .any(|delivery_root| canonical_path.starts_with(delivery_root))
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
