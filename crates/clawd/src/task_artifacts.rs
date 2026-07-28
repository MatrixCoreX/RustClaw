use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(crate) const TASK_ARTIFACT_SCHEMA_VERSION: u32 = 1;
const MAX_TASK_ARTIFACTS: usize = 32;
const DEFAULT_MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct TaskArtifactManifest {
    pub(crate) schema_version: u32,
    pub(crate) id: String,
    pub(crate) filename: String,
    pub(crate) kind: String,
    pub(crate) mime_type: String,
    pub(crate) size_bytes: u64,
    pub(crate) sha256: String,
    pub(crate) download_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) preview_url: Option<String>,
}

#[derive(Debug, Clone)]
struct ArtifactSource {
    id: Option<String>,
    path: String,
    filename: Option<String>,
    mime_type: Option<String>,
}

pub(crate) fn materialize_task_result_artifacts(
    workspace_root: &Path,
    task_id: &str,
    raw_result: &str,
) -> anyhow::Result<String> {
    let mut result = match serde_json::from_str::<Value>(raw_result) {
        Ok(value) => value,
        Err(_) => return Ok(raw_result.to_string()),
    };
    let sources = collect_artifact_sources(&result);
    if sources.is_empty() {
        return Ok(raw_result.to_string());
    }

    let workspace = workspace_root.canonicalize().map_err(|error| {
        anyhow::anyhow!(
            "artifact_workspace_canonicalize_failed:path={} error={error}",
            workspace_root.display()
        )
    })?;
    let mut manifests = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut seen_ids = HashSet::new();
    for source in sources.into_iter().take(MAX_TASK_ARTIFACTS * 2) {
        let Some(source_path) = validated_source_path(&workspace, &source.path) else {
            continue;
        };
        let source_key = source_path.to_string_lossy().to_string();
        if !seen_paths.insert(source_key) {
            continue;
        }
        let metadata = match source_path.metadata() {
            Ok(metadata) if metadata.is_file() && metadata.len() <= max_artifact_bytes() => {
                metadata
            }
            _ => continue,
        };
        let filename = safe_filename(
            source
                .filename
                .as_deref()
                .or_else(|| source_path.file_name().and_then(|value| value.to_str()))
                .unwrap_or("artifact.bin"),
        );
        let mut artifact_id = source
            .id
            .as_deref()
            .and_then(machine_id)
            .unwrap_or_else(|| derived_artifact_id(task_id, &source.path));
        if !seen_ids.insert(artifact_id.clone()) {
            artifact_id = derived_artifact_id(task_id, &source.path);
            if !seen_ids.insert(artifact_id.clone()) {
                continue;
            }
        }
        let mime_type = normalized_mime_type(source.mime_type.as_deref(), &filename);
        let destination = delivery_artifact_path(&workspace, task_id, &artifact_id, &filename);
        let sha256 = publish_artifact_file(&source_path, &destination)?;
        let base_url = format!("/v1/tasks/{task_id}/artifacts/{artifact_id}/content");
        manifests.push(TaskArtifactManifest {
            schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
            id: artifact_id,
            filename,
            kind: artifact_kind(&mime_type).to_string(),
            mime_type: mime_type.clone(),
            size_bytes: metadata.len(),
            sha256,
            download_url: base_url.clone(),
            preview_url: inline_preview_allowed(&mime_type)
                .then(|| format!("{base_url}?disposition=inline")),
        });
        if manifests.len() >= MAX_TASK_ARTIFACTS {
            break;
        }
    }
    if manifests.is_empty() {
        return Ok(raw_result.to_string());
    }
    let Some(object) = result.as_object_mut() else {
        return Ok(raw_result.to_string());
    };
    object.insert("artifacts".to_string(), serde_json::to_value(manifests)?);
    Ok(serde_json::to_string(&result)?)
}

