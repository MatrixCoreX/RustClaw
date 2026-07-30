use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use skill_sdk::{AdmissionState, HostPolicyGrant};

pub(crate) const OVERLAY_GENERATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GenerationPointer {
    pub(crate) schema_version: u32,
    pub(crate) generation: u64,
    pub(crate) generation_digest: String,
    pub(crate) activated_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GenerationRecord {
    pub(crate) schema_version: u32,
    pub(crate) generation: u64,
    pub(crate) previous_generation: Option<u64>,
    pub(crate) created_at_unix: u64,
    pub(crate) skills: BTreeMap<String, OverlaySkillRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OverlaySkillRecord {
    pub(crate) source: SkillAdmissionSource,
    pub(crate) state: AdmissionState,
    pub(crate) manifest_digest: String,
    pub(crate) metadata_digest: String,
    pub(crate) prompt_digest: String,
    pub(crate) registry_fragment_digest: Option<String>,
    pub(crate) policy_digest: Option<String>,
    pub(crate) admission_receipt_digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SkillAdmissionSource {
    BundledBase,
    ExternalOverlay,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExternalSkillMetadata {
    pub(crate) name: String,
    pub(crate) source: SkillAdmissionSource,
    pub(crate) package_manifest_path: String,
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) aliases: Vec<String>,
    #[serde(default = "default_group")]
    pub(crate) group: String,
}

fn default_group() -> String {
    "extensions".to_string()
}

#[derive(Debug, Clone)]
pub(crate) struct AdmissionMutation {
    pub(crate) metadata: ExternalSkillMetadata,
    pub(crate) prompt: String,
    pub(crate) state: AdmissionState,
    pub(crate) grant: Option<HostPolicyGrant>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct OverlaySnapshot {
    pub(crate) generation: u64,
    pub(crate) generation_digest: Option<String>,
    pub(crate) base_registry_digest: Option<String>,
    pub(crate) registry_dir: Option<PathBuf>,
    pub(crate) execution_bindings: BTreeMap<String, AdmissionExecutionBinding>,
    pub(crate) sources: BTreeMap<String, SkillAdmissionSource>,
    pub(crate) enabled: BTreeSet<String>,
    pub(crate) disabled: BTreeSet<String>,
    pub(crate) awaiting_policy: BTreeSet<String>,
    pub(crate) tombstoned: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AdmissionExecutionBinding {
    pub(crate) version: String,
    pub(crate) manifest_digest: String,
    pub(crate) install_receipt_digest: String,
    pub(crate) policy_digest: Option<String>,
    pub(crate) admission_receipt_digest: String,
}

impl OverlaySnapshot {
    pub(crate) fn state(&self, skill_name: &str) -> Option<AdmissionState> {
        if self.enabled.contains(skill_name) {
            Some(AdmissionState::Enabled)
        } else if self.disabled.contains(skill_name) {
            Some(AdmissionState::InstalledDisabled)
        } else if self.awaiting_policy.contains(skill_name) {
            Some(AdmissionState::AwaitingPolicyApproval)
        } else if self.tombstoned.contains(skill_name) {
            Some(AdmissionState::Tombstoned)
        } else {
            None
        }
    }

    pub(crate) fn source(&self, skill_name: &str) -> Option<SkillAdmissionSource> {
        self.sources.get(skill_name).copied()
    }
}
