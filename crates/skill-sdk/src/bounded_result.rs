use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{SkillSdkError, SkillSdkResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinuationDescriptor {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default)]
    pub state: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactDescriptor {
    pub id: String,
    pub path: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub sensitivity: String,
    pub read_capability: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldTruncation {
    pub original_size: u64,
    pub returned_size: u64,
    pub unit: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoundedResult<T> {
    pub value: T,
    pub complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returned_size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returned_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation: Option<ContinuationDescriptor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactDescriptor>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub field_truncations: BTreeMap<String, FieldTruncation>,
}

impl<T> BoundedResult<T> {
    pub fn complete(value: T) -> Self {
        Self {
            value,
            complete: true,
            partial_reason: None,
            original_size_bytes: None,
            returned_size_bytes: None,
            original_count: None,
            returned_count: None,
            continuation: None,
            artifacts: Vec::new(),
            field_truncations: BTreeMap::new(),
        }
    }

    pub fn with_counts(mut self, returned: u64, original: u64) -> Self {
        self.returned_count = Some(returned);
        self.original_count = Some(original);
        self
    }

    pub fn page(
        value: T,
        returned: u64,
        original: u64,
        continuation: Option<ContinuationDescriptor>,
    ) -> Self {
        let complete = continuation.is_none();
        Self {
            value,
            complete,
            partial_reason: (!complete).then(|| "result_page".to_string()),
            original_size_bytes: None,
            returned_size_bytes: None,
            original_count: Some(original),
            returned_count: Some(returned),
            continuation,
            artifacts: Vec::new(),
            field_truncations: BTreeMap::new(),
        }
    }

    pub fn with_field_truncation(
        mut self,
        field: impl Into<String>,
        original_size: u64,
        returned_size: u64,
        unit: impl Into<String>,
    ) -> Self {
        self.field_truncations.insert(
            field.into(),
            FieldTruncation {
                original_size,
                returned_size,
                unit: unit.into(),
            },
        );
        self
    }
}

impl BoundedResult<String> {
    pub fn text(
        text: &str,
        inline_bytes: usize,
        spill: Option<&ArtifactSpill>,
        label: &str,
    ) -> SkillSdkResult<Self> {
        if text.len() <= inline_bytes {
            let size = text.len() as u64;
            return Ok(Self::complete(text.to_string()).with_sizes(size, size));
        }
        let spill = spill.ok_or_else(|| {
            SkillSdkError::new(
                "bounded_result_recovery_unavailable",
                "large result requires declared skill storage for artifact recovery",
            )
        })?;
        let artifact = spill.spill_bytes(label, "text/plain; charset=utf-8", text.as_bytes())?;
        let preview_end = utf8_prefix_end(text, inline_bytes);
        let preview = text[..preview_end].to_string();
        let continuation = ContinuationDescriptor {
            kind: "artifact_range".to_string(),
            token: Some(artifact.id.clone()),
            state: json!({
                "artifact_ref": artifact.id,
                "start_byte": preview_end,
                "end_byte": artifact.size_bytes,
                "read_capability": artifact.read_capability,
            }),
        };
        Ok(Self {
            value: preview,
            complete: false,
            partial_reason: Some("inline_protocol_budget".to_string()),
            original_size_bytes: Some(text.len() as u64),
            returned_size_bytes: Some(preview_end as u64),
            original_count: None,
            returned_count: None,
            continuation: Some(continuation),
            artifacts: vec![artifact],
            field_truncations: BTreeMap::new(),
        })
    }

    pub(crate) fn with_sizes(mut self, returned: u64, original: u64) -> Self {
        self.returned_size_bytes = Some(returned);
        self.original_size_bytes = Some(original);
        self
    }
}

#[derive(Debug, Clone)]
pub struct ArtifactSpill {
    root: PathBuf,
    namespace: String,
    sensitivity: String,
}

impl ArtifactSpill {
    pub fn new(root: impl Into<PathBuf>, namespace: impl AsRef<str>) -> SkillSdkResult<Self> {
        let namespace = safe_component(namespace.as_ref(), "result")?;
        let root = root.into();
        fs::create_dir_all(&root).map_err(|error| {
            SkillSdkError::new("artifact_spill_root_create_failed", error.to_string())
        })?;
        reject_symlink(&root)?;
        apply_private_directory_permissions(&root)?;
        Ok(Self {
            root,
            namespace,
            sensitivity: "skill_owner_restricted".to_string(),
        })
    }

    pub fn from_request_context(
        context: Option<&Value>,
        namespace: impl AsRef<str>,
    ) -> SkillSdkResult<Option<Self>> {
        let Some(database_path) = context
            .and_then(|value| value.pointer("/skill_storage/database_path"))
            .and_then(Value::as_str)
        else {
            return Ok(None);
        };
        let root = Path::new(database_path)
            .parent()
            .ok_or_else(|| {
                SkillSdkError::new(
                    "artifact_spill_storage_invalid",
                    "skill storage database path has no parent",
                )
            })?
            .join("artifacts");
        Self::new(root, namespace).map(Some)
    }

    pub fn spill_bytes(
        &self,
        label: &str,
        media_type: &str,
        bytes: &[u8],
    ) -> SkillSdkResult<ArtifactDescriptor> {
        let label = safe_component(label, "result")?;
        let sha256 = format!("{:x}", Sha256::digest(bytes));
        let directory = self.root.join(&self.namespace);
        fs::create_dir_all(&directory).map_err(|error| {
            SkillSdkError::new("artifact_spill_directory_create_failed", error.to_string())
        })?;
        reject_symlink(&directory)?;
        apply_private_directory_permissions(&directory)?;
        let path = directory.join(format!("{label}-{sha256}.artifact"));
        if !path.exists() {
            atomic_private_write(&path, bytes)?;
        }
        Ok(ArtifactDescriptor {
            id: format!("skill-artifact:{sha256}"),
            path: path.to_string_lossy().to_string(),
            media_type: media_type.to_string(),
            size_bytes: bytes.len() as u64,
            sha256,
            sensitivity: self.sensitivity.clone(),
            read_capability: "artifact.read_range".to_string(),
        })
    }

    /// Streams a large text result into skill-owned storage while retaining
    /// only a UTF-8-safe inline preview in memory.
    pub fn spill_text_reader<R: Read>(
        &self,
        label: &str,
        media_type: &str,
        mut reader: R,
        preview_bytes: usize,
    ) -> SkillSdkResult<BoundedResult<String>> {
        let label = safe_component(label, "result")?;
        let directory = self.root.join(&self.namespace);
        fs::create_dir_all(&directory).map_err(|error| {
            SkillSdkError::new("artifact_spill_directory_create_failed", error.to_string())
        })?;
        reject_symlink(&directory)?;
        apply_private_directory_permissions(&directory)?;
        let temporary = directory.join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|error| {
            SkillSdkError::new("artifact_spill_write_failed", error.to_string())
        })?;
        let streamed = (|| -> SkillSdkResult<(String, u64, Vec<u8>)> {
            let mut hasher = Sha256::new();
            let mut total = 0_u64;
            let mut preview = Vec::with_capacity(preview_bytes.min(64 * 1024));
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let read = reader.read(&mut buffer).map_err(|error| {
                    SkillSdkError::new("artifact_spill_read_failed", error.to_string())
                })?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
                file.write_all(&buffer[..read]).map_err(|error| {
                    SkillSdkError::new("artifact_spill_write_failed", error.to_string())
                })?;
                let remaining = preview_bytes.saturating_sub(preview.len());
                preview.extend_from_slice(&buffer[..read.min(remaining)]);
                total = total.checked_add(read as u64).ok_or_else(|| {
                    SkillSdkError::new("artifact_spill_size_overflow", "stream size overflow")
                })?;
            }
            file.sync_all().map_err(|error| {
                SkillSdkError::new("artifact_spill_sync_failed", error.to_string())
            })?;
            Ok((format!("{:x}", hasher.finalize()), total, preview))
        })();
        let (sha256, size_bytes, preview) = match streamed {
            Ok(streamed) => streamed,
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                return Err(error);
            }
        };
        let path = directory.join(format!("{label}-{sha256}.artifact"));
        if path.exists() {
            fs::remove_file(&temporary).map_err(|error| {
                SkillSdkError::new("artifact_spill_cleanup_failed", error.to_string())
            })?;
        } else {
            fs::rename(&temporary, &path).map_err(|error| {
                let _ = fs::remove_file(&temporary);
                SkillSdkError::new("artifact_spill_publish_failed", error.to_string())
            })?;
        }
        let artifact = ArtifactDescriptor {
            id: format!("skill-artifact:{sha256}"),
            path: path.to_string_lossy().to_string(),
            media_type: media_type.to_string(),
            size_bytes,
            sha256,
            sensitivity: self.sensitivity.clone(),
            read_capability: "artifact.read_range".to_string(),
        };
        // Continuation offsets address the original artifact bytes, so retain
        // the exact source-byte length even when a lossy preview needs to
        // replace a trailing partial UTF-8 sequence.
        let returned_size = preview.len() as u64;
        let preview = String::from_utf8_lossy(&preview).to_string();
        let continuation = ContinuationDescriptor {
            kind: "artifact_range".to_string(),
            token: Some(artifact.id.clone()),
            state: json!({
                "artifact_ref": artifact.id,
                "start_byte": returned_size,
                "end_byte": artifact.size_bytes,
                "read_capability": artifact.read_capability,
            }),
        };
        Ok(BoundedResult {
            value: preview,
            complete: false,
            partial_reason: Some("inline_protocol_budget".to_string()),
            original_size_bytes: Some(size_bytes),
            returned_size_bytes: Some(returned_size),
            original_count: None,
            returned_count: None,
            continuation: Some(continuation),
            artifacts: vec![artifact],
            field_truncations: BTreeMap::new(),
        })
    }
}