pub(crate) fn manifests_from_result(result: Option<&Value>) -> Vec<TaskArtifactManifest> {
    result
        .and_then(|value| value.get("artifacts"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| serde_json::from_value::<TaskArtifactManifest>(value.clone()).ok())
        .filter(valid_manifest)
        .take(MAX_TASK_ARTIFACTS)
        .collect()
}

pub(crate) fn manifest_by_id(
    result: Option<&Value>,
    artifact_id: &str,
) -> Option<TaskArtifactManifest> {
    let artifact_id = machine_id(artifact_id)?;
    manifests_from_result(result)
        .into_iter()
        .find(|artifact| artifact.id == artifact_id)
}

pub(crate) fn delivery_artifact_path(
    workspace_root: &Path,
    task_id: &str,
    artifact_id: &str,
    filename: &str,
) -> PathBuf {
    workspace_root
        .join(".rustclaw")
        .join("artifacts")
        .join("delivery")
        .join(machine_path_component(task_id, "task"))
        .join(machine_path_component(artifact_id, "artifact"))
        .join(safe_filename(filename))
}

pub(crate) fn validated_delivery_artifact_path(
    workspace_root: &Path,
    task_id: &str,
    manifest: &TaskArtifactManifest,
) -> Option<PathBuf> {
    let workspace = workspace_root.canonicalize().ok()?;
    let task_root = workspace
        .join(".rustclaw")
        .join("artifacts")
        .join("delivery")
        .join(machine_path_component(task_id, "task"));
    let candidate = delivery_artifact_path(&workspace, task_id, &manifest.id, &manifest.filename);
    let canonical = candidate.canonicalize().ok()?;
    let canonical_task_root = task_root.canonicalize().ok()?;
    (canonical.starts_with(canonical_task_root) && canonical.is_file()).then_some(canonical)
}

pub(crate) fn cleanup_orphaned_delivery_artifacts(
    workspace_root: &Path,
    db: &Connection,
) -> anyhow::Result<usize> {
    let root = workspace_root
        .join(".rustclaw")
        .join("artifacts")
        .join("delivery");
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(task_id) = entry.file_name().to_str().and_then(machine_id) else {
            continue;
        };
        let exists = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE task_id = ?1)",
            [task_id],
            |row| row.get::<_, i64>(0),
        )? != 0;
        if !exists {
            fs::remove_dir_all(path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

pub(crate) fn inline_preview_allowed(mime_type: &str) -> bool {
    matches!(
        mime_type.split(';').next().unwrap_or_default().trim(),
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/avif"
            | "audio/mpeg"
            | "audio/wav"
            | "audio/ogg"
            | "audio/webm"
            | "audio/mp4"
            | "video/mp4"
            | "video/webm"
            | "video/quicktime"
            | "application/pdf"
    )
}

fn collect_artifact_sources(result: &Value) -> Vec<ArtifactSource> {
    let mut sources = Vec::new();
    collect_sources_from_object(result, &mut sources, true);
    for pointer in [
        "/task_journal/trace/capability_results",
        "/final_result_json/task_journal/trace/capability_results",
    ] {
        let Some(results) = result.pointer(pointer).and_then(Value::as_array) else {
            continue;
        };
        for capability_result in results {
            let status_ok = capability_result
                .get("status")
                .and_then(Value::as_str)
                .is_none_or(|status| status == "ok");
            if !status_ok {
                continue;
            }
            collect_sources_from_object(capability_result, &mut sources, true);
            if let Some(data) = capability_result.get("data") {
                collect_sources_from_object(data, &mut sources, true);
                if let Some(extra) = data.get("extra") {
                    collect_sources_from_object(extra, &mut sources, true);
                }
                if let Some(output) = data.get("output") {
                    collect_sources_from_object(output, &mut sources, true);
                }
            }
        }
    }
    sources
}

fn collect_sources_from_object(value: &Value, out: &mut Vec<ArtifactSource>, allow_outputs: bool) {
    if out.len() >= MAX_TASK_ARTIFACTS * 2 {
        return;
    }
    let Some(object) = value.as_object() else {
        return;
    };
    let dry_run = object.get("dry_run").and_then(Value::as_bool) == Some(true);
    let inherited_mime = object
        .get("mime_type")
        .or_else(|| object.get("media_type"))
        .and_then(Value::as_str);
    for key in ["artifacts", "artifact_refs", "output_artifact_refs"] {
        let Some(items) = object.get(key).and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            if let Some(source) = artifact_source(item, inherited_mime) {
                out.push(source);
            }
        }
    }
    if dry_run || !allow_outputs {
        return;
    }
    if let Some(path) = object
        .get("output_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        out.push(ArtifactSource {
            id: None,
            path: path.to_string(),
            filename: None,
            mime_type: inherited_mime.map(str::to_string),
        });
    }
    if let Some(items) = object.get("outputs").and_then(Value::as_array) {
        for item in items {
            if let Some(source) = artifact_source(item, inherited_mime) {
                out.push(source);
            }
        }
    }
    if let Some(extra) = object.get("extra") {
        collect_sources_from_object(extra, out, true);
    }
}

