use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::settings::{ExternalContextPolicy, MemoryEffectiveSettings};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemorySourceCategory {
    UserAuthored,
    AssistantAuthored,
    LocalTool,
    WorkspaceFile,
    AttachmentOcrStt,
    Web,
    McpPrivateConnector,
    Skill,
    Subagent,
    ScheduledBackground,
    GroupChannel,
}

impl MemorySourceCategory {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::UserAuthored => "user_authored",
            Self::AssistantAuthored => "assistant_authored",
            Self::LocalTool => "local_tool",
            Self::WorkspaceFile => "workspace_file",
            Self::AttachmentOcrStt => "attachment_ocr_stt",
            Self::Web => "web",
            Self::McpPrivateConnector => "mcp_private_connector",
            Self::Skill => "skill",
            Self::Subagent => "subagent",
            Self::ScheduledBackground => "scheduled_background",
            Self::GroupChannel => "group_channel",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemoryEligibilityDisposition {
    Candidate,
    EvidenceOnly,
    Excluded,
    ParentVerifierRequired,
}

impl MemoryEligibilityDisposition {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::EvidenceOnly => "evidence_only",
            Self::Excluded => "excluded",
            Self::ParentVerifierRequired => "parent_verifier_required",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct MemoryGenerationEligibilityItem {
    pub(crate) category: MemorySourceCategory,
    pub(crate) disposition: MemoryEligibilityDisposition,
    pub(crate) reason_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct MemoryGenerationEligibility {
    pub(crate) schema_version: u32,
    pub(crate) items: Vec<MemoryGenerationEligibilityItem>,
    pub(crate) external_context_policy: ExternalContextPolicy,
    pub(crate) policy_digest: String,
    pub(crate) durable_candidate_allowed: bool,
}

pub(crate) fn build_turn_eligibility(
    task: &crate::ClaimedTask,
    payload: &Value,
    settings: &MemoryEffectiveSettings,
) -> MemoryGenerationEligibility {
    let group_channel = payload
        .get("chat_type")
        .and_then(Value::as_str)
        .is_some_and(|value| matches!(value, "group" | "supergroup" | "channel"));
    let scheduled = task.kind == "scheduled" || payload.get("scheduled_job_id").is_some();
    let subagent = payload.get("parent_task_id").is_some()
        || payload.get("subagent_id").is_some()
        || payload.get("child_task_id").is_some();
    let has_attachments = payload
        .get("attachments")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty());
    let has_web = payload.get("web_context").is_some();
    let has_private_connector =
        payload.get("mcp_context").is_some() || payload.get("private_connector_context").is_some();
    let has_skill = task.kind == "run_skill" || payload.get("skill_name").is_some();
    let has_local_tool =
        payload.get("tool_results").is_some() || payload.get("local_tool_context").is_some();
    let has_workspace =
        payload.get("workspace_context").is_some() || payload.get("workspace_files").is_some();

    let mut items = vec![eligibility_item(
        MemorySourceCategory::UserAuthored,
        if group_channel {
            MemoryEligibilityDisposition::Excluded
        } else if scheduled || subagent {
            MemoryEligibilityDisposition::ParentVerifierRequired
        } else {
            MemoryEligibilityDisposition::Candidate
        },
        if group_channel {
            "group_third_party_content_excluded"
        } else if scheduled || subagent {
            "owner_verifier_required"
        } else {
            "authenticated_user_source"
        },
    )];
    items.push(eligibility_item(
        MemorySourceCategory::AssistantAuthored,
        MemoryEligibilityDisposition::EvidenceOnly,
        "assistant_output_never_self_promotes",
    ));
    items.push(eligibility_item(
        MemorySourceCategory::LocalTool,
        if has_local_tool {
            MemoryEligibilityDisposition::EvidenceOnly
        } else {
            MemoryEligibilityDisposition::Excluded
        },
        if has_local_tool {
            "verified_local_tool_evidence_only"
        } else {
            "source_absent"
        },
    ));
    items.push(eligibility_item(
        MemorySourceCategory::WorkspaceFile,
        if has_workspace {
            MemoryEligibilityDisposition::EvidenceOnly
        } else {
            MemoryEligibilityDisposition::Excluded
        },
        if has_workspace {
            "workspace_excerpt_evidence_only"
        } else {
            "source_absent"
        },
    ));
    for (present, category, reason) in [
        (
            has_attachments,
            MemorySourceCategory::AttachmentOcrStt,
            "attachment_body_not_durable",
        ),
        (
            has_web,
            MemorySourceCategory::Web,
            "external_raw_content_not_durable",
        ),
        (
            has_private_connector,
            MemorySourceCategory::McpPrivateConnector,
            "private_connector_raw_content_not_durable",
        ),
        (
            has_skill,
            MemorySourceCategory::Skill,
            "skill_raw_output_not_durable",
        ),
    ] {
        items.push(eligibility_item(
            category,
            if present {
                external_disposition(settings.external_context_policy)
            } else {
                MemoryEligibilityDisposition::Excluded
            },
            if present { reason } else { "source_absent" },
        ));
    }
    items.push(eligibility_item(
        MemorySourceCategory::Subagent,
        if subagent {
            MemoryEligibilityDisposition::ParentVerifierRequired
        } else {
            MemoryEligibilityDisposition::Excluded
        },
        if subagent {
            "subagent_parent_verifier_required"
        } else {
            "source_absent"
        },
    ));
    items.push(eligibility_item(
        MemorySourceCategory::ScheduledBackground,
        if scheduled {
            MemoryEligibilityDisposition::ParentVerifierRequired
        } else {
            MemoryEligibilityDisposition::Excluded
        },
        if scheduled {
            "scheduled_owner_verifier_required"
        } else {
            "source_absent"
        },
    ));
    items.push(eligibility_item(
        MemorySourceCategory::GroupChannel,
        MemoryEligibilityDisposition::Excluded,
        if group_channel {
            "group_third_party_content_excluded"
        } else {
            "source_absent"
        },
    ));
    let durable_candidate_allowed = settings.generate_memory
        && items.iter().any(|item| {
            item.category == MemorySourceCategory::UserAuthored
                && item.disposition == MemoryEligibilityDisposition::Candidate
        });
    let digest_value = json!({
        "schema_version": 1,
        "items": items,
        "external_context_policy": settings.external_context_policy,
        "settings_policy_digest": settings.policy_digest,
        "durable_candidate_allowed": durable_candidate_allowed,
    });
    let policy_digest = format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&digest_value).unwrap_or_default())
    );
    MemoryGenerationEligibility {
        schema_version: 1,
        items,
        external_context_policy: settings.external_context_policy,
        policy_digest,
        durable_candidate_allowed,
    }
}

fn external_disposition(policy: ExternalContextPolicy) -> MemoryEligibilityDisposition {
    match policy {
        ExternalContextPolicy::Allow => MemoryEligibilityDisposition::EvidenceOnly,
        ExternalContextPolicy::EvidenceOnly => MemoryEligibilityDisposition::EvidenceOnly,
        ExternalContextPolicy::Exclude | ExternalContextPolicy::Inherit => {
            MemoryEligibilityDisposition::Excluded
        }
    }
}

fn eligibility_item(
    category: MemorySourceCategory,
    disposition: MemoryEligibilityDisposition,
    reason_code: &str,
) -> MemoryGenerationEligibilityItem {
    MemoryGenerationEligibilityItem {
        category,
        disposition,
        reason_code: reason_code.to_string(),
    }
}

#[cfg(test)]
#[path = "eligibility_tests.rs"]
mod tests;
