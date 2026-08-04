use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PermissionMode {
    Safe,
    #[default]
    Ask,
    Yolo,
}

impl PermissionMode {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "safe" => Some(Self::Safe),
            "ask" => Some(Self::Ask),
            "yolo" => Some(Self::Yolo),
            _ => None,
        }
    }

    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Ask => "ask",
            Self::Yolo => "yolo",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ModelOverride {
    pub(crate) provider: String,
    pub(crate) model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionAttachmentRef {
    pub(crate) canonical_path: String,
    pub(crate) display_path: String,
    pub(crate) kind: String,
    pub(crate) mime_type: String,
    pub(crate) size: u64,
    pub(crate) sha256: String,
    pub(crate) materialization: String,
    pub(crate) truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WorkingDirectoryIdentity {
    pub(crate) canonical_path: String,
    pub(crate) identity_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChatSessionState {
    pub(crate) conversation_id: String,
    pub(crate) session_id: String,
    pub(crate) active_task_id: Option<String>,
    pub(crate) task_ids: Vec<String>,
    pub(crate) model_override: Option<ModelOverride>,
    pub(crate) permission_mode: PermissionMode,
    pub(crate) attachments: Vec<SessionAttachmentRef>,
    pub(crate) compacted_context_ref: Option<String>,
    pub(crate) goal_ref: Option<String>,
    pub(crate) rewind_anchor: Option<Value>,
    pub(crate) completed_side_effect_refs: Vec<String>,
    pub(crate) event_cursor: u64,
    pub(crate) working_directory: WorkingDirectoryIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChatSessionTransition {
    TaskSubmitted(String),
    TaskSelected(String),
    CursorAdvanced(u64),
    ModelChanged(Option<ModelOverride>),
    PermissionChanged(PermissionMode),
    AttachmentsCleared,
    ContextCompacted(String),
}

impl ChatSessionState {
    pub(crate) fn apply(&mut self, transition: ChatSessionTransition) -> Result<()> {
        match transition {
            ChatSessionTransition::TaskSubmitted(task_id)
            | ChatSessionTransition::TaskSelected(task_id) => {
                validate_machine_ref(&task_id, "chat_task_id_invalid")?;
                if self.task_ids.last() != Some(&task_id) {
                    self.task_ids.push(task_id.clone());
                }
                self.active_task_id = Some(task_id);
                self.event_cursor = 0;
            }
            ChatSessionTransition::CursorAdvanced(cursor) => {
                if cursor < self.event_cursor {
                    anyhow::bail!("chat_event_cursor_regression");
                }
                self.event_cursor = cursor;
            }
            ChatSessionTransition::ModelChanged(selection) => {
                self.model_override = selection;
            }
            ChatSessionTransition::PermissionChanged(mode) => {
                self.permission_mode = mode;
            }
            ChatSessionTransition::AttachmentsCleared => {
                self.attachments.clear();
            }
            ChatSessionTransition::ContextCompacted(reference) => {
                validate_machine_ref(&reference, "chat_compaction_ref_invalid")?;
                self.compacted_context_ref = Some(reference);
            }
        }
        Ok(())
    }
}

pub(crate) fn current_working_directory_identity() -> Result<WorkingDirectoryIdentity> {
    working_directory_identity(&std::env::current_dir().context("chat_current_dir_failed")?)
}

pub(crate) fn working_directory_identity(path: &Path) -> Result<WorkingDirectoryIdentity> {
    let canonical = path.canonicalize().with_context(|| {
        format!(
            "chat_working_directory_unavailable:{path}",
            path = path.display()
        )
    })?;
    let canonical_path = canonical.to_string_lossy().into_owned();
    let identity_sha256 = format!("{:x}", Sha256::digest(canonical_path.as_bytes()));
    Ok(WorkingDirectoryIdentity {
        canonical_path,
        identity_sha256,
    })
}

pub(crate) fn validate_machine_ref(value: &str, error_code: &'static str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
    {
        anyhow::bail!(error_code);
    }
    Ok(())
}

#[cfg(test)]
#[path = "chat_session_tests.rs"]
mod tests;