fn artifact_source(value: &Value, inherited_mime: Option<&str>) -> Option<ArtifactSource> {
    let object = value.as_object()?;
    let path = object
        .get("path")
        .or_else(|| object.get("output_path"))
        .and_then(Value::as_str)?
        .trim();
    if path.is_empty() {
        return None;
    }
    Some(ArtifactSource {
        id: object
            .get("id")
            .or_else(|| object.get("artifact_id"))
            .and_then(Value::as_str)
            .map(str::to_string),
        path: path.to_string(),
        filename: object
            .get("filename")
            .or_else(|| object.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        mime_type: object
            .get("mime_type")
            .or_else(|| object.get("media_type"))
            .and_then(Value::as_str)
            .or(inherited_mime)
            .map(str::to_string),
    })
}

fn validated_source_path(workspace_root: &Path, raw: &str) -> Option<PathBuf> {
    let path = Path::new(raw.trim());
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    let canonical = candidate.canonicalize().ok()?;
    (canonical.starts_with(workspace_root) && canonical.is_file()).then_some(canonical)
}

fn publish_artifact_file(source: &Path, destination: &Path) -> io::Result<String> {
    if source == destination {
        return sha256_file(source);
    }
    if let (Ok(source), Ok(destination)) = (source.canonicalize(), destination.canonicalize()) {
        if source == destination {
            return sha256_file(&source);
        }
    }
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "artifact_parent_missing"))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".artifact-{}.tmp", uuid::Uuid::new_v4()));
    let result = copy_and_hash(source, &temp).and_then(|sha256| {
        if destination.exists() {
            fs::remove_file(destination)?;
        }
        fs::rename(&temp, destination)?;
        Ok(sha256)
    });
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

fn copy_and_hash(source: &Path, destination: &Path) -> io::Result<String> {
    let mut input = File::open(source)?;
    let mut output = File::create(destination)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        output.write_all(&buffer[..read])?;
    }
    output.flush()?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut input = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn valid_manifest(manifest: &TaskArtifactManifest) -> bool {
    manifest.schema_version == TASK_ARTIFACT_SCHEMA_VERSION
        && machine_id(&manifest.id).is_some()
        && !manifest.filename.trim().is_empty()
        && manifest.download_url.starts_with("/v1/tasks/")
        && manifest.download_url.ends_with("/content")
        && manifest.sha256.len() == 64
}

fn max_artifact_bytes() -> u64 {
    std::env::var("RUSTCLAW_MAX_DELIVERY_ARTIFACT_BYTES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_MAX_ARTIFACT_BYTES)
        .max(1)
}

fn machine_id(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':')))
    .then(|| value.to_string())
}

fn machine_path_component(value: &str, fallback: &str) -> String {
    machine_id(value).unwrap_or_else(|| derived_artifact_id(fallback, value))
}

fn derived_artifact_id(task_id: &str, path: &str) -> String {
    let digest = Sha256::digest(format!("{task_id}\0{path}").as_bytes());
    format!("artifact-{}", &format!("{digest:x}")[..24])
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

fn normalized_mime_type(value: Option<&str>, filename: &str) -> String {
    let value = value.unwrap_or_default().trim().to_ascii_lowercase();
    if value.contains('/') && value.parse::<axum::http::HeaderValue>().is_ok() {
        return value;
    }
    mime_from_extension(filename).to_string()
}

fn mime_from_extension(filename: &str) -> &'static str {
    let extension = Path::new(filename)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "m4a" => "audio/mp4",
        "webm" => "video/webm",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "pdf" => "application/pdf",
        "txt" | "log" | "md" => "text/plain; charset=utf-8",
        "json" => "application/json",
        "csv" => "text/csv; charset=utf-8",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "tar" => "application/x-tar",
        _ => "application/octet-stream",
    }
}

fn artifact_kind(mime_type: &str) -> &'static str {
    let mime_type = mime_type.split(';').next().unwrap_or_default();
    if mime_type.starts_with("image/") {
        "image"
    } else if mime_type.starts_with("audio/") {
        "audio"
    } else if mime_type.starts_with("video/") {
        "video"
    } else if mime_type == "application/pdf" {
        "pdf"
    } else if mime_type.contains("officedocument") || mime_type.contains("msword") {
        "document"
    } else if matches!(
        mime_type,
        "application/zip" | "application/gzip" | "application/x-tar"
    ) {
        "archive"
    } else {
        "file"
    }
}

#[cfg(test)]
#[path = "task_artifacts_tests.rs"]
mod tests;
