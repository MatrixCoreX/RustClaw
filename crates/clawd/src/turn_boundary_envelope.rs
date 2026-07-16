use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::ClaimedTask;

pub(crate) const TURN_BOUNDARY_ENVELOPE_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TurnInputMaterialization {
    RawText,
    AttachmentOnly,
    AudioTranscript,
    TextAndAudioTranscript,
}

impl TurnInputMaterialization {
    pub(crate) fn classify(
        has_audio_transcript: bool,
        has_raw_text: bool,
        attachment_count: usize,
    ) -> Self {
        match (has_audio_transcript, has_raw_text, attachment_count > 0) {
            (true, false, _) => Self::AudioTranscript,
            (true, true, _) => Self::TextAndAudioTranscript,
            (false, false, true) => Self::AttachmentOnly,
            _ => Self::RawText,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TurnAttachmentRef {
    pub(crate) kind: String,
    pub(crate) path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TurnSafetyContext {
    pub(crate) task_identity_bound: bool,
    pub(crate) attachments_validated: bool,
    pub(crate) allow_path_outside_workspace: bool,
    pub(crate) allow_sudo: bool,
}

/// Machine-owned request context that may enter the planner before any
/// ordinary semantic interpretation. Raw user text is passed separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TurnBoundaryEnvelope {
    pub(crate) task_id: String,
    pub(crate) user_id: i64,
    pub(crate) chat_id: i64,
    pub(crate) user_key: Option<String>,
    pub(crate) channel: String,
    pub(crate) external_user_id: Option<String>,
    pub(crate) external_chat_id: Option<String>,
    pub(crate) task_kind: String,
    pub(crate) input_materialization: TurnInputMaterialization,
    pub(crate) attachment_refs: Vec<TurnAttachmentRef>,
    pub(crate) explicit_api_fields: BTreeMap<String, String>,
    pub(crate) explicit_machine_command: Option<String>,
    pub(crate) structured_locator_facts: Vec<String>,
    pub(crate) safety_context: TurnSafetyContext,
    pub(crate) permission_profile: String,
    pub(crate) budget_profile: String,
    pub(crate) raw_chars: usize,
}

impl TurnBoundaryEnvelope {
    pub(crate) fn from_claimed_task(
        task: &ClaimedTask,
        payload: &Value,
        raw_request: &str,
        input_materialization: TurnInputMaterialization,
        explicit_machine_command: Option<String>,
        allow_path_outside_workspace: bool,
        allow_sudo: bool,
    ) -> Self {
        let attachment_refs = attachment_refs(payload);
        let explicit_api_fields = explicit_api_fields(payload);
        let structured_locator_facts = structured_locator_facts(payload, &attachment_refs);
        let permission_profile = explicit_api_fields
            .get("permission_profile")
            .cloned()
            .unwrap_or_else(|| {
                if allow_path_outside_workspace || allow_sudo {
                    "elevated_runtime_policy".to_string()
                } else {
                    "workspace_scoped".to_string()
                }
            });
        let budget_profile = explicit_api_fields
            .get("budget_profile")
            .cloned()
            .unwrap_or_else(|| "adaptive".to_string());

        Self {
            task_id: task.task_id.clone(),
            user_id: task.user_id,
            chat_id: task.chat_id,
            user_key: task.user_key.clone(),
            channel: task.channel.clone(),
            external_user_id: task.external_user_id.clone(),
            external_chat_id: task.external_chat_id.clone(),
            task_kind: task.kind.clone(),
            input_materialization,
            attachment_refs,
            explicit_api_fields,
            explicit_machine_command: non_empty_bounded(explicit_machine_command.as_deref()),
            structured_locator_facts,
            safety_context: TurnSafetyContext {
                task_identity_bound: true,
                attachments_validated: true,
                allow_path_outside_workspace,
                allow_sudo,
            },
            permission_profile,
            budget_profile,
            raw_chars: raw_request.chars().count(),
        }
    }

    pub(crate) fn schema_version(&self) -> u8 {
        TURN_BOUNDARY_ENVELOPE_SCHEMA_VERSION
    }

    pub(crate) fn compact_prompt_line(&self) -> String {
        let projection = serde_json::json!({
            "schema_version": self.schema_version(),
            "task_kind": self.task_kind,
            "input_materialization": self.input_materialization,
            "channel": self.channel,
            "attachment_refs": self.attachment_refs,
            "explicit_api_fields": self.explicit_api_fields,
            "explicit_machine_command": self.explicit_machine_command,
            "structured_locator_facts": self.structured_locator_facts,
            "safety_context": self.safety_context,
            "permission_profile": self.permission_profile,
            "budget_profile": self.budget_profile,
            "raw_chars": self.raw_chars,
        });
        format!("- turn_boundary_envelope={projection}")
    }
}

fn explicit_api_fields(payload: &Value) -> BTreeMap<String, String> {
    const MACHINE_FIELDS: &[&str] = &[
        "source",
        "thread_id",
        "session_id",
        "resume_task_id",
        "checkpoint_id",
        "workspace_id",
        "permission_profile",
        "approval_policy",
        "budget_profile",
    ];

    MACHINE_FIELDS
        .iter()
        .filter_map(|field| {
            non_empty_bounded(payload.get(*field).and_then(Value::as_str))
                .map(|value| ((*field).to_string(), value))
        })
        .collect()
}

fn attachment_refs(payload: &Value) -> Vec<TurnAttachmentRef> {
    payload
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let path = non_empty_bounded(item.get("path").and_then(Value::as_str))?;
            Some(TurnAttachmentRef {
                kind: non_empty_bounded(item.get("kind").and_then(Value::as_str))
                    .unwrap_or_else(|| "file".to_string()),
                path,
                mime_type: non_empty_bounded(item.get("mime_type").and_then(Value::as_str)),
                size: item.get("size").and_then(Value::as_u64),
            })
        })
        .collect()
}

fn structured_locator_facts(payload: &Value, attachments: &[TurnAttachmentRef]) -> Vec<String> {
    const LOCATOR_FIELDS: &[&str] = &["path", "locator", "workspace_path"];
    const LOCATOR_ARRAY_FIELDS: &[&str] = &["paths", "locators"];
    let mut locators = Vec::new();
    for field in LOCATOR_FIELDS {
        if let Some(value) = non_empty_bounded(payload.get(*field).and_then(Value::as_str)) {
            push_unique(&mut locators, value);
        }
    }
    for field in LOCATOR_ARRAY_FIELDS {
        for value in payload
            .get(*field)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|value| non_empty_bounded(Some(value)))
        {
            push_unique(&mut locators, value);
        }
    }
    for attachment in attachments {
        push_unique(&mut locators, attachment.path.clone());
    }
    locators
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn non_empty_bounded(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    Some(value.chars().take(1_024).collect())
}

#[cfg(test)]
#[path = "turn_boundary_envelope_tests.rs"]
mod tests;