fn safe_component(value: &str, fallback: &str) -> SkillSdkResult<String> {
    let value = value.trim();
    let value = if value.is_empty() { fallback } else { value };
    if value.len() > 96
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || value.starts_with('.')
        || value.contains("..")
    {
        return Err(SkillSdkError::new(
            "artifact_spill_component_invalid",
            "artifact namespace and label must be safe path components",
        ));
    }
    Ok(value.to_string())
}

fn reject_symlink(path: &Path) -> SkillSdkResult<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| SkillSdkError::new("artifact_spill_metadata_failed", error.to_string()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(SkillSdkError::new(
            "artifact_spill_root_unsafe",
            "artifact spill root must be a real directory",
        ));
    }
    Ok(())
}

fn atomic_private_write(path: &Path, bytes: &[u8]) -> SkillSdkResult<()> {
    let parent = path.parent().ok_or_else(|| {
        SkillSdkError::new("artifact_spill_path_invalid", "artifact path has no parent")
    })?;
    let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| SkillSdkError::new("artifact_spill_write_failed", error.to_string()))?;
    file.write_all(bytes)
        .map_err(|error| SkillSdkError::new("artifact_spill_write_failed", error.to_string()))?;
    file.sync_all()
        .map_err(|error| SkillSdkError::new("artifact_spill_sync_failed", error.to_string()))?;
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        SkillSdkError::new("artifact_spill_publish_failed", error.to_string())
    })?;
    Ok(())
}

#[cfg(unix)]
fn apply_private_directory_permissions(path: &Path) -> SkillSdkResult<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| SkillSdkError::new("artifact_spill_permissions_failed", error.to_string()))
}

#[cfg(not(unix))]
fn apply_private_directory_permissions(_path: &Path) -> SkillSdkResult<()> {
    Ok(())
}

fn utf8_prefix_end(value: &str, limit: usize) -> usize {
    let mut end = value.len().min(limit);
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    end
}

#[cfg(test)]
#[path = "bounded_result_tests.rs"]
mod tests;
